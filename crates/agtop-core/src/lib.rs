//! agtop-core — client-agnostic session discovery and cost analysis for
//! AI coding agent transcripts.
//!
//! The crate exposes a [`Client`] trait (see [`client`]) plus concrete
//! implementations for:
//! - Claude Code  (`~/.claude/projects/*/*.jsonl`)
//! - Codex        (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`)
//! - OpenCode     (`~/.local/share/opencode/storage/session/.../ses_*.json` plus
//!   `…/message/ses_*/msg_*.json`)
//!
//! Higher-level helpers ([`discover_all`], [`analyze_all`]) fan out across
//! every registered client and return aggregated results.

pub mod aggregate;
pub mod client;
pub mod clients;
pub mod error;
pub mod litellm;
pub mod logo;
pub mod models_dev;
pub mod pricing;
pub mod process;
pub mod project;
pub mod quota;
pub mod session;
pub mod state_resolution;

// Flat re-exports for the most commonly used public API items.
// Consumers may also access sub-modules directly (e.g. `agtop_core::pricing::lookup`).
pub use client::Client;
pub use error::{Error, Result};
pub use pricing::{Plan, PlanMode, Rates};
pub use process::{Confidence, Liveness, ProcessCorrelator, ProcessInfo};
pub use session::{
    ClientKind, CostBreakdown, PlanUsage, PlanWindow, SessionAnalysis, SessionSummary, TokenTotals,
};

use std::sync::Arc;

/// Return the default set of clients.
pub fn default_clients() -> Vec<Arc<dyn Client>> {
    vec![
        Arc::new(clients::claude::ClaudeClient::default()),
        Arc::new(clients::codex::CodexClient::default()),
        Arc::new(clients::opencode::OpenCodeClient::default()),
        Arc::new(clients::copilot::CopilotClient::default()),
        Arc::new(clients::gemini_cli::GeminiCliClient::default()),
        Arc::new(clients::cursor::CursorClient::default()),
        Arc::new(clients::antigravity::AntigravityClient::default()),
    ]
}

/// Discover session summaries across all given clients.
///
/// Errors from individual clients are logged and skipped; the caller still
/// receives partial results. This mirrors the original agtop's behavior of
/// degrading gracefully when one data source is unavailable.
///
/// Clients are queried in parallel via rayon.
pub fn discover_all(clients: &[Arc<dyn Client>]) -> Vec<SessionSummary> {
    use rayon::prelude::*;
    let mut out: Vec<SessionSummary> = clients
        .par_iter()
        .flat_map(|client| match client.list_sessions() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    client = client.kind().as_str(),
                    error = %e,
                    "list_sessions failed"
                );
                Vec::new()
            }
        })
        .collect();
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

/// Collect plan-usage snapshots across all given clients. Errors from
/// any single client are logged and swallowed; the caller always gets
/// the partial result. Empty vec means "no client had anything to
/// report" — not a fatal condition.
pub fn plan_usage_all(clients: &[Arc<dyn Client>]) -> Vec<PlanUsage> {
    let sessions = discover_all(clients);
    plan_usage_all_from_summaries(clients, &sessions)
}

/// Collect plan-usage snapshots using already-discovered session summaries.
///
/// Clients are queried in parallel via rayon.
pub fn plan_usage_all_from_summaries(
    clients: &[Arc<dyn Client>],
    sessions: &[SessionSummary],
) -> Vec<PlanUsage> {
    use rayon::prelude::*;
    clients
        .par_iter()
        .flat_map(|client| match client.plan_usage_with_sessions(sessions) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    client = client.kind().as_str(),
                    error = %e,
                    "plan_usage failed"
                );
                Vec::new()
            }
        })
        .collect()
}

/// Analyze sessions using already-discovered summaries (tokens + cost).
/// Prefer this over [`analyze_all`] when you already hold the summaries
/// to avoid a redundant `discover_all` call.
pub fn analyze_all_from_summaries(
    clients: &[Arc<dyn Client>],
    summaries: &[SessionSummary],
    plan: Plan,
) -> Vec<SessionAnalysis> {
    let mut out = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let client = match clients
            .iter()
            .find(|candidate| candidate.kind() == summary.client)
        {
            Some(client) => client,
            None => continue,
        };
        match client.analyze(summary, plan) {
            Ok(a) => out.push(a),
            Err(e) => tracing::warn!(
                session = summary.session_id.as_str(),
                error = %e,
                "analyze failed"
            ),
        }
    }
    out
}

