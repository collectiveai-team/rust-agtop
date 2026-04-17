//! Pure application state for the TUI.
//!
//! This module deliberately has zero ratatui/crossterm imports. Every
//! piece of logic here — sorting, filtering, selection clamping, input
//! mode transitions — is driven by plain method calls and tested
//! without a terminal backend. The rendering layer in
//! [`super::widgets`] consumes an [`App`] snapshot via shared refs.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use agtop_core::session::{CostBreakdown, ProviderKind, SessionAnalysis, TokenTotals};

// ---------------------------------------------------------------------------
// Rolling usage history (for dashboard charts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UsagePoint {
    pub ts: DateTime<Utc>,
    pub tokens_by_provider: [u64; 3],
}

#[derive(Debug, Default)]
pub struct UsageHistory {
    points: VecDeque<UsagePoint>,
}

pub const CHART_WINDOW_MINS: i64 = 60;
const RETENTION_SECS: i64 = CHART_WINDOW_MINS * 60 * 2;

impl UsageHistory {
    pub fn push(&mut self, point: UsagePoint) {
        let cutoff = point.ts - chrono::Duration::seconds(RETENTION_SECS);
        self.points.push_back(point);
        while self.points.front().is_some_and(|p| p.ts < cutoff) {
            self.points.pop_front();
        }
    }

    pub fn points(&self) -> &VecDeque<UsagePoint> {
        &self.points
    }

    pub fn buckets_by_provider(
        &self,
        now: DateTime<Utc>,
        n_buckets: usize,
        provider: ProviderKind,
    ) -> Vec<u64> {
        self.buckets_by_provider_idx(now, n_buckets, provider_idx(provider))
    }

    fn buckets_by_provider_idx(
        &self,
        now: DateTime<Utc>,
        n_buckets: usize,
        provider_idx: usize,
    ) -> Vec<u64> {
        if n_buckets == 0 {
            return Vec::new();
        }
        let window_secs = CHART_WINDOW_MINS * 60;
        let bucket_secs = (window_secs / n_buckets as i64).max(1);
        let window_start = now - chrono::Duration::seconds(window_secs);
        let mut out = vec![0u64; n_buckets];

        for p in &self.points {
            if p.ts < window_start {
                continue;
            }
            let age_secs = (now - p.ts).num_seconds().max(0);
            let bucket_from_end = (age_secs / bucket_secs) as usize;
            if bucket_from_end >= n_buckets {
                continue;
            }
            let bucket = n_buckets - 1 - bucket_from_end;
            let v = p.tokens_by_provider[provider_idx];
            out[bucket] = out[bucket].max(v);
        }

        out
    }
}

fn provider_idx(kind: ProviderKind) -> usize {
    match kind {
        ProviderKind::Claude => 0,
        ProviderKind::Codex => 1,
        ProviderKind::OpenCode => 2,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    Classic,
    Dashboard,
}

/// Columns the user can sort the session table by. Cycles via `F6` / `>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    /// Last-active timestamp (descending = most recent first). Default.
    LastActive,
    /// Provider name, then session id (ascending, alphabetical).
    Provider,
    /// Session started-at timestamp (descending = newest first).
    Started,
    /// Model string (ascending). Unknowns sort last.
    Model,
    /// Total dollar cost (descending). Included sessions count as 0.
    Cost,
    /// Grand-total token count (descending).
    Tokens,
    /// Output-only token count (descending).
    OutputTokens,
    /// Cache token total (read + write, descending).
    CacheTokens,
}

impl SortColumn {
    /// Column immediately after `self` in the cycle order. Wraps around.
    pub fn next(self) -> Self {
        match self {
            Self::LastActive => Self::Provider,
            Self::Provider => Self::Started,
            Self::Started => Self::Model,
            Self::Model => Self::Cost,
            Self::Cost => Self::Tokens,
            Self::Tokens => Self::OutputTokens,
            Self::OutputTokens => Self::CacheTokens,
            Self::CacheTokens => Self::LastActive,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LastActive => "last-active",
            Self::Provider => "provider",
            Self::Started => "started",
            Self::Model => "model",
            Self::Cost => "cost",
            Self::Tokens => "tokens",
            Self::OutputTokens => "output",
            Self::CacheTokens => "cache",
        }
    }

    /// The natural / most-useful direction for the column. LastActive,
    /// Cost, and Tokens read best from high-to-low; Provider/Model read
    /// best alphabetically. Users can flip with the `>` prefix key.
    pub fn default_direction(self) -> SortDir {
        match self {
            Self::LastActive
            | Self::Started
            | Self::Cost
            | Self::Tokens
            | Self::OutputTokens
            | Self::CacheTokens => SortDir::Desc,
            Self::Provider | Self::Model => SortDir::Asc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn flip(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }
}

/// Bottom-panel tab selector. Only Info/Cost in the MVP; other tabs are
/// a follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Info,
    Cost,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[Tab::Info, Tab::Cost]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Cost => "Cost",
        }
    }

    pub fn cycle_forward(self) -> Self {
        match self {
            Self::Info => Self::Cost,
            Self::Cost => Self::Info,
        }
    }

    pub fn cycle_back(self) -> Self {
        self.cycle_forward()
    }
}

