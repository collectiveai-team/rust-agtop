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

/// Return the default set of providers (Claude Code, Codex, OpenCode).
pub fn default_providers() -> Vec<Arc<dyn Provider>> {
    vec![
        Arc::new(providers::claude::ClaudeProvider::default()),
        Arc::new(providers::codex::CodexProvider::default()),
        Arc::new(providers::opencode::OpenCodeProvider::default()),
    ]
}

/// Discover session summaries across all given providers.
///
/// Errors from individual providers are logged and skipped; the caller still
/// receives partial results. This mirrors the original agtop's behavior of
/// degrading gracefully when one data source is unavailable.
pub fn discover_all(providers: &[Arc<dyn Provider>]) -> Vec<SessionSummary> {
    let mut out = Vec::new();
    for p in providers {
        match p.list_sessions() {
            Ok(sessions) => out.extend(sessions),
            Err(e) => {
                tracing::warn!(provider = p.kind().as_str(), error = %e, "list_sessions failed")
            }
        }
    }
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out
}

/// Collect plan-usage snapshots across all given providers. Errors from
/// any single provider are logged and swallowed; the caller always gets
/// the partial result. Empty vec means "no provider had anything to
/// report" — not a fatal condition.
pub fn plan_usage_all(providers: &[Arc<dyn Provider>]) -> Vec<PlanUsage> {
    let mut out = Vec::new();
    for p in providers {
        match p.plan_usage() {
            Ok(entries) => out.extend(entries),
            Err(e) => {
                tracing::warn!(provider = p.kind().as_str(), error = %e, "plan_usage failed")
            }
        }
    }
    out
}

/// Analyze every discovered session (tokens + cost).
pub fn analyze_all(providers: &[Arc<dyn Provider>], plan: Plan) -> Vec<SessionAnalysis> {
    let summaries = discover_all(providers);
    let mut out = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let provider = match providers.iter().find(|p| p.kind() == summary.provider) {
            Some(p) => p,
            None => continue,
        };
        match provider.analyze(&summary, plan) {
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
