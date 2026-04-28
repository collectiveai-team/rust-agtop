#![allow(dead_code, unused)]
use chrono::{DateTime, Utc};
use ratatui::style::Style;

use crate::tui::theme as th;

const WORKING_WINDOW_SECS: i64 = 30;
/// A session reporting "waiting" that has had no activity for this long is
/// considered stale — the agent process likely died or was killed.
const WAITING_STALE_SECS: i64 = 300; // 5 minutes

pub fn display_state(
    a: &agtop_core::session::SessionAnalysis,
    now: DateTime<Utc>,
) -> (&'static str, Style) {
    let age_secs = a
        .summary
        .last_active
        .map(|ts| (now - ts).num_seconds())
        .unwrap_or(i64::MAX);

    if matches!(a.summary.parser_state, agtop_core::session::ParserState::Waiting(_)) && age_secs <= WAITING_STALE_SECS {
        return ("waiting", th::STATE_WAITING);
    }

    let is_recent = age_secs <= WORKING_WINDOW_SECS;

    if is_recent {
        ("working", th::STATE_WORKING)
    } else {
        ("stale", th::STATE_STALE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{
        ClientKind, CostBreakdown, ParserState, SessionAnalysis, SessionSummary, TokenTotals,
        WaitReason,
    };
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn analysis(state: Option<ParserState>, last_active: Option<DateTime<Utc>>) -> SessionAnalysis {
        let mut summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "sess".into(),
            last_active,
            last_active,
            Some("model".into()),
            Some("/tmp".into()),
            PathBuf::from("/tmp/sess.jsonl"),
            None,
            None,
            None,
        );
        if let Some(ps) = state {
            summary.parser_state = ps;
        }
        SessionAnalysis::new(
            summary,
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
    fn waiting_state_maps_to_waiting_style() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
        let a = analysis(Some(ParserState::Waiting(WaitReason::Input)), Some(now));

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "waiting");
        assert_eq!(style, th::STATE_WAITING);
    }

    #[test]
    fn recent_non_waiting_session_maps_to_working() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 30).unwrap();
        let a = analysis(Some(ParserState::Idle), Some(now - chrono::Duration::seconds(10)));

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "working");
        assert_eq!(style, th::STATE_WORKING);
    }

    #[test]
    fn old_non_waiting_session_maps_to_stale() {
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 1, 0).unwrap();
        let a = analysis(None, Some(now - chrono::Duration::seconds(45)));

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "stale");
        assert_eq!(style, th::STATE_STALE);
    }

    #[test]
    fn waiting_state_older_than_stale_threshold_maps_to_stale() {
        // A session stuck in "waiting" for > 5 minutes is stale (agent died).
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 10, 0).unwrap();
        let a = analysis(Some(ParserState::Waiting(WaitReason::Input)), Some(now - chrono::Duration::seconds(301)));

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "stale");
        assert_eq!(style, th::STATE_STALE);
    }

    #[test]
    fn waiting_state_within_stale_threshold_maps_to_waiting() {
        // A session in "waiting" for < 5 minutes is still shown as waiting.
        let now = Utc.with_ymd_and_hms(2026, 4, 19, 12, 10, 0).unwrap();
        let a = analysis(Some(ParserState::Waiting(WaitReason::Input)), Some(now - chrono::Duration::seconds(120)));

        let (label, style) = display_state(&a, now);

        assert_eq!(label, "waiting");
        assert_eq!(style, th::STATE_WAITING);
    }
}
