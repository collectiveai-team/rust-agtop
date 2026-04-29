//! Adapt the refresh worker's Snapshot into Dashboard component models.
//!
//! The `apply_snapshot` function is called from the App v2 event loop
//! whenever a new `RefreshMsg::Snapshot` arrives from the background worker.
// Foundation code for Plans 2-4.
#![allow(dead_code)]

use sysinfo::{MemoryRefreshKind, RefreshKind, System};

use agtop_core::session::{SessionAnalysis, SessionState};

use crate::tui::screens::aggregation::AggregationState;
use crate::tui::screens::dashboard::{
    header::HeaderModel,
    quota::QuotaPanel,
    sessions::{SessionRow, SessionsTable},
};

const ACTIVITY_HISTORY_LIMIT: usize = 30;
const DISK_ACTIVITY_MAX_BYTES_PER_SEC: f64 = 10.0 * 1024.0 * 1024.0;

fn activity_sample(analysis: &SessionAnalysis) -> f32 {
    let Some(metrics) = analysis.process_metrics.as_ref() else {
        return 0.0;
    };
    let cpu_score = metrics.cpu_percent.clamp(0.0, 100.0);
    let disk_rate = metrics.disk_read_bytes_per_sec + metrics.disk_written_bytes_per_sec;
    let disk_score = if disk_rate.is_finite() && disk_rate > 0.0 {
        ((disk_rate / DISK_ACTIVITY_MAX_BYTES_PER_SEC) * 100.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    };
    cpu_score.max(disk_score)
}

fn next_activity_samples(
    previous_samples: Option<&Vec<f32>>,
    analysis: &SessionAnalysis,
) -> Vec<f32> {
    let mut samples = previous_samples.cloned().unwrap_or_default();
    samples.push(activity_sample(analysis));
    if samples.len() > ACTIVITY_HISTORY_LIMIT {
        let drop_count = samples.len() - ACTIVITY_HISTORY_LIMIT;
        samples.drain(0..drop_count);
    }
    samples
}

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

    let previous_samples: std::collections::HashMap<String, Vec<f32>> = sessions
        .rows
        .iter()
        .map(|row| {
            (
                row.analysis.summary.session_id.clone(),
                row.activity_samples.clone(),
            )
        })
        .collect();

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
            activity_samples: next_activity_samples(previous_samples.get(&a.summary.session_id), a),
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
                    activity_samples: next_activity_samples(
                        previous_samples.get(&child.summary.session_id),
                        child,
                    ),
                    depth: 1,
                    parent_session_id: Some(a.summary.session_id.clone()),
                    is_last_child: i == last_idx,
                });
            }
        }
    }
    sessions.rows = flat_rows;
    sessions.apply_sort();
    ensure_selection(sessions);

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

    // Populate total system RAM once (it never changes at runtime).
    if header.mem_total_bytes == 0 {
        header.mem_total_bytes = read_total_memory_bytes();
    }

    // --- Aggregation ---
    aggregation.sessions = normalized;
    aggregation.recompute();
}

