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
pub mod quota;
mod sort;

// ---------------------------------------------------------------------------
// Public re-exports (keep the external API stable)
// ---------------------------------------------------------------------------

pub use cost::cost_rows;
pub use history::{UsageHistory, UsagePoint, CHART_WINDOW_MINS};
pub use sort::{SortColumn, SortDir};

use filter::matches_filter;
use history::client_idx;
use sort::sort_key;

use std::cell::RefCell;

use chrono::Utc;

use agtop_core::session::SessionAnalysis;

use super::column_config::ColumnConfig;
use agtop_core::quota::ProviderResult;

/// Pre-rendered logo cells for one provider. The number of cells equals
/// the logo column width (currently 3). At render time we just `clone`
/// these cells into the target buffer instead of going through the full
/// `Image::render` path on every frame, which used to cost ~30 ms per
/// frame on the Kitty graphics protocol because the placeholder escape
/// sequence was rebuilt for every visible row on every redraw.
pub type LogoCells = Vec<ratatui::buffer::Cell>;

struct LogoMap(std::collections::HashMap<agtop_core::ClientKind, LogoCells>);

impl std::fmt::Debug for LogoMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LogoMap").field(&self.0.len()).finish()
    }
}

// ---------------------------------------------------------------------------
// UI mode / Tab / InputMode
// ---------------------------------------------------------------------------

/// Which section the Config tab cursor is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSection {
    Clients,
    Columns,
}

/// Top-level rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    Classic,
    Dashboard,
}

/// Bottom-panel tab selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Info,
    Process,
    Cost,
    Config,
    Quota,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[Tab::Info, Tab::Process, Tab::Cost, Tab::Config, Tab::Quota]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Process => "Process",
            Self::Cost => "Cost",
            Self::Config => "Config",
            Self::Quota => "Quota",
        }
    }

    pub fn cycle_forward(self) -> Self {
        match self {
            Self::Info => Self::Process,
            Self::Process => Self::Cost,
            Self::Cost => Self::Config,
            Self::Config => Self::Quota,
            Self::Quota => Self::Info,
        }
    }

    pub fn cycle_back(self) -> Self {
        match self {
            Self::Info => Self::Quota,
            Self::Process => Self::Info,
            Self::Cost => Self::Process,
            Self::Config => Self::Cost,
            Self::Quota => Self::Config,
        }
    }
}

/// Sub-tab for the Cost Summary dashboard panel (group-by dimension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostTab {
    /// Group costs by agentic client (Claude Code, Codex, OpenCode).
    Client,
    /// Group costs by billing subscription (Claude Max, ChatGPT Plus, …).
    Subscription,
    /// Group costs by model name.
    Model,
    /// Group costs by project working directory.
    Project,
}

impl CostTab {
    pub fn all() -> &'static [CostTab] {
        &[
            CostTab::Client,
            CostTab::Subscription,
            CostTab::Model,
            CostTab::Project,
        ]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Client => "Client",
            Self::Subscription => "Subscription",
            Self::Model => "Model",
            Self::Project => "Project",
        }
    }

    pub fn cycle_forward(self) -> Self {
        match self {
            Self::Client => Self::Subscription,
            Self::Subscription => Self::Model,
            Self::Model => Self::Project,
            Self::Project => Self::Client,
        }
    }

    pub fn cycle_back(self) -> Self {
        match self {
            Self::Client => Self::Project,
            Self::Subscription => Self::Client,
            Self::Model => Self::Subscription,
            Self::Project => Self::Model,
        }
    }
}

/// Time-period filter for the Cost Summary dashboard panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostPeriod {
    /// Show all-time totals.
    Total,
    /// Show only the current calendar month.
    Month,
}

