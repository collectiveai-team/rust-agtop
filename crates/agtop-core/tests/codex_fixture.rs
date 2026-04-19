//! Integration test for Codex session parsing.
//!
//! Builds a minimal `.jsonl` fixture inline with `serde_json::json!` and
//! asserts that `CodexProvider::analyze` extracts the right token counts,
//! cost, tool-call count, and duration without touching the real filesystem.

use std::fs;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

use agtop_core::providers::codex::CodexProvider;
use agtop_core::{Plan, Provider, ProviderKind, SessionSummary};

// ---------------------------------------------------------------------------
// Minimal temp-dir helper (mirrors the one in the codex unit tests)
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
            "agtop-codex-fixture-{tag}-{}-{nanos}-{seq}",
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
// Fixture builder
// ---------------------------------------------------------------------------

/// Write a minimal Codex `.jsonl` fixture file.
///
/// The file contains:
/// - one `session_meta` record with timestamps, cwd, and session id
/// - one `turn_context` record identifying the model
/// - two `response_item` records with `type = "function_call"` (tool calls)
/// - two `event_msg / token_count` records, each contributing token deltas
///   so we can assert that both deltas are summed
fn write_codex_fixture(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("rollout-2026-04-17T10-00-00-aabbccddeeff.jsonl");

    // First token-count turn: input=100 cached_input=20 output=50
    let turn1 = serde_json::json!({
        "type": "event_msg",
        "timestamp": "2026-04-17T10:00:10Z",
        "payload": {
            "type": "token_count",
            "info": {
                "last_token_usage": {
                    "input_tokens": 100,
                    "cached_input_tokens": 20,
                    "output_tokens": 50,
                    "reasoning_output_tokens": 0
                }
            }
        }
    });

    // Second token-count turn: input=80 cached_input=10 output=30
    let turn2 = serde_json::json!({
        "type": "event_msg",
        "timestamp": "2026-04-17T10:01:00Z",
        "payload": {
            "type": "token_count",
            "info": {
                "last_token_usage": {
                    "input_tokens": 80,
                    "cached_input_tokens": 10,
                    "output_tokens": 30,
                    "reasoning_output_tokens": 5
                }
            }
        }
    });

    let session_meta = serde_json::json!({
        "type": "session_meta",
        "timestamp": "2026-04-17T10:00:00Z",
        "payload": {
            "session_id": "aabbccddeeff-fixture-session",
            "cwd": "/tmp/test-project",
            "model": "codex-mini-latest"
        }
    });

    let turn_context = serde_json::json!({
        "type": "turn_context",
        "payload": { "model": "codex-mini-latest" }
    });

    // Two function_call response_items → tool_call_count == 2
    let tool_call_1 = serde_json::json!({
        "type": "response_item",
        "timestamp": "2026-04-17T10:00:05Z",
        "payload": { "type": "function_call", "name": "bash" }
    });
    let tool_call_2 = serde_json::json!({
        "type": "response_item",
        "timestamp": "2026-04-17T10:00:55Z",
        "payload": { "type": "function_call", "name": "read_file" }
    });

    let mut content = String::new();
    for v in &[
        session_meta,
        turn_context,
        tool_call_1,
        turn1,
        tool_call_2,
        turn2,
    ] {
        content.push_str(&serde_json::to_string(v).unwrap());
        content.push('\n');
    }
    fs::write(&path, content).expect("write fixture");
    path
}

fn write_codex_metadata_fixture(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("rollout-2026-04-17T10-00-00-metadata.jsonl");
    let session_meta = serde_json::json!({
        "type": "session_meta",
        "timestamp": "2026-04-17T10:00:00Z",
        "payload": {
            "session_id": "metadata-session",
            "cwd": "/tmp/test-project",
            "model": "codex-mini-latest"
        }
    });
    let turn_context = serde_json::json!({
        "type": "turn_context",
        "payload": {
            "model": "codex-mini-latest",
            "effort": "high",
            "collaboration_mode": {
                "settings": { "reasoning_effort": "high" }
            }
        }
    });
    let waiting_call = serde_json::json!({
        "type": "response_item",
        "timestamp": "2026-04-17T10:00:05Z",
        "payload": { "type": "function_call", "name": "exec_command" }
    });

    let mut content = String::new();
    for v in &[session_meta, turn_context, waiting_call] {
        content.push_str(&serde_json::to_string(v).unwrap());
        content.push('\n');
    }
    fs::write(&path, content).expect("write metadata fixture");
    path
}

