//! Matching algorithm: sessions -> running OS processes.
//!
//! Strategy:
//! 1. Fd-match tier (High confidence): build `open_path -> pid` index
//!    from candidate processes; any session whose transcript path is in
//!    the index matches unambiguously.
//! 2. Score tier (Medium confidence): for unmatched sessions, score
//!    candidates on binary + cwd + start-time and accept a unique
//!    high-score winner.

use std::collections::{HashMap, HashSet};

use crate::process::fd::FdScanner;
use crate::process::scanner::{Candidate, Scanner};
use crate::process::transcript_paths::{expected_binaries, paths_for};
use crate::process::{Confidence, Liveness, ProcessInfo};
use crate::session::SessionSummary;

/// Run one correlation pass.
///
/// Does not track prior snapshots; `Liveness` is always `Live` here.
/// Stopped-state emission lives on `ProcessCorrelator` (task 9).
#[allow(dead_code)]
pub(crate) fn correlate(
    scanner: &dyn Scanner,
    fd_scanner: &dyn FdScanner,
    sessions: &[SessionSummary],
) -> HashMap<String, ProcessInfo> {
    let candidates = scanner.candidates();
    let mut out = HashMap::new();

    // Build reverse index path -> pid, but only for paths any session
    // actually wants; this caps fd-reads at O(candidates) regardless of
    // how many files each candidate holds.
    let interesting: HashSet<std::path::PathBuf> = sessions.iter().flat_map(paths_for).collect();

    let mut path_to_pid: HashMap<std::path::PathBuf, &Candidate> = HashMap::new();
    for c in candidates {
        let paths = fd_scanner.open_paths(c.pid);
        for p in paths {
            if interesting.contains(&p) {
                path_to_pid.insert(p, c);
            }
        }
    }

    for session in sessions {
        // Fd-match tier
        let matched = paths_for(session)
            .into_iter()
            .find_map(|p| path_to_pid.get(&p).copied());
        if let Some(c) = matched {
            out.insert(
                session.session_id.clone(),
                ProcessInfo {
                    pid: c.pid,
                    liveness: Liveness::Live,
                    match_confidence: Confidence::High,
                    parent_pid: c.parent_pid,
                },
            );
        }
        // Score tier added in task 8.
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::fd::tests::FakeFdScanner;
    use crate::process::scanner::tests::FakeScanner;
    use crate::session::ClientKind;
    use chrono::Utc;
    use std::path::PathBuf;

    fn claude_session(id: &str, path: &str) -> SessionSummary {
        SessionSummary::new(
            ClientKind::Claude,
            None,
            id.into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from(path),
            None,
            None,
            None,
            None,
        )
    }

    fn candidate(pid: u32, binary: &str, cwd: &str) -> Candidate {
        Candidate {
            pid,
            parent_pid: Some(1),
            binary: binary.into(),
            argv: vec![binary.into()],
            cwd: Some(PathBuf::from(cwd)),
            start_time: 1700000000,
        }
    }

    #[test]
    fn fd_match_produces_high_confidence() {
        let scanner = FakeScanner {
            processes: vec![candidate(42, "claude", "/home/user/proj")],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(42u32, vec![PathBuf::from("/tmp/s1.jsonl")]);
        let fd = FakeFdScanner { map: fd_map };

        let sessions = vec![claude_session("s1", "/tmp/s1.jsonl")];
        let result = correlate(&scanner, &fd, &sessions);

        let info = result.get("s1").expect("s1 must be matched");
        assert_eq!(info.pid, 42);
        assert_eq!(info.liveness, Liveness::Live);
        assert_eq!(info.match_confidence, Confidence::High);
    }

    #[test]
    fn fd_match_disambiguates_two_claudes_in_same_cwd() {
        // Two claude processes in the same cwd. Only one holds s1.jsonl open.
        let scanner = FakeScanner {
            processes: vec![
                candidate(42, "claude", "/home/user/proj"),
                candidate(43, "claude", "/home/user/proj"),
            ],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(42u32, vec![PathBuf::from("/tmp/s1.jsonl")]);
        fd_map.insert(43u32, vec![PathBuf::from("/tmp/s2.jsonl")]);
        let fd = FakeFdScanner { map: fd_map };

        let sessions = vec![
            claude_session("s1", "/tmp/s1.jsonl"),
            claude_session("s2", "/tmp/s2.jsonl"),
        ];
        let result = correlate(&scanner, &fd, &sessions);

        assert_eq!(result.get("s1").map(|i| i.pid), Some(42));
        assert_eq!(result.get("s2").map(|i| i.pid), Some(43));
    }

    #[test]
    fn no_fd_match_yields_no_entry_yet() {
        // Score tier not wired yet; without fd match, no entry.
        let scanner = FakeScanner {
            processes: vec![candidate(42, "claude", "/home/user/proj")],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        let sessions = vec![claude_session("s1", "/tmp/s1.jsonl")];
        let result = correlate(&scanner, &fd, &sessions);
        assert!(result.is_empty());
    }
}
