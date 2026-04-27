//! Info drawer: bottom-right floating panel with Summary/General/Costs/Process tabs.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::theme_v2::Theme;
use crate::tui::widgets::drawer::{self, Anchor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoTab {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawerVis { Open, Closed }

#[derive(Debug, Default)]
pub struct InfoDrawer {
    pub vis: DrawerVis,
    pub tab: InfoTab,
    /// Selected session id from the table; the drawer reads its content from this.
    pub selected_session_id: Option<String>,
}

impl Default for DrawerVis { fn default() -> Self { Self::Closed } }
impl Default for InfoTab  { fn default() -> Self { Self::Summary } }

impl InfoDrawer {
    pub fn render(&self, frame: &mut Frame<'_>, parent: Rect, theme: &Theme) {
        if self.vis == DrawerVis::Closed { return; }
        let area = drawer::rect_for(parent, Anchor::BottomRight, 0.5, 0.6);
        let title = format!(
            " Session: {id}  [1] Summary  [2] General  [3] Costs  [4] Process ",
            id = self.selected_session_id.as_deref().unwrap_or("—"),
        );
        let inner = drawer::render_chrome(frame, area, &title, theme);
        // Tab body — content wired in Tasks 15-18.
        match self.tab {
            InfoTab::Summary => {
                frame.render_widget(Paragraph::new("(Summary content — wired in Task 18)"), inner);
            }
            InfoTab::General | InfoTab::Costs | InfoTab::Process => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            format!("[{}] active", self.tab.label()),
                            Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD),
                        ),
                    ])),
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
