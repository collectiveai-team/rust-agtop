//! State resolution: derive `SessionState` from typed `ParserState` + liveness.
//!
//! `resolve_state` is the **single canonical conversion point** from parser
//! output to the UI-facing `SessionState`. All parsers produce a `ParserState`;
//! all callers read a `SessionState`; nothing else should perform this mapping.

use chrono::{DateTime, Duration, Utc};

use crate::process::Liveness;
use crate::session::{ParserState, SessionState, WarningReason};

// ── Constants ──────────────────────────────────────────────────────────────────

/// A running session with a live process is considered *stalled* (→ Warning)
/// after this much time with no log activity.
pub const DEFAULT_STALLED_AFTER: Duration = Duration::minutes(5);

/// When we have no liveness data, a session that last had activity within this
/// window is still assumed to be running.  Beyond this → Closed.
pub const RUNNING_RECENCY_WINDOW: Duration = Duration::seconds(30);

/// When a parser reported Idle (end-of-turn) but we have no liveness data, the
/// session is assumed to still be alive within this window.  Beyond → Closed.
pub const NO_LIVENESS_CLOSED_AFTER: Duration = Duration::minutes(5);

// ── Public API ─────────────────────────────────────────────────────────────────

/// Derive the canonical `SessionState` from typed parser output + OS liveness.
///
/// # Arguments
/// * `parser_state`  — what the log parser inferred about the agent's state
/// * `liveness`      — OS-level process liveness (may be absent)
/// * `last_active`   — timestamp of last log activity (may be absent)
/// * `now`           — current wall-clock time (injected for testability)
pub fn resolve_state(
    parser_state: ParserState,
    liveness: Option<Liveness>,
    last_active: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> SessionState {
    resolve_state_with_threshold(
        parser_state,
        liveness,
        last_active,
        now,
        DEFAULT_STALLED_AFTER,
    )
}

/// Same as `resolve_state` but with a configurable stall threshold (for tests).
pub fn resolve_state_with_threshold(
    parser_state: ParserState,
    liveness: Option<Liveness>,
    last_active: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    stall_threshold: Duration,
) -> SessionState {
    // Dead/stopped process → always Closed regardless of parser opinion.
    if liveness == Some(Liveness::Stopped) {
        return SessionState::Closed;
    }

    match parser_state {
        // ── Idle: agent finished its turn ──────────────────────────────────
        ParserState::Idle => {
            match liveness {
                Some(Liveness::Live) => SessionState::Idle,
                // No liveness data: trust recency.
                _ => {
                    if is_recent(last_active, now, NO_LIVENESS_CLOSED_AFTER) {
                        SessionState::Idle
                    } else {
                        SessionState::Closed
                    }
                }
            }
        }

        // ── Running: agent mid-turn ────────────────────────────────────────
        ParserState::Running => {
            match liveness {
                Some(Liveness::Live) => {
                    // Stall check: live but silent for too long → Warning.
                    if let Some(last) = last_active {
                        if now.signed_duration_since(last) >= stall_threshold {
                            return SessionState::Warning(WarningReason::Stalled { since: last });
                        }
                    }
                    SessionState::Running
                }
                // No liveness: short grace window.
                _ => {
                    if is_recent(last_active, now, RUNNING_RECENCY_WINDOW) {
                        SessionState::Running
                    } else {
                        SessionState::Closed
                    }
                }
            }
        }

        // ── Waiting: agent blocked on user / permission ────────────────────
        ParserState::Waiting(reason) => {
            match liveness {
                Some(Liveness::Live) => SessionState::Waiting(reason),
                _ => {
                    // No liveness but parser saw a wait prompt: use same grace
                    // window as Idle — likely still active.
                    if is_recent(last_active, now, NO_LIVENESS_CLOSED_AFTER) {
                        SessionState::Waiting(reason)
                    } else {
                        SessionState::Closed
                    }
                }
            }
        }

        // ── Error: parser detected an error in the log ─────────────────────
        ParserState::Error(reason) => SessionState::Error(reason),

        // ── Unknown: parser had no opinion ────────────────────────────────
        ParserState::Unknown => {
            match liveness {
                Some(Liveness::Live) => {
                    // Live but no parser signal: assume running (stall check
                    // still applies).
                    if let Some(last) = last_active {
                        if now.signed_duration_since(last) >= stall_threshold {
                            return SessionState::Warning(WarningReason::Stalled { since: last });
                        }
                    }
                    SessionState::Running
                }
                // No liveness + no parser opinion: infer from recency.
                _ => {
                    if is_recent(last_active, now, RUNNING_RECENCY_WINDOW) {
                        SessionState::Running
                    } else {
                        SessionState::Closed
                    }
                }
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Returns `true` if `last_active` is within `window` of `now`.
fn is_recent(
    last_active: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    window: Duration,
) -> bool {
    last_active.map_or(false, |t| now.signed_duration_since(t) < window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ErrorReason, WaitReason};

    #[test]
    fn idle_live_is_idle() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Idle,
            Some(Liveness::Live),
            Some(now - Duration::seconds(10)),
            now,
        );
        assert_eq!(s, SessionState::Idle);
    }

    #[test]
    fn idle_no_liveness_recent_is_idle() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Idle,
            None,
            Some(now - Duration::minutes(3)),
            now,
        );
        assert_eq!(s, SessionState::Idle);
    }

    #[test]
    fn idle_no_liveness_stale_is_closed() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Idle,
            None,
            Some(now - Duration::minutes(10)),
            now,
        );
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn running_live_recent_is_running() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Running,
            Some(Liveness::Live),
            Some(now - Duration::seconds(10)),
            now,
        );
        assert_eq!(s, SessionState::Running);
    }

    #[test]
    fn running_live_stalled_is_warning() {
        let now = Utc::now();
        let last = now - DEFAULT_STALLED_AFTER - Duration::seconds(1);
        let s = resolve_state(
            ParserState::Running,
            Some(Liveness::Live),
            Some(last),
            now,
        );
        assert!(
            matches!(s, SessionState::Warning(WarningReason::Stalled { .. })),
            "expected Warning(Stalled), got {s:?}"
        );
    }

    #[test]
    fn running_no_liveness_within_window_is_running() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Running,
            None,
            Some(now - Duration::seconds(10)),
            now,
        );
        assert_eq!(s, SessionState::Running);
    }

    #[test]
    fn running_no_liveness_outside_window_is_closed() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Running,
            None,
            Some(now - RUNNING_RECENCY_WINDOW - Duration::seconds(1)),
            now,
        );
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn waiting_live_is_waiting() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Waiting(WaitReason::Input),
            Some(Liveness::Live),
            Some(now - Duration::seconds(10)),
            now,
        );
        assert_eq!(s, SessionState::Waiting(WaitReason::Input));
    }

    #[test]
    fn error_is_error() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Error(ErrorReason::ParserDetected("boom".into())),
            Some(Liveness::Live),
            None,
            now,
        );
        assert!(
            matches!(s, SessionState::Error(_)),
            "expected Error, got {s:?}"
        );
    }

    #[test]
    fn unknown_live_recent_is_running() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Unknown,
            Some(Liveness::Live),
            Some(now - Duration::seconds(10)),
            now,
        );
        assert_eq!(s, SessionState::Running);
    }

    #[test]
    fn unknown_dead_is_closed() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Unknown,
            Some(Liveness::Stopped),
            None,
            now,
        );
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn stopped_liveness_is_closed() {
        let now = Utc::now();
        let s = resolve_state(
            ParserState::Idle,
            Some(Liveness::Stopped),
            None,
            now,
        );
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn default_stalled_after_is_5_minutes() {
        assert_eq!(DEFAULT_STALLED_AFTER, Duration::minutes(5));
    }

    #[test]
    fn running_recency_window_is_30_seconds() {
        assert_eq!(RUNNING_RECENCY_WINDOW, Duration::seconds(30));
    }
}
