//! Adapt the refresh worker's Snapshot into Dashboard component models.
//!
//! The `apply_snapshot` function is called from the App v2 event loop
//! whenever a new `RefreshMsg::Snapshot` arrives from the background worker.
// Foundation code for Plans 2-4.
#![allow(dead_code)]

use agtop_core::session::SessionState;

use crate::tui::screens::aggregation::AggregationState;
use crate::tui::screens::dashboard::{
    header::HeaderModel,
    quota::QuotaPanel,
    sessions::{SessionRow, SessionsTable},
};

/// Apply a fresh set of session analyses to the dashboard component models.
pub fn apply_analyses(
    analyses: &[agtop_core::session::SessionAnalysis],
    header: &mut HeaderModel,
    sessions: &mut SessionsTable,
    _quota: &mut QuotaPanel,
    aggregation: &mut AggregationState,
    refresh_secs: u64,
) {
    // --- Sessions ---
    sessions.rows = analyses
        .iter()
        .map(|a| {
            let kind = a.summary.client;
            let label = kind.as_str().to_string();
            SessionRow {
                analysis: a.clone(),
                client_kind: kind,
                client_label: label,
                activity_samples: vec![],
            }
        })
        .collect();
    sessions.apply_sort();

    // --- Header counts ---
    let active = analyses
        .iter()
        .filter(|a| {
            a.session_state
                .as_ref()
                .map(|s| s.is_active())
                .unwrap_or(false)
        })
        .count();
    let idle = analyses
        .iter()
        .filter(|a| matches!(a.session_state, Some(SessionState::Idle)))
        .count();

    header.sessions_active = active;
    header.sessions_idle = idle;
    header.sessions_today = analyses.len();
    header.refresh_secs = refresh_secs;
    header.clock = chrono::Local::now().format("%H:%M:%S").to_string();

    // Process metrics for header CPU/mem (use aggregate from first available).
    // The refresh worker sends per-session metrics; we average for the header bar.
    let metrics_list: Vec<_> = analyses
        .iter()
        .filter_map(|a| a.process_metrics.as_ref())
        .collect();
    if !metrics_list.is_empty() {
        let avg_cpu = metrics_list.iter().map(|m| m.cpu_percent).sum::<f32>() / metrics_list.len() as f32;
        let total_mem: u64 = metrics_list.iter().map(|m| m.memory_bytes).sum();
        header.cpu_history.push(avg_cpu);
        if header.cpu_history.len() > 30 { header.cpu_history.remove(0); }
        header.mem_used_bytes = total_mem;
    }

    // --- Aggregation ---
    aggregation.sessions = analyses.to_vec();
    aggregation.recompute();
}
