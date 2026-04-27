use chrono::{DateTime, Duration, Utc};

use crate::process::Liveness;
use crate::session::{SessionAnalysis, SessionState};

pub const DEFAULT_STALLED_AFTER: Duration = Duration::minutes(5);

/// Threshold for sessions running long tool operations before considering them stalled.
/// Reserved for future use when tool-type classification is available.
#[allow(dead_code)]
pub const LONG_TOOL_STALLED_AFTER: Duration = Duration::minutes(10);

/// Returns true if the session has a PID and hasn't been confirmed stopped.
/// Sessions with `liveness = None` (not yet polled) are treated optimistically as live.
#[must_use]
fn is_live(a: &SessionAnalysis) -> bool {
    a.pid.is_some() && !matches!(a.liveness, Some(Liveness::Stopped))
}

#[must_use]
fn latest_activity(a: &SessionAnalysis) -> Option<DateTime<Utc>> {
    a.children
        .iter()
        .filter_map(latest_activity)
        .chain(a.summary.last_active)
        .max()
}

#[must_use]
fn priority(state: SessionState) -> u8 {
    match state {
        SessionState::AwaitingPermission => 0,
        SessionState::AwaitingInput => 1,
        SessionState::Running => 2,
        SessionState::Stalled => 3,
        SessionState::Idle => 4,
        SessionState::Closed => 5,
        SessionState::Unknown => 6,
    }
}

pub fn resolve_session_states(analyses: &mut [SessionAnalysis], now: DateTime<Utc>) {
    for analysis in analyses {
        resolve_one(analysis, now);
    }
}

fn resolve_one(analysis: &mut SessionAnalysis, now: DateTime<Utc>) -> SessionState {
    for child in &mut analysis.children {
        resolve_one(child, now);
    }

    if !is_live(analysis) {
        analysis.summary.state = Some(SessionState::Closed);
        analysis.summary.state_detail = Some("liveness=none".to_string());
        return SessionState::Closed;
    }

    let child_best = analysis
        .children
        .iter()
        .filter_map(|child| {
            child
                .summary
                .state
                .map(|state| (state, child.summary.session_id.as_str()))
        })
        .filter(|(state, _)| {
            !matches!(
                state,
                SessionState::Closed | SessionState::Unknown | SessionState::Idle
            )
        })
        .min_by_key(|(state, _)| priority(*state));

    if let Some((state, child_id)) = child_best {
        analysis.summary.state = Some(state);
        analysis.summary.state_detail = Some(format!("child={child_id}:{}", state.as_str()));
        return state;
    }

    let state = analysis.summary.state.unwrap_or(SessionState::Unknown);
    let resolved = if state == SessionState::Running {
        let inactive = latest_activity(analysis)
            .map(|last| now.signed_duration_since(last) >= DEFAULT_STALLED_AFTER)
            .unwrap_or(false);
        if inactive {
            analysis.summary.state_detail = Some(format!(
                "stalled.no_activity={}m",
                DEFAULT_STALLED_AFTER.num_minutes()
            ));
            SessionState::Stalled
        } else {
            SessionState::Running
        }
    } else {
        state
    };

    analysis.summary.state = Some(resolved);
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn analysis(
        id: &str,
        state: Option<SessionState>,
        last_active: DateTime<Utc>,
        live: bool,
    ) -> SessionAnalysis {
        let mut a = SessionAnalysis::new(
            SessionSummary::new(
                ClientKind::Claude,
                None,
                id.to_string(),
                Some(last_active),
                Some(last_active),
                Some("model".to_string()),
                Some("/tmp".to_string()),
                PathBuf::from(format!("/tmp/{id}.jsonl")),
                state,
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
        );
        if live {
            a.pid = Some(42);
            a.liveness = Some(Liveness::Live);
        }
        a
    }

    #[test]
    fn non_live_sessions_resolve_to_closed() {
        let now = Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap();
        let mut analyses = vec![analysis("s1", Some(SessionState::Idle), now, false)];

        resolve_session_states(&mut analyses, now);

        assert_eq!(analyses[0].summary.state, Some(SessionState::Closed));
        assert_eq!(
            analyses[0].summary.state_detail.as_deref(),
            Some("liveness=none")
        );
    }

    #[test]
    fn child_awaiting_permission_bubbles_to_parent() {
        let now = Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap();
        let mut parent = analysis("parent", Some(SessionState::Idle), now, true);
        parent.children = vec![analysis(
            "child",
            Some(SessionState::AwaitingPermission),
            now,
            true,
        )];
        let mut analyses = vec![parent];

        resolve_session_states(&mut analyses, now);

        assert_eq!(
            analyses[0].summary.state,
            Some(SessionState::AwaitingPermission)
        );
        assert!(analyses[0]
            .summary
            .state_detail
            .as_deref()
            .unwrap()
            .contains("child=child"));
    }

    #[test]
    fn active_child_running_keeps_idle_parent_running() {
        let now = Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap();
        let mut parent = analysis("parent", Some(SessionState::Idle), now, true);
        parent.children = vec![analysis("child", Some(SessionState::Running), now, true)];
        let mut analyses = vec![parent];

        resolve_session_states(&mut analyses, now);

        assert_eq!(analyses[0].summary.state, Some(SessionState::Running));
    }

    #[test]
    fn old_running_session_without_child_progress_resolves_to_stalled() {
        let now = Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap();
        let old = now - Duration::minutes(6);
        let mut analyses = vec![analysis("s1", Some(SessionState::Running), old, true)];

        resolve_session_states(&mut analyses, now);

        assert_eq!(analyses[0].summary.state, Some(SessionState::Stalled));
        assert_eq!(
            analyses[0].summary.state_detail.as_deref(),
            Some("stalled.no_activity=5m")
        );
    }

    #[test]
    fn grandchild_awaiting_permission_bubbles_to_grandparent() {
        let now = Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap();
        let mut grandparent = analysis("gp", Some(SessionState::Idle), now, true);
        let mut parent = analysis("parent", Some(SessionState::Idle), now, true);
        parent.children = vec![analysis(
            "child",
            Some(SessionState::AwaitingPermission),
            now,
            true,
        )];
        grandparent.children = vec![parent];
        let mut analyses = vec![grandparent];

        resolve_session_states(&mut analyses, now);

        assert_eq!(
            analyses[0].summary.state,
            Some(SessionState::AwaitingPermission)
        );
    }

    #[test]
    fn recent_running_session_stays_running() {
        let now = Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap();
        let recent = now - Duration::minutes(2);
        let mut analyses = vec![analysis("s1", Some(SessionState::Running), recent, true)];

        resolve_session_states(&mut analyses, now);

        assert_eq!(analyses[0].summary.state, Some(SessionState::Running));
    }
}