/// Analyze every discovered session (tokens + cost).
///
/// If you already have the summaries from [`discover_all`], call
/// [`analyze_all_from_summaries`] instead to avoid the duplicate
/// `discover_all` scan.
pub fn analyze_all(clients: &[Arc<dyn Client>], plan: Plan) -> Vec<SessionAnalysis> {
    let summaries = discover_all(clients);
    analyze_all_from_summaries(clients, &summaries, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn analyze_all_from_summaries_is_consistent_with_analyze_all() {
        // Empty client list → both functions return empty vec
        let clients: Vec<Arc<dyn Client>> = vec![];
        let summaries = discover_all(&clients);
        let via_from_summaries = analyze_all_from_summaries(&clients, &summaries, Plan::Retail);
        let via_analyze_all = analyze_all(&clients, Plan::Retail);
        assert_eq!(via_from_summaries.len(), via_analyze_all.len());
    }

    #[test]
    fn analyze_all_from_summaries_uses_precomputed_summaries() {
        #[derive(Debug)]
        struct MockClient;

        impl Client for MockClient {
            fn kind(&self) -> ClientKind {
                ClientKind::Claude
            }

            fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
                Ok(vec![SessionSummary::new(
                    ClientKind::Claude,
                    None,
                    "test-session-1".to_string(),
                    None,
                    None,
                    Some("claude-3-5-sonnet".to_string()),
                    None,
                    PathBuf::from("/tmp/test-session.jsonl"),
                    None,
                    None,
                    None,
                    None,
                )])
            }

            fn analyze(&self, summary: &SessionSummary, _plan: Plan) -> Result<SessionAnalysis> {
                Ok(SessionAnalysis::new(
                    summary.clone(),
                    TokenTotals {
                        input: 42,
                        ..TokenTotals::default()
                    },
                    CostBreakdown::default(),
                    None,
                    0,
                    None,
                    None,
                    None,
                    None,
                    None,
                ))
            }
        }

        let clients: Vec<Arc<dyn Client>> = vec![Arc::new(MockClient)];

        // discover_all should return the one summary from MockClient
        let summaries = discover_all(&clients);
        assert_eq!(summaries.len(), 1, "expected one summary from MockClient");
        assert_eq!(summaries[0].session_id, "test-session-1");

        // analyze_all_from_summaries should use those summaries and return one analysis
        let analyses = analyze_all_from_summaries(&clients, &summaries, Plan::Retail);
        assert_eq!(analyses.len(), 1, "expected one analysis");
        assert_eq!(
            analyses[0].tokens.input, 42,
            "expected the mock's token count"
        );
        assert_eq!(analyses[0].summary.session_id, "test-session-1");
    }

    /// Two mock clients each sleep 200ms in list_sessions. Sequential
    /// discover_all → ≥400ms wall time. Parallel via rayon → <300ms.
    #[test]
    fn discover_all_runs_clients_in_parallel() {
        use std::time::{Duration, Instant};

        #[derive(Debug)]
        struct SlowMockA;
        #[derive(Debug)]
        struct SlowMockB;
        impl Client for SlowMockA {
            fn kind(&self) -> ClientKind {
                ClientKind::Claude
            }
            fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
                std::thread::sleep(Duration::from_millis(200));
                Ok(vec![])
            }
            fn analyze(&self, _s: &SessionSummary, _p: crate::Plan) -> Result<SessionAnalysis> {
                unreachable!()
            }
        }
        impl Client for SlowMockB {
            fn kind(&self) -> ClientKind {
                ClientKind::Codex
            }
            fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
                std::thread::sleep(Duration::from_millis(200));
                Ok(vec![])
            }
            fn analyze(&self, _s: &SessionSummary, _p: crate::Plan) -> Result<SessionAnalysis> {
                unreachable!()
            }
        }

        let clients: Vec<Arc<dyn Client>> = vec![Arc::new(SlowMockA), Arc::new(SlowMockB)];
        let t0 = Instant::now();
        let _ = discover_all(&clients);
        let elapsed = t0.elapsed();

        assert!(
            elapsed < Duration::from_millis(350),
            "discover_all was sequential ({}ms) — expected parallel (<350ms)",
            elapsed.as_millis()
        );
    }
}
