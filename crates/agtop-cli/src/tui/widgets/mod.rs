//! Ratatui widget modules. Each module exposes a single `render` fn
//! taking a frame + area + app snapshot; all state lives upstream in
//! [`crate::tui::app`].

pub mod cost_tab;
pub mod info_tab;
pub mod session_table;
