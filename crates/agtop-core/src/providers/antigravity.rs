//! Antigravity provider — VSCode-fork IDE ("Jetski" agent).
//!
//! Antigravity stores session state in a SQLite `.vscdb` file with
//! protobuf-encoded blobs. Without the proto schema we cannot decode
//! structured fields; instead we extract UUID-shaped and ISO-timestamp-
//! shaped strings via regex-free byte scanning, which gives us session IDs
//! and approximate timestamps.
//!
//! Token counts are deferred (all zero) until the proto schema is known.

use std::path::PathBuf;

use crate::error::Result;
use crate::pricing::Plan;
use crate::provider::Provider;
use crate::providers::util::mtime;
use crate::session::{CostBreakdown, ProviderKind, SessionAnalysis, SessionSummary, TokenTotals};

/// SQLite key that holds session trajectory summaries (protobuf blob).
const TRAJECTORY_KEY: &str = "antigravityUnifiedStateSync.trajectorySummaries";
/// SQLite key for subscription/user status (protobuf blob).
const USER_STATUS_KEY: &str = "antigravityUnifiedStateSync.userStatus";

#[derive(Debug, Clone)]
pub struct AntigravityProvider {
    pub state_db: PathBuf,
}

impl Default for AntigravityProvider {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            state_db: home
                .join(".config")
                .join("Antigravity")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
        }
    }
}

impl Provider for AntigravityProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Antigravity
    }

    fn display_name(&self) -> &'static str {
        "Antigravity"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !self.state_db.exists() {
            return Ok(vec![]);
        }

        let conn = match rusqlite::Connection::open_with_flags(
            &self.state_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(_) => return Ok(vec![]),
        };

        // Read the trajectory blob.
        let blob: Vec<u8> = match conn.query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            rusqlite::params![TRAJECTORY_KEY],
            |row| row.get(0),
        ) {
            Ok(b) => b,
            Err(_) => return Ok(vec![]),
        };

        // Read subscription hint from userStatus blob (best-effort string scan).
        let sub_blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = ?1",
                rusqlite::params![USER_STATUS_KEY],
                |row| row.get(0),
            )
            .ok();
        let subscription = sub_blob
            .as_deref()
            .and_then(|b| extract_subscription_hint(b));

        // Extract UUID-like strings from the protobuf blob.
        let ids = extract_uuids(&blob);
        let db_mtime = mtime(&self.state_db);

        let sessions = ids
            .into_iter()
            .map(|id| {
                SessionSummary::new(
                    ProviderKind::Antigravity,
                    subscription.clone(),
                    id,
                    None,     // started_at — not decodable without schema
                    db_mtime, // last_active — use DB mtime as best proxy
                    None,     // model
                    None,     // cwd
                    self.state_db.clone(),
                    None,
                    None,
                    None,
                    None,
                )
            })
            .collect();

        Ok(sessions)
    }

    fn analyze(&self, summary: &SessionSummary, _plan: Plan) -> Result<SessionAnalysis> {
        // Token counts unavailable without proto schema — return zero-cost analysis.
        let tokens = TokenTotals::default();
        let cost = CostBreakdown::default();
        Ok(SessionAnalysis::new(
            summary.clone(),
            tokens,
            cost,
            None, // effective_model
            0,    // subagent_file_count
            None, // tool_call_count
            None, // duration_secs
            None, // context_used_pct
            None, // context_used_tokens
            None, // context_window
        ))
    }
}

/// Extract UUID-shaped strings (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`)
/// from a raw byte slice by scanning for the pattern without regex.
/// Returns deduplicated results preserving first-seen order.
fn extract_uuids(data: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i + 36 <= data.len() {
        if looks_like_uuid_at(data, i) {
            let s = std::str::from_utf8(&data[i..i + 36])
                .unwrap_or("")
                .to_string();
            if !out.contains(&s) {
                out.push(s);
            }
            i += 36;
        } else {
            i += 1;
        }
    }
    out
}

/// Returns true if bytes at `pos` form a UUID (8-4-4-4-12 hex with dashes).
fn looks_like_uuid_at(data: &[u8], pos: usize) -> bool {
    let pattern = [8, 4, 4, 4, 12usize];
    let dashes = [8, 13, 18, 23usize];
    for &d in &dashes {
        if pos + d >= data.len() || data[pos + d] != b'-' {
            return false;
        }
    }
    let mut idx = pos;
    for (group, &len) in pattern.iter().enumerate() {
        for _ in 0..len {
            if idx >= data.len() || !data[idx].is_ascii_hexdigit() {
                return false;
            }
            idx += 1;
        }
        if group < 4 {
            idx += 1; // skip dash
        }
    }
    true
}

/// Scan a blob for a known subscription plan string hint.
/// Antigravity reuses VSCode-fork patterns; look for recognizable plan names.
fn extract_subscription_hint(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    for keyword in &["pro", "enterprise", "free", "trial", "business"] {
        if text.to_ascii_lowercase().contains(keyword) {
            let capitalized = {
                let mut chars = keyword.chars();
                match chars.next() {
                    Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            };
            return Some(capitalized);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_dir_returns_empty() {
        let p = AntigravityProvider {
            state_db: std::path::PathBuf::from("/tmp/does-not-exist-agtop-test/state.vscdb"),
        };
        assert!(p.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn uuid_extraction_finds_valid_uuids() {
        let input = b"junk\x00\x0102742fb3-d98e-4fa2-8184-2fddd7ee544d\x00more junk";
        let ids = extract_uuids(input);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "02742fb3-d98e-4fa2-8184-2fddd7ee544d");
    }

    #[test]
    fn uuid_extraction_deduplicates() {
        let uuid = b"02742fb3-d98e-4fa2-8184-2fddd7ee544d";
        let mut input = uuid.to_vec();
        input.extend_from_slice(b"middle");
        input.extend_from_slice(uuid);
        let ids = extract_uuids(&input);
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn subscription_hint_extracts_pro() {
        let data = b"\x08\x01pro_plan_active\x00";
        assert_eq!(extract_subscription_hint(data), Some("Pro".to_string()));
    }
}
