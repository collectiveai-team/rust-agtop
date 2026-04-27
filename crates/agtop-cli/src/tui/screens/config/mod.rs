//! Config screen — placeholder until Plan 4.
// Foundation code for Plan 4.
#![allow(dead_code)]

use ratatui::{layout::Rect, widgets::Paragraph, Frame};
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct ConfigState;

impl ConfigState {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let _ = theme;
        frame.render_widget(
            Paragraph::new("Config view — coming in Plan 4. Press [d] to return."),
            area,
        );
    }
}