/// Insert or update a single session row from the streaming Phase 2 pipeline.
/// Does not rebuild the whole list — O(n) scan for an existing entry, then insert/replace.
///
/// Mirrors the depth=0 row-construction logic in `apply_analyses` but skips
/// the children expansion: streamed analyses arrive with their own children
/// already attached. Children are only rendered when the parent is expanded
/// (state held in `sessions.collapsed`); on first stream-in we treat the
/// parent like any newly-observed parent and let the auto-collapse policy run.
pub fn apply_session_added(
    analysis: SessionAnalysis,
    header: &mut HeaderModel,
    sessions: &mut SessionsTable,
    _quota: &mut QuotaPanel,
    aggregation: &mut AggregationState,
) {
    let normalized = normalize_analysis(&analysis);
    let session_id = normalized.summary.session_id.clone();
    let kind = normalized.summary.client;

    // Mirror auto-collapse policy: a brand-new parent starts collapsed.
    if !normalized.children.is_empty() && !sessions.known_parents.contains(&session_id) {
        sessions.collapsed.insert(session_id.clone());
    }
    if !normalized.children.is_empty() {
        sessions.known_parents.insert(session_id.clone());
    }

    let new_row = SessionRow {
        analysis: normalized,
        client_kind: kind,
        client_label: kind.as_str().to_string(),
        activity_samples: vec![],
        depth: 0,
        parent_session_id: None,
        is_last_child: false,
    };

    if let Some(pos) = sessions
        .rows
        .iter()
        .position(|r| r.depth == 0 && r.analysis.summary.session_id == session_id)
    {
        sessions.rows[pos] = new_row;
    } else {
        sessions.rows.push(new_row);
        sessions.apply_sort();
    }
    ensure_selection(sessions);

    // Recompute header counts from current depth-0 rows.
    let depth0_iter = || sessions.rows.iter().filter(|r| r.depth == 0);
    header.sessions_active = depth0_iter()
        .filter(|r| {
            r.analysis
                .session_state
                .as_ref()
                .map(|s| s.is_active())
                .unwrap_or(false)
        })
        .count();
    header.sessions_idle = depth0_iter()
        .filter(|r| matches!(r.analysis.session_state, Some(SessionState::Idle)))
        .count();

    // Recompute today count.
    let depth0_analyses: Vec<SessionAnalysis> = depth0_iter().map(|r| r.analysis.clone()).collect();
    header.sessions_today = count_today(&depth0_analyses);

    // Recompute aggregation from current depth-0 rows.
    aggregation.sessions = depth0_analyses;
    aggregation.recompute();
}

