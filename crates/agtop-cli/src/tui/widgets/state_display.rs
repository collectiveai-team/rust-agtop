use chrono::{DateTime, Utc};
use ratatui::style::Style;

use crate::tui::theme as th;

pub fn display_state(
    a: &agtop_core::session::SessionAnalysis,
    _now: DateTime<Utc>,
) -> (&'static str, Style) {
    if a.pid.is_none() || matches!(a.liveness, Some(agtop_core::process::Liveness::Stopped)) {
        return ("closed", th::STATE_CLOSED);
    }

    match a
        .summary
        .state
        .unwrap_or(agtop_core::session::SessionState::Unknown)
    {
        agtop_core::session::SessionState::Running => ("run", th::STATE_RUNNING),
        agtop_core::session::SessionState::Idle => ("idle", th::STATE_IDLE),
        agtop_core::session::SessionState::AwaitingInput => ("input", th::STATE_AWAITING_INPUT),
        agtop_core::session::SessionState::AwaitingPermission => {
            ("permit", th::STATE_AWAITING_PERMISSION)
        }
        agtop_core::session::SessionState::Stalled => ("stalled", th::STATE_STALLED),
        agtop_core::session::SessionState::Closed => ("closed", th::STATE_CLOSED),
        agtop_core::session::SessionState::Unknown => ("?", th::STATE_UNKNOWN),
        _ => ("?", th::STATE_UNKNOWN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::process::Liveness;
    use agtop_core::session::{
        ClientKind, CostBreakdown, SessionAnalysis, SessionState, SessionSummary, TokenTotals,
    };
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn analysis(
        state: Option<SessionState>,
        last_active: Option<DateTime<Utc>>,
        pid: Option<u32>,
        liveness: Option<Liveness>,
    ) -> SessionAnalysis {
        let mut a = SessionAnalysis::new(
            SessionSummary::new(
                ClientKind::Claude,
                None,
                "sess".into(),
                last_active,
                last_active,
                Some("model".into()),
                Some("/tmp".into()),
                PathBuf::from("/tmp/sess.jsonl"),
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
        a.pid = pid;
        a.liveness = liveness;
        a
    }

    #[test]
    fn live_running_session_displays_run() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
        let a = analysis(
            Some(SessionState::Running),
            Some(now),
            Some(42),
            Some(Liveness::Live),
        );

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "run");
        assert_eq!(style, th::STATE_RUNNING);
    }

    #[test]
    fn live_awaiting_permission_session_displays_permit() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
        let a = analysis(
            Some(SessionState::AwaitingPermission),
            Some(now),
            Some(42),
            Some(Liveness::Live),
        );

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "permit");
        assert_eq!(style, th::STATE_AWAITING_PERMISSION);
    }

    #[test]
    fn live_idle_session_displays_idle() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
        let a = analysis(
            Some(SessionState::Idle),
            Some(now),
            Some(42),
            Some(Liveness::Live),
        );

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "idle");
        assert_eq!(style, th::STATE_IDLE);
    }

    #[test]
    fn no_pid_session_displays_closed() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
        let a = analysis(Some(SessionState::Running), Some(now), None, None);

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "closed");
        assert_eq!(style, th::STATE_CLOSED);
    }

    #[test]
    fn stopped_liveness_displays_closed() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
        let a = analysis(
            Some(SessionState::Running),
            Some(now),
            Some(42),
            Some(Liveness::Stopped),
        );

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "closed");
        assert_eq!(style, th::STATE_CLOSED);
    }
}
