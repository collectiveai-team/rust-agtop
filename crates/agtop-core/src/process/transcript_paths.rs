//! Per-client knowledge: given a `SessionSummary`, which OS-level file
//! paths should a live agent process have open? And which binary names
//! does that client usually run under?
//!
//! Keeping this table next to the correlator (rather than in each client
//! parser) avoids teaching every client about OS processes.

use std::path::PathBuf;

use crate::session::{ClientKind, SessionSummary};

/// Return the set of file paths that a process running `session` is
/// expected to be holding open.
#[allow(dead_code)]
pub(crate) fn paths_for(session: &SessionSummary) -> Vec<PathBuf> {
    match session.client {
        // JSONL transcripts: the file itself is held open for writes.
        ClientKind::Claude
        | ClientKind::Codex
        | ClientKind::GeminiCli
        | ClientKind::Copilot
        | ClientKind::Cursor => vec![session.data_path.clone()],

        // SQLite-backed clients: the DB plus WAL+SHM are open while the
        // process is writing. WAL is the most reliable signal because
        // it's created the moment a write begins.
        ClientKind::OpenCode | ClientKind::Antigravity => {
            let base = session.data_path.clone();
            vec![
                base.clone(),
                append_suffix(&base, "-wal"),
                append_suffix(&base, "-shm"),
            ]
        }
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

fn append_suffix(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
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
    fn sqlite_clients_expect_db_wal_shm_triple() {
        let s = summary(ClientKind::OpenCode, "/tmp/storage.db");
        let paths = paths_for(&s);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/storage.db"),
                PathBuf::from("/tmp/storage.db-wal"),
                PathBuf::from("/tmp/storage.db-shm"),
            ]
        );
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
