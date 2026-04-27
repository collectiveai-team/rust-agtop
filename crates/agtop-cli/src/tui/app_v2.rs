//! New top-level App with `Screen` dispatch. Coexists with `tui::app` during migration.
// Foundation code for Plans 2-4.
#![allow(dead_code)]

use ratatui::{layout::{Constraint, Direction, Layout, Rect}, Frame};

use crate::tui::input::AppEvent;
use crate::tui::msg::{Msg, ScreenId};
use crate::tui::screens::{aggregation::AggregationState, config::ConfigState, dashboard::DashboardState};
use crate::tui::theme_v2::{self, Theme};
use crate::tui::widgets::tab_bar;

pub struct App {
    pub current: ScreenId,
    pub theme: Theme,
    pub show_help: bool,
    pub running: bool,

    pub dashboard: DashboardState,
    pub aggregation: AggregationState,
    pub config: ConfigState,
}

impl Default for App {
    fn default() -> Self {
        Self {
            current: ScreenId::Dashboard,
            theme: theme_v2::vscode_dark_plus::theme(),
            show_help: false,
            running: true,
            dashboard: DashboardState::default(),
            aggregation: Default::default(),
            config: Default::default(),
        }
    }
}

impl App {
    /// Apply a `Msg` to the App state.
    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::SwitchScreen(id) => self.current = id,
            Msg::ShowHelp => self.show_help = true,
            Msg::Escape => self.show_help = false,
            Msg::Quit => self.running = false,
            Msg::Noop => {}
        }
    }

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        tab_bar::render(frame, layout[0], self.current, env!("CARGO_PKG_VERSION"), &self.theme);
        match self.current {
            ScreenId::Dashboard => self.dashboard.render(frame, layout[1], &self.theme),
            ScreenId::Aggregation => self.aggregation.render(frame, layout[1], &self.theme),
            ScreenId::Config => self.config.render(frame, layout[1], &self.theme),
        }
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        // Global keymap first.
        if let AppEvent::Key(k) = event {
            if let Some(m) = self.global_keymap(*k) { return Some(m); }
        }
        // Then route to active screen.
        match self.current {
            ScreenId::Dashboard => self.dashboard.handle_event(event),
            ScreenId::Aggregation => None,
            ScreenId::Config => None,
        }
    }
}

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    /// Translate a global keypress into a `Msg`, or `None` if the key should
    /// be routed to the active screen instead.
    #[must_use]
    pub fn global_keymap(&self, key: KeyEvent) -> Option<Msg> {
        if !key.modifiers.is_empty() && key.modifiers != KeyModifiers::SHIFT {
            return None;
        }
        match key.code {
            KeyCode::Char('q') => Some(Msg::Quit),
            KeyCode::Char('?') => Some(Msg::ShowHelp),
            KeyCode::Char('d') => Some(Msg::SwitchScreen(ScreenId::Dashboard)),
            KeyCode::Char('a') => Some(Msg::SwitchScreen(ScreenId::Aggregation)),
            KeyCode::Char('c') => Some(Msg::SwitchScreen(ScreenId::Config)),
            KeyCode::Esc => {
                if self.show_help {
                    Some(Msg::Escape)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_screen_changes_current() {
        let mut app = App::default();
        app.update(Msg::SwitchScreen(ScreenId::Aggregation));
        assert_eq!(app.current, ScreenId::Aggregation);
    }

    #[test]
    fn quit_clears_running() {
        let mut app = App::default();
        assert!(app.running);
        app.update(Msg::Quit);
        assert!(!app.running);
    }

    #[test]
    fn help_toggle_is_show_then_escape() {
        let mut app = App::default();
        app.update(Msg::ShowHelp);
        assert!(app.show_help);
        app.update(Msg::Escape);
        assert!(!app.show_help);
    }
}

#[cfg(test)]
mod keymap_tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn k(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn q_quits() {
        let app = App::default();
        assert_eq!(app.global_keymap(k('q')), Some(Msg::Quit));
    }

    #[test]
    fn d_switches_to_dashboard() {
        let app = App::default();
        assert_eq!(app.global_keymap(k('d')), Some(Msg::SwitchScreen(ScreenId::Dashboard)));
    }

    #[test]
    fn a_switches_to_aggregation() {
        let app = App::default();
        assert_eq!(app.global_keymap(k('a')), Some(Msg::SwitchScreen(ScreenId::Aggregation)));
    }

    #[test]
    fn c_switches_to_config() {
        let app = App::default();
        assert_eq!(app.global_keymap(k('c')), Some(Msg::SwitchScreen(ScreenId::Config)));
    }

    #[test]
    fn ctrl_q_does_not_quit() {
        let mut key = k('q');
        key.modifiers = KeyModifiers::CONTROL;
        let app = App::default();
        assert_eq!(app.global_keymap(key), None);
    }
}
