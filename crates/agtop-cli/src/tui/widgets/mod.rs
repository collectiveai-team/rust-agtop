//! Ratatui widget modules. Each module exposes a single `render` fn
//! taking a frame + area + app snapshot; all state lives upstream in
//! [`crate::tui::app`].

pub mod config_tab;
pub mod cost_tab;
pub mod dashboard_cost;
pub mod dashboard_plan;
pub mod dashboard_usage;
pub mod info_tab;
pub mod process_tab;
pub mod quota_bar;
pub mod quota_tab;
pub mod session_table;
pub mod state_display;

/// Right-aligned key–value row used by Info and Process tabs.
pub(super) fn kv_line(key: &'static str, value: String) -> ratatui::prelude::Line<'static> {
    use crate::tui::theme as th;
    use ratatui::prelude::*;
    Line::from(vec![
        Span::styled(format!("{key:>16}"), th::INFO_KEY),
        Span::raw("  "),
        Span::raw(value),
    ])
}
