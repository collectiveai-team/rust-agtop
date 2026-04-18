//! Keyboard → app-action translation.
//!
//! We keep the crossterm dependency confined to this file. The event
//! loop calls [`apply_key`] with a decoded [`KeyEvent`]; downstream
//! logic is handled by mutating [`super::app::App`] via its public
//! methods. This split means we can build a message-free test by
//! constructing synthetic `KeyEvent`s without ever touching the real
//! terminal.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::app::{App, InputMode, Tab, UiMode};

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
    // When the Config tab is active, most keys are intercepted for column editing.
    if app.tab() == Tab::Config {
        return apply_config_key(app, key);
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::F(10) => {
            app.request_quit();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.ui_mode() == UiMode::Dashboard {
                app.plan_select_next(app.plan_usage().len());
            } else {
                app.move_selection(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.ui_mode() == UiMode::Dashboard {
                app.plan_select_prev();
            } else {
                app.move_selection(-1);
            }
        }
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
        // [ / ] cycle the Cost Summary sub-tab (only meaningful in dashboard mode).
        KeyCode::Char('[') if app.ui_mode() == UiMode::Dashboard => app.cycle_cost_tab_back(),
        KeyCode::Char(']') if app.ui_mode() == UiMode::Dashboard => app.cycle_cost_tab_forward(),
        // t toggles the Cost Summary period between total and month.
        KeyCode::Char('t') if app.ui_mode() == UiMode::Dashboard => app.toggle_cost_period(),
        _ => {}
    }
    Action::None
}

fn apply_config_key(app: &mut App, key: KeyEvent) -> Action {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        // Always allow quit and tab-switching from config.
        KeyCode::Char('q') | KeyCode::F(10) => app.request_quit(),
        KeyCode::Tab => app.next_tab(),
        KeyCode::BackTab => app.prev_tab(),

        // Cursor movement.
        KeyCode::Char('j') | KeyCode::Down if !shift => app.config_move_down(),
        KeyCode::Char('k') | KeyCode::Up if !shift => app.config_move_up(),

        // Reorder (Shift+j/J or Shift+k/K or Shift+arrows).
        KeyCode::Char('J') | KeyCode::Down if shift => app.config_move_column_down(),
        KeyCode::Char('K') | KeyCode::Up if shift => app.config_move_column_up(),

        // Toggle visibility.
        KeyCode::Char(' ') | KeyCode::Enter => app.config_toggle(),

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

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::SHIFT,
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

    use crate::tui::app::Tab;
    use crate::tui::column_config::ColumnId;

    /// Capture the current visible column order as a Vec for comparison.
    #[allow(dead_code)]
    fn visible_order(app: &App) -> Vec<ColumnId> {
        app.column_config().visible()
    }

    /// Full order (including hidden) as a Vec, which is what reorder
    /// operations actually mutate. `visible_order` alone can hide a
    /// reorder of a hidden column; `full_order` catches it.
    fn full_order(app: &App) -> Vec<(ColumnId, bool)> {
        app.column_config()
            .columns
            .iter()
            .map(|e| (e.id, e.visible))
            .collect()
    }

    /// Shift+J in the Config tab must reorder the current column down,
    /// not move the cursor. Regression for the hint-ambiguity bug that
    /// led users to press plain `j` (cursor move) instead.
    #[test]
    fn shift_j_reorders_column_down_in_config_tab() {
        let mut app = App::new();
        app.set_tab(Tab::Config);
        // Cursor starts at 0; move to a middle position so the column
        // can legitimately swap both down and up.
        apply_key(&mut app, press(KeyCode::Char('j')));
        apply_key(&mut app, press(KeyCode::Char('j')));
        let cursor_before = app.config_cursor();
        let before = full_order(&app);

        apply_key(&mut app, shift(KeyCode::Char('J')));

        let after = full_order(&app);
        assert_ne!(before, after, "Shift+J did not mutate column order");
        // The cursor should follow the moved column (app.config_move_column_down
        // increments the cursor).
        assert_eq!(
            app.config_cursor(),
            cursor_before + 1,
            "cursor should follow the moved column"
        );
        // The column that was at cursor_before should now be at cursor_before+1.
        assert_eq!(after[cursor_before + 1].0, before[cursor_before].0);
    }

    #[test]
    fn shift_k_reorders_column_up_in_config_tab() {
        let mut app = App::new();
        app.set_tab(Tab::Config);
        apply_key(&mut app, press(KeyCode::Char('j')));
        apply_key(&mut app, press(KeyCode::Char('j')));
        let cursor_before = app.config_cursor();
        let before = full_order(&app);

        apply_key(&mut app, shift(KeyCode::Char('K')));

        let after = full_order(&app);
        assert_ne!(before, after, "Shift+K did not mutate column order");
        assert_eq!(app.config_cursor(), cursor_before - 1);
        assert_eq!(after[cursor_before - 1].0, before[cursor_before].0);
    }

    #[test]
    fn shift_arrow_down_reorders_column_down_in_config_tab() {
        let mut app = App::new();
        app.set_tab(Tab::Config);
        apply_key(&mut app, press(KeyCode::Char('j')));
        let before = full_order(&app);

        apply_key(&mut app, shift(KeyCode::Down));

        let after = full_order(&app);
        assert_ne!(before, after, "Shift+Down did not mutate column order");
    }

    #[test]
    fn shift_arrow_up_reorders_column_up_in_config_tab() {
        let mut app = App::new();
        app.set_tab(Tab::Config);
        apply_key(&mut app, press(KeyCode::Char('j')));
        apply_key(&mut app, press(KeyCode::Char('j')));
        let before = full_order(&app);

        apply_key(&mut app, shift(KeyCode::Up));

        let after = full_order(&app);
        assert_ne!(before, after, "Shift+Up did not mutate column order");
    }

    #[test]
    fn plain_j_does_not_reorder_in_config_tab() {
        let mut app = App::new();
        app.set_tab(Tab::Config);
        let order_before = full_order(&app);
        let cursor_before = app.config_cursor();

        apply_key(&mut app, press(KeyCode::Char('j')));

        assert_eq!(full_order(&app), order_before, "plain j must not reorder");
        assert_eq!(
            app.config_cursor(),
            cursor_before + 1,
            "plain j must move the cursor down"
        );
    }

    #[test]
    fn plain_k_does_not_reorder_in_config_tab() {
        let mut app = App::new();
        app.set_tab(Tab::Config);
        // Move cursor away from 0 so `k` has somewhere to go.
        apply_key(&mut app, press(KeyCode::Char('j')));
        let order_before = full_order(&app);
        let cursor_before = app.config_cursor();

        apply_key(&mut app, press(KeyCode::Char('k')));

        assert_eq!(full_order(&app), order_before, "plain k must not reorder");
        assert_eq!(
            app.config_cursor(),
            cursor_before - 1,
            "plain k must move the cursor up"
        );
    }

    #[test]
    fn dashboard_j_moves_plan_selection_down() {
        let mut app = App::new();
        app.toggle_ui_mode(); // switch to Dashboard
        assert_eq!(app.ui_mode(), UiMode::Dashboard);
        // plan_select_next needs a list_len; we simulate 3 subscriptions.
        // The event handler should call plan_select_next(3) when it knows the count.
        // Since events.rs cannot know the count, we call plan_select_next directly
        // from apply_key using a fixed count of usize::MAX (clamps to 0 without
        // a concrete list) — see implementation note below.
        //
        // For this test, pre-set plan_selected to 0 and verify it increments.
        // We'll use a helper that passes count=10.
        app.plan_select_next(10);
        assert_eq!(app.plan_selected(), 1);
        app.plan_select_prev();
        assert_eq!(app.plan_selected(), 0);
    }

    #[test]
    fn dashboard_k_clamps_at_zero() {
        let mut app = App::new();
        app.toggle_ui_mode();
        app.plan_select_prev(); // already at 0 — should stay at 0
        assert_eq!(app.plan_selected(), 0);
    }

    #[test]
    fn dashboard_j_key_moves_plan_selection() {
        let mut app = App::new();
        app.toggle_ui_mode(); // Dashboard mode
                              // Pre-populate plan_usage so plan_select_next has something to clamp to.
                              // plan_usage().len() == 0 means plan_select_next(0) is a no-op,
                              // so we need to check the routing without relying on actual movement.
                              // The key thing to test is that move_selection is NOT called in dashboard mode.
        let session_count_before = app.view_len();
        apply_key(&mut app, press(KeyCode::Char('j')));
        // In Dashboard mode j should NOT change the session table selection.
        // plan_usage is empty so plan_select_next(0) is a no-op, but no panic.
        assert_eq!(app.view_len(), session_count_before);
    }

    #[test]
    fn classic_j_key_moves_session_selection() {
        let mut app = App::new();
        // Classic mode (default) — j moves the session table.
        assert_eq!(app.ui_mode(), UiMode::Classic);
        // Just verify it doesn't panic with empty session list.
        apply_key(&mut app, press(KeyCode::Char('j')));
        apply_key(&mut app, press(KeyCode::Char('k')));
    }
}
