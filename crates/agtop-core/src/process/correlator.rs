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
            continue;
        }

        // Score tier: find the best candidate via cwd + binary + time.
        let mut best: Option<(u32, u32, Option<u32>)> = None; // (score, pid, parent)
        let mut tie = false;
        for c in candidates {
            let score = score_candidate(c, session);
            if score < 2 {
                continue;
            }
            match best {
                None => best = Some((score, c.pid, c.parent_pid)),
                Some((s, _, _)) if score > s => {
                    best = Some((score, c.pid, c.parent_pid));
                    tie = false;
                }
                Some((s, _, _)) if score == s => {
                    tie = true;
                }
                _ => {}
            }
        }
        if let (Some((_, pid, parent_pid)), false) = (best, tie) {
            out.insert(
                session.session_id.clone(),
                ProcessInfo {
                    pid,
                    liveness: Liveness::Live,
                    match_confidence: Confidence::Medium,
                    parent_pid,
                },
            );
        }
    }

    out
}

/// Score a candidate process against a session. Each criterion adds 1.
/// Returns 0..=3.
fn score_candidate(c: &Candidate, s: &SessionSummary) -> u32 {
    let mut score = 0;

    // Binary matches an expected name for this client.
    if expected_binaries(s.client).iter().any(|&b| c.binary == b) {
        score += 1;
    }

    // cwd exact match.
    if let (Some(cc), Some(sc)) = (&c.cwd, &s.cwd) {
        if cc.as_os_str() == std::ffi::OsStr::new(sc) {
            score += 1;
        }
    }

    // Process start time falls inside the session's observed window.
    if let (Some(started), Some(last_active)) = (s.started_at, s.last_active) {
        let started = started.timestamp() as u64;
        let last_active = last_active.timestamp() as u64;
        if c.start_time >= started && c.start_time <= last_active {
            score += 1;
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::fd::tests::FakeFdScanner;
    use crate::process::scanner::tests::FakeScanner;
    use crate::session::ClientKind;
    use chrono::{Duration, Utc};
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
    fn no_fd_match_falls_back_to_score_tier() {
        // No fd match; binary + cwd match => score 2 => Medium confidence.
        let scanner = FakeScanner {
            processes: vec![candidate(42, "claude", "/home/user/proj")],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        let sessions = vec![claude_session("s1", "/tmp/s1.jsonl")];
        let result = correlate(&scanner, &fd, &sessions);
        let info = result.get("s1").expect("s1 must be matched by score tier");
        assert_eq!(info.pid, 42);
        assert_eq!(info.match_confidence, Confidence::Medium);
    }

    #[test]
    fn score_tier_matches_medium_confidence_when_unique() {
        // No fd info. Candidate cwd + binary + start_time all line up.
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::Claude,
            None,
            "s1".into(),
            Some(now - Duration::minutes(10)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/tmp/s1.jsonl"),
            None,
            None,
            None,
            None,
        );
        let mut c = candidate(42, "claude", "/home/user/proj");
        c.start_time = (now - Duration::minutes(5)).timestamp() as u64;

        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };

        let result = correlate(&scanner, &fd, &[s]);
        let info = result.get("s1").expect("s1 must be matched");
        assert_eq!(info.pid, 42);
        assert_eq!(info.match_confidence, Confidence::Medium);
    }

    #[test]
    fn score_tier_refuses_ambiguous_tie() {
        // Two candidates that score identically => no match.
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::Claude,
            None,
            "s1".into(),
            Some(now - Duration::minutes(10)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/tmp/s1.jsonl"),
            None,
            None,
            None,
            None,
        );
        let mut c1 = candidate(42, "claude", "/home/user/proj");
        let mut c2 = candidate(43, "claude", "/home/user/proj");
        c1.start_time = (now - Duration::minutes(5)).timestamp() as u64;
        c2.start_time = (now - Duration::minutes(3)).timestamp() as u64;

        let scanner = FakeScanner {
            processes: vec![c1, c2],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };

        let result = correlate(&scanner, &fd, &[s]);
        assert!(result.is_empty(), "ambiguous match should not be emitted");
    }

    #[test]
    fn score_tier_rejects_low_score() {
        // Only binary matches; cwd mismatch, no time overlap => score 1 => reject.
        let s = claude_session("s1", "/tmp/s1.jsonl");
        let c = candidate(42, "claude", "/elsewhere");
        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        let result = correlate(&scanner, &fd, &[s]);
        assert!(result.is_empty());
    }
}
