//! Config screen: search + sidebar + detail.

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

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::input::AppEvent;
use crate::tui::msg::{ConfigSection, Msg};
use crate::tui::theme_v2::Theme;
use crate::tui::widgets::icon::Icon;

#[derive(Debug, Default)]
pub struct ConfigState {
    pub current_section: ConfigSection,
    pub detail: detail::DetailModel,
    pub search: String,
    pub search_focused: bool,
    pub nerd_font: bool,
    pub focus: Focus,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Sidebar,
    Detail,
    Search,
}

impl ConfigState {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // search
                Constraint::Min(0),    // body
                Constraint::Length(1), // footer
            ])
            .split(area);

        // Search bar
        let search_icon = Icon::Search.render(self.nerd_font);
        let search_prefix = if !search_icon.is_empty() { format!(" {search_icon} Search: ") } else { " /  Search: ".to_string() };
        let search_style = if self.focus == Focus::Search {
            Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_muted)
        };
        let search_line = Line::from(vec![
            Span::styled(search_prefix, search_style),
            Span::styled(self.search.clone(), Style::default().fg(theme.fg_default)),
        ]);
        frame.render_widget(Paragraph::new(search_line), layout[0]);

        // Body: sidebar + detail
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(22), Constraint::Min(0)])
            .split(layout[1]);

        // Sidebar border highlight when focused
        let sidebar_area = body[0];
        if self.focus == Focus::Sidebar {
            let block = Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(theme.border_focused));
            frame.render_widget(block, sidebar_area);
        }
        sidebar::render(frame, body[0], self.current_section, self.nerd_font, theme);
        detail::render(frame, body[1], self.current_section, &self.detail, theme);

        // Footer
        let footer = Line::from(Span::styled(
            " [↑↓] navigate  [Tab] switch pane  [Enter] edit  [/] search  [Esc] back  [?] help ",
            Style::default().fg(theme.fg_muted),
        ));
        frame.render_widget(Paragraph::new(footer), layout[2]);
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let AppEvent::Key(KeyEvent { code, modifiers, .. }) = event else { return None };

        // Search input absorbs all keys when focused.
        if self.focus == Focus::Search {
            match code {
                KeyCode::Esc => { self.search.clear(); self.focus = Focus::Sidebar; return Some(Msg::Noop); }
                KeyCode::Enter => { self.focus = Focus::Sidebar; return Some(Msg::Noop); }
                KeyCode::Backspace => { self.search.pop(); return Some(Msg::ConfigSearch(self.search.clone())); }
                KeyCode::Char(c) if modifiers.is_empty() || *modifiers == KeyModifiers::SHIFT => {
                    self.search.push(*c);
                    return Some(Msg::ConfigSearch(self.search.clone()));
                }
                _ => return None,
            }
        }

        match code {
            KeyCode::Char('/') => { self.focus = Focus::Search; Some(Msg::Noop) }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Detail,
                    Focus::Detail => Focus::Sidebar,
                    Focus::Search => Focus::Sidebar,
                };
                Some(Msg::Noop)
            }
            KeyCode::Up | KeyCode::Char('k') if self.focus == Focus::Sidebar => {
                self.current_section = prev_section(self.current_section);
                Some(Msg::SelectConfigSection(self.current_section))
            }
            KeyCode::Down | KeyCode::Char('j') if self.focus == Focus::Sidebar => {
                self.current_section = next_section(self.current_section);
                Some(Msg::SelectConfigSection(self.current_section))
            }
            _ => None,
        }
    }
}

fn next_section(s: ConfigSection) -> ConfigSection {
    use ConfigSection::*;
    match s {
        Appearance => Columns,
        Columns => Refresh,
        Refresh => Clients,
        Clients => Keybinds,
        Keybinds => DataSources,
        DataSources => About,
        About => Appearance,
    }
}

fn prev_section(s: ConfigSection) -> ConfigSection {
    use ConfigSection::*;
    match s {
        Appearance => About,
        Columns => Appearance,
        Refresh => Columns,
        Clients => Refresh,
        Keybinds => Clients,
        DataSources => Keybinds,
        About => DataSources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn k(c: char) -> AppEvent {
        AppEvent::Key(KeyEvent {
            code: KeyCode::Char(c), modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press, state: KeyEventState::NONE,
        })
    }

    #[test]
    fn slash_focuses_search() {
        let mut s = ConfigState::default();
        s.handle_event(&k('/'));
        assert_eq!(s.focus, Focus::Search);
    }

    #[test]
    fn search_input_appends_chars() {
        let mut s = ConfigState::default();
        s.focus = Focus::Search;
        s.handle_event(&k('a'));
        s.handle_event(&k('b'));
        assert_eq!(s.search, "ab");
    }

    #[test]
    fn down_arrow_moves_section_when_sidebar_focused() {
        let mut s = ConfigState::default();
        assert_eq!(s.current_section, ConfigSection::Appearance);
        let ev = AppEvent::Key(KeyEvent {
            code: KeyCode::Down, modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press, state: KeyEventState::NONE,
        });
        s.handle_event(&ev);
        assert_eq!(s.current_section, ConfigSection::Columns);
    }
}
