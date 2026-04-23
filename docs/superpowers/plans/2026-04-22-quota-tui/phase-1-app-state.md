# Phase 1 — App state for quota

> **For agentic workers:** Implement tasks in order, top to bottom. Each task is self-contained; do not skip ahead. Commit after each task. Use `superpowers:test-driven-development`.

**Goal:** Add `ProviderSlot`, `QuotaState`, and all `App` fields/methods needed to hold quota data, plus the `Tab::Quota` enum variant. No rendering or worker changes yet.

**Spec sections covered:** "Data model", "App additions", "Tab enum".

---

## Task 1: Add `Tab::Quota` enum variant

**Files:**
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/agtop-cli/src/tui/app/mod.rs` inside the existing `#[cfg(test)] mod tests` block (or create one at the end if absent):

```rust
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
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli tab_quota_tests`
Expected: FAIL — "variant `Quota` not found" or similar.

- [ ] **Step 3: Add the variant**

Modify `crates/agtop-cli/src/tui/app/mod.rs` around line 64. Replace:

```rust
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
```

With:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Info,
    Cost,
    Config,
    Quota,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[Tab::Info, Tab::Cost, Tab::Config, Tab::Quota]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Cost => "Cost",
            Self::Config => "Config",
            Self::Quota => "Quota",
        }
    }

    pub fn cycle_forward(self) -> Self {
        match self {
            Self::Info => Self::Cost,
            Self::Cost => Self::Config,
            Self::Config => Self::Quota,
            Self::Quota => Self::Info,
        }
    }

    pub fn cycle_back(self) -> Self {
        match self {
            Self::Info => Self::Quota,
            Self::Cost => Self::Info,
            Self::Config => Self::Cost,
            Self::Quota => Self::Config,
        }
    }
}
```

Note: `Hash` was added to the derive — it's needed because later tasks use `Tab` in HashSet for tests.

- [ ] **Step 4: Fix `render_bottom_panel` match in `tui/mod.rs`**

At `crates/agtop-cli/src/tui/mod.rs` around line 522-527 and line 560-572, the existing code exhaustively matches `app.tab()` on the old three variants. The build will break until we add the `Quota` arm. Update both locations:

Replace (around line 522):
```rust
    let tab_bar = Tabs::new(titles)
        .select(match app.tab() {
            Tab::Info => 0,
            Tab::Cost => 1,
            Tab::Config => 2,
        })
        .block(Block::default().borders(Borders::NONE))
        .divider("│");
```

With:
```rust
    let tab_bar = Tabs::new(titles)
        .select(match app.tab() {
            Tab::Info => 0,
            Tab::Cost => 1,
            Tab::Config => 2,
            Tab::Quota => 3,
        })
        .block(Block::default().borders(Borders::NONE))
        .divider("│");
```

Replace (around line 560):
```rust
    match app.tab() {
        Tab::Info => widgets::info_tab::render(frame, rows[1], app),
        Tab::Cost => widgets::cost_tab::render(frame, rows[1], app),
        Tab::Config => widgets::config_tab::render(
            frame,
            rows[1],
            app,
            widgets::config_tab::ConfigRenderOut {
                client_rows: &mut layout.config_client_rows,
                column_rows: &mut layout.config_column_rows,
            },
        ),
    }
```

With:
```rust
    match app.tab() {
        Tab::Info => widgets::info_tab::render(frame, rows[1], app),
        Tab::Cost => widgets::cost_tab::render(frame, rows[1], app),
        Tab::Config => widgets::config_tab::render(
            frame,
            rows[1],
            app,
            widgets::config_tab::ConfigRenderOut {
                client_rows: &mut layout.config_client_rows,
                column_rows: &mut layout.config_column_rows,
            },
        ),
        // Phase 3 will wire this up to widgets::quota_tab::render.
        // For now, render a placeholder so the build stays green.
        Tab::Quota => {
            use ratatui::widgets::Paragraph;
            let p = Paragraph::new("(quota tab not yet implemented)");
            frame.render_widget(p, rows[1]);
        }
    }
```

- [ ] **Step 5: Run tests — expect success**

Run: `cargo test -p agtop-cli`
Expected: PASS (all existing tests + 3 new quota tab tests).

- [ ] **Step 6: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs crates/agtop-cli/src/tui/mod.rs
git commit -m "quota-tui(phase-1): add Tab::Quota enum variant"
```

