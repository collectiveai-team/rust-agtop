use crate::pricing::Plan;
use crate::session::{ClientKind, PlanUsage, SessionAnalysis, SessionSummary};
use crate::Result;

/// A client knows how to:
/// 1. Discover session artifacts on disk, returning lightweight summaries.
/// 2. Re-read a single session and compute token totals + cost.
///
/// Implementations must be `Send + Sync` so the CLI/TUI can call them from
/// any thread. They should be cheap to construct (no filesystem work in
/// `Default::default()` or `new`); defer real work to `list_sessions`.
pub trait Client: std::fmt::Debug + Send + Sync {
    fn kind(&self) -> ClientKind;

    /// Human-readable name (e.g. "Claude Code").
    fn display_name(&self) -> &'static str {
        self.kind().as_str()
    }

    /// Return all sessions this client can see. MUST NOT panic on
    /// missing/unreadable files; return `Ok(vec![])` when the data
    /// directory does not exist.
    fn list_sessions(&self) -> Result<Vec<SessionSummary>>;

    /// Re-read `summary.data_path` and produce a full analysis (tokens +
    /// cost) under the given billing `plan`.
    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis>;

    /// Return zero-or-more plan-usage snapshots the client can source
    /// from local auth/state files. The default implementation returns
    /// an empty vec, so clients that don't have a plan signal (or
    /// haven't implemented it yet) remain valid. MUST NOT panic; return
    /// `Ok(vec![])` on missing files or parse failures and log via
    /// `tracing` if useful.
    fn plan_usage(&self) -> Result<Vec<PlanUsage>> {
        Ok(Vec::new())
    }

    /// Context-aware plan usage hook. Clients that need to correlate
    /// quota refreshes with recently discovered sessions can override
    /// this method; everyone else keeps the plain `plan_usage` behavior.
    fn plan_usage_with_sessions(&self, sessions: &[SessionSummary]) -> Result<Vec<PlanUsage>> {
        let _ = sessions;
        self.plan_usage()
    }

    /// Optional client-specific parent/child relationship hook.
    /// The default implementation returns an empty vec so clients
    /// without child-session support remain valid. MUST NOT panic;
    /// on errors, clients should log via `tracing` if useful and
    /// return `Ok(vec![])`.
    fn children(&self, _parent: &SessionSummary) -> Result<Vec<SessionSummary>> {
        Ok(Vec::new())
    }
}
