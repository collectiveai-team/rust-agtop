//! Application message bus. Components emit `Msg` from `handle_event`,
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]

//! the App's `update()` consumes them and mutates state.
//!
//! New variants are added as new screens/components are introduced.

/// All things the App can be asked to do.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Switch to another top-level screen (Dashboard / Aggregation / Config).
    SwitchScreen(ScreenId),
    /// Open the help overlay.
    ShowHelp,
    /// Close any modal/overlay or return focus to the parent panel.
    Escape,
    /// Quit the app.
    Quit,
    /// No-op (placeholder when an event is handled but state doesn't change).
    Noop,
    /// Request an immediate quota fetch (out-of-band refresh, bypassing the
    /// normal interval). Emitted by the QuotaPanel when the user activates
    /// the `[r]efresh` button via key (`r`) or mouse click. The TUI driver
    /// loop translates this into a manual trigger on the refresh worker.
    RefreshQuota,
    /// Aggregation screen: change group-by dimension.
    SetGroupBy(agtop_core::aggregate::GroupBy),
    /// Aggregation screen: change time range.
    SetTimeRange(agtop_core::aggregate::TimeRange),
    /// Aggregation screen: cycle sort column.
    CycleAggregationSort,
    /// Aggregation screen: toggle sort direction.
    ToggleAggregationSortDir,
    /// Aggregation screen: drill into a group.
    DrillIntoGroup(String),
    /// Aggregation screen: close drill-down overlay.
    CloseDrillDown,
    // Config screen messages
    SelectConfigSection(ConfigSection),
    SetThemeName(String),
    SetTrueColor(crate::tui::theme_v2::color::TrueColorMode),
    SetMouseCapture(bool),
    SetVersionBadge(bool),
    SetHeaderDensity(HeaderDensity),
    SetAnimations(bool),
    SetNerdFont(bool),
    SetClientColor(agtop_core::session::ClientKind, ratatui::style::Color),
    SetStatusColor(StatusSlot, ratatui::style::Color),
    SetRefreshInterval(u64),
    SetStalledThreshold(u64),
    ToggleColumnVisibility(crate::tui::column_config::ColumnId),
    MoveColumn { col: crate::tui::column_config::ColumnId, dir: MoveDir },
    ToggleClient(agtop_core::session::ClientKind, bool),
    SetDataSourcePath(agtop_core::session::ClientKind, String),
    SaveConfig,
    ConfigSearch(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenId {
    Dashboard,
    Aggregation,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigSection {
    #[default]
    Appearance,
    Columns,
    Refresh,
    Clients,
    Keybinds,
    DataSources,
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderDensity {
    Compact,
    Normal,
    Detailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusSlot {
    Live,
    Waiting,
    Warning,
    Error,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDir {
    Up,
    Down,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_is_clone_and_eq() {
        let a = Msg::SwitchScreen(ScreenId::Dashboard);
        let b = a.clone();
        assert_eq!(a, b);
    }
}
