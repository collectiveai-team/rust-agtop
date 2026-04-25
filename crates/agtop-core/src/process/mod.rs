//! Session <-> OS process correlation.
//!
//! See `docs/specs/2026-04-24-session-pid-tracking-design.md`.
//!
//! Public entry point is [`ProcessCorrelator`]. Construct one once, call
//! [`ProcessCorrelator::snapshot`] per refresh with the sessions you
//! want correlated, attach the returned [`ProcessInfo`] to each
//! `SessionAnalysis` before rendering.
//!
//! This module must never panic or return `Err` from `snapshot` — it's
//! a best-effort observability feature layered on top of the core
//! session display. All errors are logged at `debug!` and degraded away.

pub(crate) mod correlator;
pub(crate) mod fd;
pub(crate) mod scanner;
pub(crate) mod transcript_paths;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::process::correlator::correlate;
use crate::process::fd::{default_fd_scanner, FdScanner};
use crate::process::scanner::{Scanner, SysinfoScanner};
use crate::session::SessionSummary;

/// How certain we are about a given PID-to-session match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// The matched process was observed holding the session's transcript file open.
    High,
    /// Matched on cwd + binary + start-time overlap, unambiguously.
    Medium,
}

/// Whether the matched process is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Liveness {
    Live,
    Stopped,
}

/// Per-session OS-process information attached after correlation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub liveness: Liveness,
    pub match_confidence: Confidence,
    pub parent_pid: Option<u32>,
}

/// Correlates a set of sessions to currently-running OS processes.
pub struct ProcessCorrelator {
    pub(crate) scanner: Box<dyn Scanner + Send + Sync>,
    pub(crate) fd_scanner: Box<dyn FdScanner + Send + Sync>,
    /// Previous snapshot, for emitting one transient `Stopped` frame when
    /// a matched process goes away.
    prior: HashMap<String, ProcessInfo>,
    /// Sessions that already emitted their `Stopped` frame last snapshot
    /// and should now disappear from the map.
    drop_next: std::collections::HashSet<String>,
}

impl Default for ProcessCorrelator {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessCorrelator {
    pub fn new() -> Self {
        Self::with_scanners(Box::new(SysinfoScanner::new()), default_fd_scanner())
    }

    pub(crate) fn with_scanners(
        scanner: Box<dyn Scanner + Send + Sync>,
        fd_scanner: Box<dyn FdScanner + Send + Sync>,
    ) -> Self {
        Self {
            scanner,
            fd_scanner,
            prior: HashMap::new(),
            drop_next: std::collections::HashSet::new(),
        }
    }

    pub fn snapshot(&mut self, sessions: &[SessionSummary]) -> HashMap<String, ProcessInfo> {
        // Fast path: nothing to correlate — skip the OS scan entirely.
        if sessions.is_empty() && self.prior.is_empty() {
            return HashMap::new();
        }
        self.scanner.refresh();
        let mut fresh = correlate(self.scanner.as_ref(), self.fd_scanner.as_ref(), sessions);

        // Live candidate PIDs we saw this cycle; used to decide whether a
        // previously-matched session's pid is gone.
        let alive_pids: std::collections::HashSet<u32> =
            self.scanner.candidates().iter().map(|c| c.pid).collect();

        // For sessions matched previously but not this cycle: if the prior
        // pid is no longer alive AND we haven't already sent the Stopped
        // frame, emit one Stopped frame.
        let mut new_drop_next = std::collections::HashSet::new();
        for (sid, prior_info) in &self.prior {
            if fresh.contains_key(sid) {
                continue;
            }
            if self.drop_next.contains(sid) {
                continue; // already emitted Stopped last time; drop now.
            }
            if !alive_pids.contains(&prior_info.pid) {
                fresh.insert(
                    sid.clone(),
                    ProcessInfo {
                        pid: prior_info.pid,
                        liveness: Liveness::Stopped,
                        match_confidence: prior_info.match_confidence,
                        parent_pid: prior_info.parent_pid,
                    },
                );
                new_drop_next.insert(sid.clone());
            }
        }

        self.drop_next = new_drop_next;
        self.prior = fresh.clone();
        fresh
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::process::fd::tests::FakeFdScanner;
    use crate::process::scanner::tests::FakeScanner;
    use crate::process::scanner::Candidate;
    use crate::session::ClientKind;
    use chrono::Utc;
    use std::path::PathBuf;

    fn session(id: &str, path: &str) -> SessionSummary {
        SessionSummary::new(
            ClientKind::Claude,
            None,
            id.into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            None,
            PathBuf::from(path),
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn stopped_is_emitted_once_then_drops() {
        let sessions = vec![session("s1", "/tmp/s1.jsonl")];
        let path = PathBuf::from("/tmp/s1.jsonl");

        // Cycle 1: process 42 holds s1 open -> Live.
        let scanner = Box::new(FakeScanner {
            processes: vec![Candidate {
                pid: 42,
                parent_pid: Some(1),
                binary: "claude".into(),
                argv: vec!["claude".into()],
                cwd: None,
                start_time: 1700000000,
            }],
        });
        let mut fd_map = std::collections::HashMap::new();
        fd_map.insert(42u32, vec![path.clone()]);
        let fd = Box::new(FakeFdScanner { map: fd_map });

        let mut c = ProcessCorrelator::with_scanners(scanner, fd);
        let first = c.snapshot(&sessions);
        assert_eq!(first.get("s1").map(|i| i.liveness), Some(Liveness::Live));

        // Cycle 2: process 42 no longer in candidates -> Stopped once.
        c.scanner = Box::new(FakeScanner { processes: vec![] });
        c.fd_scanner = Box::new(FakeFdScanner {
            map: std::collections::HashMap::new(),
        });
        let second = c.snapshot(&sessions);
        assert_eq!(
            second.get("s1").map(|i| i.liveness),
            Some(Liveness::Stopped)
        );

        // Cycle 3: dropped.
        let third = c.snapshot(&sessions);
        assert!(
            !third.contains_key("s1"),
            "stopped entry should drop on next tick"
        );
    }
}