impl CostPeriod {
    pub fn toggle(self) -> Self {
        match self {
            Self::Total => Self::Month,
            Self::Month => Self::Total,
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
// Quota state
// ---------------------------------------------------------------------------

/// One slot per provider, tracking the most recent fetch and the
/// most recent successful fetch. Rendering policy defined in the spec:
/// - (None, ok)    → normal render
/// - (None, err)   → error row, no gauges
/// - (Some, ok)    → normal render
/// - (Some, err)   → stale gauges + inline warning
#[derive(Debug, Clone)]
pub struct ProviderSlot {
    pub last_good: Option<ProviderResult>,
    pub current: ProviderResult,
}

impl ProviderSlot {
    /// Create a fresh slot from the first fetch result for a provider.
    /// If the result is ok, it becomes both `current` and `last_good`.
    pub fn new(result: ProviderResult) -> Self {
        let last_good = if result.ok {
            Some(result.clone())
        } else {
            None
        };
        Self {
            last_good,
            current: result,
        }
    }

    /// Upsert a new fetch result into this slot.
    /// - `current` is always replaced.
    /// - `last_good` is replaced only if the new result is ok.
    pub fn upsert(&mut self, result: ProviderResult) {
        if result.ok {
            self.last_good = Some(result.clone());
        }
        self.current = result;
    }
}

/// Top-level state of the quota subsystem as seen by the UI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum QuotaState {
    /// Quota pane has never been opened in this session.
    #[default]
    Idle,
    /// First fetch is in flight; no slot results yet.
    #[allow(dead_code)]
    Loading,
    /// At least one fetch cycle has completed; slots may be populated.
    Ready,
    /// First fetch failed before any result arrived. `String` is the error message.
    Error(String),
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
    /// Plan usage snapshots per client.
    plan_usage: Vec<agtop_core::PlanUsage>,
    /// Active sub-tab in the Cost Summary dashboard panel.
    cost_tab: CostTab,
    /// Period filter (total vs current month) for the Cost Summary panel.
    cost_period: CostPeriod,
    /// Scroll offset for the Cost Summary breakdown rows (0 = top).
    cost_scroll: usize,
    /// Selected subscription index in the plan-usage list (dashboard mode).
    plan_selected: usize,
    /// Persistent column configuration (visibility + order).
    column_config: ColumnConfig,
    /// Selected row index in the Config tab column list.
    config_cursor: usize,
    /// Cached sorted+filtered view: indices into `self.sessions`.
    /// `None` means stale; recomputed lazily on the next `view()` call.
    /// Interior mutability so `view()` stays `&self` (required by all
    /// widget render functions that borrow `App` immutably).
    view_cache: RefCell<Option<Vec<usize>>>,
    /// Session IDs that are currently expanded to show their children.
    expanded_sessions: std::collections::HashSet<String>,
    /// Shared enabled-client set. Some when running under the TUI (set
    /// by `tui::run`); None in unit tests that don't need the wire-up.
    enabled_arc: Option<
        std::sync::Arc<std::sync::RwLock<std::collections::HashSet<agtop_core::ClientKind>>>,
    >,
    logos: LogoMap,
    /// Quota subsystem state.
    quota_slots: Vec<ProviderSlot>,
    /// Coarse state: idle/loading/ready/error. Drives full-pane placeholder rendering.
    quota_state: QuotaState,
    /// Index into `quota_slots` for the selected provider (Dashboard pane).
    /// Clamped in accessor, not written-through.
    selected_provider: usize,
    /// Scroll offset for the Google per-model list within the selected provider's detail.
    /// Reset to 0 when `selected_provider` changes.
    model_scroll: usize,
    /// Horizontal scroll offset for the Classic Quota tab card row.
    /// Leftmost visible card index.
    card_scroll: usize,
    /// Active theme. Initialized from the VS Code Dark+ palette.
    /// Plan 4 will wire this to a user-configurable setting.
    pub theme: crate::tui::theme_v2::Theme,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let column_config = ColumnConfig::load();
        let sort_col = column_config.sort_col;
        let sort_dir = column_config.sort_dir;
        Self {
            sessions: Vec::new(),
            selected_idx: None,
            sticky_id: None,
            filter: String::new(),
            sort_col,
            sort_dir,
            tab: Tab::Info,
            mode: InputMode::Normal,
            should_quit: false,
            refresh_count: 0,
            last_error: None,
            ui_mode: UiMode::Classic,
            history: UsageHistory::default(),
            plan_usage: Vec::new(),
            cost_tab: CostTab::Client,
            cost_period: CostPeriod::Total,
            cost_scroll: 0,
            plan_selected: 0,
            column_config,
            config_cursor: 0,
            view_cache: RefCell::new(None),
            expanded_sessions: std::collections::HashSet::new(),
            enabled_arc: None,
            logos: LogoMap(std::collections::HashMap::new()),
            quota_slots: Vec::new(),
            quota_state: QuotaState::default(),
            selected_provider: 0,
            model_scroll: 0,
            card_scroll: 0,
            theme: crate::tui::theme_v2::vscode_dark_plus::theme(),
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
    #[allow(dead_code)]
    pub fn plan_usage(&self) -> &[agtop_core::PlanUsage] {
        &self.plan_usage
    }
    pub fn quota_slots(&self) -> &[ProviderSlot] {
        &self.quota_slots
    }
    pub fn quota_state(&self) -> &QuotaState {
        &self.quota_state
    }
    pub fn selected_provider(&self) -> usize {
        self.selected_provider
            .min(self.quota_slots.len().saturating_sub(1))
    }
    #[allow(dead_code)]
    pub fn model_scroll(&self) -> usize {
        self.model_scroll
    }
    pub fn card_scroll(&self) -> usize {
        self.card_scroll
    }
    pub fn cost_tab(&self) -> CostTab {
        self.cost_tab
    }
    pub fn cost_period(&self) -> CostPeriod {
        self.cost_period
    }
    pub fn cost_scroll(&self) -> usize {
        self.cost_scroll
    }
    #[allow(dead_code)]
    pub fn plan_selected(&self) -> usize {
        self.plan_selected
    }

    /// Move the subscription list selection down by 1, clamped to the list length.
    #[allow(dead_code)]
    pub fn plan_select_next(&mut self, list_len: usize) {
        if list_len > 0 {
            self.plan_selected = (self.plan_selected + 1).min(list_len - 1);
        }
    }

    /// Move the subscription list selection up by 1, clamped to 0.
    #[allow(dead_code)]
    pub fn plan_select_prev(&mut self) {
        self.plan_selected = self.plan_selected.saturating_sub(1);
    }

    pub fn sessions(&self) -> &[SessionAnalysis] {
        &self.sessions
    }
    pub fn column_config(&self) -> &ColumnConfig {
        &self.column_config
    }

    /// Mutable access to the column configuration (used in tests for setup).
    #[cfg(test)]
    pub(crate) fn column_config_mut(&mut self) -> &mut ColumnConfig {
        &mut self.column_config
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

    pub fn set_config_cursor(&mut self, idx: usize) {
        let max = self.config_total_rows().saturating_sub(1);
        self.config_cursor = idx.min(max);
    }

    /// Total virtual rows in the Config tab: clients + columns.
    pub fn config_total_rows(&self) -> usize {
        self.column_config.clients.len() + self.column_config.columns.len()
    }

    /// Which section the current cursor is in.
    pub fn config_section_at(&self, idx: usize) -> ConfigSection {
        if idx < self.column_config.clients.len() {
            ConfigSection::Clients
        } else {
            ConfigSection::Columns
        }
    }

    /// Section-local index. Callers combine this with `config_section_at`.
    pub fn config_local_idx(&self, idx: usize) -> usize {
        match self.config_section_at(idx) {
            ConfigSection::Clients => idx,
            ConfigSection::Columns => idx - self.column_config.clients.len(),
        }
    }

    pub fn config_move_up(&mut self) {
        if self.config_cursor > 0 {
            self.config_cursor -= 1;
        }
    }

    pub fn config_move_down(&mut self) {
        let max = self.config_total_rows().saturating_sub(1);
        if self.config_cursor < max {
            self.config_cursor += 1;
        }
    }

    /// Toggle whatever item the cursor is on. Shared by keyboard (Space/Enter)
    /// and mouse (click on row).
    pub fn toggle_cursor_item(&mut self) {
        match self.config_section_at(self.config_cursor) {
            ConfigSection::Clients => {
                let local = self.config_local_idx(self.config_cursor);
                self.column_config.toggle_client(local);
                if let Some(arc) = &self.enabled_arc {
                    if let Ok(mut guard) = arc.write() {
                        *guard = self.column_config.enabled_clients();
                    }
                }
            }
            ConfigSection::Columns => {
                let local = self.config_local_idx(self.config_cursor);
                self.column_config.toggle(local);
                self.column_config.save();
            }
        }
    }

    /// Reorder only applies to the Columns section; no-op when the cursor
    /// sits on a client row. Keeps keyboard shortcuts harmless.
    pub fn config_move_column_up(&mut self) {
        if self.config_section_at(self.config_cursor) != ConfigSection::Columns {
            return;
        }
        let local = self.config_local_idx(self.config_cursor);
        if local > 0 {
            self.column_config.move_up(local);
            self.config_cursor -= 1;
            self.column_config.save();
        }
    }

    pub fn config_move_column_down(&mut self) {
        if self.config_section_at(self.config_cursor) != ConfigSection::Columns {
            return;
        }
        let local = self.config_local_idx(self.config_cursor);
        let col_max = self.column_config.columns.len().saturating_sub(1);
        if local < col_max {
            self.column_config.move_down(local);
            self.config_cursor += 1;
            self.column_config.save();
        }
    }

    /// Read-only snapshot of the currently-enabled clients (for tests
    /// and for seeding the shared Arc<RwLock<...>> at startup).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn enabled_clients_set(&self) -> std::collections::HashSet<agtop_core::ClientKind> {
        self.column_config.enabled_clients()
    }

    pub fn attach_enabled_arc(
        &mut self,
        arc: std::sync::Arc<std::sync::RwLock<std::collections::HashSet<agtop_core::ClientKind>>>,
    ) {
        self.enabled_arc = Some(arc);
    }

    pub fn set_logos(
        &mut self,
        logos: std::collections::HashMap<agtop_core::ClientKind, LogoCells>,
    ) {
        self.logos = LogoMap(logos);
    }

    pub fn logo(&self, client: agtop_core::ClientKind) -> Option<&LogoCells> {
        self.logos.0.get(&client)
    }

    /// True when at least one logo is loaded. Callers that need to
    /// reserve a logo column (session table, info tab) should hide
    /// that column when this is false — e.g. on terminals without a
    /// graphics protocol where the logo would otherwise be invisible
    /// noise.
    pub fn has_logos(&self) -> bool {
        !self.logos.0.is_empty()
    }

    /// Backward-compat alias used by events.rs until it's updated.
    pub fn config_toggle(&mut self) {
        self.toggle_cursor_item();
    }

    /// Build the sorted + filtered view of sessions that the table widget
    /// should render.  Returns `Vec<&SessionAnalysis>` so we avoid cloning
    /// the underlying data on every draw.
    ///
    /// The result is cached (as indices into `self.sessions`) behind
    /// `view_cache` and recomputed only when the cache is explicitly
    /// invalidated by a mutation method.
    pub fn view(&self) -> Vec<&SessionAnalysis> {
        self.ensure_view_cache();
        self.iter_with_kinds().into_iter().map(|(a, _)| a).collect()
    }

    /// Like `view()` but also returns whether each entry is a child row.
    pub fn view_with_kinds(&self) -> Vec<(&SessionAnalysis, bool)> {
        self.ensure_view_cache();
        self.iter_with_kinds()
    }

    /// Toggle expanded state for a session.
    ///
    /// Note: does NOT invalidate the view cache. The cache stores only sorted
    /// parent indices; expansion state is applied on top at read time, so the
    /// cache remains valid after a toggle.
    pub fn toggle_expand(&mut self, session_id: &str) {
        if self.expanded_sessions.contains(session_id) {
            self.expanded_sessions.remove(session_id);
        } else {
            self.expanded_sessions.insert(session_id.to_owned());
        }
        self.reconcile_selection();
    }

    // ---- private view helpers ----------------------------------------------

    /// Ensure the view cache (sorted+filtered parent indices) is populated.
    fn ensure_view_cache(&self) {
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
    }

    /// Iterate over all visible rows (parents + expanded children) with a
    /// boolean flag indicating whether each row is a child.
    ///
    /// Caller must ensure the view cache is populated before calling this.
    fn iter_with_kinds(&self) -> Vec<(&SessionAnalysis, bool)> {
        let mut result = Vec::new();
        for &i in self.view_cache.borrow().as_ref().unwrap().iter() {
            let parent = &self.sessions[i];
            result.push((parent, false));
            if !parent.children.is_empty()
                && self.expanded_sessions.contains(&parent.summary.session_id)
            {
                for child in &parent.children {
                    result.push((child, true));
                }
            }
        }
        result
    }

    /// Returns true if the given session is currently expanded.
    pub fn is_expanded(&self, session_id: &str) -> bool {
        self.expanded_sessions.contains(session_id)
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
        let mut tokens_by_client = [0u64; 7];
        for a in &sessions {
            let idx = client_idx(a.summary.client);
            let tok = a.tokens.grand_total();
            tokens_by_client[idx] += tok;
            for c in &a.children {
                tokens_by_client[idx] += c.tokens.grand_total();
            }
        }
        self.history.push(UsagePoint {
            ts: Utc::now(),
            tokens_by_client,
        });

        self.sessions = sessions;
        self.plan_usage = plan_usage;
        // Clamp plan_selected so it never points past the end of the new vec.
        let plan_len = self.plan_usage.len();
        if plan_len == 0 {
            self.plan_selected = 0;
        } else {
            self.plan_selected = self.plan_selected.min(plan_len - 1);
        }
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
        self.column_config.set_sort(self.sort_col, self.sort_dir);
        self.invalidate_view_cache();
        self.select_first();
    }

    /// Flip the sort direction. Moves the cursor to the top.
    pub fn flip_sort_direction(&mut self) {
        self.sort_dir = self.sort_dir.flip();
        self.column_config.set_sort(self.sort_col, self.sort_dir);
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
        self.column_config.set_sort(self.sort_col, self.sort_dir);
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

    /// Cycle the Cost Summary panel sub-tab forward (Client → Subscription → Model → Project → …).
    pub fn cycle_cost_tab_forward(&mut self) {
        self.cost_tab = self.cost_tab.cycle_forward();
        self.cost_scroll = 0;
    }

    /// Cycle the Cost Summary panel sub-tab backward.
    pub fn cycle_cost_tab_back(&mut self) {
        self.cost_tab = self.cost_tab.cycle_back();
        self.cost_scroll = 0;
    }

    /// Directly set the active Cost Summary sub-tab (e.g. on mouse click).
    pub fn set_cost_tab(&mut self, tab: CostTab) {
        self.cost_tab = tab;
        self.cost_scroll = 0;
    }

    /// Toggle the Cost Summary period between Total and Month.
    pub fn toggle_cost_period(&mut self) {
        self.cost_period = self.cost_period.toggle();
        self.cost_scroll = 0;
    }

    /// Directly set the Cost Summary period (e.g. on mouse click).
    pub fn set_cost_period(&mut self, period: CostPeriod) {
        self.cost_period = period;
        self.cost_scroll = 0;
    }

    /// Scroll the Cost Summary breakdown rows down by `delta`.
    /// `total_rows` is the number of data rows so we can clamp.
    pub fn scroll_cost_down(&mut self, delta: usize, total_rows: usize, visible_rows: usize) {
        let max = total_rows.saturating_sub(visible_rows);
        self.cost_scroll = (self.cost_scroll + delta).min(max);
    }

    /// Scroll the Cost Summary breakdown rows up by `delta`.
    pub fn scroll_cost_up(&mut self, delta: usize) {
        self.cost_scroll = self.cost_scroll.saturating_sub(delta);
    }

    pub fn set_ui_mode(&mut self, mode: UiMode) {
        self.ui_mode = mode;
    }

    /// Merge a batch of fetch results into `quota_slots`, upserting by
    /// `provider_id`. Always transitions state to `QuotaState::Ready`.
    ///
    /// Slot preservation: existing slots for providers NOT in `results`
    /// are left untouched. This matches the spec's policy of keeping
    /// last-known-good around.
    pub fn apply_quota_results(&mut self, results: Vec<ProviderResult>) {
        for result in results {
            if let Some(existing) = self
                .quota_slots
                .iter_mut()
                .find(|s| s.current.provider_id == result.provider_id)
            {
                existing.upsert(result);
            } else {
                self.quota_slots.push(ProviderSlot::new(result));
            }
        }
        self.quota_state = QuotaState::Ready;
    }

    /// Set `QuotaState::Loading`. Typically called when a `QuotaCmd::Start`
    /// is dispatched to the worker.
    #[allow(dead_code)]
    pub fn set_quota_loading(&mut self) {
        self.quota_state = QuotaState::Loading;
    }

    /// Surface a fetch-level error. Only transitions to `Error` if we
    /// haven't yet reached `Ready`; once `Ready`, per-slot `current.error`
    /// carries per-provider errors instead.
    pub fn set_quota_error(&mut self, message: String) {
        if self.quota_state != QuotaState::Ready {
            self.quota_state = QuotaState::Error(message);
        }
    }

    /// Advance the selected provider index by 1, clamping at the last slot.
    /// Resets `model_scroll` to 0 on change.
    pub fn quota_select_next(&mut self) {
        let len = self.quota_slots.len();
        if len == 0 {
            return;
        }
        let before = self.selected_provider;
        self.selected_provider = (self.selected_provider + 1).min(len - 1);
        if self.selected_provider != before {
            self.model_scroll = 0;
        }
    }

    /// Decrement the selected provider index by 1, clamping at 0.
    /// Resets `model_scroll` to 0 on change.
    pub fn quota_select_prev(&mut self) {
        let before = self.selected_provider;
        self.selected_provider = self.selected_provider.saturating_sub(1);
        if self.selected_provider != before {
            self.model_scroll = 0;
        }
    }

    /// Scroll the Classic Quota tab card row left by 1 (clamped at 0).
    pub fn quota_card_scroll_left(&mut self) {
        self.card_scroll = self.card_scroll.saturating_sub(1);
    }

    /// Scroll the Classic Quota tab card row right by 1.
    /// `cards_visible` is how many cards fit in the current render area.
    /// Clamped at `quota_slots.len().saturating_sub(cards_visible)`.
    pub fn quota_card_scroll_right(&mut self, cards_visible: usize) {
        let max = self.quota_slots.len().saturating_sub(cards_visible.max(1));
        self.card_scroll = (self.card_scroll + 1).min(max);
    }

    /// Test-only helper to set `model_scroll` directly. Production code
    /// should not need this; `quota_select_*` is the normal path.
    #[cfg(test)]
    pub fn set_model_scroll_for_test(&mut self, v: usize) {
        self.model_scroll = v;
    }

    pub fn model_scroll_down(&mut self, visible_rows: usize) {
        let max = self.model_scroll_max(visible_rows);
        if self.model_scroll < max {
            self.model_scroll += 1;
        }
    }

    pub fn model_scroll_up(&mut self) {
        self.model_scroll = self.model_scroll.saturating_sub(1);
    }

    fn model_scroll_max(&self, visible_rows: usize) -> usize {
        let slot = self.quota_slots.get(self.selected_provider);
        let total = slot
            .and_then(|s| s.current.usage.as_ref())
            .map(|u| u.models.len())
            .unwrap_or(0)
            .max(1);
        total.saturating_sub(visible_rows)
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
    use agtop_core::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;

    fn sample(
        id: &str,
        client: ClientKind,
        model: &str,
        cost: f64,
        tokens: u64,
    ) -> SessionAnalysis {
        let summary = SessionSummary::new(
            client,
            None,
            id.into(),
            Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()),
            Some(Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap()),
            Some(model.into()),
            Some("/tmp/proj".into()),
            PathBuf::from(format!("/tmp/{id}.jsonl")),
            None,
            None,
            None,
            None,
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

    fn sample_with_children(parent_id: &str, child_ids: &[&str]) -> SessionAnalysis {
        let mut parent = sample(parent_id, ClientKind::Claude, "claude-opus-4-6", 1.0, 100);
        parent.children = child_ids
            .iter()
            .map(|id| sample(id, ClientKind::Claude, "claude-opus-4-6", 0.5, 50))
            .collect();
        parent
    }

    #[test]
    fn toggle_expand_shows_children() {
        let mut app = App::new();
        app.set_sessions(vec![sample_with_children("parent", &["c1", "c2"])]);
        assert_eq!(app.view_len(), 1);
        app.toggle_expand("parent");
        assert_eq!(app.view_len(), 3);
    }

    #[test]
    fn toggle_expand_twice_collapses() {
        let mut app = App::new();
        app.set_sessions(vec![sample_with_children("parent", &["c1", "c2"])]);
        app.toggle_expand("parent");
        assert_eq!(app.view_len(), 3);
        app.toggle_expand("parent");
        assert_eq!(app.view_len(), 1);
    }

    #[test]
    fn expand_survives_refresh() {
        let mut app = App::new();
        let sessions = vec![sample_with_children("parent", &["c1"])];
        app.set_sessions(sessions.clone());
        app.toggle_expand("parent");
        assert_eq!(app.view_len(), 2);
        app.set_sessions(sessions);
        assert_eq!(app.view_len(), 2);
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
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Codex, "gpt-5", 2.0, 200),
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
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Codex, "gpt-5", 2.0, 200),
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
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Codex, "gpt-5", 2.0, 200),
            sample(
                "c",
                ClientKind::OpenCode,
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
            sample("abcd", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("efgh", ClientKind::Codex, "gpt-5", 2.0, 200),
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
            ClientKind::Claude,
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
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Claude, "claude-opus-4-6", 3.0, 200),
            sample("c", ClientKind::Claude, "claude-opus-4-6", 2.0, 300),
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
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Claude, "claude-opus-4-6", 1.0, 900),
            sample("c", ClientKind::Claude, "claude-opus-4-6", 1.0, 500),
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
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Claude, "claude-opus-4-6", 3.0, 200),
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
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Codex, "gpt-5", 2.0, 200),
            sample("c", ClientKind::OpenCode, "gpt-5", 3.0, 300),
        ]);
        app.move_selection(1);
        assert_eq!(app.selected().unwrap().1.summary.session_id, "b");

