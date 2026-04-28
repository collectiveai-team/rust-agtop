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
        let quota_rows = self.quota.mode.rows_needed();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // header
                Constraint::Min(0),    // sessions
                Constraint::Length(quota_rows),
            ])
            .split(area);

        header::render(frame, layout[0], &self.header, theme);
        self.sessions.render(frame, layout[1], theme);
        if quota_rows > 0 {
            self.quota.render(frame, layout[2], theme);
        }
        // Info drawer overlay — anchored to the SESSIONS area only so it
        // never overlaps the quota panel below. When quota grows (Long
        // mode), sessions shrinks and the drawer follows automatically.
        self.info.render(frame, layout[1], theme);
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

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use crate::tui::screens::dashboard::info_drawer::{DrawerVis, InfoDrawer};
    use crate::tui::screens::dashboard::quota::QuotaMode;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_dashboard(state: &mut DashboardState, w: u16, h: u16) {
        let theme = vscode_dark_plus::theme();
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| state.render(f, Rect::new(0, 0, w, h), &theme))
            .unwrap();
    }

    #[test]
    fn open_drawer_does_not_overlap_quota_panel() {
        // The drawer must anchor to the sessions panel only — never extend
        // into the quota row beneath it. Use Long quota mode so the quota
        // row is tall enough that any leakage from a full-area drawer would
        // be visible.
        let mut state = DashboardState::default();
        state.info = InfoDrawer {
            vis: DrawerVis::Open,
            ..InfoDrawer::default()
        };
        state.quota.mode = QuotaMode::Long;

        let total_w: u16 = 200;
        let total_h: u16 = 40;
        render_dashboard(&mut state, total_w, total_h);

        let drawer_area = state
            .info
            .last_area
            .expect("drawer must have a recorded last_area when open");

        // Recompute the layout the same way DashboardState::render does.
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(state.quota.mode.rows_needed()),
            ])
            .split(Rect::new(0, 0, total_w, total_h));
        let sessions_area = layout[1];
        let quota_area = layout[2];

        // Drawer must be fully inside the sessions area.
        assert!(
            drawer_area.y >= sessions_area.y
                && drawer_area.y + drawer_area.height <= sessions_area.y + sessions_area.height,
            "drawer y-range {:?} must be inside sessions y-range {:?}",
            (drawer_area.y, drawer_area.y + drawer_area.height),
            (sessions_area.y, sessions_area.y + sessions_area.height),
        );

        // It must never overlap the quota panel vertically.
        let drawer_y_end = drawer_area.y + drawer_area.height;
        assert!(
            drawer_y_end <= quota_area.y,
            "drawer extends to row {drawer_y_end} but quota panel starts at row {} — drawer must not overlap quota",
            quota_area.y
        );
    }
}