---

## Task 2: Add `ProviderSlot` and `QuotaState`

**Files:**
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`

- [ ] **Step 1: Write the failing test**

Append to the test module at the bottom of `crates/agtop-cli/src/tui/app/mod.rs`:

```rust
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
        assert_eq!(slot.last_good.as_ref().unwrap().fetched_at, old_fetched + 1000);
    }

    #[test]
    fn quota_state_default_is_idle() {
        let s: QuotaState = Default::default();
        assert_eq!(s, QuotaState::Idle);
    }
}
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli quota_state_tests`
Expected: FAIL — `ProviderSlot` and `QuotaState` are not defined.

- [ ] **Step 3: Add the types**

In `crates/agtop-cli/src/tui/app/mod.rs`, after the existing imports near the top (around line 32 after `use super::column_config::ColumnConfig;`), add:

```rust
use agtop_core::quota::ProviderResult;
```

Then, anywhere between the `InputMode` enum and the `App` struct (a good spot is right before line 178 `// ---- App ----`), add:

```rust
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
        let last_good = if result.ok { Some(result.clone()) } else { None };
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaState {
    /// Quota pane has never been opened in this session.
    Idle,
    /// First fetch is in flight; no slot results yet.
    Loading,
    /// At least one fetch cycle has completed; slots may be populated.
    Ready,
    /// First fetch failed before any result arrived. `String` is the error message.
    Error(String),
}

impl Default for QuotaState {
    fn default() -> Self {
        Self::Idle
    }
}
```

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli quota_state_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs
git commit -m "quota-tui(phase-1): add ProviderSlot and QuotaState types"
```

---

## Task 3: Add quota fields to `App`

**Files:**
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`

- [ ] **Step 1: Write the failing test**

Append to the `quota_state_tests` module in `crates/agtop-cli/src/tui/app/mod.rs`:

```rust
    #[test]
    fn app_starts_with_empty_quota_state() {
        let app = App::new();
        assert!(app.quota_slots().is_empty());
        assert_eq!(app.quota_state(), &QuotaState::Idle);
        assert_eq!(app.selected_provider(), 0);
        assert_eq!(app.model_scroll(), 0);
        assert_eq!(app.card_scroll(), 0);
    }
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli app_starts_with_empty_quota_state`
Expected: FAIL — methods not defined.

- [ ] **Step 3: Add the fields**

In `crates/agtop-cli/src/tui/app/mod.rs`, add these fields to the `App` struct (after line 240, immediately before the closing brace of `pub struct App {`):

```rust
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
```

Then initialize them in `App::new()` (inside the `Self { ... }` literal, just before the closing brace on line ~279):

```rust
            quota_slots: Vec::new(),
            quota_state: QuotaState::default(),
            selected_provider: 0,
            model_scroll: 0,
            card_scroll: 0,
```

Add accessor methods in the `impl App` block, anywhere in the "read-only accessors" section (around line 314 after `plan_usage`):

```rust
    pub fn quota_slots(&self) -> &[ProviderSlot] {
        &self.quota_slots
    }
    pub fn quota_state(&self) -> &QuotaState {
        &self.quota_state
    }
    pub fn selected_provider(&self) -> usize {
        self.selected_provider.min(self.quota_slots.len().saturating_sub(1))
    }
    pub fn model_scroll(&self) -> usize {
        self.model_scroll
    }
    pub fn card_scroll(&self) -> usize {
        self.card_scroll
    }
```

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli app_starts_with_empty_quota_state`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs
git commit -m "quota-tui(phase-1): add quota fields to App"
```

---

## Task 4: Add `apply_quota_results`, `set_quota_error`, `set_quota_loading`

**Files:**
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`

- [ ] **Step 1: Write the failing test**

Append to the `quota_state_tests` module:

```rust
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
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli apply_quota_results_sets_ready_and_upserts_by_id`
Expected: FAIL — methods not defined.

- [ ] **Step 3: Add the methods**

In `crates/agtop-cli/src/tui/app/mod.rs`, add to the `impl App` block (pick a logical spot, e.g. after `set_snapshot` if that method is nearby, or just before the closing `}` of `impl App`):

```rust
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
```

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli quota_state_tests`
Expected: PASS (all 9+ tests in the module).

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs
git commit -m "quota-tui(phase-1): add apply_quota_results / set_quota_loading / set_quota_error"
```

---

## Task 5: Selection and scroll methods

**Files:**
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`

