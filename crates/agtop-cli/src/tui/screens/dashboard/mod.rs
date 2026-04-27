//! Dashboard screen.
// Foundation code for Plan 2.
#![allow(dead_code)]

pub mod header;
pub mod info_drawer;
pub mod quota;
pub mod sessions;

use ratatui::{layout::Rect, Frame};
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct DashboardState {
    // Filled in throughout this plan.
}

impl DashboardState {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        // Placeholder for now: just renders the header at the top.
        let header_area = Rect { height: 3, ..area };
        header::render(frame, header_area, &header::HeaderModel::default(), theme);
    }
}

pub use header::HeaderModel;
