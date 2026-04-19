use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Which agent produced a session.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Claude,
    Codex,
    OpenCode,
    Copilot,
    #[serde(rename = "gemini-cli")]
    GeminiCli,
    Cursor,
    Antigravity,
}

impl ClientKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Copilot => "copilot",
            Self::GeminiCli => "gemini-cli",
            Self::Cursor => "cursor",
            Self::Antigravity => "antigravity",
        }
    }

    /// Every `ClientKind` variant, in a stable display order.
    /// Keep this in sync with the enum definition — a missing variant
    /// here silently excludes that client from default-enabled sets.
    #[must_use]
    pub const fn all() -> &'static [ClientKind] {
        &[
            Self::Claude,
            Self::Codex,
            Self::OpenCode,
            Self::Copilot,
            Self::GeminiCli,
            Self::Cursor,
            Self::Antigravity,
        ]
    }
}

impl std::fmt::Display for ClientKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lightweight session metadata derived from file headers/names.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub client: ClientKind,
    /// Billing/auth bucket for this session when known, e.g. "Max 5x",
    /// "ChatGPT Plus", or "API key".
    #[serde(default)]
    pub subscription: Option<String>,
    pub session_id: String,
    pub started_at: Option<DateTime<Utc>>,
    pub last_active: Option<DateTime<Utc>>,
    pub model: Option<String>,
    /// Working directory (best-effort). Used for display labels.
    pub cwd: Option<String>,
    /// Coarse workflow state such as `waiting` or `stopped`.
    #[serde(default)]
    pub state: Option<String>,
    /// Client-specific explanation of the derived state.
    #[serde(default)]
    pub state_detail: Option<String>,
    /// Explicit reasoning/model effort when the client exposes it.
    #[serde(default)]
    pub model_effort: Option<String>,
    /// Client-specific explanation of where the effort came from.
    #[serde(default)]
    pub model_effort_detail: Option<String>,
    /// Human-readable session title when the client stores one (e.g. the
    /// first user message summary in OpenCode).
    #[serde(default)]
    pub session_title: Option<String>,
    /// Primary data file / directory for this session.
    pub data_path: PathBuf,
}

/// Aggregated token counts across all turns in a session.
///
/// Fields map to the vocabulary each client exposes; not every client
/// populates every field.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenTotals {
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
    pub reasoning_output: u64,
    /// Claude-style 5-minute ephemeral cache writes.
    pub cache_write_5m: u64,
    /// Claude-style 1-hour ephemeral cache writes.
    pub cache_write_1h: u64,
    /// Claude-style cache reads (cheap).
    pub cache_read: u64,
}

impl SessionSummary {
    /// Construct a [`SessionSummary`] with all fields explicitly specified.
    ///
    /// Prefer this over struct-literal syntax so callers remain compatible
    /// when new (non-exhaustive) fields are added.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: ClientKind,
        subscription: Option<String>,
        session_id: String,
        started_at: Option<DateTime<Utc>>,
        last_active: Option<DateTime<Utc>>,
        model: Option<String>,
        cwd: Option<String>,
        data_path: std::path::PathBuf,
        state: Option<String>,
        state_detail: Option<String>,
        model_effort: Option<String>,
        model_effort_detail: Option<String>,
    ) -> Self {
        Self {
            client,
            subscription,
            session_id,
            started_at,
            last_active,
            model,
            cwd,
            state,
            state_detail,
            model_effort,
            model_effort_detail,
            session_title: None,
            data_path,
        }
    }
}

impl TokenTotals {
    /// Grand total of every distinct bucket (input + output + cache activity).
    #[must_use]
    pub fn grand_total(&self) -> u64 {
        self.input
            + self.cached_input
            + self.output
            + self.cache_write_5m
            + self.cache_write_1h
            + self.cache_read
    }
}

/// Dollar-denominated cost breakdown (USD).
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub total: f64,
    /// True when the active billing plan marks this session as included
    /// (e.g. Claude Max, ChatGPT Plus) and the dollar figures are zero.
    pub included: bool,
}

/// Full analysis of a single session.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAnalysis {
    pub summary: SessionSummary,
    pub tokens: TokenTotals,
    pub cost: CostBreakdown,
    /// Model actually observed during token accounting (may differ from the
    /// summary model if a session uses multiple).
    pub effective_model: Option<String>,
    /// Number of Claude subagent sidechain transcripts folded into
    /// `tokens` / `cost` (0 when the session has none, or for non-Claude
    /// providers). Defaults to 0 on deserialization so older JSON
    /// consumers remain compatible.
    #[serde(default)]
    pub subagent_file_count: usize,
    /// Number of tool invocations observed in the transcript (client
    /// specific best-effort). `None` when unavailable.
    #[serde(default)]
    pub tool_call_count: Option<u64>,
    /// Session wall-clock duration in seconds. `None` when we cannot
    /// infer both start and end timestamps.
    #[serde(default)]
    pub duration_secs: Option<u64>,
    /// Peak per-turn context usage as a percentage of the model context
    /// window (0..=100+), when the client exposes both values.
    #[serde(default)]
    pub context_used_pct: Option<f64>,
    /// Raw token count at the peak-utilization turn (numerator of
    /// `context_used_pct`). `None` when `context_used_pct` is `None`.
    #[serde(default)]
    pub context_used_tokens: Option<u64>,
    /// Model context window size in tokens used to compute
    /// `context_used_pct`. `None` when `context_used_pct` is `None`.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Child subagent sessions, if this client exposes a parent/child
    /// relationship. Empty by default; populated by the refresh layer.
    #[serde(default)]
    pub children: Vec<SessionAnalysis>,
    /// Number of agent/assistant turns observed in the transcript.
    /// `None` when the client does not expose this information.
    #[serde(default)]
    pub agent_turns: Option<u64>,
    /// Number of user turns observed in the transcript.
    /// `None` when the client does not expose this information.
    #[serde(default)]
    pub user_turns: Option<u64>,
    /// Inferred project name (e.g. from `git remote get-url origin`).
    /// `None` when unavailable.
    #[serde(default)]
    pub project_name: Option<String>,
}

