//! Persistent column configuration for the session table.
//!
//! Columns can be reordered and toggled visible/hidden. The config is
//! persisted as JSON to `~/.config/agtop/columns.json`.

use serde::{Deserialize, Serialize};

use crate::tui::app::{SortColumn, SortDir};

fn default_sort_col() -> SortColumn {
    SortColumn::LastActive
}

fn default_sort_dir() -> SortDir {
    SortColumn::LastActive.default_direction()
}

/// All column identifiers in the session table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnId {
    Provider,
    Subscription,
    Session,
    Started,
    Age,
    Model,
    Cwd,
    Tokens,
    OutputTokens,
    CacheTokens,
    Cost,
    ToolCalls,
    Duration,
    LastActive,
    State,
    Effort,
}

impl ColumnId {
    /// All columns in their default order.
    pub fn all() -> &'static [ColumnId] {
        &[
            ColumnId::Provider,
            ColumnId::Subscription,
            ColumnId::Session,
            ColumnId::Started,
            ColumnId::Age,
            ColumnId::Model,
            ColumnId::Cwd,
            ColumnId::Tokens,
            ColumnId::OutputTokens,
            ColumnId::CacheTokens,
            ColumnId::Cost,
            ColumnId::ToolCalls,
            ColumnId::Duration,
            ColumnId::LastActive,
            ColumnId::State,
            ColumnId::Effort,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ColumnId::Provider => "PROVIDER",
            ColumnId::Subscription => "SUB",
            ColumnId::Session => "SESSION",
            ColumnId::Started => "STARTED",
            ColumnId::Age => "AGE",
            ColumnId::Model => "MODEL",
            ColumnId::Cwd => "CWD",
            ColumnId::Tokens => "TOK",
            ColumnId::OutputTokens => "OUT",
            ColumnId::CacheTokens => "CACHE",
            ColumnId::Cost => "COST$",
            ColumnId::ToolCalls => "TOOLS",
            ColumnId::Duration => "DUR",
            ColumnId::LastActive => "LAST ACTIVE",
            ColumnId::State => "STATE",
            ColumnId::Effort => "EFFORT",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ColumnId::Provider => "Agentic provider (claude/codex/opencode)",
            ColumnId::Subscription => "Billing subscription label",
            ColumnId::Session => "Short session ID",
            ColumnId::Started => "Session start timestamp",
            ColumnId::Age => "Time since last activity",
            ColumnId::Model => "Model name",
            ColumnId::Cwd => "Working directory",
            ColumnId::Tokens => "Total token count",
            ColumnId::OutputTokens => "Output tokens",
            ColumnId::CacheTokens => "Cache tokens (read + write)",
            ColumnId::Cost => "Dollar cost",
            ColumnId::ToolCalls => "Tool call count",
            ColumnId::Duration => "Session duration",
            ColumnId::LastActive => "Last active timestamp",
            ColumnId::State => "Session workflow state",
            ColumnId::Effort => "Model reasoning effort",
        }
    }

    /// Fixed display width in terminal columns. `None` for flexible columns.
    pub fn fixed_width(self) -> Option<u16> {
        match self {
            ColumnId::Provider => Some(8),
            ColumnId::Subscription => Some(16),
            ColumnId::Session => Some(12),
            ColumnId::Started => Some(16),
            ColumnId::Age => Some(5),
            ColumnId::Model => Some(24),
            ColumnId::Cwd => None, // flexible
            ColumnId::Tokens => Some(8),
            ColumnId::OutputTokens => Some(8),
            ColumnId::CacheTokens => Some(8),
            ColumnId::Cost => Some(10),
            ColumnId::ToolCalls => Some(6),
            ColumnId::Duration => Some(8),
            ColumnId::LastActive => Some(16),
            ColumnId::State => Some(10),
            ColumnId::Effort => Some(8),
        }
    }

    /// The corresponding sort column, if this column is sortable.
    pub fn sort_col(self) -> Option<SortColumn> {
        match self {
            ColumnId::Provider => Some(SortColumn::Provider),
            ColumnId::Subscription => None,
            ColumnId::Session => None,
            ColumnId::Started => Some(SortColumn::Started),
            ColumnId::Age => Some(SortColumn::LastActive),
            ColumnId::Model => Some(SortColumn::Model),
            ColumnId::Cwd => None,
            ColumnId::Tokens => Some(SortColumn::Tokens),
            ColumnId::OutputTokens => Some(SortColumn::OutputTokens),
            ColumnId::CacheTokens => Some(SortColumn::CacheTokens),
            ColumnId::Cost => Some(SortColumn::Cost),
            ColumnId::ToolCalls => Some(SortColumn::ToolCalls),
            ColumnId::Duration => Some(SortColumn::Duration),
            ColumnId::LastActive => Some(SortColumn::LastActive),
            ColumnId::State => None,
            ColumnId::Effort => None,
        }
    }

    /// True for the one variable-width column (CWD).
    pub fn is_flexible(self) -> bool {
        self.fixed_width().is_none()
    }
}

