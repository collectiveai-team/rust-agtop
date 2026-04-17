//! Pure application state for the TUI.
//!
//! This module deliberately has zero ratatui/crossterm imports. Every
//! piece of logic here — sorting, filtering, selection clamping, input
//! mode transitions — is driven by plain method calls and tested
//! without a terminal backend. The rendering layer in
//! [`super::widgets`] consumes an [`App`] snapshot via shared refs.

mod cost;
mod filter;
mod history;
mod sort;

// ---------------------------------------------------------------------------
// Public re-exports (keep the external API stable)
// ---------------------------------------------------------------------------

pub use cost::cost_rows;
pub use history::{UsageHistory, UsagePoint, CHART_WINDOW_MINS};
pub use sort::{SortColumn, SortDir};

use filter::matches_filter;
use history::provider_idx;
use sort::sort_key;

use std::cell::RefCell;

use chrono::Utc;

use agtop_core::session::SessionAnalysis;

use super::column_config::ColumnConfig;

// ---------------------------------------------------------------------------
// UI mode / Tab / InputMode
// ---------------------------------------------------------------------------

/// Top-level rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    Classic,
    Dashboard,
}

/// Bottom-panel tab selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Info,
    Cost,
    Config,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[Tab::Info, Tab::Cost, Tab::Config]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Cost => "Cost",
            Self::Config => "Config",
        }
    }

    pub fn cycle_forward(self) -> Self {
        match self {
            Self::Info => Self::Cost,
            Self::Cost => Self::Config,
            Self::Config => Self::Info,
        }
    }

    pub fn cycle_back(self) -> Self {
        match self {
            Self::Info => Self::Config,
            Self::Cost => Self::Info,
            Self::Config => Self::Cost,
        }
    }
}

