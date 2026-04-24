//! Persistent column configuration for the session table.
//!
//! Columns can be reordered and toggled visible/hidden. The config is
//! persisted as JSON to `~/.config/agtop/columns.json`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::tui::app::{SortColumn, SortDir};
use agtop_core::ClientKind;

/// All column identifiers in the session table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnId {
    #[serde(alias = "provider")]
    Client,
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
    AgentTurns,
    UserTurns,
    Context,
    Project,
    SessionName,
}

impl ColumnId {
    /// All columns in their default order.
    pub fn all() -> &'static [ColumnId] {
        &[
            ColumnId::Session,
            ColumnId::Started,
            ColumnId::LastActive,
            ColumnId::Age,
            ColumnId::Duration,
            ColumnId::Client,
            ColumnId::Subscription,
            ColumnId::Model,
            ColumnId::Effort,
            ColumnId::State,
            ColumnId::Tokens,
            ColumnId::OutputTokens,
            ColumnId::CacheTokens,
            ColumnId::ToolCalls,
            ColumnId::AgentTurns,
            ColumnId::UserTurns,
            ColumnId::Context,
            ColumnId::Cost,
            ColumnId::Project,
            ColumnId::SessionName,
            ColumnId::Cwd,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ColumnId::Client => "CLIENT",
            ColumnId::Subscription => "SUB",
            ColumnId::Session => "SESSION",
            ColumnId::Started => "STARTED",
            ColumnId::Age => "AGE",
            ColumnId::Model => "MODEL",
            ColumnId::Cwd => "CWD",
            ColumnId::Tokens => "TOKENS",
            ColumnId::OutputTokens => "OUT",
            ColumnId::CacheTokens => "CACHE",
            ColumnId::Cost => "COST$",
            ColumnId::ToolCalls => "TOOLS",
            ColumnId::Duration => "DUR",
            ColumnId::LastActive => "LAST ACTIVE",
            ColumnId::State => "STATE",
            ColumnId::Effort => "EFFORT",
            ColumnId::AgentTurns => "AGENT",
            ColumnId::UserTurns => "USER",
            ColumnId::Context => "CONTEXT",
            ColumnId::Project => "PROJECT",
            ColumnId::SessionName => "NAME",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ColumnId::Client => "Agentic client (claude/codex/opencode)",
            ColumnId::Subscription => "Billing subscription label",
            ColumnId::Session => "Short session ID",
            ColumnId::Started => "Session start timestamp",
            ColumnId::Age => "Time since last activity",
            ColumnId::Model => "Model name",
            ColumnId::Cwd => "Working directory",
            ColumnId::Tokens => "Total token count (input + output + cache)",
            ColumnId::OutputTokens => "Output tokens",
            ColumnId::CacheTokens => "Cache tokens (read + write)",
            ColumnId::Cost => "Dollar cost",
            ColumnId::ToolCalls => "Tool call count",
            ColumnId::Duration => "Session duration",
            ColumnId::LastActive => "Last active timestamp",
            ColumnId::State => "Session workflow state",
            ColumnId::Effort => "Model reasoning effort",
            ColumnId::AgentTurns => "Number of agent/assistant turns",
            ColumnId::UserTurns => "Number of user turns",
            ColumnId::Context => "Peak context window usage",
            ColumnId::Project => "Inferred project name (from git remote)",
            ColumnId::SessionName => "Session title/name set by the agent",
        }
    }

    /// Fixed display width in terminal columns. `None` for flexible columns.
    pub fn fixed_width(self) -> Option<u16> {
        match self {
            ColumnId::Client => Some(8),
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
            ColumnId::AgentTurns => Some(6),
            ColumnId::UserTurns => Some(6),
            ColumnId::Context => Some(20),
            ColumnId::Project => Some(16),
            ColumnId::SessionName => Some(24),
        }
    }

    /// The corresponding sort column, if this column is sortable.
    pub fn sort_col(self) -> Option<SortColumn> {
        match self {
            ColumnId::Client => Some(SortColumn::Client),
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
            ColumnId::AgentTurns => Some(SortColumn::AgentTurns),
            ColumnId::UserTurns => Some(SortColumn::UserTurns),
            ColumnId::Context => None,
            ColumnId::Project => Some(SortColumn::Project),
            ColumnId::SessionName => None,
        }
    }

    /// True for the one variable-width column (CWD).
    pub fn is_flexible(self) -> bool {
        self.fixed_width().is_none()
    }
}

