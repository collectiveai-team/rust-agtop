//! agtop-core — provider-agnostic session discovery and cost analysis for
//! AI coding agent transcripts.
//!
//! The crate exposes a [`Provider`] trait (see [`provider`]) plus concrete
//! implementations for:
//! - Claude Code  (`~/.claude/projects/*/*.jsonl`)
//! - Codex        (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`)
//! - OpenCode     (`~/.local/share/opencode/storage/session/.../ses_*.json` plus
//!   `…/message/ses_*/msg_*.json`)
//!
//! Higher-level helpers ([`discover_all`], [`analyze_all`]) fan out across
//! every registered provider and return aggregated results.

pub mod error;
pub mod litellm;
pub mod pricing;
pub mod provider;
pub mod providers;
pub mod session;

// Flat re-exports for the most commonly used public API items.
// Consumers may also access sub-modules directly (e.g. `agtop_core::pricing::lookup`).
pub use error::{Error, Result};
pub use pricing::{Plan, PlanMode, Rates};
pub use provider::Provider;
pub use session::{
    CostBreakdown, PlanUsage, PlanWindow, ProviderKind, SessionAnalysis, SessionSummary,
    TokenTotals,
};

use std::sync::Arc;

/// Return the default set of providers.
pub fn default_providers() -> Vec<Arc<dyn Provider>> {
    vec![
        Arc::new(providers::claude::ClaudeProvider::default()),
        Arc::new(providers::codex::CodexProvider::default()),
        Arc::new(providers::opencode::OpenCodeProvider::default()),
        Arc::new(providers::copilot::CopilotProvider::default()),
        Arc::new(providers::gemini_cli::GeminiCliProvider::default()),
        Arc::new(providers::cursor::CursorProvider::default()),
        Arc::new(providers::antigravity::AntigravityProvider::default()),
    ]
}

/// Discover session summaries across all given providers.
///
/// Errors from individual providers are logged and skipped; the caller still
/// receives partial results. This mirrors the original agtop's behavior of
/// degrading gracefully when one data source is unavailable.
///
/// Providers are queried in parallel via rayon.
pub fn discover_all(providers: &[Arc<dyn Provider>]) -> Vec<SessionSummary> {
    use rayon::prelude::*;
    let mut out: Vec<SessionSummary> = providers
        .par_iter()
        .flat_map(|p| match p.list_sessions() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    provider = p.kind().as_str(),
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

/// Collect plan-usage snapshots across all given providers. Errors from
/// any single provider are logged and swallowed; the caller always gets
/// the partial result. Empty vec means "no provider had anything to
/// report" — not a fatal condition.
pub fn plan_usage_all(providers: &[Arc<dyn Provider>]) -> Vec<PlanUsage> {
    let sessions = discover_all(providers);
    plan_usage_all_from_summaries(providers, &sessions)
}

/// Collect plan-usage snapshots using already-discovered session summaries.
///
/// Providers are queried in parallel via rayon.
pub fn plan_usage_all_from_summaries(
    providers: &[Arc<dyn Provider>],
    sessions: &[SessionSummary],
) -> Vec<PlanUsage> {
    use rayon::prelude::*;
    providers
        .par_iter()
        .flat_map(|p| match p.plan_usage_with_sessions(sessions) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    provider = p.kind().as_str(),
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
    providers: &[Arc<dyn Provider>],
    summaries: &[SessionSummary],
    plan: Plan,
) -> Vec<SessionAnalysis> {
    let mut out = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let provider = match providers.iter().find(|p| p.kind() == summary.provider) {
            Some(p) => p,
            None => continue,
        };
        match provider.analyze(summary, plan) {
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
pub fn analyze_all(providers: &[Arc<dyn Provider>], plan: Plan) -> Vec<SessionAnalysis> {
    let summaries = discover_all(providers);
    analyze_all_from_summaries(providers, &summaries, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn analyze_all_from_summaries_is_consistent_with_analyze_all() {
        // Empty provider list → both functions return empty vec
        let providers: Vec<Arc<dyn Provider>> = vec![];
        let summaries = discover_all(&providers);
        let via_from_summaries = analyze_all_from_summaries(&providers, &summaries, Plan::Retail);
        let via_analyze_all = analyze_all(&providers, Plan::Retail);
        assert_eq!(via_from_summaries.len(), via_analyze_all.len());
    }

    #[test]
    fn analyze_all_from_summaries_uses_precomputed_summaries() {
        #[derive(Debug)]
        struct MockProvider;

        impl Provider for MockProvider {
            fn kind(&self) -> ProviderKind {
                ProviderKind::Claude
            }

            fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
                Ok(vec![SessionSummary::new(
                    ProviderKind::Claude,
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

        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider)];

        // discover_all should return the one summary from MockProvider
        let summaries = discover_all(&providers);
        assert_eq!(summaries.len(), 1, "expected one summary from MockProvider");
        assert_eq!(summaries[0].session_id, "test-session-1");

        // analyze_all_from_summaries should use those summaries and return one analysis
        let analyses = analyze_all_from_summaries(&providers, &summaries, Plan::Retail);
        assert_eq!(analyses.len(), 1, "expected one analysis");
        assert_eq!(
            analyses[0].tokens.input, 42,
            "expected the mock's token count"
        );
        assert_eq!(analyses[0].summary.session_id, "test-session-1");
    }

    /// Two mock providers each sleep 200ms in list_sessions. Sequential
    /// discover_all → ≥400ms wall time. Parallel via rayon → <300ms.
    #[test]
    fn discover_all_runs_providers_in_parallel() {
        use std::time::{Duration, Instant};

        #[derive(Debug)]
        struct SlowMockA;
        #[derive(Debug)]
        struct SlowMockB;
        impl Provider for SlowMockA {
            fn kind(&self) -> ProviderKind {
                ProviderKind::Claude
            }
            fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
                std::thread::sleep(Duration::from_millis(200));
                Ok(vec![])
            }
            fn analyze(&self, _s: &SessionSummary, _p: crate::Plan) -> Result<SessionAnalysis> {
                unreachable!()
            }
        }
        impl Provider for SlowMockB {
            fn kind(&self) -> ProviderKind {
                ProviderKind::Codex
            }
            fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
                std::thread::sleep(Duration::from_millis(200));
                Ok(vec![])
            }
            fn analyze(&self, _s: &SessionSummary, _p: crate::Plan) -> Result<SessionAnalysis> {
                unreachable!()
            }
        }

        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(SlowMockA), Arc::new(SlowMockB)];
        let t0 = Instant::now();
        let _ = discover_all(&providers);
        let elapsed = t0.elapsed();

        assert!(
            elapsed < Duration::from_millis(350),
            "discover_all was sequential ({}ms) — expected parallel (<350ms)",
            elapsed.as_millis()
        );
    }
}
