use crate::pricing::Plan;
use crate::session::{PlanUsage, ProviderKind, SessionAnalysis, SessionSummary};
use crate::Result;

/// A provider knows how to:
/// 1. Discover session artifacts on disk, returning lightweight summaries.
/// 2. Re-read a single session and compute token totals + cost.
///
/// Implementations must be `Send + Sync` so the CLI/TUI can call them from
/// any thread. They should be cheap to construct (no filesystem work in
/// `Default::default()` or `new`); defer real work to `list_sessions`.
pub trait Provider: std::fmt::Debug + Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// Human-readable name (e.g. "Claude Code").
    fn display_name(&self) -> &'static str {
        self.kind().as_str()
    }

    /// Return all sessions this provider can see. MUST NOT panic on
    /// missing/unreadable files; return `Ok(vec![])` when the data
    /// directory does not exist.
    fn list_sessions(&self) -> Result<Vec<SessionSummary>>;

    /// Re-read `summary.data_path` and produce a full analysis (tokens +
    /// cost) under the given billing `plan`.
    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis>;

    /// Return zero-or-more plan-usage snapshots the provider can source
    /// from local auth/state files. The default implementation returns
    /// an empty vec, so providers that don't have a plan signal (or
    /// haven't implemented it yet) remain valid. MUST NOT panic; return
    /// `Ok(vec![])` on missing files or parse failures and log via
    /// `tracing` if useful.
    fn plan_usage(&self) -> Result<Vec<PlanUsage>> {
        Ok(Vec::new())
    }
}
