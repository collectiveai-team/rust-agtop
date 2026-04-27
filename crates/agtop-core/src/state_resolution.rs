//! State resolution: derive `SessionState` from `SessionAnalysis` heuristics.
//!
//! This module contains the logic that converts a session analysis (which may
//! have string-based state from parsers) into the canonical `SessionState` enum.

use chrono::{DateTime, Duration, Utc};

use crate::session::{SessionAnalysis, SessionState, WaitReason, WarningReason};

/// Default threshold after which a running session with no activity is
/// considered stalled (warning state).
pub const DEFAULT_STALLED_AFTER: Duration = Duration::minutes(5);

/// Returns the timestamp of the last observed activity for a session.
fn latest_activity(analysis: &SessionAnalysis) -> Option<DateTime<Utc>> {
    analysis.summary.last_active
}

/// Infer `SessionState` from parsed string state + liveness heuristics.
///
/// Consults `analysis.summary.state` (the string set by parsers) plus
/// the `last_active` timestamp for stall detection. The canonical
/// `SessionState` is returned.
///
/// Use `DEFAULT_STALLED_AFTER` as the `threshold` argument unless you
/// need a custom value (e.g. for tests).
pub fn resolve_state(analysis: &mut SessionAnalysis, now: DateTime<Utc>) -> SessionState {
    resolve_state_with_threshold(analysis, now, DEFAULT_STALLED_AFTER)
}

/// Same as `resolve_state` but with a configurable stall threshold.
pub fn resolve_state_with_threshold(
    analysis: &mut SessionAnalysis,
    now: DateTime<Utc>,
    threshold: Duration,
) -> SessionState {
    // Map the string state set by parsers.
    let raw = analysis.summary.state.as_deref().unwrap_or("");

    // "waiting" maps to Waiting::Input (parsers use this for tool calls pending).
    // "stopped" / "closed" / empty with no recent activity → Closed.
    // "running" or recent activity → Running (subject to stall check).
    let base = match raw {
        "waiting" => SessionState::Waiting(WaitReason::Input),
        "stopped" => SessionState::Closed,
        "closed" => SessionState::Closed,
        "idle" => SessionState::Idle,
        "running" => SessionState::Running,
        // Unknown/empty: infer from recency.
        _ => SessionState::Running,
    };

    // Stall detection: if the session is Running and has had no activity
    // for longer than the threshold, emit Warning(Stalled).
    if matches!(base, SessionState::Running) {
        if let Some(last) = latest_activity(analysis) {
            if now.signed_duration_since(last) >= threshold {
                analysis.summary.state_detail = Some(format!(
                    "stalled.no_activity={}m",
                    threshold.num_minutes()
                ));
                return SessionState::Warning(WarningReason::Stalled { since: last });
            }
        }
    }

    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};

    fn build_running_analysis_with_last_activity(
        last: DateTime<Utc>,
    ) -> SessionAnalysis {
        SessionAnalysis::new(
            SessionSummary::new(
                ClientKind::Claude,
                None,
                "test-session".to_string(),
                None,
                Some(last),
                None,
                None,
                std::path::PathBuf::from("/tmp/test.jsonl"),
                Some("running".to_string()),
                None,
                None,
                None,
            ),
            TokenTotals::default(),
            CostBreakdown::default(),
            None,
            0,
            None,
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn resolve_state_uses_custom_threshold() {
        let now = Utc::now();
        let last = now - Duration::seconds(30);
        let mut analysis = build_running_analysis_with_last_activity(last);

        // With a 10-second threshold, the session should be stalled.
        let state = resolve_state_with_threshold(&mut analysis, now, Duration::seconds(10));
        assert!(
            matches!(state, SessionState::Warning(WarningReason::Stalled { .. })),
            "expected Warning(Stalled), got {state:?}"
        );

        // Reset analysis for second assertion.
        let mut analysis = build_running_analysis_with_last_activity(last);

        // With a 60-second threshold, the session should still be Running.
        let state = resolve_state_with_threshold(&mut analysis, now, Duration::seconds(60));
        assert_eq!(state, SessionState::Running);
    }

    #[test]
    fn waiting_string_maps_to_waiting_input() {
        let now = Utc::now();
        let mut a = build_running_analysis_with_last_activity(now);
        a.summary.state = Some("waiting".to_string());
        let state = resolve_state(&mut a, now);
        assert_eq!(state, SessionState::Waiting(WaitReason::Input));
    }

    #[test]
    fn stopped_string_maps_to_closed() {
        let now = Utc::now();
        let mut a = build_running_analysis_with_last_activity(now);
        a.summary.state = Some("stopped".to_string());
        let state = resolve_state(&mut a, now);
        assert_eq!(state, SessionState::Closed);
    }

    #[test]
    fn default_threshold_is_5_minutes() {
        assert_eq!(DEFAULT_STALLED_AFTER, Duration::minutes(5));
    }
}
