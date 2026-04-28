//! Integration tests for OpenCode session parsing — SQLite path and JSON path.
//!
//! - `opencode_sqlite_*`: use an in-memory (file-backed temp) SQLite DB.
//! - `opencode_json_*`: use hand-rolled JSON message files in a temp tree.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

use agtop_core::clients::opencode::OpenCodeClient;
use agtop_core::session::ParserState;
use agtop_core::{Client, ClientKind, Plan, SessionSummary};

// ---------------------------------------------------------------------------
// Temp-dir helper
// ---------------------------------------------------------------------------

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "agtop-opencode-fixture-{tag}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create tmp dir");
        Self(path)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Shared fixture data
// ---------------------------------------------------------------------------

const SESSION_ID: &str = "ses_fixture01";
const MODEL_ID: &str = "anthropic/claude-sonnet-4-5";

/// Build a `SessionSummary` pointing at a given data path.
fn make_summary(data_path: PathBuf) -> SessionSummary {
    use chrono::{TimeZone, Utc};
    let started = Utc.with_ymd_and_hms(2026, 4, 17, 10, 0, 0).unwrap();
    let ended = Utc.with_ymd_and_hms(2026, 4, 17, 10, 5, 0).unwrap(); // 5 minutes
    SessionSummary::new(
        ClientKind::OpenCode,
        None,
        SESSION_ID.into(),
        Some(started),
        Some(ended),
        Some(MODEL_ID.into()),
        Some("/tmp/opencode-test".into()),
        data_path,
        None,
        None,
        None,
    )
}

/// Return two assistant-message JSON values that together contribute:
///   turn1: input=200 output=80 cache.read=40 cache.write=10
///   turn2: input=150 output=60 cache.read=30 cache.write=5
///            + finish=tool-calls (both) → tool_call_count == 2
fn assistant_turns() -> [serde_json::Value; 2] {
    [
        serde_json::json!({
            "role": "assistant",
            "modelID": MODEL_ID,
            "finish": "tool-calls",
            "cost": 0.015,
            "tokens": {
                "input": 200,
                "output": 80,
                "reasoning": 0,
                "cache": { "read": 40, "write": 10 }
            }
        }),
        serde_json::json!({
            "role": "assistant",
            "modelID": MODEL_ID,
            "finish": "tool-calls",
            "cost": 0.010,
            "tokens": {
                "input": 150,
                "output": 60,
                "reasoning": 5,
                "cache": { "read": 30, "write": 5 }
            }
        }),
    ]
}

// Expected totals:
// input         = 200 + 150     = 350
// output        = 80  + 60      = 140
// reasoning     = 0   + 5       = 5
// cache_read    = 40  + 30      = 70  (also becomes cached_input)
// cache_write_5m= 10  + 5       = 15
// tool_calls    = 2
// cost_reported = 0.015 + 0.010 = 0.025  (used as fallback if model unknown)

// ---------------------------------------------------------------------------
// SQLite fixture helpers
// ---------------------------------------------------------------------------

fn init_db(root: &Path) -> PathBuf {
    let db_path = root.join("opencode.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open db");
    conn.execute(
        "CREATE TABLE session (
             id            TEXT PRIMARY KEY,
             parent_id     TEXT,
             directory     TEXT,
             time_created  INTEGER,
             time_updated  INTEGER,
             time_archived INTEGER
         )",
        [],
    )
    .expect("create session table");
    conn.execute(
        "CREATE TABLE message (
             id           TEXT PRIMARY KEY,
             session_id   TEXT NOT NULL,
             time_created INTEGER,
             data         TEXT NOT NULL
         )",
        [],
    )
    .expect("create message table");
    db_path
}

fn insert_assistant_msg(root: &Path, msg_id: &str, session_id: &str, data: &serde_json::Value) {
    let conn = rusqlite::Connection::open(root.join("opencode.db")).expect("open db");
    conn.execute(
        "INSERT INTO message(id, session_id, time_created, data) VALUES(?1, ?2, ?3, ?4)",
        rusqlite::params![
            msg_id,
            session_id,
            1_744_880_400i64, // arbitrary ms timestamp
            serde_json::to_string(data).unwrap()
        ],
    )
    .expect("insert message");
}

