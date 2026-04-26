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
