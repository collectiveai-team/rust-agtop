//! Aggregation screen — placeholder until Plan 3.
// Foundation code for Plan 3.
#![allow(dead_code)]

pub mod controls;
pub mod table;
pub mod drilldown;

use ratatui::{layout::Rect, widgets::Paragraph, Frame};
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct AggregationState;

impl AggregationState {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let _ = theme;
        frame.render_widget(
            Paragraph::new("Aggregation view — coming in Plan 3. Press [d] to return."),
            area,
        );
    }
}