/// What the keyboard is currently doing. In `Normal`, all bindings are
/// active. In `Filter`, printable characters append to the filter buffer
/// and Enter/Esc return to `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
}

/// Top-level TUI state.
#[derive(Debug)]
pub struct App {
    /// Every session the backend currently knows about, in the order
    /// returned by `discover_all` (newest first). We do not mutate this
    /// vector after assignment; sorting happens on the view list.
    sessions: Vec<SessionAnalysis>,
    /// Index into `view` for the highlighted row, or `None` when empty.
    /// Stored as `Option<usize>` so the "no selection" state is explicit
    /// rather than an implicit 0.
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
    /// Monotonic refresh counter. Incremented each time a fresh snapshot
    /// is swapped in; useful for status-bar footers and tests.
    refresh_count: u64,
    /// Last error we want to surface in the footer (e.g. refresh
    /// failure). Cleared on the next successful refresh.
    last_error: Option<String>,
    /// Classic table/tabs view vs btop-like dashboard.
    ui_mode: UiMode,
    /// Rolling aggregate usage points for spark/line charts.
    history: UsageHistory,
    /// Plan usage snapshots per provider.
    plan_usage: Vec<agtop_core::PlanUsage>,
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

    /// Build the sorted + filtered view of sessions that the table
    /// widget should render. Returns `Vec<&SessionAnalysis>` so we avoid
    /// cloning the underlying data on every draw; the borrow is valid
    /// until `App` is next mutated.
    pub fn view(&self) -> Vec<&SessionAnalysis> {
        let mut v: Vec<&SessionAnalysis> = self
            .sessions
            .iter()
            .filter(|a| matches_filter(a, &self.filter))
            .collect();
        sort_view(&mut v, self.sort_col, self.sort_dir);
        v
    }

    /// Convenience: the count of sessions currently visible.
    pub fn view_len(&self) -> usize {
        self.sessions
            .iter()
            .filter(|a| matches_filter(a, &self.filter))
            .count()
    }

    /// Total session count, pre-filter.
    pub fn total_count(&self) -> usize {
        self.sessions.len()
    }

    /// Return the selected row as `(index_in_view, SessionAnalysis)` or
    /// `None` when the list is empty. Walks the view so the caller does
    /// not need to recompute it.
    pub fn selected(&self) -> Option<(usize, &SessionAnalysis)> {
        let view = self.view();
        let idx = self.selected_idx?;
        view.get(idx).map(|a| (idx, *a))
    }

    pub fn selected_idx(&self) -> Option<usize> {
        self.selected_idx
    }

    // ---- mutations ---------------------------------------------------------

    /// Replace the underlying session list with a fresh snapshot. Tries
    /// to preserve the cursor on whichever session was selected before
    /// by matching on `session_id`; otherwise clamps to a valid row.
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
    /// direction so the first press of `>` from any state lands on a
    /// "sensible" ordering. Moves the cursor to the top of the new order.
    pub fn cycle_sort_column(&mut self) {
        self.sort_col = self.sort_col.next();
        self.sort_dir = self.sort_col.default_direction();
        self.select_first();
    }

    /// Flip the sort direction (ascending ↔ descending). Moves the cursor
    /// to the top so the user immediately sees the new first row.
    pub fn flip_sort_direction(&mut self) {
        self.sort_dir = self.sort_dir.flip();
        self.select_first();
    }

    /// Sort by `col` via mouse click on a header cell.
    /// - If `col` is already the active sort column, toggle the direction.
    /// - Otherwise, switch to `col` using its default direction.
    ///   In both cases the cursor jumps to the top of the new order.
    pub fn set_sort_column(&mut self, col: SortColumn) {
        if self.sort_col == col {
            self.sort_dir = self.sort_dir.flip();
        } else {
            self.sort_col = col;
            self.sort_dir = col.default_direction();
        }
        self.select_first();
    }

    pub fn next_tab(&mut self) {
        self.tab = self.tab.cycle_forward();
    }

    pub fn prev_tab(&mut self) {
        self.tab = self.tab.cycle_back();
    }

    /// Directly set the active tab. Currently only used from tests and
    /// prefs-restore paths; kept in the public API so the upcoming UI
    /// prefs persistence (v0.2 follow-up) can reinstate saved state.
    #[allow(dead_code)]
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// Enter filter-input mode. Preserves any existing filter string so
    /// `/` + Enter is a quick "edit current filter" gesture.
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
        self.reconcile_selection();
    }

    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c.to_ascii_lowercase());
        self.reconcile_selection();
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
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
        // Resolve all view-dependent values before touching `self`
        // again; otherwise the borrow checker (correctly) rejects the
        // follow-up writes because `view` still borrows `self`.
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

