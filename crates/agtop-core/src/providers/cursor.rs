//! Cursor client — agent transcript JSONL + SQLite state DB.
//!
//! Agent transcripts live at:
//!   `~/.cursor/projects/<workspace>/agent-transcripts/<uuid>/<uuid>.jsonl`
//!
//! Subscription type is read from VS Code-style SQLite state DB:
//!   `~/.config/Cursor/User/globalStorage/state.vscdb`
//!   key: `cursorAuth/stripeMembershipType`
//!
//! Token counts are NOT available locally. The gRPC-Web API at
//! `api2.cursor.sh` would be needed for per-session token counts.
//! For now we return zero tokens (same as Copilot) but populate
//! session metadata fully from the transcripts.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::client::Client;
use crate::error::Result;
use crate::pricing::Plan;
use crate::providers::util::{dir_exists, for_each_jsonl, mtime, parse_ts, DiscoverCache};
use crate::session::{ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals};

#[derive(Debug)]
pub struct CursorClient {
    pub projects_root: PathBuf,
    pub state_db: PathBuf,
    pub discover_cache: Mutex<DiscoverCache>,
}

impl Default for CursorClient {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

        #[cfg(target_os = "macos")]
        let code_user = home
            .join("Library")
            .join("Application Support")
            .join("Cursor")
            .join("User");
        #[cfg(not(target_os = "macos"))]
        let code_user = home.join(".config").join("Cursor").join("User");

        Self {
            projects_root: home.join(".cursor").join("projects"),
            state_db: code_user.join("globalStorage").join("state.vscdb"),
            discover_cache: Mutex::default(),
        }
    }
}

impl Client for CursorClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Cursor
    }

    fn display_name(&self) -> &'static str {
        "Cursor"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !dir_exists(&self.projects_root) {
            return Ok(vec![]);
        }

        let subscription = read_subscription_from_db(&self.state_db);

        let mut out = Vec::new();

        let workspace_dirs = match fs::read_dir(&self.projects_root) {
            Ok(d) => d,
            Err(_) => return Ok(out),
        };

        for ws_entry in workspace_dirs.flatten() {
            let ws_path = ws_entry.path();
            if !ws_path.is_dir() {
                continue;
            }
            let workspace_name = ws_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string);

            let transcripts_dir = ws_path.join("agent-transcripts");
            if !transcripts_dir.is_dir() {
                continue;
            }

            let composer_dirs = match fs::read_dir(&transcripts_dir) {
                Ok(d) => d,
                Err(_) => continue,
            };

            for composer_entry in composer_dirs.flatten() {
                let composer_path = composer_entry.path();
                if !composer_path.is_dir() {
                    continue;
                }
                let composer_id = match composer_path.file_name().and_then(|n| n.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let transcript_file = composer_path.join(format!("{}.jsonl", composer_id));
                if !transcript_file.exists() {
                    continue;
                }

                let cid = composer_id.clone();
                let wn = workspace_name.clone();
                let sub = subscription.clone();
                let tf = transcript_file.clone();
                let cached = {
                    let mut guard = self.discover_cache.lock().unwrap();
                    guard.get_or_insert_with(&tf, || parse_cursor_transcript(&tf, cid, wn, sub))
                };
                match cached {
                    Ok(s) => out.push(s),
                    Err(e) => {
                        tracing::debug!(
                            path = %transcript_file.display(),
                            error = %e,
                            "skip cursor transcript"
                        );
                    }
                }
            }
        }

        {
            use std::collections::HashSet;
            let live_paths: HashSet<&std::path::Path> =
                out.iter().map(|s| s.data_path.as_path()).collect();
            self.discover_cache
                .lock()
                .unwrap()
                .retain_paths(&live_paths);
        }

        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, _plan: Plan) -> Result<SessionAnalysis> {
        // Parse transcript for tool call count and duration; tokens remain zero.
        let path = &summary.data_path;
        let (tool_call_count, duration_secs) = parse_transcript_metadata(path);

        Ok(SessionAnalysis::new(
            summary.clone(),
            TokenTotals::default(),
            CostBreakdown::default(),
            summary.model.clone(),
            0,
            tool_call_count,
            duration_secs,
            None,
            None,
            None,
        ))
    }
}

fn parse_cursor_transcript(
    path: &std::path::Path,
    session_id: String,
    cwd: Option<String>,
    subscription: Option<String>,
) -> Result<SessionSummary> {
    let mut model: Option<String> = None;
    let mut earliest: Option<DateTime<Utc>> = None;
    let mut latest: Option<DateTime<Utc>> = None;
    let mut seen = 0usize;

    for_each_jsonl(path, |v| {
        seen += 1;
        if seen > 50 {
            return;
        }

        if model.is_none() {
            if let Some(m) = v.get("modelId").and_then(|x| x.as_str()) {
                if !m.is_empty() {
                    model = Some(m.to_string());
                }
            }
        }

        if let Some(ts) = v
            .get("createdAt")
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
        {
            earliest = Some(match earliest {
                Some(cur) if cur <= ts => cur,
                _ => ts,
            });
            latest = Some(match latest {
                Some(cur) if cur >= ts => cur,
                _ => ts,
            });
        }
    })?;

    let last_active = latest.or_else(|| mtime(path));

    Ok(SessionSummary::new(
        ClientKind::Cursor,
        subscription,
        session_id,
        earliest,
        last_active,
        model,
        cwd,
        path.to_path_buf(),
        None,
        None,
        None,
        None,
    ))
}