fn insert_session(
    root: &Path,
    session_id: &str,
    parent_id: Option<&str>,
    directory: &str,
    time_created_ms: i64,
    time_updated_ms: i64,
) {
    let conn = rusqlite::Connection::open(root.join("opencode.db")).expect("open db");
    conn.execute(
        "INSERT INTO session(id, parent_id, directory, time_created, time_updated, time_archived)
         VALUES(?1, ?2, ?3, ?4, ?5, NULL)",
        rusqlite::params![
            session_id,
            parent_id,
            directory,
            time_created_ms,
            time_updated_ms,
        ],
    )
    .expect("insert session");
}

// ---------------------------------------------------------------------------
// SQLite path tests
// ---------------------------------------------------------------------------

#[test]
fn opencode_sqlite_token_counts_are_summed() {
    let tmp = TmpDir::new("sqlite-tokens");
    init_db(&tmp.0);
    for (i, turn) in assistant_turns().iter().enumerate() {
        insert_assistant_msg(&tmp.0, &format!("msg_{i}"), SESSION_ID, turn);
    }

    let client = OpenCodeClient {
        storage_root: tmp.0.clone(),
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("opencode.db"));
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    let t = &analysis.tokens;
    assert_eq!(t.input, 350, "input");
    assert_eq!(t.output, 140, "output");
    assert_eq!(t.reasoning_output, 5, "reasoning");
    assert_eq!(t.cache_read, 70, "cache_read");
    assert_eq!(t.cache_write_5m, 15, "cache_write_5m");
    assert_eq!(
        t.cached_input, t.cache_read,
        "cached_input mirrors cache_read"
    );
}

#[test]
fn opencode_sqlite_tool_call_count() {
    let tmp = TmpDir::new("sqlite-tool-calls");
    init_db(&tmp.0);
    for (i, turn) in assistant_turns().iter().enumerate() {
        insert_assistant_msg(&tmp.0, &format!("msg_{i}"), SESSION_ID, turn);
    }

    let client = OpenCodeClient {
        storage_root: tmp.0.clone(),
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("opencode.db"));
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    assert_eq!(
        analysis.tool_call_count,
        Some(2),
        "both turns have finish=tool-calls"
    );
}

#[test]
fn opencode_sqlite_duration_from_summary() {
    let tmp = TmpDir::new("sqlite-duration");
    init_db(&tmp.0);
    for (i, turn) in assistant_turns().iter().enumerate() {
        insert_assistant_msg(&tmp.0, &format!("msg_{i}"), SESSION_ID, turn);
    }

    let client = OpenCodeClient {
        storage_root: tmp.0.clone(),
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("opencode.db"));
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    // summary has started_at=10:00:00Z, last_active=10:05:00Z → 300 s
    assert_eq!(
        analysis.duration_secs,
        Some(300),
        "duration derived from summary timestamps"
    );
}

#[test]
fn opencode_sqlite_retail_cost_is_positive() {
    let tmp = TmpDir::new("sqlite-cost");
    init_db(&tmp.0);
    for (i, turn) in assistant_turns().iter().enumerate() {
        insert_assistant_msg(&tmp.0, &format!("msg_{i}"), SESSION_ID, turn);
    }

    let client = OpenCodeClient {
        storage_root: tmp.0.clone(),
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("opencode.db"));
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    assert!(
        !analysis.cost.included,
        "retail plan should not be included"
    );
    assert!(
        analysis.cost.total > 0.0,
        "retail cost should be positive (got {})",
        analysis.cost.total
    );
}

