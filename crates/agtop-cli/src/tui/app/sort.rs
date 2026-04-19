//! Sort state and sort-key logic for the session table.

use agtop_core::session::{SessionAnalysis, TokenTotals};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Columns the user can sort the session table by. Cycles via `F6` / `>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortColumn {
    /// Last-active timestamp (descending = most recent first). Default.
    LastActive,
    /// Provider name, then session id (ascending, alphabetical).
    Provider,
    /// Session started-at timestamp (descending = newest first).
    Started,
    /// Model string (ascending). Unknowns sort last.
    Model,
    /// Total dollar cost (descending). Included sessions count as 0.
    Cost,
    /// Grand-total token count (descending).
    Tokens,
    /// Output-only token count (descending).
    OutputTokens,
    /// Cache token total (read + write, descending).
    CacheTokens,
    /// Number of tool calls (descending). None sorts last.
    ToolCalls,
    /// Session wall-clock duration in seconds (descending). None sorts last.
    Duration,
    /// Number of agent turns (descending). None sorts last.
    AgentTurns,
    /// Number of user turns (descending). None sorts last.
    UserTurns,
    /// Project name (ascending, alphabetical). None sorts last.
    Project,
}

impl SortColumn {
    /// Column immediately after `self` in the cycle order. Wraps around.
    pub fn next(self) -> Self {
        match self {
            Self::LastActive => Self::Provider,
            Self::Provider => Self::Started,
            Self::Started => Self::Model,
            Self::Model => Self::Cost,
            Self::Cost => Self::Tokens,
            Self::Tokens => Self::OutputTokens,
            Self::OutputTokens => Self::CacheTokens,
            Self::CacheTokens => Self::ToolCalls,
            Self::ToolCalls => Self::Duration,
            Self::Duration => Self::AgentTurns,
            Self::AgentTurns => Self::UserTurns,
            Self::UserTurns => Self::Project,
            Self::Project => Self::LastActive,
        }
    }

    /// Short display label used in the header and footer.
    pub fn label(self) -> &'static str {
        match self {
            Self::LastActive => "last-active",
            Self::Provider => "provider",
            Self::Started => "started",
            Self::Model => "model",
            Self::Cost => "cost",
            Self::Tokens => "tokens",
            Self::OutputTokens => "output",
            Self::CacheTokens => "cache",
            Self::ToolCalls => "tool-calls",
            Self::Duration => "duration",
            Self::AgentTurns => "agent-turns",
            Self::UserTurns => "user-turns",
            Self::Project => "project",
        }
    }

    /// The natural / most-useful direction for the column. Descending for
    /// numeric columns (highest first); ascending for text columns.
    pub fn default_direction(self) -> SortDir {
        match self {
            Self::LastActive
            | Self::Started
            | Self::Cost
            | Self::Tokens
            | Self::OutputTokens
            | Self::CacheTokens
            | Self::ToolCalls
            | Self::Duration
            | Self::AgentTurns
            | Self::UserTurns => SortDir::Desc,
            Self::Provider | Self::Model | Self::Project => SortDir::Asc,
        }
    }
}

/// Sort direction toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    /// Return the opposite direction.
    pub fn flip(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }
}

// ---------------------------------------------------------------------------
// Sort key
// ---------------------------------------------------------------------------

/// Compare two sessions by `col` for ascending order.
/// Callers invert the result for descending order.
pub(super) fn sort_key(
    a: &SessionAnalysis,
    b: &SessionAnalysis,
    col: SortColumn,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match col {
        SortColumn::LastActive => a.summary.last_active.cmp(&b.summary.last_active),
        SortColumn::Provider => {
            let p = a.summary.provider.as_str().cmp(b.summary.provider.as_str());
            if p == Ordering::Equal {
                a.summary.session_id.cmp(&b.summary.session_id)
            } else {
                p
            }
        }
        SortColumn::Started => a.summary.started_at.cmp(&b.summary.started_at),
        SortColumn::Model => cmp_opt_str(a.summary.model.as_deref(), b.summary.model.as_deref()),
        SortColumn::Cost => a
            .cost
            .total
            .partial_cmp(&b.cost.total)
            .unwrap_or(Ordering::Equal),
        SortColumn::Tokens => grand_total(&a.tokens).cmp(&grand_total(&b.tokens)),
        SortColumn::OutputTokens => a.tokens.output.cmp(&b.tokens.output),
        SortColumn::CacheTokens => cache_total(&a.tokens).cmp(&cache_total(&b.tokens)),
        SortColumn::ToolCalls => cmp_opt_u64(a.tool_call_count, b.tool_call_count),
        SortColumn::Duration => cmp_opt_u64(a.duration_secs, b.duration_secs),
        SortColumn::AgentTurns => cmp_opt_u64(a.agent_turns, b.agent_turns),
        SortColumn::UserTurns => cmp_opt_u64(a.user_turns, b.user_turns),
        SortColumn::Project => cmp_opt_str(a.project_name.as_deref(), b.project_name.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn grand_total(t: &TokenTotals) -> u64 {
    t.grand_total()
}

fn cache_total(t: &TokenTotals) -> u64 {
    t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input
}

/// `None` sorts after everything regardless of direction.
fn cmp_opt_u64(a: Option<u64>, b: Option<u64>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// `None` sorts after everything regardless of direction.
fn cmp_opt_str(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