        app.set_sessions(vec![
            sample("z", ClientKind::Claude, "claude-opus-4-6", 5.0, 500),
            sample("b", ClientKind::Codex, "gpt-5", 2.0, 200),
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("c", ClientKind::OpenCode, "gpt-5", 3.0, 300),
        ]);
        let (_, sel) = app.selected().unwrap();
        assert_eq!(sel.summary.session_id, "b");
    }

    #[test]
    fn selection_clamps_when_session_disappears() {
        let mut app = App::new();
        app.set_sessions(vec![
            sample("a", ClientKind::Claude, "claude-opus-4-6", 1.0, 100),
            sample("b", ClientKind::Codex, "gpt-5", 2.0, 200),
            sample("c", ClientKind::OpenCode, "gpt-5", 3.0, 300),
        ]);
        app.move_selection(2);
        app.set_sessions(vec![sample(
            "a",
            ClientKind::Claude,
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
            ClientKind::Claude,
            "claude-opus-4-6",
            1.0,
            100,
        )]);
        app.set_sessions(vec![]);
        assert!(app.selected().is_none());
        assert_eq!(app.selected_idx(), None);
    }

    #[test]
    fn tab_process_is_between_info_and_cost() {
        assert_eq!(Tab::Info.cycle_forward(), Tab::Process);
        assert_eq!(Tab::Process.cycle_forward(), Tab::Cost);
        assert_eq!(Tab::Cost.cycle_back(), Tab::Process);
        assert_eq!(Tab::Process.cycle_back(), Tab::Info);
    }

    #[test]
    fn tab_cycles() {
        let mut app = App::new();
        assert_eq!(app.tab(), Tab::Info);
        app.next_tab();
        assert_eq!(app.tab(), Tab::Process);
        app.next_tab();
        assert_eq!(app.tab(), Tab::Cost);
        app.next_tab();
        assert_eq!(app.tab(), Tab::Config);
        app.next_tab();
        assert_eq!(app.tab(), Tab::Quota);
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

    #[test]
    fn plan_select_next_increments() {
        let mut app = App::new();
        app.plan_select_next(3);
        assert_eq!(app.plan_selected(), 1);
    }

    #[test]
    fn plan_select_next_clamps_at_end() {
        let mut app = App::new();
        app.plan_select_next(1); // list of 1, already at max
        assert_eq!(app.plan_selected(), 0);
        app.plan_select_next(2); // moves to 1
        assert_eq!(app.plan_selected(), 1);
        app.plan_select_next(2); // already at max (index 1 = last in list of 2)
        assert_eq!(app.plan_selected(), 1);
    }

    #[test]
    fn plan_select_next_noop_when_empty() {
        let mut app = App::new();
        app.plan_select_next(0);
        assert_eq!(app.plan_selected(), 0);
    }

    #[test]
    fn plan_select_prev_clamps_at_zero() {
        let mut app = App::new();
        app.plan_select_prev();
        assert_eq!(app.plan_selected(), 0);
    }

    #[test]
    fn plan_select_prev_decrements() {
        let mut app = App::new();
        app.plan_select_next(3);
        app.plan_select_next(3);
        assert_eq!(app.plan_selected(), 2);
        app.plan_select_prev();
        assert_eq!(app.plan_selected(), 1);
    }

    #[test]
    fn config_cursor_walks_clients_then_columns() {
        let app = App::new();
        let n_clients = app.column_config().clients.len();
        let n_columns = app.column_config().columns.len();
        assert_eq!(app.config_total_rows(), n_clients + n_columns);
        assert_eq!(app.config_section_at(0), ConfigSection::Clients);
        assert_eq!(app.config_section_at(n_clients - 1), ConfigSection::Clients);
        assert_eq!(app.config_section_at(n_clients), ConfigSection::Columns);
    }

    #[test]
    fn toggle_cursor_item_on_provider_flips_enabled() {
        let mut app = App::new();
        let kind = app.column_config().clients[0].kind;
        let before = app.enabled_clients_set().contains(&kind);
        app.set_config_cursor(0);
        app.toggle_cursor_item();
        let after = app.enabled_clients_set().contains(&kind);
        assert_ne!(before, after);
    }

    #[test]
    fn toggle_cursor_item_on_column_flips_visibility() {
        let mut app = App::new();
        let n_clients = app.column_config().clients.len();
        // The Session column (columns[0]) cannot be hidden; use the second
        // column (index 1) which is a regular toggleable column.
        let col_idx = 1;
        app.set_config_cursor(n_clients + col_idx);
        let was = app.column_config().columns[col_idx].visible;
        app.toggle_cursor_item();
        assert_eq!(app.column_config().columns[col_idx].visible, !was);
    }

    #[test]
    fn config_move_down_clamps_to_total_rows() {
        let mut app = App::new();
        let max = app.config_total_rows() - 1;
        for _ in 0..100 {
            app.config_move_down();
        }
        assert_eq!(app.config_cursor(), max);
    }

    #[test]
    fn toggle_client_updates_shared_enabled_arc() {
        use agtop_core::ClientKind;
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        let mut app = App::new();
        let arc: Arc<RwLock<HashSet<ClientKind>>> =
            Arc::new(RwLock::new(ClientKind::all().iter().copied().collect()));
        app.attach_enabled_arc(Arc::clone(&arc));

        let kind = app.column_config().clients[0].kind;
        app.set_config_cursor(0);
        app.toggle_cursor_item();

        let live = arc.read().unwrap();
        assert!(!live.contains(&kind), "disabled client still in shared set");
    }
}

