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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenId {
    Dashboard,
    Aggregation,
    Config,
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
