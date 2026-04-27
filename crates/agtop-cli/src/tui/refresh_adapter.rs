//! Adapt the refresh worker's Snapshot into Dashboard component models.
//!
//! The `apply_snapshot` function is called from the App v2 event loop
//! whenever a new `RefreshMsg::Snapshot` arrives from the background worker.
// Foundation code for Plans 2-4.
#![allow(dead_code)]

use agtop_core::{
    process::Liveness,
    session::{SessionAnalysis, SessionState, WaitReason, WarningReason},
};

use crate::tui::screens::aggregation::AggregationState;
use crate::tui::screens::dashboard::{
    header::HeaderModel,
    quota::QuotaPanel,
    sessions::{SessionRow, SessionsTable},
};

/// Apply a fresh set of session analyses to the dashboard component models.
pub fn apply_analyses(
    analyses: &[SessionAnalysis],
    header: &mut HeaderModel,
    sessions: &mut SessionsTable,
    _quota: &mut QuotaPanel,
    aggregation: &mut AggregationState,
    refresh_secs: u64,
) {
    let normalized: Vec<SessionAnalysis> = analyses.iter().map(normalize_analysis).collect();

    // --- Sessions ---
    let mut flat_rows: Vec<SessionRow> = Vec::new();
    for a in &normalized {
        let kind = a.summary.client;
        let label = kind.as_str().to_string();
        flat_rows.push(SessionRow {
            analysis: a.clone(),
            client_kind: kind,
            client_label: label.clone(),
            activity_samples: vec![],
            depth: 0,
            parent_session_id: None,
        });
        // Insert children unless this parent is collapsed.
        if !a.children.is_empty() && !sessions.collapsed.contains(&a.summary.session_id) {
            let mut children: Vec<&SessionAnalysis> = a.children.iter().collect();
            // Sort children by started_at descending (newest first).
            children.sort_by(|x, y| y.summary.started_at.cmp(&x.summary.started_at));
            for child in children {
                let child_kind = child.summary.client;
                flat_rows.push(SessionRow {
                    analysis: child.clone(),
                    client_kind: child_kind,
                    client_label: child_kind.as_str().to_string(),
                    activity_samples: vec![],
                    depth: 1,
                    parent_session_id: Some(a.summary.session_id.clone()),
                });
            }
        }
    }
    sessions.rows = flat_rows;
    sessions.apply_sort();

    // --- Header counts ---
    let active = normalized
        .iter()
        .filter(|a| {
            a.session_state
                .as_ref()
                .map(|s| s.is_active())
                .unwrap_or(false)
        })
        .count();
    let idle = normalized
        .iter()
        .filter(|a| matches!(a.session_state, Some(SessionState::Idle)))
        .count();

    header.sessions_active = active;
    header.sessions_idle = idle;
    header.sessions_today = count_today(&normalized);
    header.refresh_secs = refresh_secs;
    header.clock = chrono::Local::now().format("%H:%M:%S").to_string();

    // Process metrics for header CPU/mem (use aggregate from first available).
    // The refresh worker sends per-session metrics; we average for the header bar.
    let metrics_list: Vec<_> = normalized
        .iter()
        .filter_map(|a| a.process_metrics.as_ref())
        .collect();
    if !metrics_list.is_empty() {
        let avg_cpu =
            metrics_list.iter().map(|m| m.cpu_percent).sum::<f32>() / metrics_list.len() as f32;
        let total_mem: u64 = metrics_list.iter().map(|m| m.memory_bytes).sum();
        header.cpu_history.push(avg_cpu);
        if header.cpu_history.len() > 30 {
            header.cpu_history.remove(0);
        }
        header.mem_used_bytes = total_mem;
    }

    // --- Aggregation ---
    aggregation.sessions = normalized;
    aggregation.recompute();
}