/// Keyboard input mode. In `Normal`, all bindings are active. In `Filter`,
/// printable characters append to the filter buffer and Enter/Esc return
/// to `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// Top-level TUI state.
#[derive(Debug)]
pub struct App {
    /// Every session the backend currently knows about, in the order
    /// returned by `discover_all` (newest first). We do not mutate this
    /// vector after assignment; sorting happens on the view list.
    sessions: Vec<SessionAnalysis>,
    /// Index into `view` for the highlighted row, or `None` when empty.
    selected_idx: Option<usize>,
    /// Session id that *was* selected before the last refresh. Used to
    /// re-find the same row after a background update changes the order
    /// or count — keeps the cursor glued to the user's chosen session.
    sticky_id: Option<String>,
    /// Lowercased filter string (empty = no filter).
    filter: String,
    /// Sort state.
    sort_col: SortColumn,
    sort_dir: SortDir,
    /// Active bottom-panel tab.
    tab: Tab,
    /// Keyboard mode.
    mode: InputMode,
    /// Set to true by the event loop when the user wants to quit.
    should_quit: bool,
    /// Monotonic refresh counter.
    refresh_count: u64,
    /// Last error to surface in the footer. Cleared on the next successful refresh.
    last_error: Option<String>,
    /// Classic table/tabs view vs btop-like dashboard.
    ui_mode: UiMode,
    /// Rolling aggregate usage points for spark/line charts.
    history: UsageHistory,
    /// Plan usage snapshots per provider.
    plan_usage: Vec<agtop_core::PlanUsage>,
    /// Persistent column configuration (visibility + order).
    column_config: ColumnConfig,
    /// Selected row index in the Config tab column list.
    config_cursor: usize,
    /// Cached sorted+filtered view: indices into `self.sessions`.
    /// `None` means stale; recomputed lazily on the next `view()` call.
    /// Interior mutability so `view()` stays `&self` (required by all
    /// widget render functions that borrow `App` immutably).
    view_cache: RefCell<Option<Vec<usize>>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            selected_idx: None,
            sticky_id: None,
            filter: String::new(),
            sort_col: SortColumn::LastActive,
            sort_dir: SortColumn::LastActive.default_direction(),
            tab: Tab::Info,
            mode: InputMode::Normal,
            should_quit: false,
            refresh_count: 0,
            last_error: None,
            ui_mode: UiMode::Classic,
            history: UsageHistory::default(),
            plan_usage: Vec::new(),
            column_config: ColumnConfig::load(),
            config_cursor: 0,
            view_cache: RefCell::new(None),
        }
    }

    // ---- read-only accessors ------------------------------------------------

    pub fn mode(&self) -> InputMode {
        self.mode
    }
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }
    pub fn filter(&self) -> &str {
        &self.filter
    }
    pub fn sort_col(&self) -> SortColumn {
        self.sort_col
    }
    pub fn sort_dir(&self) -> SortDir {
        self.sort_dir
    }
    pub fn tab(&self) -> Tab {
        self.tab
    }
    pub fn refresh_count(&self) -> u64 {
        self.refresh_count
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    pub fn ui_mode(&self) -> UiMode {
        self.ui_mode
    }
    pub fn history(&self) -> &UsageHistory {
        &self.history
    }
    pub fn plan_usage(&self) -> &[agtop_core::PlanUsage] {
        &self.plan_usage
    }
    pub fn sessions(&self) -> &[SessionAnalysis] {
        &self.sessions
    }
    pub fn column_config(&self) -> &ColumnConfig {
        &self.column_config
    }
    /// Save column config to disk (called after mutations in the config tab).
    #[allow(dead_code)]
    pub fn save_column_config(&self) {
        self.column_config.save();
    }

    // ---- Config tab ---------------------------------------------------------

    pub fn config_cursor(&self) -> usize {
        self.config_cursor
    }

    pub fn config_move_up(&mut self) {
        if self.config_cursor > 0 {
            self.config_cursor -= 1;
        }
    }

    pub fn config_move_down(&mut self) {
        let max = self.column_config.columns.len().saturating_sub(1);
        if self.config_cursor < max {
            self.config_cursor += 1;
        }
    }

    pub fn config_toggle(&mut self) {
        self.column_config.toggle(self.config_cursor);
        self.column_config.save();
    }

    pub fn config_move_column_up(&mut self) {
        if self.config_cursor > 0 {
            self.column_config.move_up(self.config_cursor);
            self.config_cursor -= 1;
            self.column_config.save();
        }
    }

    pub fn config_move_column_down(&mut self) {
        let max = self.column_config.columns.len().saturating_sub(1);
        if self.config_cursor < max {
            self.column_config.move_down(self.config_cursor);
            self.config_cursor += 1;
            self.column_config.save();
        }
    }

    /// Build the sorted + filtered view of sessions that the table widget
    /// should render.  Returns `Vec<&SessionAnalysis>` so we avoid cloning
    /// the underlying data on every draw.
    ///
    /// The result is cached (as indices into `self.sessions`) behind
    /// `view_cache` and recomputed only when the cache is explicitly
    /// invalidated by a mutation method.
    pub fn view(&self) -> Vec<&SessionAnalysis> {
        if self.view_cache.borrow().is_none() {
            let mut indexed: Vec<(usize, &SessionAnalysis)> = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, a)| matches_filter(a, &self.filter))
                .collect();
            indexed.sort_by(|(_, a), (_, b)| {
                let ord = sort_key(a, b, self.sort_col);
                match self.sort_dir {
                    SortDir::Asc => ord,
                    SortDir::Desc => ord.reverse(),
                }
            });
            let indices: Vec<usize> = indexed.into_iter().map(|(i, _)| i).collect();
            *self.view_cache.borrow_mut() = Some(indices);
        }
        self.view_cache
            .borrow()
            .as_ref()
            .unwrap()
            .iter()
            .map(|&i| &self.sessions[i])
            .collect()
    }

    /// Convenience: the count of sessions currently visible.
    #[inline]
    pub fn view_len(&self) -> usize {
        self.view().len()
    }

    /// Mark the view cache stale. Must be called after every mutation
    /// that can change which sessions are visible or their order.
    fn invalidate_view_cache(&self) {
        *self.view_cache.borrow_mut() = None;
    }

    /// Total session count, pre-filter.
    pub fn total_count(&self) -> usize {
        self.sessions.len()
    }

    /// Return the selected row as `(index_in_view, SessionAnalysis)` or
    /// `None` when the list is empty.
    pub fn selected(&self) -> Option<(usize, &SessionAnalysis)> {
        let view = self.view();
        let idx = self.selected_idx?;
        view.get(idx).map(|a| (idx, *a))
    }

    pub fn selected_idx(&self) -> Option<usize> {
        self.selected_idx
    }

    // ---- mutations ---------------------------------------------------------

    /// Replace the underlying session list with a fresh snapshot.
    #[allow(dead_code)]
    pub fn set_sessions(&mut self, sessions: Vec<SessionAnalysis>) {
        self.set_snapshot(sessions, self.plan_usage.clone());
    }

    /// Replace sessions + plan-usage in a single atomic swap.
    pub fn set_snapshot(
        &mut self,
        sessions: Vec<SessionAnalysis>,
        plan_usage: Vec<agtop_core::PlanUsage>,
    ) {
        let mut tokens_by_provider = [0u64; 3];
        for a in &sessions {
            let idx = provider_idx(a.summary.provider);
            let tok = a.tokens.grand_total();
            tokens_by_provider[idx] += tok;
        }
        self.history.push(UsagePoint {
            ts: Utc::now(),
            tokens_by_provider,
        });

        self.sessions = sessions;
        self.plan_usage = plan_usage;
        self.refresh_count = self.refresh_count.saturating_add(1);
        self.last_error = None;
        self.invalidate_view_cache();
        self.reconcile_selection();
    }

    /// Record a refresh failure for the footer. Doesn't clear the
    /// existing session list — stale data is strictly better than none.
    pub fn set_refresh_error(&mut self, msg: impl Into<String>) {
        self.last_error = Some(msg.into());
    }

    /// Move the cursor by `delta` rows. Positive = down, negative = up.
    /// Saturates at the ends; no-op on an empty list.
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.view_len();
        if len == 0 {
            self.selected_idx = None;
            self.sticky_id = None;
            return;
        }
        let cur = self.selected_idx.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        self.selected_idx = Some(next);
        self.update_sticky();
    }

    /// Jump to the first visible row.
    pub fn select_first(&mut self) {
        if self.view_len() == 0 {
            self.selected_idx = None;
            self.sticky_id = None;
            return;
        }
        self.selected_idx = Some(0);
        self.update_sticky();
    }

    /// Select a row by its absolute index in the current view. Clamps to
    /// the last valid index; no-op on an empty list.
    pub fn select_at(&mut self, idx: usize) {
        let len = self.view_len();
        if len == 0 {
            self.selected_idx = None;
            self.sticky_id = None;
            return;
        }
        self.selected_idx = Some(idx.min(len - 1));
        self.update_sticky();
    }

    /// Jump to the last visible row.
    pub fn select_last(&mut self) {
        let len = self.view_len();
        if len == 0 {
            self.selected_idx = None;
            self.sticky_id = None;
            return;
        }
        self.selected_idx = Some(len - 1);
        self.update_sticky();
    }

    /// Cycle sort column forward, snapping to the new column's default
    /// direction. Moves the cursor to the top of the new order.
    pub fn cycle_sort_column(&mut self) {
        self.sort_col = self.sort_col.next();
        self.sort_dir = self.sort_col.default_direction();
        self.invalidate_view_cache();
        self.select_first();
    }

    /// Flip the sort direction. Moves the cursor to the top.
    pub fn flip_sort_direction(&mut self) {
        self.sort_dir = self.sort_dir.flip();
        self.invalidate_view_cache();
        self.select_first();
    }

    /// Sort by `col` via mouse click on a header cell.
    /// - If `col` is already the active sort column, toggle the direction.
    /// - Otherwise, switch to `col` using its default direction.
    pub fn set_sort_column(&mut self, col: SortColumn) {
        if self.sort_col == col {
            self.sort_dir = self.sort_dir.flip();
        } else {
            self.sort_col = col;
            self.sort_dir = col.default_direction();
        }
        self.invalidate_view_cache();
        self.select_first();
    }

    pub fn next_tab(&mut self) {
        self.tab = self.tab.cycle_forward();
    }

    pub fn prev_tab(&mut self) {
        self.tab = self.tab.cycle_back();
    }

    /// Directly set the active tab.
    #[allow(dead_code)]
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// Enter filter-input mode.
    pub fn enter_filter_mode(&mut self) {
        self.mode = InputMode::Filter;
    }

    /// Leave filter mode without discarding the filter buffer.
    pub fn confirm_filter(&mut self) {
        self.mode = InputMode::Normal;
    }

    /// Leave filter mode and clear the buffer.
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.mode = InputMode::Normal;
        self.invalidate_view_cache();
        self.reconcile_selection();
    }

    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c.to_ascii_lowercase());
        self.invalidate_view_cache();
        self.reconcile_selection();
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.invalidate_view_cache();
        self.reconcile_selection();
    }

    /// Ask the event loop to exit on its next tick.
    pub fn request_quit(&mut self) {
        self.should_quit = true;
    }

    pub fn toggle_ui_mode(&mut self) {
        self.ui_mode = match self.ui_mode {
            UiMode::Classic => UiMode::Dashboard,
            UiMode::Dashboard => UiMode::Classic,
        };
    }

    pub fn set_ui_mode(&mut self, mode: UiMode) {
        self.ui_mode = mode;
    }

    // ---- internal helpers --------------------------------------------------

    fn update_sticky(&mut self) {
        let view = self.view();
        self.sticky_id = self
            .selected_idx
            .and_then(|i| view.get(i))
            .map(|a| a.summary.session_id.clone());
    }

    /// After a data change (new snapshot, filter edit, sort change), try
    /// to put the cursor back on the same session_id the user selected.
    /// If that session is no longer visible, clamp the cursor into range.
    fn reconcile_selection(&mut self) {
        let (new_idx, new_sticky) = {
            let view = self.view();
            if view.is_empty() {
                (None, None)
            } else if let Some(id) = &self.sticky_id {
                match view.iter().position(|a| &a.summary.session_id == id) {
                    Some(pos) => (Some(pos), Some(id.clone())),
                    None => {
                        let idx = self
                            .selected_idx
                            .unwrap_or(0)
                            .min(view.len().saturating_sub(1));
                        (
                            Some(idx),
                            view.get(idx).map(|a| a.summary.session_id.clone()),
                        )
                    }
                }
            } else {
                let idx = self
                    .selected_idx
                    .unwrap_or(0)
                    .min(view.len().saturating_sub(1));
                (
                    Some(idx),
                    view.get(idx).map(|a| a.summary.session_id.clone()),
                )
            }
        };
        self.selected_idx = new_idx;
        self.sticky_id = new_sticky;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{CostBreakdown, ProviderKind, SessionSummary, TokenTotals};
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;

    fn sample(
        id: &str,
        provider: ProviderKind,
        model: &str,
        cost: f64,
        tokens: u64,
    ) -> SessionAnalysis {
        let summary = SessionSummary::new(
            provider,
            None,
            id.into(),
            Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()),
            Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()),
            Some(model.into()),
            Some("/tmp/proj".into()),
            PathBuf::from(format!("/tmp/{id}.jsonl")),
        );
        let mut token_totals = TokenTotals::default();
        token_totals.input = tokens;
        let mut cost_breakdown = CostBreakdown::default();
        cost_breakdown.total = cost;
        SessionAnalysis::new(
            summary,
            token_totals,
            cost_breakdown,
            Some(model.into()),
            0,
            None,
            Some(0),
            None,
            None,
            None,
        )
    }

    #[test]
    fn empty_app_has_no_selection() {
        let app = App::new();
        assert!(app.selected().is_none());
        assert_eq!(app.view_len(), 0);
        assert_eq!(app.total_count(), 0);
    }

    #[test]
    fn set_sessions_selects_first_row() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Codex, "gpt-5", 2.0, 200),
        ]);
        assert_eq!(app.view_len(), 2);
        let (idx, sel) = app.selected().expect("selection set after first snapshot");
        assert_eq!(idx, 0);
        assert_eq!(sel.summary.session_id, "a");
    }

    #[test]
    fn move_selection_saturates() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Codex, "gpt-5", 2.0, 200),
        ]);
        app.move_selection(-5);
        assert_eq!(app.selected_idx(), Some(0));
        app.move_selection(100);
        assert_eq!(app.selected_idx(), Some(1));
    }

    #[test]
    fn select_first_last() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Codex, "gpt-5", 2.0, 200),
            sample(
                "c",
                ProviderKind::OpenCode,
                "anthropic/claude-haiku-4.5",
                3.0,
                300,
            ),
        ]);
        app.select_last();
        assert_eq!(app.selected_idx(), Some(2));
        app.select_first();
        assert_eq!(app.selected_idx(), Some(0));
    }

    #[test]
    fn filter_matches_id_or_model_or_cwd() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("abcd", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("efgh", ProviderKind::Codex, "gpt-5", 2.0, 200),
        ]);
        app.enter_filter_mode();
        for c in "opus".chars() {
            app.push_filter_char(c);
        }
        assert_eq!(app.view_len(), 1);
        assert_eq!(app.selected().unwrap().1.summary.session_id, "abcd");
        app.clear_filter();
        assert_eq!(app.view_len(), 2);
    }

    #[test]
    fn filter_is_case_insensitive() {
        let mut app = App::new();
        app.set_sessions(vec![sample(
            "abcd",
            ProviderKind::Claude,
            "Claude-Opus-4-6",
            1.0,
            100,
        )]);
        app.enter_filter_mode();
        for c in "CLAUDE".chars() {
            app.push_filter_char(c);
        }
        assert_eq!(app.view_len(), 1);
    }

    #[test]
    fn sort_cost_desc_is_default_for_cost_column() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Claude, "claude-opus-4-6", 3.0, 200),
            sample("c", ProviderKind::Claude, "claude-opus-4-6", 2.0, 300),
        ]);
        while app.sort_col() != SortColumn::Cost {
            app.cycle_sort_column();
        }
        let view = app.view();
        assert_eq!(view[0].summary.session_id, "b");
        assert_eq!(view[2].summary.session_id, "a");
    }

    #[test]
    fn sort_tokens_desc_ranks_high_token_first() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Claude, "claude-opus-4-6", 1.0, 900),
            sample("c", ProviderKind::Claude, "claude-opus-4-6", 1.0, 500),
        ]);
        while app.sort_col() != SortColumn::Tokens {
            app.cycle_sort_column();
        }
        let view = app.view();
        assert_eq!(view[0].summary.session_id, "b");
        assert_eq!(view[1].summary.session_id, "c");
        assert_eq!(view[2].summary.session_id, "a");
    }

    #[test]
    fn flip_sort_direction_inverts_order() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Claude, "claude-opus-4-6", 3.0, 200),
        ]);
        while app.sort_col() != SortColumn::Cost {
            app.cycle_sort_column();
        }
        assert_eq!(app.view()[0].summary.session_id, "b");
        app.flip_sort_direction();
        assert_eq!(app.view()[0].summary.session_id, "a");
    }

    #[test]
    fn selection_follows_session_across_refresh() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Codex, "gpt-5", 2.0, 200),
            sample("c", ProviderKind::OpenCode, "gpt-5", 3.0, 300),
        ]);
        app.move_selection(1);
        assert_eq!(app.selected().unwrap().1.summary.session_id, "b");

        app.set_sessions(vec![
            sample("z", ProviderKind::Claude, "claude-opus-4-6", 5.0, 500),
            sample("b", ProviderKind::Codex, "gpt-5", 2.0, 200),
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("c", ProviderKind::OpenCode, "gpt-5", 3.0, 300),
        ]);
        let (_, sel) = app.selected().unwrap();
        assert_eq!(sel.summary.session_id, "b");
    }

    #[test]
    fn selection_clamps_when_session_disappears() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ProviderKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ProviderKind::Codex, "gpt-5", 2.0, 200),
            sample("c", ProviderKind::OpenCode, "gpt-5", 3.0, 300),
        ]);
        app.move_selection(2);
        app.set_sessions(vec![sample(
            "a",
            ProviderKind::Claude,
            "claude-opus-4-6",
            1.0,
            100,
        )]);
        assert_eq!(app.selected().unwrap().1.summary.session_id, "a");
    }

    #[test]
    fn empty_view_clears_selection() {
        let mut app = App::new();
        app.set_sessions(vec![sample(
            "a",
            ProviderKind::Claude,
            "claude-opus-4-6",
            1.0,
            100,
        )]);
        app.set_sessions(vec![]);
        assert!(app.selected().is_none());
        assert_eq!(app.selected_idx(), None);
    }

    #[test]
    fn tab_cycles() {
        let mut app = App::new();
        assert_eq!(app.tab(), Tab::Info);
        app.next_tab();
        assert_eq!(app.tab(), Tab::Cost);
        app.next_tab();
        assert_eq!(app.tab(), Tab::Config);
        app.next_tab();
        assert_eq!(app.tab(), Tab::Info);
    }

    #[test]
    fn filter_mode_toggles() {
        let mut app = App::new();
        assert_eq!(app.mode(), InputMode::Normal);
        app.enter_filter_mode();
        assert_eq!(app.mode(), InputMode::Filter);
        app.confirm_filter();
        assert_eq!(app.mode(), InputMode::Normal);
    }

    #[test]
    fn pop_filter_char_works() {
        let mut app = App::new();
        app.enter_filter_mode();
        app.push_filter_char('a');
        app.push_filter_char('b');
        assert_eq!(app.filter(), "ab");
        app.pop_filter_char();
        assert_eq!(app.filter(), "a");
    }

    #[test]
    fn refresh_count_increments() {
        let mut app = App::new();
        assert_eq!(app.refresh_count(), 0);
        app.set_sessions(vec![]);
        assert_eq!(app.refresh_count(), 1);
        app.set_sessions(vec![]);
        assert_eq!(app.refresh_count(), 2);
    }
}