- [ ] **Step 1: Write the failing test**

Append to `quota_state_tests`:

```rust
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
        assert_eq!(app.model_scroll(), 0, "switching providers resets model_scroll");
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
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli quota_select_next_clamps_at_last`
Expected: FAIL — methods missing.

- [ ] **Step 3: Add the methods**

In the `impl App` block, append:

```rust
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
        let max = self
            .quota_slots
            .len()
            .saturating_sub(cards_visible.max(1));
        self.card_scroll = (self.card_scroll + 1).min(max);
    }

    /// Test-only helper to set `model_scroll` directly. Production code
    /// should not need this; `quota_select_*` is the normal path.
    #[cfg(test)]
    pub fn set_model_scroll_for_test(&mut self, v: usize) {
        self.model_scroll = v;
    }
```

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli quota_state_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs
git commit -m "quota-tui(phase-1): add quota selection and card scroll methods"
```

---

## Task 6: Preferred-window resolver

**Files:**
- Create: `crates/agtop-cli/src/tui/app/quota.rs`
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/agtop-cli/src/tui/app/quota.rs` with:

```rust
//! Quota-specific helpers that don't belong on `App` itself.

use agtop_core::quota::{ProviderId, Usage, UsageWindow};

/// Resolve the "preferred window" for a provider — the single window
/// used in the Classic tab cards and Dashboard list short view.
///
/// Returns `(label, &UsageWindow)` pairs from the provider's `Usage`
/// according to the per-provider preference table in the spec.
///
/// Falls back chains:
/// - Claude / Codex / z.ai → `5h` → first window
/// - Copilot / CopilotAddon → `premium` → first window
/// - Google → first model's `5h` → first model's `daily` → first model's first window
///
/// Returns `None` when the provider has no windows at all.
pub fn preferred_window(
    provider_id: ProviderId,
    usage: &Usage,
) -> Option<(String, &UsageWindow)> {
    match provider_id {
        ProviderId::Claude | ProviderId::Codex | ProviderId::Zai => find_any(usage, &["5h"]),
        ProviderId::Copilot | ProviderId::CopilotAddon => find_any(usage, &["premium"]),
        ProviderId::Google => preferred_google(usage),
    }
}

fn find_any<'a>(
    usage: &'a Usage,
    preferred_labels: &[&str],
) -> Option<(String, &'a UsageWindow)> {
    for pref in preferred_labels {
        if let Some(w) = usage.windows.get(*pref) {
            return Some(((*pref).to_string(), w));
        }
    }
    usage
        .windows
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v))
}

fn preferred_google(usage: &Usage) -> Option<(String, &UsageWindow)> {
    // Google: top-level windows is empty by spec; look into models.
    let (first_model_key, first_model_windows) = usage.models.iter().next()?;
    for pref in &["5h", "daily"] {
        if let Some(w) = first_model_windows.get(*pref) {
            return Some((format!("{}::{}", first_model_key, pref), w));
        }
    }
    first_model_windows
        .iter()
        .next()
        .map(|(k, v)| (format!("{}::{}", first_model_key, k), v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::quota::{Usage, UsageWindow};
    use indexmap::IndexMap;

    fn uw(pct: f64) -> UsageWindow {
        UsageWindow {
            used_percent: Some(pct),
            window_seconds: None,
            reset_at: None,
            value_label: None,
        }
    }

    fn usage_with(pairs: &[(&str, f64)]) -> Usage {
        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        for (k, v) in pairs {
            windows.insert((*k).to_string(), uw(*v));
        }
        Usage {
            windows,
            models: Default::default(),
            extras: Default::default(),
        }
    }

    #[test]
    fn claude_prefers_5h() {
        let u = usage_with(&[("7d", 10.0), ("5h", 72.0)]);
        let (label, w) = preferred_window(ProviderId::Claude, &u).unwrap();
        assert_eq!(label, "5h");
        assert_eq!(w.used_percent, Some(72.0));
    }

    #[test]
    fn claude_falls_back_to_first_when_5h_missing() {
        let u = usage_with(&[("7d", 10.0)]);
        let (label, _) = preferred_window(ProviderId::Claude, &u).unwrap();
        assert_eq!(label, "7d");
    }

    #[test]
    fn copilot_prefers_premium() {
        let u = usage_with(&[("chat", 0.0), ("premium", 50.0)]);
        let (label, w) = preferred_window(ProviderId::Copilot, &u).unwrap();
        assert_eq!(label, "premium");
        assert_eq!(w.used_percent, Some(50.0));
    }

    #[test]
    fn copilot_addon_prefers_premium() {
        let u = usage_with(&[("premium", 85.0)]);
        let (label, _) = preferred_window(ProviderId::CopilotAddon, &u).unwrap();
        assert_eq!(label, "premium");
    }

    #[test]
    fn zai_prefers_5h_falls_through_to_first() {
        let u = usage_with(&[("monthly", 31.0)]);
        let (label, _) = preferred_window(ProviderId::Zai, &u).unwrap();
        assert_eq!(label, "monthly");
    }

    #[test]
    fn google_uses_first_model_with_5h_preference() {
        use indexmap::IndexMap;
        let mut m1: IndexMap<String, UsageWindow> = IndexMap::new();
        m1.insert("daily".into(), uw(20.0));
        m1.insert("5h".into(), uw(95.0));
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        models.insert("gemini/gemini-2.5-pro".into(), m1);

        let u = Usage {
            windows: Default::default(),
            models,
            extras: Default::default(),
        };
        let (label, w) = preferred_window(ProviderId::Google, &u).unwrap();
        assert_eq!(label, "gemini/gemini-2.5-pro::5h");
        assert_eq!(w.used_percent, Some(95.0));
    }

    #[test]
    fn google_with_only_daily() {
        use indexmap::IndexMap;
        let mut m1: IndexMap<String, UsageWindow> = IndexMap::new();
        m1.insert("daily".into(), uw(33.0));
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        models.insert("gemini/gemini-2.5-pro".into(), m1);

        let u = Usage {
            windows: Default::default(),
            models,
            extras: Default::default(),
        };
        let (label, _) = preferred_window(ProviderId::Google, &u).unwrap();
        assert_eq!(label, "gemini/gemini-2.5-pro::daily");
    }

    #[test]
    fn empty_usage_returns_none() {
        let u = usage_with(&[]);
        assert!(preferred_window(ProviderId::Claude, &u).is_none());
    }

    #[test]
    fn google_empty_models_returns_none() {
        let u = Usage {
            windows: Default::default(),
            models: Default::default(),
            extras: Default::default(),
        };
        assert!(preferred_window(ProviderId::Google, &u).is_none());
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/agtop-cli/src/tui/app/mod.rs`, add at the top of the module declarations (around line 9 near the existing `mod cost;`):

