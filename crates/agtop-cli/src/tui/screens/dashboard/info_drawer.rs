//! Info drawer: bottom-right floating panel with Summary/General/Costs/Process tabs.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::screens::dashboard::sessions::SessionRow;
use crate::tui::theme_v2::Theme;
use crate::tui::widgets::drawer::{self, Anchor};
use super::{info_costs, info_general, info_process, info_summary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawerVis {
    #[default]
    Closed,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InfoTab {
    #[default]
    Summary,
    General,
    Costs,
    Process,
}

impl InfoTab {
    pub const ALL: [InfoTab; 4] = [Self::Summary, Self::General, Self::Costs, Self::Process];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::General => "General",
            Self::Costs => "Costs",
            Self::Process => "Process",
        }
    }
}

#[derive(Debug, Default)]
pub struct InfoDrawer {
    pub vis: DrawerVis,
    pub tab: InfoTab,
    /// Selected session row from the table; drives all tab bodies.
    pub selected_row: Option<SessionRow>,
    /// Last area occupied by the drawer (set during render). Used to block
    /// click-through to the sessions table behind the drawer.
    pub last_area: Option<Rect>,
}

impl InfoDrawer {
    /// Sync the selected row from the sessions table. Call after every
    /// selection change.
    pub fn set_row(&mut self, row: Option<SessionRow>) {
        self.selected_row = row;
    }

    /// Convenience accessor for the selected session id (for the drawer title).
    fn selected_session_id(&self) -> Option<&str> {
        self.selected_row
            .as_ref()
            .map(|r| r.analysis.summary.session_id.as_str())
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, parent: Rect, theme: &Theme) {
        if self.vis == DrawerVis::Closed {
            self.last_area = None;
            return;
        }
        let area = drawer::rect_for(parent, Anchor::BottomRight, 0.5, 0.6);
        self.last_area = Some(area);

        let id_str = self.selected_session_id().unwrap_or("—");
        let tabs_full  = " [1] Summary  [2] General  [3] Costs  [4] Process ";
        let tabs_short = " [1]Sum [2]Gen [3]Cost [4]Proc ";
        let id_part = format!(" Session: {id_str} ");
        let tabs_part = if (area.width as usize) >= id_part.len() + tabs_full.len() {
            tabs_full
        } else {
            tabs_short
        };
        let title = format!("{id_part}{tabs_part}");
        let inner = drawer::render_chrome(frame, area, &title, theme);

        // Tab body — dispatch to real content modules when a row is selected;
        // show a friendly placeholder otherwise.
        match (self.tab, self.selected_row.as_ref()) {
            (InfoTab::Summary, Some(row)) => {
                use agtop_core::session::SessionState;
                let state = row
                    .analysis
                    .session_state
                    .clone()
                    .unwrap_or(SessionState::Closed);
                let model = info_summary::SummaryModel {
                    analysis: &row.analysis,
                    client_label: &row.client_label,
                    client_kind: row.client_kind,
                    state: &state,
                    recent_turns: vec![],
                    nerd_font: false,
                };
                info_summary::render(frame, inner, &model, theme);
            }
            (InfoTab::General, Some(row)) => {
                info_general::render(frame, inner, &row.analysis, theme);
            }
            (InfoTab::Costs, Some(row)) => {
                info_costs::render(frame, inner, &row.analysis, theme);
            }
            (InfoTab::Process, Some(row)) => {
                info_process::render(frame, inner, &row.analysis, &[], theme);
            }
            (_, None) => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "No session selected — press j/k or click a row",
                        Style::default().fg(theme.fg_muted),
                    ))),
                    inner,
                );
            }
        }
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let AppEvent::Key(KeyEvent { code, modifiers, .. }) = event else { return None };
        if !modifiers.is_empty() && *modifiers != KeyModifiers::SHIFT { return None; }
        match code {
            KeyCode::Char('i') if self.vis == DrawerVis::Open => {
                self.vis = DrawerVis::Closed;
                Some(Msg::Noop)
            }
            KeyCode::Esc if self.vis == DrawerVis::Open => {
                self.vis = DrawerVis::Closed;
                Some(Msg::Noop)
            }
            KeyCode::Char('i') => {
                self.vis = DrawerVis::Open;
                self.tab = InfoTab::Summary;
                Some(Msg::Noop)
            }
            KeyCode::Char('1') if self.vis == DrawerVis::Open => { self.tab = InfoTab::Summary; Some(Msg::Noop) }
            KeyCode::Char('2') if self.vis == DrawerVis::Open => { self.tab = InfoTab::General; Some(Msg::Noop) }
            KeyCode::Char('3') if self.vis == DrawerVis::Open => { self.tab = InfoTab::Costs; Some(Msg::Noop) }
            KeyCode::Char('4') if self.vis == DrawerVis::Open => { self.tab = InfoTab::Process; Some(Msg::Noop) }
            KeyCode::Tab if self.vis == DrawerVis::Open => {
                self.tab = match self.tab {
                    InfoTab::Summary => InfoTab::General,
                    InfoTab::General => InfoTab::Costs,
                    InfoTab::Costs   => InfoTab::Process,
                    InfoTab::Process => InfoTab::Summary,
                };
                Some(Msg::Noop)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn k(c: char) -> AppEvent {
        AppEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn i_toggles_open_close() {
        let mut d = InfoDrawer::default();
        assert_eq!(d.vis, DrawerVis::Closed);
        d.handle_event(&k('i'));
        assert_eq!(d.vis, DrawerVis::Open);
        d.handle_event(&k('i'));
        assert_eq!(d.vis, DrawerVis::Closed);
    }

    #[test]
    fn open_drawer_defaults_to_summary_tab() {
        let mut d = InfoDrawer::default();
        d.handle_event(&k('i'));
        assert_eq!(d.tab, InfoTab::Summary);
    }

    #[test]
    fn tab_keys_switch_tabs_when_open() {
        let mut d = InfoDrawer::default();
        d.handle_event(&k('i'));
        d.handle_event(&k('2'));
        assert_eq!(d.tab, InfoTab::General);
        d.handle_event(&k('3'));
        assert_eq!(d.tab, InfoTab::Costs);
        d.handle_event(&k('4'));
        assert_eq!(d.tab, InfoTab::Process);
    }

    #[test]
    fn tab_keys_inert_when_closed() {
        let mut d = InfoDrawer::default();
        d.handle_event(&k('2'));
        assert_eq!(d.vis, DrawerVis::Closed);
        assert_eq!(d.tab, InfoTab::Summary); // unchanged from default
    }
}