#[test]
fn opencode_sqlite_children_follow_parent_id() {
    let tmp = TmpDir::new("sqlite-children");
    init_db(&tmp.0);
    insert_session(
        &tmp.0,
        SESSION_ID,
        None,
        "/tmp/opencode-parent",
        1_744_880_100_000,
        1_744_880_200_000,
    );
    insert_session(
        &tmp.0,
        "ses_child_b",
        Some(SESSION_ID),
        "/tmp/opencode-child-b",
        1_744_880_300_000,
        1_744_880_500_000,
    );
    insert_session(
        &tmp.0,
        "ses_child_a",
        Some(SESSION_ID),
        "/tmp/opencode-child-a",
        1_744_880_250_000,
        1_744_880_400_000,
    );
    insert_assistant_msg(
        &tmp.0,
        "msg_child_b",
        "ses_child_b",
        &serde_json::json!({
            "role": "assistant",
            "modelID": MODEL_ID,
            "providerID": "anthropic",
            "finish": "stop"
        }),
    );
    insert_assistant_msg(
        &tmp.0,
        "msg_child_a",
        "ses_child_a",
        &serde_json::json!({
            "role": "assistant",
            "modelID": MODEL_ID,
            "providerID": "anthropic",
            "finish": "tool-calls"
        }),
    );

    let client = OpenCodeClient {
        storage_root: tmp.0.clone(),
        discover_cache: Mutex::default(),
    };
    let parent = make_summary(tmp.0.join("opencode.db"));

    let children = client.children(&parent).expect("children should succeed");

    assert_eq!(children.len(), 2, "two child sessions should be returned");
    assert_eq!(children[0].session_id, "ses_child_b");
    assert_eq!(children[1].session_id, "ses_child_a");
    assert_eq!(children[0].cwd.as_deref(), Some("/tmp/opencode-child-b"));
    assert_eq!(children[0].model.as_deref(), Some(MODEL_ID));
    assert_eq!(children[0].parser_state, ParserState::Idle);
    assert_eq!(children[1].parser_state, ParserState::Running);
}

// ---------------------------------------------------------------------------
// JSON (legacy) path tests
// ---------------------------------------------------------------------------

/// Write the two assistant turns as individual `.json` files in the
/// `storage/message/<session_id>/` directory expected by the JSON path.
fn setup_json_fixture(root: &Path) -> PathBuf {
    let msg_dir = root.join("storage").join("message").join(SESSION_ID);
    fs::create_dir_all(&msg_dir).expect("create msg_dir");
    for (i, turn) in assistant_turns().iter().enumerate() {
        let p = msg_dir.join(format!("msg_{i:04}.json"));
        fs::write(&p, serde_json::to_string(turn).unwrap()).expect("write msg file");
    }
    // Also add a non-assistant message that must be ignored.
    let user_msg = serde_json::json!({ "role": "user", "content": "hello" });
    fs::write(
        msg_dir.join("user_0000.json"),
        serde_json::to_string(&user_msg).unwrap(),
    )
    .expect("write user msg");
    root.to_path_buf()
}

#[test]
fn opencode_json_token_counts_are_summed() {
    let tmp = TmpDir::new("json-tokens");
    let storage_root = setup_json_fixture(&tmp.0);

    let client = OpenCodeClient {
        storage_root,
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("session.json")); // data_path unused for JSON path
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    let t = &analysis.tokens;
    assert_eq!(t.input, 350, "input");
    assert_eq!(t.output, 140, "output");
    assert_eq!(t.reasoning_output, 5, "reasoning");
    assert_eq!(t.cache_read, 70, "cache_read");
    assert_eq!(t.cache_write_5m, 15, "cache_write_5m");
    assert_eq!(
        t.cached_input, t.cache_read,
        "cached_input mirrors cache_read"
    );
}

#[test]
fn opencode_json_tool_call_count() {
    let tmp = TmpDir::new("json-tool-calls");
    let storage_root = setup_json_fixture(&tmp.0);

    let client = OpenCodeClient {
        storage_root,
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("session.json"));
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    assert_eq!(
        analysis.tool_call_count,
        Some(2),
        "both turns have finish=tool-calls"
    );
}

#[test]
fn opencode_json_user_messages_are_ignored() {
    // The fixture includes a user-role message; verify token totals are
    // unchanged (i.e. the user message was skipped).
    let tmp = TmpDir::new("json-role-filter");
    let storage_root = setup_json_fixture(&tmp.0);

    let client = OpenCodeClient {
        storage_root,
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("session.json"));
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    // If the user message were counted, input would be > 350.
    assert_eq!(analysis.tokens.input, 350, "user messages not counted");
}

#[test]
fn opencode_json_retail_cost_is_positive() {
    let tmp = TmpDir::new("json-cost");
    let storage_root = setup_json_fixture(&tmp.0);

    let client = OpenCodeClient {
        storage_root,
        discover_cache: Mutex::default(),
    };
    let summary = make_summary(tmp.0.join("session.json"));
    let analysis = client
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    assert!(
        !analysis.cost.included,
        "retail plan should not be included"
    );
    assert!(
        analysis.cost.total > 0.0,
        "retail cost should be positive (got {})",
        analysis.cost.total
    );
}