```rust
pub mod quota;
```

- [ ] **Step 3: Run tests — expect success**

Run: `cargo test -p agtop-cli app::quota::tests`
Expected: PASS (9 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-cli/src/tui/app/quota.rs crates/agtop-cli/src/tui/app/mod.rs
git commit -m "quota-tui(phase-1): add preferred_window resolver with per-provider fallbacks"
```

---

## Task 7: Verify whole crate builds and all tests pass

- [ ] **Step 1: Run full test suite**

Run: `cargo test -p agtop-cli`
Expected: PASS. No compilation errors anywhere in agtop-cli.

Run: `cargo clippy -p agtop-cli -- -D warnings`
Expected: No warnings.

- [ ] **Step 2: If clippy finds issues, fix them in-place**

Common ones to expect:
- unused import for `ProviderResult` if Phase 2 hasn't arrived yet → suppress with `#[allow(unused)]` or remove the re-export until Phase 2 needs it (prefer removing unused code).
- unused method warnings on the new App methods → `#[allow(dead_code)]` temporarily. Phases 2-5 will use them.

Apply `#[allow(dead_code)]` to each new method on `App` that isn't yet wired up:

```rust
#[allow(dead_code)] // wired up in phase 2+
pub fn apply_quota_results(...) { ... }
```

Same for `set_quota_loading`, `set_quota_error`, `quota_select_next`, `quota_select_prev`, `quota_card_scroll_left`, `quota_card_scroll_right`, and the accessors `quota_slots`, `quota_state`, `selected_provider`, `model_scroll`, `card_scroll`.

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "quota-tui(phase-1): suppress dead-code warnings on unwired methods"
```

Phase 1 complete. `App` now holds all quota state and types; rendering and worker integration come in later phases.
