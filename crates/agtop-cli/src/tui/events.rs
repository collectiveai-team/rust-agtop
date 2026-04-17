//! Keyboard → app-action translation.
//!
//! We keep the crossterm dependency confined to this file. The event
//! loop calls [`apply_key`] with a decoded [`KeyEvent`]; downstream
//! logic is handled by mutating [`super::app::App`] via its public
//! methods. This split means we can build a message-free test by
//! constructing synthetic `KeyEvent`s without ever touching the real
//! terminal.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::app::{App, InputMode};

/// A hint returned from [`apply_key`] so the event loop can take
/// side-effect actions the pure state can't model (e.g. "refresh now").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// No external action required.
    None,
    /// Trigger a synchronous / manual refresh of the session list.
    ManualRefresh,
}

/// Translate a crossterm key event into a state mutation + optional
/// side-effect hint. Returns `Action::None` for all pure-state
/// transitions and `Action::ManualRefresh` when the user asks for a
/// new snapshot right now.
///
/// Key bindings mirror `htop` where practical:
/// - q / F10 / Ctrl-C: quit
/// - j / k / arrows: move selection
/// - g / Home, G / End: jump to top / bottom
/// - PgUp / PgDn: move ±10 rows
/// - /: enter filter mode (Esc to exit, Enter to confirm, Backspace to edit)
/// - F6 or >: cycle sort column
/// - i: flip sort direction (Asc ↔ Desc)
/// - F5 or r: manual refresh
/// - Tab / Shift-Tab: cycle bottom panel
/// - d: toggle dashboard/classic layout
pub fn apply_key(app: &mut App, key: KeyEvent) -> Action {
    // ratatui's docs and tui-textarea both recommend ignoring Release
    // events on Windows — they'd otherwise double-fire every binding.
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return Action::None;
    }

    // Ctrl-C always quits regardless of mode; mirrors the --watch loop.
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        app.request_quit();
        return Action::None;
    }

    match app.mode() {
        InputMode::Filter => apply_filter_key(app, key),
        InputMode::Normal => apply_normal_key(app, key),
    }
}

fn apply_filter_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            // Esc clears the filter entirely and returns to normal —
            // this is the "oops, never mind" behavior users expect from
            // htop and fzf.
            app.clear_filter();
        }
        KeyCode::Enter => {
            // Enter commits the current buffer and returns to normal
            // mode. The buffer is NOT cleared.
            app.confirm_filter();
        }
        KeyCode::Backspace => app.pop_filter_char(),
        KeyCode::Char(c) => {
            // Ignore Ctrl-<letter> sequences while typing; they mostly
            // collide with terminal bindings (Ctrl-C handled above).
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.push_filter_char(c);
            }
        }
        _ => {}
    }
    Action::None
}

fn apply_normal_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') | KeyCode::F(10) => {
            app.request_quit();
        }
        KeyCode::Char('j') | KeyCode::Down => app.move_selection(1),
        KeyCode::Char('k') | KeyCode::Up => app.move_selection(-1),
        KeyCode::PageDown => app.move_selection(10),
        KeyCode::PageUp => app.move_selection(-10),
        KeyCode::Home | KeyCode::Char('g') => app.select_first(),
        KeyCode::End | KeyCode::Char('G') => app.select_last(),
        KeyCode::Char('/') => app.enter_filter_mode(),
        KeyCode::Esc => {
            // Esc in normal mode clears any active filter — handy when
            // the filter has scrolled the row of interest off-screen.
            app.clear_filter();
        }
        KeyCode::F(6) | KeyCode::Char('>') => app.cycle_sort_column(),
        KeyCode::Char('i') => app.flip_sort_direction(),
        KeyCode::F(5) | KeyCode::Char('r') => return Action::ManualRefresh,
        KeyCode::Char('d') => app.toggle_ui_mode(),
        KeyCode::Tab => app.next_tab(),
        KeyCode::BackTab => app.prev_tab(),
        _ => {}
    }
    Action::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn q_quits() {
        let mut app = App::new();
        assert_eq!(apply_key(&mut app, press(KeyCode::Char('q'))), Action::None);
        assert!(app.should_quit());
    }

    #[test]
    fn ctrl_c_quits_in_any_mode() {
        let mut app = App::new();
        app.enter_filter_mode();
        apply_key(&mut app, ctrl(KeyCode::Char('c')));
        assert!(app.should_quit());
    }

    #[test]
    fn slash_enters_filter_mode() {
        let mut app = App::new();
        apply_key(&mut app, press(KeyCode::Char('/')));
        assert_eq!(app.mode(), InputMode::Filter);
    }

    #[test]
    fn filter_mode_captures_chars() {
        let mut app = App::new();
        apply_key(&mut app, press(KeyCode::Char('/')));
        for c in ['o', 'p', 'u', 's'] {
            apply_key(&mut app, press(KeyCode::Char(c)));
        }
        assert_eq!(app.filter(), "opus");
    }

    #[test]
    fn esc_clears_filter() {
        let mut app = App::new();
        apply_key(&mut app, press(KeyCode::Char('/')));
        apply_key(&mut app, press(KeyCode::Char('x')));
        apply_key(&mut app, press(KeyCode::Esc));
        assert_eq!(app.filter(), "");
        assert_eq!(app.mode(), InputMode::Normal);
    }

    #[test]
    fn enter_in_filter_mode_confirms() {
        let mut app = App::new();
        apply_key(&mut app, press(KeyCode::Char('/')));
        apply_key(&mut app, press(KeyCode::Char('a')));
        apply_key(&mut app, press(KeyCode::Enter));
        assert_eq!(app.filter(), "a");
        assert_eq!(app.mode(), InputMode::Normal);
    }

    #[test]
    fn r_triggers_manual_refresh() {
        let mut app = App::new();
        assert_eq!(
            apply_key(&mut app, press(KeyCode::Char('r'))),
            Action::ManualRefresh
        );
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = App::new();
        let release = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };
        apply_key(&mut app, release);
        assert!(!app.should_quit());
    }
}