fn parse_transcript_metadata(path: &std::path::Path) -> (Option<u64>, Option<u64>) {
    let mut tool_calls: u64 = 0;
    let mut earliest: Option<DateTime<Utc>> = None;
    let mut latest: Option<DateTime<Utc>> = None;

    let _ = for_each_jsonl(path, |v| {
        // Count tool_use content blocks.
        if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
            for part in content {
                if part.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    tool_calls += 1;
                }
            }
        }

        if let Some(ts) = v
            .get("createdAt")
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
        {
            earliest = Some(match earliest {
                Some(cur) if cur <= ts => cur,
                _ => ts,
            });
            latest = Some(match latest {
                Some(cur) if cur >= ts => cur,
                _ => ts,
            });
        }
    });

    let tool_call_count = if tool_calls > 0 {
        Some(tool_calls)
    } else {
        None
    };
    let duration_secs = earliest.zip(latest).and_then(|(s, e)| {
        if e >= s {
            Some((e - s).num_seconds() as u64)
        } else {
            None
        }
    });
    (tool_call_count, duration_secs)
}

/// Read `cursorAuth/stripeMembershipType` from `state.vscdb`.
fn read_subscription_from_db(db_path: &std::path::Path) -> Option<String> {
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;

    let raw: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'cursorAuth/stripeMembershipType'",
            [],
            |row| row.get(0),
        )
        .ok()?;

    // The value may be a bare string or a JSON-quoted string.
    let cleaned = raw.trim().trim_matches('"').to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(normalize_cursor_plan(&cleaned))
    }
}

fn normalize_cursor_plan(raw: &str) -> String {
    match raw.to_ascii_uppercase().as_str() {
        "FREE" => "Free".to_string(),
        "FREE_TRIAL" => "Free Trial".to_string(),
        "PRO" => "Pro".to_string(),
        "PRO_PLUS" => "Pro+".to_string(),
        "ULTRA" => "Ultra".to_string(),
        "ENTERPRISE" => "Enterprise".to_string(),
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::sync::Mutex;

    struct TestDir {
        path: std::path::PathBuf,
    }
    impl TestDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("agtop-cursor-{}-{}", name, std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn missing_dir_returns_empty() {
        let p = CursorClient {
            projects_root: std::path::PathBuf::from("/no/such/path"),
            state_db: std::path::PathBuf::from("/no/such/state.vscdb"),
            discover_cache: Mutex::default(),
        };
        assert!(p.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn parses_agent_transcript_jsonl() {
        let td = TestDir::new("transcript");
        let composer_id = "02742fb3-d98e-4fa2-8184-2fddd7ee544d";
        let transcript_dir = td
            .path
            .join("myworkspace")
            .join("agent-transcripts")
            .join(composer_id);
        fs::create_dir_all(&transcript_dir).unwrap();

        let transcript_file = transcript_dir.join(format!("{}.jsonl", composer_id));
        let line1 = r#"{"role":"user","message":{"content":[{"type":"text","text":"hello"}]},"createdAt":"2026-04-10T10:00:00Z","modelId":"gpt-4.1"}"#;
        let line2 = r#"{"role":"assistant","message":{"content":[{"type":"text","text":"hi"}]},"createdAt":"2026-04-10T10:01:00Z","modelId":"gpt-4.1"}"#;
        let mut f = fs::File::create(&transcript_file).unwrap();
        writeln!(f, "{}", line1).unwrap();
        writeln!(f, "{}", line2).unwrap();

        let p = CursorClient {
            projects_root: td.path.clone(),
            state_db: std::path::PathBuf::from("/no/such/state.vscdb"),
            discover_cache: Mutex::default(),
        };
        let sessions = p.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, composer_id);
        assert_eq!(sessions[0].client, ClientKind::Cursor);
        assert_eq!(sessions[0].model.as_deref(), Some("gpt-4.1"));
        assert_eq!(sessions[0].cwd.as_deref(), Some("myworkspace"));
    }

    #[test]
    fn normalize_plan_names() {
        assert_eq!(normalize_cursor_plan("PRO"), "Pro");
        assert_eq!(normalize_cursor_plan("PRO_PLUS"), "Pro+");
        assert_eq!(normalize_cursor_plan("ULTRA"), "Ultra");
        assert_eq!(normalize_cursor_plan("FREE"), "Free");
        assert_eq!(normalize_cursor_plan("FREE_TRIAL"), "Free Trial");
        assert_eq!(normalize_cursor_plan("ENTERPRISE"), "Enterprise");
        assert_eq!(normalize_cursor_plan("unknown_tier"), "unknown_tier");
    }
}
