//! Dashboard screen — composes header, sessions table, quota panel, info drawer.
// Foundation code for Plan 2.
#![allow(dead_code)]

pub mod header;
pub mod info_costs;
pub mod info_drawer;
pub mod info_general;
pub mod info_process;
pub mod info_summary;
pub mod quota;
pub mod sessions;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct DashboardState {
    pub header: header::HeaderModel,
    pub sessions: sessions::SessionsTable,
    pub quota: quota::QuotaPanel,
    pub info: info_drawer::InfoDrawer,
}

impl DashboardState {
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let quota_rows = match self.quota.mode {
            quota::QuotaMode::Hidden => 0,
            quota::QuotaMode::Short => 4,
            quota::QuotaMode::Long => 12,
        };
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),               // header
                Constraint::Min(0),                  // sessions
                Constraint::Length(quota_rows as u16),
            ])
            .split(area);

        header::render(frame, layout[0], &self.header, theme);
        self.sessions.render(frame, layout[1], theme);
        if quota_rows > 0 {
            self.quota.render(frame, layout[2], theme);
        }
        // Info drawer overlay (after main content so it floats above).
        self.info.render(frame, area, theme);
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        // Dispatch order: drawer (always — handles 'i' open and tab keys when open)
        // > quota > sessions > nothing.
        if let Some(m) = self.info.handle_event(event) {
            return Some(m);
        }
        // When the drawer is open, swallow left-clicks that fall inside its
        // rendered area so they never reach the sessions table behind it.
        if self.info.vis == info_drawer::DrawerVis::Open {
            if let AppEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                row,
                column,
                ..
            }) = event
            {
                if let Some(area) = self.info.last_area {
                    let inside = *column >= area.x
                        && *column < area.x + area.width
                        && *row >= area.y
                        && *row < area.y + area.height;
                    if inside {
                        return Some(Msg::Noop);
                    }
                }
            }
        }
        if let Some(m) = self.quota.handle_event(event) {
            return Some(m);
        }
        if let Some(m) = self.sessions.handle_event(event) {
            // Sync the drawer's selected row whenever the table selection changes.
            let row = self
                .sessions
                .state
                .selected()
                .and_then(|i| self.sessions.rows.get(i))
                .cloned();
            self.info.set_row(row);
            return Some(m);
        }
        None
    }
}

#[allow(unused_imports)]
pub use header::HeaderModel;
