//! Per-client knowledge: given a `SessionSummary`, which OS-level file
//! paths should a live agent process have open? And which binary names
//! does that client usually run under?
//!
//! Keeping this table next to the correlator (rather than in each client
//! parser) avoids teaching every client about OS processes.

use std::path::PathBuf;

use crate::session::{ClientKind, SessionSummary};

/// Return the set of file paths that uniquely identify `session` and would
/// be expected open by a process running it.
///
/// Returning an empty vec disables the fd-tier match for that client; the
/// correlator falls back to argv-tier and cwd-tier matching.
///
/// SQLite-backed clients (OpenCode, Antigravity) intentionally return an
/// empty list: the DB file is shared by every session of that client, so
/// holding it open does NOT identify a single session — it only identifies
/// the daemon. Matching on it produces a false-positive that stamps the
/// same PID onto every session. Use argv-tier (`-s/--session <uuid>`) or
/// cwd+recency tier instead.
#[allow(dead_code)]
pub(crate) fn paths_for(session: &SessionSummary) -> Vec<PathBuf> {
    match session.client {
        // JSONL transcripts: the file itself is held open for writes.
        ClientKind::Claude
        | ClientKind::Codex
        | ClientKind::GeminiCli
        | ClientKind::Copilot
        | ClientKind::Cursor => vec![session.data_path.clone()],

        // SQLite-backed clients: shared DB file does not identify a
        // single session. See doc comment above.
        ClientKind::OpenCode | ClientKind::Antigravity => Vec::new(),
    }
}

/// Return the binary names we expect for `client`. Used to boost match
/// scores in the fallback tier.
#[allow(dead_code)]
pub(crate) fn expected_binaries(client: ClientKind) -> &'static [&'static str] {
    match client {
        ClientKind::Claude => &["claude"],
        ClientKind::Codex => &["codex"],
        ClientKind::GeminiCli => &["gemini", "node"],
        ClientKind::OpenCode => &["opencode"],
        ClientKind::Copilot => &["copilot", "gh-copilot"],
        ClientKind::Cursor => &["cursor", "cursor-agent"],
        ClientKind::Antigravity => &["antigravity"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn summary(client: ClientKind, data_path: &str) -> SessionSummary {
        SessionSummary::new(
            client,
            None,
            "id".into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            None,
            PathBuf::from(data_path),
            None,
            None,
            None,
        )
    }

    #[test]
    fn jsonl_clients_expect_the_transcript_path_itself() {
        for client in [
            ClientKind::Claude,
            ClientKind::Codex,
            ClientKind::GeminiCli,
            ClientKind::Copilot,
            ClientKind::Cursor,
        ] {
            let s = summary(client, "/tmp/session.jsonl");
            let paths = paths_for(&s);
            assert_eq!(
                paths,
                vec![PathBuf::from("/tmp/session.jsonl")],
                "{:?}",
                client
            );
        }
    }

    #[test]
    fn sqlite_clients_have_no_fd_paths() {
        // SQLite DBs are shared across sessions; holding them open does
        // NOT identify a single session. Returning empty disables the
        // fd-tier match for these clients (correlator falls back to
        // argv- and cwd-tier matching).
        for client in [ClientKind::OpenCode, ClientKind::Antigravity] {
            let s = summary(client, "/tmp/storage.db");
            assert!(
                paths_for(&s).is_empty(),
                "fd-tier must be disabled for {client:?}"
            );
        }
    }

    #[test]
    fn expected_binaries_has_entry_for_every_client_kind() {
        for &client in ClientKind::all() {
            assert!(
                !expected_binaries(client).is_empty(),
                "no expected binaries for {:?}",
                client
            );
        }
    }
}
