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
    SubscriptionLogo, // internal — never persisted, always derived from Subscription
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
    Pid,
    /// Live CPU usage of the matched process (percentage).
    Cpu,
    /// Resident memory of the matched process (bytes, compact format).
    Memory,
    /// Virtual memory of the matched process (bytes, compact format).
    VirtualMemory,
    /// Cumulative disk bytes read by the matched process.
    DiskRead,
    /// Cumulative disk bytes written by the matched process.
    DiskWritten,
    /// Current in-flight tool/action descriptor for running sessions.
    Action,
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
            ColumnId::Pid,
            ColumnId::Cpu,
            ColumnId::Memory,
            ColumnId::VirtualMemory,
            ColumnId::DiskRead,
            ColumnId::DiskWritten,
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
            ColumnId::Action,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            ColumnId::Client => "CLIENT",
            ColumnId::Subscription => "SUB",
            ColumnId::SubscriptionLogo => "", // no header label
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
            ColumnId::Pid => "PID",
            ColumnId::Cpu => "CPU",
            ColumnId::Memory => "MEM",
            ColumnId::VirtualMemory => "VSZ",
            ColumnId::DiskRead => "DISK R",
            ColumnId::DiskWritten => "DISK W",
            ColumnId::Action => "ACTION",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ColumnId::Client => "Agentic client (claude/codex/opencode)",
            ColumnId::Subscription => "Billing subscription label",
            ColumnId::SubscriptionLogo => "Subscription provider logo",
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
            ColumnId::Pid => "OS process ID of the live agent CLI",
            ColumnId::Cpu => "Live CPU usage of the matched process (%)",
            ColumnId::Memory => "Live resident memory of the matched process",
            ColumnId::VirtualMemory => "Live virtual memory of the matched process",
            ColumnId::DiskRead => "Cumulative bytes read from disk (since process start)",
            ColumnId::DiskWritten => "Cumulative bytes written to disk (since process start)",
            ColumnId::Action => "Current in-flight tool/action descriptor for running sessions",
        }
    }

    /// Fixed display width in terminal columns. `None` for flexible columns.
    pub fn fixed_width(self) -> Option<u16> {
        match self {
            ColumnId::Client => Some(10),
            ColumnId::Subscription => Some(16),
            ColumnId::SubscriptionLogo => Some(3),
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
            ColumnId::Pid => Some(7), // 6 digits + padding
            ColumnId::Cpu => Some(6),
            ColumnId::Memory => Some(7),
            ColumnId::VirtualMemory => Some(7),
            ColumnId::DiskRead => Some(8),
            ColumnId::DiskWritten => Some(8),
            ColumnId::Action => Some(20),
        }
    }

    /// The corresponding sort column, if this column is sortable.
    pub fn sort_col(self) -> Option<SortColumn> {
        match self {
            ColumnId::Client => Some(SortColumn::Client),
            ColumnId::Subscription => None,
            ColumnId::SubscriptionLogo => None,
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
            ColumnId::Pid => None, // not sortable
            ColumnId::Cpu => None,
            ColumnId::Memory => None,
            ColumnId::VirtualMemory => None,
            ColumnId::DiskRead => None,
            ColumnId::DiskWritten => None,
            ColumnId::Action => None, // not sortable (transient string)
        }
    }

    /// True for the one variable-width column (CWD).
    pub fn is_flexible(self) -> bool {
        self.fixed_width().is_none()
    }
}

/// Returns the spec'd default visible column set for the new Plan 2 sessions table.
///
/// Note: The state DOT is not a separate ColumnId — it's rendered by the sessions
/// table component as a fixed leading cell. The textual STATE column remains
/// available but hidden by default.
#[must_use]
pub fn default_visible_v2() -> Vec<ColumnId> {
    vec![
        ColumnId::Session,
        ColumnId::Age,
        ColumnId::Action,
        // ACTIVITY (sparkline) is rendered by the table component as a
        // dedicated visualization cell, not via ColumnId.
        ColumnId::Client,
        ColumnId::Subscription,
        ColumnId::Model,
        ColumnId::Cpu,
        ColumnId::Memory,
        ColumnId::Tokens,
        ColumnId::Cost,
        ColumnId::Project,
        ColumnId::SessionName,
    ]
}