#[cfg(test)]
mod quota_state_tests {
    use super::*;
    use agtop_core::quota::{ProviderId, ProviderResult};

    fn ok_result(id: ProviderId) -> ProviderResult {
        ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: true,
            usage: None,
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        }
    }

    fn err_result(id: ProviderId) -> ProviderResult {
        ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: false,
            usage: None,
            error: Some(agtop_core::quota::QuotaError {
                kind: agtop_core::quota::ErrorKind::Transport,
                detail: "boom".into(),
            }),
            fetched_at: 0,
            meta: Default::default(),
        }
    }

    fn make_google_usage(count: usize) -> ProviderResult {
        use agtop_core::quota::{Usage, UsageWindow};
        use indexmap::IndexMap;
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        for i in 0..count {
            let mut wins: IndexMap<String, UsageWindow> = IndexMap::new();
            wins.insert(
                "daily".into(),
                UsageWindow {
                    used_percent: Some(50.0),
                    window_seconds: Some(86400),
                    reset_at: None,
                    value_label: None,
                },
            );
            models.insert(format!("gemini/gemini-2.5-flash-{i}"), wins);
        }
        ProviderResult {
            provider_id: ProviderId::Google,
            provider_name: "Google",
            configured: true,
            ok: true,
            usage: Some(Usage {
                windows: Default::default(),
                models,
                extras: Default::default(),
            }),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        }
    }

    #[test]
    fn provider_slot_new_sets_last_good_only_if_ok() {
        let slot_ok = ProviderSlot::new(ok_result(ProviderId::Claude));
        assert!(slot_ok.last_good.is_some());
        assert!(slot_ok.current.ok);

        let slot_err = ProviderSlot::new(err_result(ProviderId::Claude));
        assert!(slot_err.last_good.is_none());
        assert!(!slot_err.current.ok);
    }

    #[test]
    fn provider_slot_upsert_preserves_last_good_on_error() {
        let mut slot = ProviderSlot::new(ok_result(ProviderId::Claude));
        assert!(slot.last_good.is_some());
        let err = err_result(ProviderId::Claude);
        slot.upsert(err);
        assert!(slot.last_good.is_some(), "last_good survives error");
        assert!(!slot.current.ok, "current reflects new failure");
    }

    #[test]
    fn provider_slot_upsert_updates_last_good_on_new_ok() {
        let mut slot = ProviderSlot::new(ok_result(ProviderId::Claude));
        let old_fetched = slot.last_good.as_ref().unwrap().fetched_at;
        let mut newer = ok_result(ProviderId::Claude);
        newer.fetched_at = old_fetched + 1000;
        slot.upsert(newer);
        assert_eq!(
            slot.last_good.as_ref().unwrap().fetched_at,
            old_fetched + 1000
        );
    }

    #[test]
    fn quota_state_default_is_idle() {
        let s: QuotaState = Default::default();
        assert_eq!(s, QuotaState::Idle);
    }

    #[test]
    fn app_starts_with_empty_quota_state() {
        let app = App::new();
        assert!(app.quota_slots().is_empty());
        assert_eq!(app.quota_state(), &QuotaState::Idle);
        assert_eq!(app.selected_provider(), 0);
        assert_eq!(app.model_scroll(), 0);
        assert_eq!(app.card_scroll(), 0);
    }

    #[test]
    fn apply_quota_results_sets_ready_and_upserts_by_id() {
        let mut app = App::new();
        app.apply_quota_results(vec![
            ok_result(ProviderId::Claude),
            err_result(ProviderId::Codex),
        ]);
        assert_eq!(app.quota_state(), &QuotaState::Ready);
        assert_eq!(app.quota_slots().len(), 2);

        // Second batch: replace Codex with ok, leave Claude alone.
        app.apply_quota_results(vec![ok_result(ProviderId::Codex)]);
        assert_eq!(app.quota_slots().len(), 2);
        let codex = app
            .quota_slots()
            .iter()
            .find(|s| s.current.provider_id == ProviderId::Codex)
            .unwrap();
        assert!(codex.current.ok);
        assert!(codex.last_good.is_some());
    }

    #[test]
    fn apply_quota_results_preserves_last_good_across_failure() {
        let mut app = App::new();
        app.apply_quota_results(vec![ok_result(ProviderId::Claude)]);
        app.apply_quota_results(vec![err_result(ProviderId::Claude)]);
        let slot = &app.quota_slots()[0];
        assert!(slot.last_good.is_some());
        assert!(!slot.current.ok);
    }

    #[test]
    fn set_quota_loading_transitions_from_idle() {
        let mut app = App::new();
        assert_eq!(app.quota_state(), &QuotaState::Idle);
        app.set_quota_loading();
        assert_eq!(app.quota_state(), &QuotaState::Loading);
    }

    #[test]
    fn set_quota_error_before_ready_sets_error_state() {
        let mut app = App::new();
        app.set_quota_error("dns failure".into());
        assert_eq!(app.quota_state(), &QuotaState::Error("dns failure".into()));
    }

    #[test]
    fn set_quota_error_after_ready_leaves_ready() {
        // After slots are populated, a subsequent fetch failure should be
        // reflected per-slot (via apply_quota_results), NOT by blowing the
        // whole state back to Error. set_quota_error is only meaningful
        // before the first successful batch arrives.
        let mut app = App::new();
        app.apply_quota_results(vec![ok_result(ProviderId::Claude)]);
        app.set_quota_error("should be ignored".into());
        assert_eq!(app.quota_state(), &QuotaState::Ready);
    }

    #[test]
    fn quota_select_next_clamps_at_last() {
        let mut app = App::new();
        app.apply_quota_results(vec![
            ok_result(ProviderId::Claude),
            ok_result(ProviderId::Codex),
        ]);
        assert_eq!(app.selected_provider(), 0);
        app.quota_select_next();
        assert_eq!(app.selected_provider(), 1);
        app.quota_select_next();
        assert_eq!(app.selected_provider(), 1, "clamps at last slot");
    }

    #[test]
    fn quota_select_prev_clamps_at_zero() {
        let mut app = App::new();
        app.apply_quota_results(vec![ok_result(ProviderId::Claude)]);
        app.quota_select_prev();
        assert_eq!(app.selected_provider(), 0);
    }

    #[test]
    fn quota_select_resets_model_scroll() {
        let mut app = App::new();
        app.apply_quota_results(vec![
            ok_result(ProviderId::Claude),
            ok_result(ProviderId::Google),
        ]);
        // Simulate scrolling models on the first slot, then switching.
        app.set_model_scroll_for_test(5);
        assert_eq!(app.model_scroll(), 5);
        app.quota_select_next();
        assert_eq!(
            app.model_scroll(),
            0,
            "switching providers resets model_scroll"
        );
    }

    #[test]
    fn quota_card_scroll_left_clamps_at_zero() {
        let mut app = App::new();
        app.quota_card_scroll_left();
        assert_eq!(app.card_scroll(), 0);
    }

    #[test]
    fn quota_card_scroll_right_clamps_at_max() {
        let mut app = App::new();
        app.apply_quota_results(vec![
            ok_result(ProviderId::Claude),
            ok_result(ProviderId::Codex),
            ok_result(ProviderId::Google),
        ]);
        // With cards_visible=2 and 3 slots, max scroll = 1.
        app.quota_card_scroll_right(2);
        assert_eq!(app.card_scroll(), 1);
        app.quota_card_scroll_right(2);
        assert_eq!(app.card_scroll(), 1, "clamps at slots - visible");
    }

    #[test]
    fn quota_card_scroll_right_noop_when_all_visible() {
        let mut app = App::new();
        app.apply_quota_results(vec![ok_result(ProviderId::Claude)]);
        // 1 slot, 5 visible → no scroll possible.
        app.quota_card_scroll_right(5);
        assert_eq!(app.card_scroll(), 0);
    }

    #[test]
    fn model_scroll_down_increments() {
        let mut app = App::new();
        app.apply_quota_results(vec![make_google_usage(5)]);
        assert_eq!(app.model_scroll(), 0);
        app.model_scroll_down(3);
        assert_eq!(app.model_scroll(), 1);
        app.model_scroll_down(3);
        assert_eq!(app.model_scroll(), 2);
    }

    #[test]
    fn model_scroll_down_clamps_at_max() {
        let mut app = App::new();
        app.apply_quota_results(vec![make_google_usage(5)]);
        for _ in 0..10 {
            app.model_scroll_down(3);
        }
        assert!(
            app.model_scroll() <= 2,
            "scroll should clamp so last item is visible"
        );
    }

    #[test]
    fn model_scroll_up_clamps_at_zero() {
        let mut app = App::new();
        app.apply_quota_results(vec![make_google_usage(5)]);
        app.model_scroll_down(3);
        app.model_scroll_up();
        assert_eq!(app.model_scroll(), 0);
        app.model_scroll_up();
        assert_eq!(app.model_scroll(), 0);
    }
}

#[cfg(test)]
mod tab_quota_tests {
    use super::Tab;

    #[test]
    fn tab_all_includes_quota() {
        assert!(Tab::all().contains(&Tab::Quota));
    }

    #[test]
    fn tab_quota_has_title() {
        assert_eq!(Tab::Quota.title(), "Quota");
    }

    #[test]
    fn tab_cycle_forward_includes_quota() {
        // Cycle through all tabs starting from Info; Quota must appear exactly once.
        let mut seen = std::collections::HashSet::new();
        let mut t = Tab::Info;
        for _ in 0..8 {
            seen.insert(t);
            t = t.cycle_forward();
        }
        assert!(seen.contains(&Tab::Quota));
    }
}
