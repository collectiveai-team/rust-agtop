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
}