/// Persisted column configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnConfig {
    /// Columns in display order. Only entries in this list are shown.
    pub columns: Vec<ColumnEntry>,
    /// Active sort column. Defaults to `LastActive` for new/missing configs.
    #[serde(default = "default_sort_col")]
    pub sort_col: SortColumn,
    /// Active sort direction. Defaults to the sort column's natural direction.
    #[serde(default = "default_sort_dir")]
    pub sort_dir: SortDir,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnEntry {
    pub id: ColumnId,
    pub visible: bool,
}

impl Default for ColumnConfig {
    fn default() -> Self {
        let sort_col = default_sort_col();
        Self {
            columns: ColumnId::all()
                .iter()
                .map(|&id| ColumnEntry {
                    id,
                    // Hide ToolCalls and Duration by default to keep the table width manageable.
                    visible: !matches!(id, ColumnId::ToolCalls | ColumnId::Duration),
                })
                .collect(),
            sort_col,
            sort_dir: sort_col.default_direction(),
        }
    }
}

impl ColumnConfig {
    /// Returns ordered list of visible column IDs.
    pub fn visible(&self) -> Vec<ColumnId> {
        self.columns
            .iter()
            .filter(|e| e.visible)
            .map(|e| e.id)
            .collect()
    }

    /// Toggle visibility of a column by index into `self.columns`.
    pub fn toggle(&mut self, idx: usize) {
        if let Some(entry) = self.columns.get_mut(idx) {
            entry.visible = !entry.visible;
        }
    }

    /// Move column at `idx` up (toward index 0).
    pub fn move_up(&mut self, idx: usize) {
        if idx > 0 && idx < self.columns.len() {
            self.columns.swap(idx - 1, idx);
        }
    }

    /// Move column at `idx` down.
    pub fn move_down(&mut self, idx: usize) {
        if idx + 1 < self.columns.len() {
            self.columns.swap(idx, idx + 1);
        }
    }

    // ---- Sort persistence ---------------------------------------------------

    /// Update the persisted sort state and save to disk.
    pub fn set_sort(&mut self, col: SortColumn, dir: SortDir) {
        self.sort_col = col;
        self.sort_dir = dir;
        self.save();
    }

    // ---- Persistence --------------------------------------------------------

    fn config_path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|p| p.join("agtop").join("columns.json"))
    }

    /// Load from disk, returning `Default` when the file does not exist
    /// or is unreadable/malformed.
    pub fn load() -> Self {
        #[cfg(test)]
        return Self::default();

        #[cfg(not(test))]
        {
            let Some(path) = Self::config_path() else {
                return Self::default();
            };
            match std::fs::read_to_string(&path) {
                Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        }
    }

    /// Persist to disk. Errors are silently ignored (best-effort).
    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }
}