/// Does `a` match the current text filter? Empty filter → match all.
/// The filter is lowercased by `push_filter_char`; we lowercase the
/// haystack fields inline. We match against session id (short + full),
/// model, effective_model, cwd, and provider name, which covers the
/// cases users actually search for.
fn matches_filter(a: &SessionAnalysis, filter_lower: &str) -> bool {
    if filter_lower.is_empty() {
        return true;
    }
    let s = &a.summary;
    let candidates: [Option<&str>; 5] = [
        Some(s.session_id.as_str()),
        s.model.as_deref(),
        a.effective_model.as_deref(),
        s.cwd.as_deref(),
        Some(s.provider.as_str()),
    ];
    candidates
        .iter()
        .flatten()
        .any(|hay| hay.to_ascii_lowercase().contains(filter_lower))
}

/// In-place sort by the user's chosen column + direction. Stable so
/// secondary ordering (mtime-desc from `discover_all`) is preserved
/// within groups.
fn sort_view(view: &mut [&SessionAnalysis], col: SortColumn, dir: SortDir) {
    use std::cmp::Ordering;
    view.sort_by(|a, b| {
        let ord = match col {
            SortColumn::LastActive => a.summary.last_active.cmp(&b.summary.last_active),
            SortColumn::Provider => {
                let p = a.summary.provider.as_str().cmp(b.summary.provider.as_str());
                if p == Ordering::Equal {
                    a.summary.session_id.cmp(&b.summary.session_id)
                } else {
                    p
                }
            }
            SortColumn::Started => a.summary.started_at.cmp(&b.summary.started_at),
            SortColumn::Model => {
                cmp_opt_str(a.summary.model.as_deref(), b.summary.model.as_deref())
            }
            SortColumn::Cost => a
                .cost
                .total
                .partial_cmp(&b.cost.total)
                .unwrap_or(Ordering::Equal),
            SortColumn::Tokens => grand_total(&a.tokens).cmp(&grand_total(&b.tokens)),
            SortColumn::OutputTokens => a.tokens.output.cmp(&b.tokens.output),
            SortColumn::CacheTokens => cache_total(&a.tokens).cmp(&cache_total(&b.tokens)),
        };
        match dir {
            SortDir::Asc => ord,
            SortDir::Desc => ord.reverse(),
        }
    });
}

/// Treat `None` as "sorts after everything" regardless of direction so
/// unknown-model rows never fight for the top slot.
fn cmp_opt_str(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn grand_total(t: &TokenTotals) -> u64 {
    t.grand_total()
}

fn cache_total(t: &TokenTotals) -> u64 {
    t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input
}

/// Small helper used by the Cost tab to format a single row uniformly.
/// Lives here (rather than in widgets) so it's unit-testable without a
/// backend.
pub fn cost_row(label: &'static str, tokens: u64, dollars: f64) -> (&'static str, String, String) {
    (label, format_tokens(tokens), format_dollars(dollars))
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}G", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.2}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn format_dollars(d: f64) -> String {
    if d == 0.0 {
        "-".into()
    } else {
        // Four decimal places everywhere — session costs are typically
        // in the $0.001–$10 range, so a uniform width keeps columns
        // aligned without hiding sub-cent figures.
        format!("${:.4}", d)
    }
}

/// Public render-helper: the cost tab needs a predictable set of rows,
/// in a predictable order, whether or not every token bucket is
/// populated. Returning them from a pure function keeps the widget
/// trivial and snapshot-testable.
pub fn cost_rows(
    tokens: &TokenTotals,
    cost: &CostBreakdown,
) -> Vec<(&'static str, String, String)> {
    vec![
        cost_row("input", tokens.input, cost.input),
        cost_row("cached_input", tokens.cached_input, cost.cached_input),
        cost_row("output", tokens.output, cost.output),
        cost_row("cache_write_5m", tokens.cache_write_5m, cost.cache_write_5m),
        cost_row("cache_write_1h", tokens.cache_write_1h, cost.cache_write_1h),
        cost_row("cache_read", tokens.cache_read, cost.cache_read),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{ProviderKind, SessionSummary};
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;

    fn sample(
        id: &str,
        provider: ProviderKind,
        model: &str,
        cost: f64,
        tokens: u64,
    ) -> SessionAnalysis {
        SessionAnalysis {
            summary: SessionSummary {
                provider,
                session_id: id.into(),
                started_at: Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()),
                last_active: Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()),
                model: Some(model.into()),
                cwd: Some("/tmp/proj".into()),
                data_path: PathBuf::from(format!("/tmp/{id}.jsonl")),
            },
            tokens: TokenTotals {
                input: tokens,
                ..Default::default()
            },
            cost: CostBreakdown {
                total: cost,
                ..Default::default()
            },
            effective_model: Some(model.into()),
            subagent_file_count: 0,
            tool_call_count: None,
            duration_secs: Some(0),
            context_used_pct: None,
        }
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
        // Default column is LastActive; cycle forward to Cost.
        while app.sort_col() != SortColumn::Cost {
            app.cycle_sort_column();
        }
        let view = app.view();
        assert_eq!(view[0].summary.session_id, "b"); // highest cost first
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
        app.move_selection(1); // select "b"
        assert_eq!(app.selected().unwrap().1.summary.session_id, "b");

        // Simulate an updated snapshot where the order changes and a new
        // row is prepended. Selection should stay pinned to "b".
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
        app.move_selection(2); // select "c"
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