/// Count sessions whose `started_at` is on or after local midnight today.
fn count_today(analyses: &[SessionAnalysis]) -> usize {
    use chrono::TimeZone;
    let today_local = chrono::Local::now().date_naive();
    let midnight_utc = chrono::Local
        .from_local_datetime(&today_local.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);
    analyses
        .iter()
        .filter(|a| {
            a.summary
                .started_at
                .map(|t| t >= midnight_utc)
                .unwrap_or(false)
        })
        .count()
}

fn normalize_analysis(analysis: &SessionAnalysis) -> SessionAnalysis {
    let mut analysis = analysis.clone();
    // Always re-derive the state when liveness info is present (i.e. process
    // correlation has run).  A session that the parser classified as "closed"
    // may still have a live PID, and the liveness check in derive_state is
    // authoritative over the parser's static snapshot.
    if analysis.liveness.is_some() || analysis.session_state.is_none() {
        analysis.session_state = Some(derive_state(&analysis));
    }
    analysis
}

fn derive_state(analysis: &SessionAnalysis) -> SessionState {
    if matches!(analysis.liveness, Some(Liveness::Stopped)) {
        return SessionState::Closed;
    }

    if !matches!(analysis.liveness, Some(Liveness::Live)) {
        return SessionState::Closed;
    }

    match analysis.summary.state.as_deref() {
        Some("waiting") => SessionState::Waiting(WaitReason::Input),
        Some("idle") => SessionState::Idle,
        Some("stopped" | "closed") => SessionState::Closed,
        _ => {
            if let Some(last) = analysis.summary.last_active {
                let stalled_after = chrono::Duration::minutes(5);
                if chrono::Utc::now().signed_duration_since(last) >= stalled_after {
                    return SessionState::Warning(WarningReason::Stalled { since: last });
                }
            }
            SessionState::Running
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::process::{Confidence, Liveness, ProcessMetrics};
    use agtop_core::session::{
        ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
    };

    fn analysis(id: &str) -> SessionAnalysis {
        SessionAnalysis::new(
            SessionSummary::new(
                ClientKind::Claude,
                None,
                id.to_string(),
                None,
                Some(chrono::Utc::now()),
                None,
                None,
                std::path::PathBuf::from("/tmp/session.jsonl"),
                None,
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

    fn apply_one(a: SessionAnalysis) -> (HeaderModel, SessionsTable) {
        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[a],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );
        (header, sessions)
    }

    #[test]
    fn live_process_without_parser_state_is_active_running() {
        let mut a = analysis("live");
        a.pid = Some(1234);
        a.liveness = Some(Liveness::Live);
        a.match_confidence = Some(Confidence::Medium);
        a.process_metrics = Some(ProcessMetrics {
            cpu_percent: 12.0,
            memory_bytes: 1024,
            virtual_memory_bytes: 2048,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
        });

        let (header, sessions) = apply_one(a);

        assert_eq!(header.sessions_active, 1);
        assert_eq!(header.sessions_idle, 0);
        assert!(matches!(
            sessions.rows[0].analysis.session_state,
            Some(SessionState::Running)
        ));
    }

    #[test]
    fn today_count_excludes_yesterday_sessions() {
        use chrono::TimeZone;
        let today_local = chrono::Local::now().date_naive();
        let midnight_utc = chrono::Local
            .from_local_datetime(&today_local.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        let mut a_today = analysis("today");
        a_today.summary.started_at = Some(midnight_utc + chrono::Duration::hours(1));

        let mut a_yesterday = analysis("yesterday");
        a_yesterday.summary.started_at = Some(midnight_utc - chrono::Duration::hours(1));

        let mut a_no_date = analysis("nodate");
        a_no_date.summary.started_at = None;

        let normalized: Vec<SessionAnalysis> =
            vec![a_today, a_yesterday, a_no_date].iter().map(normalize_analysis).collect();
        assert_eq!(count_today(&normalized), 1, "only 'today' session counts");
    }

    #[test]
    fn historical_session_without_parser_state_stays_closed() {
        let (header, sessions) = apply_one(analysis("historical"));

        assert_eq!(header.sessions_active, 0);
        assert_eq!(header.sessions_idle, 0);
        assert!(matches!(
            sessions.rows[0].analysis.session_state,
            Some(SessionState::Closed)
        ));
    }

    #[test]
    fn live_process_with_idle_state_counted_as_idle() {
        let mut a = analysis("idle-session");
        a.pid = Some(5678);
        a.liveness = Some(Liveness::Live);
        a.match_confidence = Some(Confidence::Medium);
        a.process_metrics = Some(ProcessMetrics {
            cpu_percent: 0.0,
            memory_bytes: 512,
            virtual_memory_bytes: 1024,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
        });
        a.summary.state = Some("idle".to_string());

        let (header, sessions) = apply_one(a);

        // is_active() is true for Idle, so it counts as active too.
        assert_eq!(header.sessions_active, 1, "idle sessions are active");
        assert_eq!(header.sessions_idle, 1, "idle sessions are also idle");
        assert!(matches!(
            sessions.rows[0].analysis.session_state,
            Some(SessionState::Idle)
        ));
    }

    #[test]
    fn children_appear_as_depth_1_rows_after_parent() {
        use agtop_core::session::{SessionAnalysis, SessionSummary, TokenTotals, CostBreakdown, ClientKind};
        use agtop_core::process::Liveness;
        use chrono::Utc;

        let mut parent = analysis("parent-1");
        parent.liveness = Some(Liveness::Live);

        let child_summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "child-1".to_string(),
            None,
            Some(Utc::now()),
            None,
            None,
            std::path::PathBuf::from("/tmp/child.jsonl"),
            None,
            None,
            None,
            None,
        );
        let child = SessionAnalysis::new(
            child_summary,
            TokenTotals::default(),
            CostBreakdown::default(),
            None, 0, None, None, None, None, None,
        );
        parent.children = vec![child];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(&[parent], &mut header, &mut sessions, &mut quota, &mut aggregation, 5);

        assert_eq!(sessions.rows.len(), 2, "parent + 1 child = 2 rows");
        assert_eq!(sessions.rows[0].depth, 0, "parent is depth 0");
        assert_eq!(sessions.rows[1].depth, 1, "child is depth 1");
        assert_eq!(
            sessions.rows[1].parent_session_id.as_deref(),
            Some("parent-1"),
            "child parent_session_id must point to parent"
        );
    }

    #[test]
    fn collapsed_parent_hides_children() {
        use agtop_core::session::{SessionAnalysis, SessionSummary, TokenTotals, CostBreakdown, ClientKind};
        use agtop_core::process::Liveness;
        use chrono::Utc;

        let mut parent = analysis("parent-collapsed");
        parent.liveness = Some(Liveness::Live);
        let child_summary = SessionSummary::new(
            ClientKind::Claude, None, "child-collapsed".to_string(), None,
            Some(Utc::now()), None, None, std::path::PathBuf::from("/tmp/c.jsonl"),
            None, None, None, None,
        );
        let child = SessionAnalysis::new(
            child_summary, TokenTotals::default(), CostBreakdown::default(),
            None, 0, None, None, None, None, None,
        );
        parent.children = vec![child];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        sessions.collapsed.insert("parent-collapsed".to_string());
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(&[parent], &mut header, &mut sessions, &mut quota, &mut aggregation, 5);

        assert_eq!(sessions.rows.len(), 1, "collapsed parent hides children");
    }

    #[test]
    fn stopped_process_is_closed_not_counted() {
        let mut a = analysis("stopped-session");
        a.pid = Some(9999);
        a.liveness = Some(Liveness::Stopped);
        a.match_confidence = Some(Confidence::Medium);

        let (header, sessions) = apply_one(a);

        assert_eq!(header.sessions_active, 0);
        assert_eq!(header.sessions_idle, 0);
        assert!(matches!(
            sessions.rows[0].analysis.session_state,
            Some(SessionState::Closed)
        ));
    }
}
