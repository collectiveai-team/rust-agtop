//! Config screen — VS Code Settings-style sidebar + detail.

pub mod sidebar;
pub mod detail;
pub mod controls;
pub mod color_picker;
pub mod sections {
    pub mod appearance;
    pub mod columns;
    pub mod refresh;
    pub mod clients;
    pub mod keybinds;
    pub mod data_sources;
    pub mod about;
}

use ratatui::{layout::Rect, Frame};
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct ConfigState {
    // Filled in Task 8.
}

impl ConfigState {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let _ = (frame, area, theme);
        // Replaced in Task 8.
    }
}
