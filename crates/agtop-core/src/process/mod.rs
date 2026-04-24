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
    // Fields added in later tasks.
    _placeholder: (),
}

impl Default for ProcessCorrelator {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessCorrelator {
    pub fn new() -> Self {
        Self { _placeholder: () }
    }

    /// Refresh OS-process state and match against the given sessions.
    /// Returns a map keyed by `session_id`; sessions with no match are
    /// absent from the map. Never panics, never returns Err.
    pub fn snapshot(&mut self, _sessions: &[SessionSummary]) -> HashMap<String, ProcessInfo> {
        HashMap::new()
    }
}