/// Returns the default visibility for a column in a fresh or migrated config.
fn default_visible(id: ColumnId) -> bool {
    matches!(
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
            | ColumnId::Pid
            | ColumnId::Cpu
            | ColumnId::Memory
    )
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
                    visible: default_visible(id),
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
        // Append any new columns not present in the persisted config.
        // This ensures old configs gain new columns on upgrade without
        // losing their existing column order or visibility.
        let existing_ids: Vec<ColumnId> = self.columns.iter().map(|e| e.id).collect();
        for &id in ColumnId::all() {
            if !existing_ids.contains(&id) {
                self.columns.push(ColumnEntry {
                    id,
                    visible: default_visible(id),
                });
            }
        }
        self
    }

    /// Returns ordered list of visible column IDs, including the
    /// `SubscriptionLogo` slot before `Subscription` when both
    /// `Subscription` is visible and `with_logo` is true.
    ///
    /// `SubscriptionLogo` is never stored in `self.columns` — it is
    /// derived from `Subscription` at view time. Pass `with_logo=false`
    /// when running on a terminal that cannot render an actual image
    /// in 3 cells (i.e. halfblocks fallback) so the column doesn't
    /// reserve dead space.
    pub fn visible_ext(&self, with_logo: bool) -> Vec<ColumnId> {
        let mut result = Vec::new();
        for entry in &self.columns {
            if !entry.visible {
                continue;
            }
            if with_logo && entry.id == ColumnId::Subscription {
                result.push(ColumnId::SubscriptionLogo);
            }
            result.push(entry.id);
        }
        result
    }

    /// Backwards-compatible wrapper that always includes the logo
    /// column. New call sites should use `visible_ext` and pass the
    /// runtime "logos available" flag. Currently used only by the
    /// test suite, where logo availability is irrelevant.
    #[cfg(test)]
    pub fn visible(&self) -> Vec<ColumnId> {
        self.visible_ext(true)
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
mod subscription_logo_tests {
    use super::*;

    #[test]
    fn subscription_logo_not_in_config_tab_columns() {
        // SubscriptionLogo must never appear in the full column list visible
        // to the config tab (i.e., ColumnId::all() must not include it).
        assert!(
            !ColumnId::all().contains(&ColumnId::SubscriptionLogo),
            "SubscriptionLogo should not appear in ColumnId::all()"
        );
    }

    #[test]
    fn subscription_logo_always_before_subscription_in_visible() {
        let cfg = ColumnConfig::default();
        let visible = cfg.visible();
        let sub_pos = visible
            .iter()
            .position(|&c| c == ColumnId::Subscription)
            .expect("Subscription should be visible by default");
        assert!(sub_pos > 0, "SubscriptionLogo should precede Subscription");
        assert_eq!(
            visible[sub_pos - 1],
            ColumnId::SubscriptionLogo,
            "SubscriptionLogo must immediately precede Subscription"
        );
    }

    #[test]
    fn toggling_subscription_also_toggles_logo() {
        let mut cfg = ColumnConfig::default();
        // Find Subscription index in internal columns list.
        let sub_idx = cfg
            .columns
            .iter()
            .position(|e| e.id == ColumnId::Subscription)
            .expect("Subscription must be in columns");
        let was_visible = cfg.columns[sub_idx].visible;
        cfg.toggle(sub_idx);
        // Subscription itself must have flipped.
        assert_eq!(
            cfg.columns[sub_idx].visible, !was_visible,
            "Subscription visibility should flip"
        );
        // After toggle, visible() should no longer include SubscriptionLogo
        // (because Subscription is now hidden).
        let new_visible = cfg.visible();
        assert!(
            !new_visible.contains(&ColumnId::SubscriptionLogo),
            "SubscriptionLogo should not be visible when Subscription is hidden"
        );

        // Toggle back on — logo must reappear.
        cfg.toggle(sub_idx);
        let re_visible = cfg.visible();
        assert!(
            re_visible.contains(&ColumnId::SubscriptionLogo),
            "logo reappears when Subscription is re-enabled"
        );
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
        // After normalization, missing columns are appended so the total count
        // equals the full column list. The originally persisted column (Client,
        // from the "provider" alias) must remain first.
        assert_eq!(cfg.columns.len(), ColumnId::all().len());
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
    fn old_config_gets_new_metric_columns_appended() {
        let json = r#"{"columns":[{"id":"session","visible":true},{"id":"pid","visible":true}],"sort_col":"last_active","sort_dir":"desc","clients":[]}"#;
        let cfg: ColumnConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            cfg.columns.len(),
            ColumnId::all().len(),
            "normalize must not produce duplicate columns"
        );
        // New columns must be appended with their default visibility.
        assert!(
            cfg.columns.iter().any(|c| c.id == ColumnId::Cpu),
            "Cpu column must be present after normalization"
        );
        assert!(
            cfg.columns.iter().any(|c| c.id == ColumnId::Memory),
            "Memory column must be present after normalization"
        );
        // Cpu and Memory are visible by default; the others are hidden.
        let cpu = cfg.columns.iter().find(|c| c.id == ColumnId::Cpu).unwrap();
        let mem = cfg
            .columns
            .iter()
            .find(|c| c.id == ColumnId::Memory)
            .unwrap();
        assert!(cpu.visible, "Cpu should be visible by default");
        assert!(mem.visible, "Memory should be visible by default");
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
            ColumnId::Pid,
            ColumnId::Cpu,
            ColumnId::Memory,
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

    #[test]
    fn default_visible_matches_spec() {
        let v = default_visible_v2();
        assert_eq!(v.first(), Some(&ColumnId::Session));
        assert!(v.contains(&ColumnId::Action));
        assert!(v.contains(&ColumnId::Client));
        assert!(v.contains(&ColumnId::Subscription));
        assert!(v.contains(&ColumnId::Cost));
        // STATE column is hidden by default — should NOT appear.
        assert!(!v.contains(&ColumnId::State));
    }
}