impl SessionAnalysis {
    /// Construct a [`SessionAnalysis`] with all fields explicitly specified.
    ///
    /// Prefer this over struct-literal syntax so callers remain compatible
    /// when new (non-exhaustive) fields are added.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        summary: SessionSummary,
        tokens: TokenTotals,
        cost: CostBreakdown,
        effective_model: Option<String>,
        subagent_file_count: usize,
        tool_call_count: Option<u64>,
        duration_secs: Option<u64>,
        context_used_pct: Option<f64>,
        context_used_tokens: Option<u64>,
        context_window: Option<u64>,
    ) -> Self {
        Self {
            summary,
            tokens,
            cost,
            effective_model,
            subagent_file_count,
            tool_call_count,
            duration_secs,
            context_used_pct,
            context_used_tokens,
            context_window,
            children: Vec::new(),
            agent_turns: None,
            user_turns: None,
            project_name: None,
        }
    }
}

impl PlanWindow {
    /// Construct a [`PlanWindow`] with all fields explicitly specified.
    pub fn new(
        label: String,
        utilization: Option<f64>,
        reset_at: Option<DateTime<Utc>>,
        reset_hint: Option<String>,
        binding: bool,
    ) -> Self {
        Self {
            label,
            utilization,
            reset_at,
            reset_hint,
            binding,
        }
    }
}

impl PlanUsage {
    /// Construct a [`PlanUsage`] with all fields explicitly specified.
    pub fn new(
        client: ClientKind,
        label: String,
        plan_name: Option<String>,
        windows: Vec<PlanWindow>,
        last_limit_hit: Option<DateTime<Utc>>,
        note: Option<String>,
    ) -> Self {
        Self {
            client,
            label,
            plan_name,
            windows,
            last_limit_hit,
            note,
        }
    }
}

// ---------------------------------------------------------------------------
// Plan usage (for the --dashboard pane)
// ---------------------------------------------------------------------------

/// One rate-limit window (e.g. Anthropic's 5-hour rolling cap).
///
/// `utilization` is a fraction in 0.0..=1.0 where available; `reset_at` is
/// the UTC time the window resets, also when available. Providers may
/// populate only a subset of these fields; renderers must treat every
/// field as optional.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanWindow {
    /// Short label shown next to the gauge, e.g. "5h" or "7d".
    pub label: String,
    /// Utilization as a fraction [0.0, 1.0]. `None` when the client
    /// does not expose a gauge (e.g. Claude Code).
    pub utilization: Option<f64>,
    /// When the window resets, in UTC. `None` when unknown.
    pub reset_at: Option<DateTime<Utc>>,
    /// Free-form human text from the client (e.g. Claude's "resets 3pm
    /// (America/Buenos_Aires)"). Used when we have no structured reset_at.
    pub reset_hint: Option<String>,
    /// True when this window is the representative/binding one for the plan
    /// ("representative-claim" in Anthropic's unified rate-limit protocol).
    #[serde(default)]
    pub binding: bool,
}

/// Plan + usage snapshot for a (client, auth) pair.
///
/// A single client can contribute multiple entries when the user has
/// more than one auth (e.g. Claude Code on Anthropic Max AND OpenCode on
/// the same Anthropic Max — each produces its own `PlanUsage` because the
/// data sources are different).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUsage {
    pub client: ClientKind,
    /// Client-qualified label for the card header, e.g. "Claude Code ·
    /// Max 5x" or "OpenCode · anthropic (Max)". Free-form; renderers
    /// display verbatim.
    pub label: String,
    /// Plan name when known (e.g. "max", "max_5x", "plus", "pro").
    pub plan_name: Option<String>,
    /// Zero or more usage windows. Typically "5h" + "7d" for Anthropic,
    /// "primary" + "secondary" for OpenAI/Codex when populated. Empty
    /// when no utilization data is available (e.g. Claude Code).
    #[serde(default)]
    pub windows: Vec<PlanWindow>,
    /// Most recent moment the user was observed to hit a rate limit.
    /// Used for providers that don't expose gauges but do record
    /// limit-hit events (Claude Code's synthetic error messages).
    pub last_limit_hit: Option<DateTime<Utc>>,
    /// Free-form note rendered below the gauges, e.g. "waiting for
    /// backend data" when the schema slot exists but is null.
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_kind_all_lists_every_variant() {
        let all = ClientKind::all();
        assert_eq!(all.len(), 7, "expected all 7 providers: {all:?}");
        // Spot-check a couple of variants to guard against silent drift.
        assert!(all.contains(&ClientKind::Claude));
        assert!(all.contains(&ClientKind::Antigravity));
    }
}
