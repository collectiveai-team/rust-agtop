//! Drill-down overlay — full impl in Task 6.

use ratatui::{layout::Rect, Frame};

use agtop_core::aggregate::GroupBy;
use agtop_core::session::SessionAnalysis;

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct DrillDown {
    open: bool,
}

impl DrillDown {
    pub fn is_open(&self) -> bool { self.open }
    pub fn open(&mut self, _label: String, _sessions: &[SessionAnalysis], _by: GroupBy) {
        self.open = true;
    }
    pub fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _theme: &Theme) {}
    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{KeyCode, KeyEvent};
        if let AppEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) = event {
            self.open = false;
            return Some(Msg::Noop);
        }
        None
    }
}