fn default_sort_col() -> SortColumn {
    SortColumn::LastActive
}

fn default_sort_dir() -> SortDir {
    SortColumn::LastActive.default_direction()
}

/// Persisted column configuration.
#[derive(Debug, Clone, Serialize)]
pub struct ColumnConfig {
    /// Columns in display order. Only entries in this list are shown.
    pub columns: Vec<ColumnEntry>,
    /// Active sort column. Defaults to `LastActive` for new/missing configs.
    #[serde(default = "default_sort_col")]
    pub sort_col: SortColumn,
    /// Active sort direction. Defaults to the sort column's natural direction.
    #[serde(default = "default_sort_dir")]
    pub sort_dir: SortDir,
    /// Which clients are shown. Defaults to all enabled.
    #[serde(default = "default_clients_cfg")]
    pub clients: Vec<ClientEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnEntry {
    pub id: ColumnId,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEntry {
    pub kind: ClientKind,
    pub enabled: bool,
}

fn default_clients_cfg() -> Vec<ClientEntry> {
    ClientKind::all()
        .iter()
        .map(|&kind| ClientEntry {
            kind,
            enabled: true,
        })
        .collect()
}

impl Default for ColumnConfig {
    fn default() -> Self {
        let sort_col = default_sort_col();
        Self {
            columns: ColumnId::all()
                .iter()
                .map(|&id| ColumnEntry {
                    id,
                    visible: matches!(
                        id,
                        ColumnId::Session
                            | ColumnId::Age
                            | ColumnId::Client
                            | ColumnId::Subscription
                            | ColumnId::Model
                            | ColumnId::Effort
                            | ColumnId::State
                            | ColumnId::Tokens
                            | ColumnId::OutputTokens
                            | ColumnId::CacheTokens
                            | ColumnId::Context
                            | ColumnId::Cost
                            | ColumnId::Project
                    ),
                })
                .collect(),
            sort_col,
            sort_dir: sort_col.default_direction(),
            clients: default_clients_cfg(),
        }
    }
}

impl ColumnConfig {
    fn normalize(mut self) -> Self {
        if self.sort_col == SortColumn::Cost {
            self.sort_col = SortColumn::LastActive;
            self.sort_dir = SortColumn::LastActive.default_direction();
        }
        self
    }

    /// Returns ordered list of visible column IDs.
    pub fn visible(&self) -> Vec<ColumnId> {
        self.columns
            .iter()
            .filter(|e| e.visible)
            .map(|e| e.id)
            .collect()
    }

    /// Toggle visibility of a column by index into `self.columns`.
    /// The `Session` column is always visible and cannot be hidden.
    pub fn toggle(&mut self, idx: usize) {
        if let Some(entry) = self.columns.get_mut(idx) {
            if entry.id == ColumnId::Session {
                return;
            }
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

    // ---- Client filtering -------------------------------------------------

    /// All currently-enabled clients as a set (the shape the worker wants).
    pub fn enabled_clients(&self) -> HashSet<ClientKind> {
        self.clients
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.kind)
            .collect()
    }

    /// Flip enabled state of client at `idx` and persist to disk.
    pub fn toggle_client(&mut self, idx: usize) {
        if let Some(entry) = self.clients.get_mut(idx) {
            entry.enabled = !entry.enabled;
            self.save();
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
        // In tests always return the default so that tests which call
        // set_sort() → save() don't corrupt subsequent tests that create
        // a fresh App via App::new().
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

impl<'de> Deserialize<'de> for ColumnConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawColumnConfig {
            columns: Vec<ColumnEntry>,
            #[serde(default = "default_sort_col")]
            sort_col: SortColumn,
            #[serde(default = "default_sort_dir")]
            sort_dir: SortDir,
            clients: Option<Vec<ClientEntry>>,
            providers: Option<Vec<ClientEntry>>,
        }

        let raw = RawColumnConfig::deserialize(deserializer)?;
        let clients = match (raw.clients, raw.providers) {
            (Some(clients), _) => clients,
            (None, Some(providers)) => providers,
            (None, None) => default_clients_cfg(),
        };
        Ok(Self {
            columns: raw.columns,
            sort_col: raw.sort_col,
            sort_dir: raw.sort_dir,
            clients,
        }
        .normalize())
    }
}

#[cfg(test)]
mod cfg_client_tests {
    use super::*;
    use agtop_core::ClientKind;

    #[test]
    fn default_enables_all_clients() {
        let cfg = ColumnConfig::default();
        assert_eq!(cfg.clients.len(), ClientKind::all().len());
        assert!(cfg.clients.iter().all(|e| e.enabled));
    }

    #[test]
    fn provider_column_uses_client_labeling() {
        assert_eq!(ColumnId::Client.label(), "CLIENT");
        assert_eq!(
            ColumnId::Client.description(),
            "Agentic client (claude/codex/opencode)"
        );
    }

    #[test]
    fn enabled_clients_returns_hashset_of_enabled_kinds() {
        let mut cfg = ColumnConfig::default();
        // Disable the first entry.
        let disabled_kind = cfg.clients[0].kind;
        cfg.clients[0].enabled = false;
        let live = cfg.enabled_clients();
        assert!(!live.contains(&disabled_kind));
        assert_eq!(live.len(), cfg.clients.len() - 1);
    }

    #[test]
    fn toggle_client_flips_enabled_flag() {
        let mut cfg = ColumnConfig::default();
        let was = cfg.clients[0].enabled;
        cfg.toggle_client(0);
        assert_eq!(cfg.clients[0].enabled, !was);
    }

    #[test]
    fn deserialize_missing_clients_field_defaults_to_all_enabled() {
        // Historical config format with no `clients` field.
        let json = r#"{
            "columns": [],
            "sort_col": "last_active",
            "sort_dir": "desc"
        }"#;
        let cfg: ColumnConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(cfg.clients.len(), ClientKind::all().len());
        assert!(cfg.clients.iter().all(|e| e.enabled));
    }

    #[test]
    fn deserialize_legacy_provider_config_migrates_to_clients() {
        let json = r#"{
            "columns": [{"id": "provider", "visible": true}],
            "sort_col": "provider",
            "sort_dir": "asc",
            "providers": [{"kind": "claude", "enabled": true}]
        }"#;

        let cfg: ColumnConfig =
            serde_json::from_str(json).expect("deserialize legacy provider config");
        assert_eq!(cfg.columns.len(), 1);
        assert_eq!(cfg.columns[0].id, ColumnId::Client);
        assert_eq!(cfg.sort_col, SortColumn::Client);
        assert_eq!(cfg.sort_dir, SortDir::Asc);
        assert_eq!(cfg.clients.len(), 1);
        assert_eq!(cfg.clients[0].kind, ClientKind::Claude);
        assert!(cfg.clients[0].enabled);
    }

    #[test]
    fn deserialize_legacy_cost_default_migrates_to_last_active() {
        let json = r#"{
            "columns": [],
            "sort_col": "cost",
            "sort_dir": "desc",
            "clients": []
        }"#;

        let cfg: ColumnConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(cfg.sort_col, SortColumn::LastActive);
        assert_eq!(cfg.sort_dir, SortColumn::LastActive.default_direction());
    }

    #[test]
    fn deserialize_non_legacy_sort_keeps_user_choice() {
        let json = r#"{
            "columns": [],
            "sort_col": "tokens",
            "sort_dir": "asc",
            "clients": []
        }"#;

        let cfg: ColumnConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(cfg.sort_col, SortColumn::Tokens);
        assert_eq!(cfg.sort_dir, SortDir::Asc);
    }

    #[test]
    fn default_visible_columns_match_design() {
        let cfg = ColumnConfig::default();
        let visible: Vec<ColumnId> = cfg.visible();
        // These should be on by default.
        for id in &[
            ColumnId::Session,
            ColumnId::Age,
            ColumnId::Client,
            ColumnId::Model,
            ColumnId::Tokens,
            ColumnId::Context,
            ColumnId::Cost,
            ColumnId::Project,
        ] {
            assert!(
                visible.contains(id),
                "{:?} should be visible by default",
                id
            );
        }
        // These should be off by default.
        for id in &[
            ColumnId::Started,
            ColumnId::LastActive,
            ColumnId::Duration,
            ColumnId::Cwd,
            ColumnId::ToolCalls,
            ColumnId::AgentTurns,
            ColumnId::UserTurns,
        ] {
            assert!(
                !visible.contains(id),
                "{:?} should be hidden by default",
                id
            );
        }
    }
}
