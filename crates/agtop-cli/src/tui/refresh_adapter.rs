//! Adapt the refresh worker's Snapshot into Dashboard component models.
//!
//! The `apply_snapshot` function is called from the App v2 event loop
//! whenever a new `RefreshMsg::Snapshot` arrives from the background worker.
// Foundation code for Plans 2-4.
#![allow(dead_code)]

use agtop_core::session::{SessionAnalysis, SessionState};

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
    // Auto-collapse newly-observed parents so the tree starts collapsed by
    // default. We track parents we've ever seen in `known_parents`; only
    // first-time parents get auto-inserted into `collapsed`. This preserves
    // user toggles: once they expand a parent (removing it from
    // `collapsed`), subsequent refreshes won't re-collapse it.
    for a in &normalized {
        if !a.children.is_empty() && !sessions.known_parents.contains(&a.summary.session_id) {
            sessions.collapsed.insert(a.summary.session_id.clone());
        }
        if !a.children.is_empty() {
            sessions.known_parents.insert(a.summary.session_id.clone());
        }
    }

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
            is_last_child: false,
        });
        // Insert children unless this parent is collapsed.
        if !a.children.is_empty() && !sessions.collapsed.contains(&a.summary.session_id) {
            let mut children: Vec<&SessionAnalysis> = a.children.iter().collect();
            // Sort children by started_at descending (newest first).
            children.sort_by(|x, y| y.summary.started_at.cmp(&x.summary.started_at));
            let last_idx = children.len().saturating_sub(1);
            for (i, child) in children.into_iter().enumerate() {
                let child_kind = child.summary.client;
                flat_rows.push(SessionRow {
                    analysis: child.clone(),
                    client_kind: child_kind,
                    client_label: child_kind.as_str().to_string(),
                    activity_samples: vec![],
                    depth: 1,
                    parent_session_id: Some(a.summary.session_id.clone()),
                    is_last_child: i == last_idx,
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
    analysis.session_state = Some(agtop_core::state_resolution::resolve_state(
        analysis.summary.parser_state.clone(),
        analysis.liveness,
        analysis.summary.last_active,
        chrono::Utc::now(),
    ));
    analysis
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

        let normalized: Vec<SessionAnalysis> = [a_today, a_yesterday, a_no_date]
            .iter()
            .map(normalize_analysis)
            .collect();
        assert_eq!(count_today(&normalized), 1, "only 'today' session counts");
    }

    #[test]
    fn historical_session_without_parser_state_stays_closed() {
        // A session with NO liveness AND NO recent activity is closed.
        // Updated semantics: derive_state now falls back to recency-based
        // classification when liveness is None (mirrors v1 behavior). A
        // truly historical session (last_active well in the past) still
        // resolves to Closed via the recency fallback.
        let mut a = analysis("historical");
        a.summary.last_active = Some(chrono::Utc::now() - chrono::Duration::hours(2));
        let (header, sessions) = apply_one(a);

        assert_eq!(header.sessions_active, 0);
        assert_eq!(header.sessions_idle, 0);
        assert!(matches!(
            sessions.rows[0].analysis.session_state,
            Some(SessionState::Closed)
        ));
    }

    #[test]
    fn unmatched_recent_session_is_running_not_closed() {
        // ROOT-CAUSE FIX for state-dot bug: when the OS-process correlator
        // fails to match a session (heuristic miss, /proc permission, etc.),
        // `liveness` stays None. v1 ignored liveness entirely and used
        // recent-activity classification. v2 must do the same fallback so
        // very-recent sessions don't render as Closed.
        let mut a = analysis("unmatched-but-active");
        a.summary.last_active = Some(chrono::Utc::now() - chrono::Duration::seconds(5));
        // No liveness, no parser state.
        a.liveness = None;

        let (header, sessions) = apply_one(a);

        assert!(
            matches!(
                sessions.rows[0].analysis.session_state,
                Some(SessionState::Running)
            ),
            "unmatched session with last_active in last 30s must render as Running, not Closed (got {:?})",
            sessions.rows[0].analysis.session_state
        );
        assert_eq!(
            header.sessions_active, 1,
            "unmatched-but-recent session must count as active for the header"
        );
    }

    #[test]
    fn unmatched_waiting_session_renders_as_waiting() {
        // Parser said state = "waiting". Without liveness, v2 must still
        // honor that — same as v1's display_state.
        let mut a = analysis("unmatched-waiting");
        a.summary.last_active = Some(chrono::Utc::now() - chrono::Duration::seconds(10));
        a.liveness = None;
        a.summary.parser_state =
            agtop_core::session::ParserState::Waiting(agtop_core::session::WaitReason::Input);

        let (_header, sessions) = apply_one(a);

        assert!(
            matches!(
                sessions.rows[0].analysis.session_state,
                Some(SessionState::Waiting(_))
            ),
            "unmatched session with parser-reported waiting state must render as Waiting (got {:?})",
            sessions.rows[0].analysis.session_state
        );
    }

    #[test]
    fn unmatched_stale_session_renders_as_closed() {
        // Past the 30-second running window with no liveness data → Closed
        // (resolve_state's canonical behavior: no liveness + Unknown + stale = Closed).
        let mut a = analysis("unmatched-stalled");
        a.summary.last_active = Some(chrono::Utc::now() - chrono::Duration::seconds(120));
        a.liveness = None;
        // No parser state, no summary state.

        let (_header, sessions) = apply_one(a);

        assert!(
            matches!(
                sessions.rows[0].analysis.session_state,
                Some(SessionState::Closed)
            ),
            "unmatched session with last_active 2m ago (no liveness) must render as Closed (got {:?})",
            sessions.rows[0].analysis.session_state
        );
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
        a.summary.parser_state = agtop_core::session::ParserState::Idle;

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
        use agtop_core::process::Liveness;
        use agtop_core::session::{
            ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
        };
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
        );
        let child = SessionAnalysis::new(
            child_summary,
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
        parent.children = vec![child];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        // Mark the parent as already known + expanded so apply_analyses does
        // not auto-collapse it (new-parent default behavior). This test
        // verifies child row construction, not the auto-collapse policy.
        sessions.known_parents.insert("parent-1".to_string());
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[parent],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

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
    fn new_parent_with_children_starts_collapsed() {
        use agtop_core::process::Liveness;
        use agtop_core::session::{
            ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
        };
        use chrono::Utc;

        let mut parent = analysis("brand-new-parent");
        parent.liveness = Some(Liveness::Live);
        let child_summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "child-of-new".to_string(),
            None,
            Some(Utc::now()),
            None,
            None,
            std::path::PathBuf::from("/tmp/c.jsonl"),
            None,
            None,
            None,
        );
        let child = SessionAnalysis::new(
            child_summary,
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
        parent.children = vec![child];

        let mut header = HeaderModel::default();
        // Fresh table — parent has not been seen before.
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[parent],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        assert_eq!(
            sessions.rows.len(),
            1,
            "newly-seen parent must start collapsed (children hidden)"
        );
        assert!(
            sessions.collapsed.contains("brand-new-parent"),
            "new parent must be auto-added to collapsed"
        );
        assert!(
            sessions.known_parents.contains("brand-new-parent"),
            "parent must be marked as known"
        );
    }

    #[test]
    fn user_expanded_parent_stays_expanded_across_refreshes() {
        use agtop_core::process::Liveness;
        use agtop_core::session::{
            ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
        };
        use chrono::Utc;

        let mut parent = analysis("user-toggled");
        parent.liveness = Some(Liveness::Live);
        let child_summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "ut-child".to_string(),
            None,
            Some(Utc::now()),
            None,
            None,
            std::path::PathBuf::from("/tmp/c.jsonl"),
            None,
            None,
            None,
        );
        parent.children = vec![SessionAnalysis::new(
            child_summary,
            TokenTotals::default(),
            CostBreakdown::default(),
            None,
            0,
            None,
            None,
            None,
            None,
            None,
        )];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        // Simulate "first refresh saw parent, then user expanded it":
        sessions.known_parents.insert("user-toggled".to_string());
        // collapsed is empty — user opened the tree.
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[parent],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        assert!(
            !sessions.collapsed.contains("user-toggled"),
            "user-expanded parent must NOT be re-collapsed on refresh"
        );
        assert_eq!(
            sessions.rows.len(),
            2,
            "user-expanded parent must show its child"
        );
    }

    #[test]
    fn collapsed_parent_hides_children() {
        use agtop_core::process::Liveness;
        use agtop_core::session::{
            ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
        };
        use chrono::Utc;

        let mut parent = analysis("parent-collapsed");
        parent.liveness = Some(Liveness::Live);
        let child_summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "child-collapsed".to_string(),
            None,
            Some(Utc::now()),
            None,
            None,
            std::path::PathBuf::from("/tmp/c.jsonl"),
            None,
            None,
            None,
        );
        let child = SessionAnalysis::new(
            child_summary,
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
        parent.children = vec![child];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        sessions.collapsed.insert("parent-collapsed".to_string());
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[parent],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        assert_eq!(sessions.rows.len(), 1, "collapsed parent hides children");
    }

    #[test]
    fn last_child_is_marked_is_last_child_true() {
        use agtop_core::process::Liveness;
        use agtop_core::session::{
            ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
        };
        use chrono::Utc;

        // Helper to build a child SessionAnalysis with a unique id.
        let mk_child = |id: &str| {
            let summary = SessionSummary::new(
                ClientKind::Claude,
                None,
                id.to_string(),
                None,
                Some(Utc::now()),
                None,
                None,
                std::path::PathBuf::from(format!("/tmp/{id}.jsonl")),
                None,
                None,
                None,
            );
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
        };

        let mut parent = analysis("multi-child-parent");
        parent.liveness = Some(Liveness::Live);
        // Children sorted by started_at desc inside apply_analyses; we use
        // identical timestamps so the order is preserved as inserted.
        parent.children = vec![mk_child("c1"), mk_child("c2"), mk_child("c3")];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        // Mark parent as known + expanded so children render.
        sessions
            .known_parents
            .insert("multi-child-parent".to_string());
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[parent],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        // 1 parent + 3 children = 4 rows.
        assert_eq!(sessions.rows.len(), 4);
        assert_eq!(sessions.rows[0].depth, 0);
        // Parent row is never a tree-leaf glyph holder.
        assert!(
            !sessions.rows[0].is_last_child,
            "parent (depth 0) is_last_child must be false"
        );
        // Among the three children, exactly one — the LAST in render order — is_last_child.
        let child_rows: Vec<&SessionRow> = sessions.rows.iter().filter(|r| r.depth == 1).collect();
        assert_eq!(child_rows.len(), 3);
        assert!(!child_rows[0].is_last_child, "first child must not be last");
        assert!(
            !child_rows[1].is_last_child,
            "middle child must not be last"
        );
        assert!(
            child_rows[2].is_last_child,
            "last child must be is_last_child = true"
        );
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