#[test]
fn codex_fixture_extracts_effort_and_waiting_state() {
    let tmp = TmpDir::new("metadata");
    let fixture_path = write_codex_metadata_fixture(tmp.0.as_path());
    let provider = CodexProvider {
        sessions_root: tmp.0.clone(),
        auth_path: tmp.0.join("auth.json"),
        discover_cache: Mutex::default(),
    };

    let sessions = provider.list_sessions().expect("list_sessions");
    let summary = sessions
        .into_iter()
        .find(|s| s.data_path == fixture_path)
        .expect("fixture summary");

    assert_eq!(summary.model_effort.as_deref(), Some("high"));
    assert_eq!(
        summary.model_effort_detail.as_deref(),
        Some("turn_context.effort")
    );
    assert_eq!(summary.state.as_deref(), Some("waiting"));
    assert_eq!(
        summary.state_detail.as_deref(),
        Some("response_item:function_call")
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn codex_fixture_token_counts_are_summed() {
    let tmp = TmpDir::new("tokens");
    let fixture_path = write_codex_fixture(tmp.0.as_path());

    let summary = SessionSummary::new(
        ProviderKind::Codex,
        None,
        "aabbccddeeff-fixture-session".into(),
        None,
        None,
        Some("codex-mini-latest".into()),
        Some("/tmp/test-project".into()),
        fixture_path,
        None,
        None,
        None,
        None,
    );

    let provider = CodexProvider {
        sessions_root: tmp.0.clone(),
        auth_path: tmp.0.join("auth.json"), // won't be read for analyze()
        discover_cache: Mutex::default(),
    };

    let analysis = provider
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    let t = &analysis.tokens;
    // turn1: input=100 cached_input=20 output=50 reasoning=0
    // turn2: input=80  cached_input=10 output=30 reasoning=5
    assert_eq!(t.input, 180, "summed input tokens");
    assert_eq!(t.cached_input, 30, "summed cached_input tokens");
    assert_eq!(t.output, 80, "summed output tokens");
    assert_eq!(t.reasoning_output, 5, "summed reasoning tokens");
}

#[test]
fn codex_fixture_tool_call_count() {
    let tmp = TmpDir::new("tool-calls");
    let fixture_path = write_codex_fixture(tmp.0.as_path());

    let summary = SessionSummary::new(
        ProviderKind::Codex,
        None,
        "aabbccddeeff-fixture-session".into(),
        None,
        None,
        Some("codex-mini-latest".into()),
        Some("/tmp/test-project".into()),
        fixture_path,
        None,
        None,
        None,
        None,
    );

    let provider = CodexProvider {
        sessions_root: tmp.0.clone(),
        auth_path: tmp.0.join("auth.json"),
        discover_cache: Mutex::default(),
    };

    let analysis = provider
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    assert_eq!(
        analysis.tool_call_count,
        Some(2),
        "two function_call items → tool_call_count = 2"
    );
}

#[test]
fn codex_fixture_duration_from_timestamps() {
    let tmp = TmpDir::new("duration");
    let fixture_path = write_codex_fixture(tmp.0.as_path());

    let summary = SessionSummary::new(
        ProviderKind::Codex,
        None,
        "aabbccddeeff-fixture-session".into(),
        None,
        None,
        Some("codex-mini-latest".into()),
        Some("/tmp/test-project".into()),
        fixture_path,
        None,
        None,
        None,
        None,
    );

    let provider = CodexProvider {
        sessions_root: tmp.0.clone(),
        auth_path: tmp.0.join("auth.json"),
        discover_cache: Mutex::default(),
    };

    let analysis = provider
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    // Earliest timestamp in file: session_meta at 10:00:00Z
    // Latest timestamp in file:   second event_msg at 10:01:00Z → 60 seconds
    let dur = analysis
        .duration_secs
        .expect("duration_secs should be Some");
    assert_eq!(dur, 60, "duration from first to last timestamp");
}

#[test]
fn codex_fixture_retail_cost_is_positive() {
    let tmp = TmpDir::new("cost");
    let fixture_path = write_codex_fixture(tmp.0.as_path());

    let summary = SessionSummary::new(
        ProviderKind::Codex,
        None,
        "aabbccddeeff-fixture-session".into(),
        None,
        None,
        Some("codex-mini-latest".into()),
        Some("/tmp/test-project".into()),
        fixture_path,
        None,
        None,
        None,
        None,
    );

    let provider = CodexProvider {
        sessions_root: tmp.0.clone(),
        auth_path: tmp.0.join("auth.json"),
        discover_cache: Mutex::default(),
    };

    let analysis = provider
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
