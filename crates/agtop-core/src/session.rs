use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Which agent produced a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Claude,
    Codex,
    OpenCode,
}

impl ProviderKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lightweight session metadata derived from file headers/names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub provider: ProviderKind,
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
    /// Primary data file / directory for this session.
    pub data_path: PathBuf,
}

/// Aggregated token counts across all turns in a session.
///
/// Fields map to the vocabulary each provider exposes; not every provider
/// populates every field.
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

impl TokenTotals {
    /// Grand total of every distinct bucket (input + output + cache activity).
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
    /// Number of tool invocations observed in the transcript (provider
    /// specific best-effort). `None` when unavailable.
    #[serde(default)]
    pub tool_call_count: Option<u64>,
    /// Session wall-clock duration in seconds. `None` when we cannot
    /// infer both start and end timestamps.
    #[serde(default)]
    pub duration_secs: Option<u64>,
    /// Peak per-turn context usage as a percentage of the model context
    /// window (0..=100+), when the provider exposes both values.
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanWindow {
    /// Short label shown next to the gauge, e.g. "5h" or "7d".
    pub label: String,
    /// Utilization as a fraction [0.0, 1.0]. `None` when the provider
    /// does not expose a gauge (e.g. Claude Code).
    pub utilization: Option<f64>,
    /// When the window resets, in UTC. `None` when unknown.
    pub reset_at: Option<DateTime<Utc>>,
    /// Free-form human text from the provider (e.g. Claude's "resets 3pm
    /// (America/Buenos_Aires)"). Used when we have no structured reset_at.
    pub reset_hint: Option<String>,
    /// True when this window is the representative/binding one for the plan
    /// ("representative-claim" in Anthropic's unified rate-limit protocol).
    #[serde(default)]
    pub binding: bool,
}

/// Plan + usage snapshot for a (provider, auth) pair.
///
/// A single provider can contribute multiple entries when the user has
/// more than one auth (e.g. Claude Code on Anthropic Max AND OpenCode on
/// the same Anthropic Max — each produces its own `PlanUsage` because the
/// data sources are different).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUsage {
    pub provider: ProviderKind,
    /// Provider-qualified label for the card header, e.g. "Claude Code ·
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