fn ensure_selection(sessions: &mut SessionsTable) {
    if sessions.state.selected().is_none() && !sessions.rows.is_empty() {
        sessions.state.select(Some(0));
    }
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

/// Read total system physical RAM via sysinfo. Returns 0 on failure.
fn read_total_memory_bytes() -> u64 {
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    sys.total_memory()
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

    fn analysis_with_metrics(
        id: &str,
        cpu_percent: f32,
        read_rate: f64,
        write_rate: f64,
    ) -> SessionAnalysis {
        let mut a = analysis(id);
        a.process_metrics = Some(ProcessMetrics {
            cpu_percent,
            memory_bytes: 1024,
            virtual_memory_bytes: 2048,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
            disk_read_bytes_per_sec: read_rate,
            disk_written_bytes_per_sec: write_rate,
        });
        a
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
            disk_read_bytes_per_sec: 0.0,
            disk_written_bytes_per_sec: 0.0,
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
            disk_read_bytes_per_sec: 0.0,
            disk_written_bytes_per_sec: 0.0,
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
    fn activity_samples_use_max_of_cpu_and_normalized_disk() {
        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();

        let disk_heavy = analysis_with_metrics("disk", 5.0, 5.0 * 1_048_576.0, 0.0);
        apply_analyses(
            &[disk_heavy],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        assert_eq!(sessions.rows.len(), 1);
        let sample = sessions.rows[0].activity_samples.last().copied().unwrap();
        assert_eq!(sample, 50.0f32, "5 MiB/s should normalize to exactly 50");

        let cpu_heavy = analysis_with_metrics("disk", 80.0, 1.0 * 1_048_576.0, 0.0);
        apply_analyses(
            &[cpu_heavy],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        assert_eq!(sessions.rows[0].activity_samples.len(), 2);
        assert_eq!(
            sessions.rows[0].activity_samples.last().copied(),
            Some(80.0)
        );
    }

    #[test]
    fn activity_samples_are_preserved_capped_and_zero_without_metrics() {
        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();

        for i in 0..35 {
            let analysis = if i == 34 {
                analysis("same")
            } else {
                analysis_with_metrics("same", i as f32, 0.0, 0.0)
            };
            apply_analyses(
                &[analysis],
                &mut header,
                &mut sessions,
                &mut quota,
                &mut aggregation,
                5,
            );
        }

        let samples = &sessions.rows[0].activity_samples;
        assert_eq!(samples.len(), 30);
        assert_eq!(samples.last().copied(), Some(0.0));
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

    #[test]
    fn dashboard_syncs_info_selection_after_refresh_changes_state() {
        use crate::tui::screens::dashboard::DashboardState;
        use agtop_core::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};

        fn sync_analysis(state: SessionState) -> SessionAnalysis {
            let summary = SessionSummary::new(
                ClientKind::OpenCode,
                None,
                "ses_sync".into(),
                None,
                Some(chrono::Utc::now()),
                None,
                None,
                std::path::PathBuf::new(),
                None,
                None,
                None,
            );
            let mut a = SessionAnalysis::new(
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
            );
            match state {
                SessionState::Running => {
                    a.liveness = Some(Liveness::Live);
                    a.summary.last_active = Some(chrono::Utc::now());
                }
                SessionState::Idle => {
                    a.liveness = Some(Liveness::Live);
                    a.summary.parser_state = agtop_core::session::ParserState::Idle;
                    a.summary.last_active = Some(chrono::Utc::now());
                }
                other => {
                    a.summary.last_active = Some(chrono::Utc::now() - chrono::Duration::hours(2));
                    a.session_state = Some(other);
                }
            }
            a
        }

        let mut dashboard = DashboardState::default();
        dashboard.sessions.state.select(Some(0));
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[sync_analysis(SessionState::Running)],
            &mut dashboard.header,
            &mut dashboard.sessions,
            &mut dashboard.quota,
            &mut aggregation,
            2,
        );
        dashboard.sync_info_selection();
        assert!(matches!(
            dashboard
                .info
                .selected_row
                .as_ref()
                .unwrap()
                .analysis
                .session_state,
            Some(SessionState::Running)
        ));

        apply_analyses(
            &[sync_analysis(SessionState::Idle)],
            &mut dashboard.header,
            &mut dashboard.sessions,
            &mut dashboard.quota,
            &mut aggregation,
            2,
        );
        dashboard.sync_info_selection();
        assert!(matches!(
            dashboard
                .info
                .selected_row
                .as_ref()
                .unwrap()
                .analysis
                .session_state,
            Some(SessionState::Idle)
        ));
    }

    #[test]
    fn first_refresh_selects_first_session_for_info_drawer() {
        use crate::tui::screens::dashboard::DashboardState;
        use agtop_core::session::{
            ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
        };

        let summary = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "ses_first".into(),
            None,
            Some(chrono::Utc::now()),
            None,
            None,
            std::path::PathBuf::new(),
            None,
            None,
            None,
        );
        let analysis = SessionAnalysis::new(
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
        );

        let mut dashboard = DashboardState::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(
            &[analysis],
            &mut dashboard.header,
            &mut dashboard.sessions,
            &mut dashboard.quota,
            &mut aggregation,
            2,
        );
        dashboard.sync_info_selection();

        assert_eq!(dashboard.sessions.state.selected(), Some(0));
        assert_eq!(
            dashboard.info.selected_row.as_ref().map(|row| row
                .analysis
                .summary
                .session_id
                .as_str()),
            Some("ses_first")
        );
    }

    #[test]
    fn apply_session_added_inserts_new_row() {
        use agtop_core::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};
        use chrono::Utc;
        let summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "ses_new".to_string(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            None,
            std::path::PathBuf::from("/tmp/fake.jsonl"),
            None,
            None,
            None,
        );
        let a = SessionAnalysis::new(
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
        );

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();

        apply_session_added(a, &mut header, &mut sessions, &mut quota, &mut aggregation);

        assert_eq!(sessions.rows.len(), 1);
        assert_eq!(sessions.rows[0].analysis.summary.session_id, "ses_new");
    }

    #[test]
    fn apply_session_added_replaces_existing_row() {
        use agtop_core::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};
        use chrono::Utc;
        fn mk(id: &str) -> SessionAnalysis {
            let summary = SessionSummary::new(
                ClientKind::Claude,
                None,
                id.to_string(),
                Some(Utc::now()),
                Some(Utc::now()),
                None,
                None,
                std::path::PathBuf::from("/tmp/fake.jsonl"),
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
        }

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();

        apply_session_added(
            mk("ses_dup"),
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
        );
        apply_session_added(
            mk("ses_dup"),
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
        );

        assert_eq!(
            sessions.rows.len(),
            1,
            "duplicate session_id should replace, not add"
        );
    }
}
