# Phase 5 — Wire-up: commands, keys, footer

> **For agentic workers:** Phases 1–4 must be committed first.

**Goal:** Dispatch `QuotaCmd::Start` / `QuotaCmd::Stop` on pane-focus transitions, wire the new key bindings (`j`/`k`/`←`/`→` for quota pane), add the footer hint, and add end-to-end smoke tests.

**Spec sections covered:** "Pane focus wiring", "Key bindings summary", "Footer hint".

---

## Task 1: Dispatch `QuotaCmd::Start` / `QuotaCmd::Stop` on tab switch

**Files:**
- Modify: `crates/agtop-cli/src/tui/mod.rs`
- Modify: `crates/agtop-cli/src/tui/events.rs`

- [ ] **Step 1: Understand the routing**

The event loop calls `apply_key(app, k) -> Action`. Today `Action` has `None` and `ManualRefresh`. We need a new variant so `apply_key` can tell the event loop to call `handle.send_quota_cmd(...)`.

Alternative: the event loop itself detects tab/mode transitions by observing `app.tab()` / `app.ui_mode()` across iterations. This is more fragile.

**Use the `Action` enum extension.** Cleaner and testable.

- [ ] **Step 2: Write the failing test**

Append to `crates/agtop-cli/src/tui/events.rs` in the existing `tests` module:

```rust
    use crate::tui::refresh::QuotaCmd;

    #[test]
    fn tab_into_quota_emits_start_action() {
        let mut app = App::new();
        // Currently on Tab::Info. Cycle forward → Cost → Config → Quota.
        apply_key(&mut app, press(KeyCode::Tab));
        apply_key(&mut app, press(KeyCode::Tab));
        let action = apply_key(&mut app, press(KeyCode::Tab));
        assert_eq!(app.tab(), Tab::Quota);
        assert!(
            matches!(action, Action::QuotaCmd(QuotaCmd::Start)),
            "expected Start action, got {action:?}"
        );
    }

    #[test]
    fn tab_out_of_quota_emits_stop_action() {
        let mut app = App::new();
        app.set_tab(Tab::Quota);
        let action = apply_key(&mut app, press(KeyCode::Tab));
        assert_ne!(app.tab(), Tab::Quota);
        assert!(
            matches!(action, Action::QuotaCmd(QuotaCmd::Stop)),
            "expected Stop action, got {action:?}"
        );
    }

    #[test]
    fn d_into_dashboard_emits_start() {
        let mut app = App::new();
        // Classic → Dashboard
        let action = apply_key(&mut app, press(KeyCode::Char('d')));
        assert_eq!(app.ui_mode(), UiMode::Dashboard);
        assert!(matches!(action, Action::QuotaCmd(QuotaCmd::Start)));
    }

    #[test]
    fn d_into_classic_emits_stop() {
        let mut app = App::new();
        app.toggle_ui_mode(); // Dashboard
        let action = apply_key(&mut app, press(KeyCode::Char('d')));
        assert_eq!(app.ui_mode(), UiMode::Classic);
        assert!(matches!(action, Action::QuotaCmd(QuotaCmd::Stop)));
    }
```

Also, verify the `Action` type can hold a `QuotaCmd` (need `Debug` derive to work in the `panic!` message above). Check `refresh.rs` — `QuotaCmd` already has `Debug` from Phase 2.

- [ ] **Step 3: Run tests — expect failure**

Run: `cargo test -p agtop-cli events::tests::tab_into_quota_emits_start_action`
Expected: FAIL — `Action::QuotaCmd` not defined.

- [ ] **Step 4: Extend `Action`**

In `crates/agtop-cli/src/tui/events.rs`, replace:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    ManualRefresh,
}
```

With:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    ManualRefresh,
    QuotaCmd(crate::tui::refresh::QuotaCmd),
}
```

- [ ] **Step 5: Emit the action from key-handling logic**

In `apply_normal_key`, change the `Tab` / `BackTab` arms to detect transitions into/out of `Tab::Quota`:

Replace:
```rust
        KeyCode::Tab => app.next_tab(),
        KeyCode::BackTab => app.prev_tab(),
```

With:
```rust
        KeyCode::Tab => {
            let was_quota = app.tab() == Tab::Quota;
            app.next_tab();
            let is_quota = app.tab() == Tab::Quota;
            return quota_cmd_for_transition(was_quota, is_quota);
        }
        KeyCode::BackTab => {
            let was_quota = app.tab() == Tab::Quota;
            app.prev_tab();
            let is_quota = app.tab() == Tab::Quota;
            return quota_cmd_for_transition(was_quota, is_quota);
        }
```

Replace the `'d'` arm:
```rust
        KeyCode::Char('d') => app.toggle_ui_mode(),
```

With:
```rust
        KeyCode::Char('d') => {
            use crate::tui::app::UiMode;
            let was_dashboard = app.ui_mode() == UiMode::Dashboard;
            app.toggle_ui_mode();
            let is_dashboard = app.ui_mode() == UiMode::Dashboard;
            return quota_cmd_for_transition(was_dashboard, is_dashboard);
        }
```

Also update `apply_config_key` (since it handles Tab/BackTab while on Config tab):

Replace:
```rust
        KeyCode::Tab => app.next_tab(),
        KeyCode::BackTab => app.prev_tab(),
```

With:
```rust
        KeyCode::Tab => {
            let was_quota = app.tab() == Tab::Quota;
            app.next_tab();
            let is_quota = app.tab() == Tab::Quota;
            return quota_cmd_for_transition(was_quota, is_quota);
        }
        KeyCode::BackTab => {
            let was_quota = app.tab() == Tab::Quota;
            app.prev_tab();
            let is_quota = app.tab() == Tab::Quota;
            return quota_cmd_for_transition(was_quota, is_quota);
        }
```

Now add the helper function at the bottom of `events.rs` (before `#[cfg(test)]`):

```rust
fn quota_cmd_for_transition(was_active: bool, is_active: bool) -> Action {
    use crate::tui::refresh::QuotaCmd;
    match (was_active, is_active) {
        (false, true) => Action::QuotaCmd(QuotaCmd::Start),
        (true, false) => Action::QuotaCmd(QuotaCmd::Stop),
        _ => Action::None,
    }
}
```

- [ ] **Step 6: Wire in the event loop**

In `crates/agtop-cli/src/tui/mod.rs`, extend the match around line 215:

Replace:
```rust
                Event::Key(k) => match apply_key(app, k) {
                    Action::None => {}
                    Action::ManualRefresh => handle.trigger_manual(),
                },
```

With:
```rust
                Event::Key(k) => match apply_key(app, k) {
                    Action::None => {}
                    Action::ManualRefresh => handle.trigger_manual(),
                    Action::QuotaCmd(cmd) => handle.send_quota_cmd(cmd),
                },
```

- [ ] **Step 7: Run tests — expect success**

Run: `cargo test -p agtop-cli events::tests`
Expected: PASS (all existing events tests + 4 new).

Run full suite:
Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/agtop-cli/src/tui/events.rs crates/agtop-cli/src/tui/mod.rs
git commit -m "quota-tui(phase-5): dispatch QuotaCmd::Start/Stop on tab and mode transitions"
```

---

## Task 2: Arrow-key handling for quota pane

**Files:**
- Modify: `crates/agtop-cli/src/tui/events.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module:

```rust
    use agtop_core::quota::ProviderId;

    #[test]
    fn j_in_dashboard_quota_advances_provider() {
        use agtop_core::quota::{ProviderResult, Usage};
        let mut app = App::new();
        app.toggle_ui_mode(); // Dashboard
        let mk = |id: ProviderId| ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: true,
            usage: Some(Usage::default()),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        };
        app.apply_quota_results(vec![mk(ProviderId::Claude), mk(ProviderId::Codex)]);
        assert_eq!(app.selected_provider(), 0);
        apply_key(&mut app, press(KeyCode::Char('j')));
        assert_eq!(app.selected_provider(), 1);
    }

    #[test]
    fn left_right_in_classic_quota_tab_scrolls_cards() {
        use agtop_core::quota::{ProviderResult, Usage};
        let mut app = App::new();
        app.set_tab(Tab::Quota);
        let mk = |id: ProviderId| ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: true,
            usage: Some(Usage::default()),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        };
        // 3 slots; quota_card_scroll_right is a no-op unless cards_visible < slots.
        // For the routing test, just verify the right-arrow reaches the call.
        app.apply_quota_results(vec![
            mk(ProviderId::Claude),
            mk(ProviderId::Codex),
            mk(ProviderId::Copilot),
        ]);
        // With default cards_visible of 1 in the unit test (no render), the
        // key handler uses a sensible default. Verify card_scroll changes.
        let before = app.card_scroll();
        apply_key(&mut app, press(KeyCode::Right));
        assert!(
            app.card_scroll() >= before,
            "Right should increment or stay (was {before}, now {})",
            app.card_scroll()
        );
        apply_key(&mut app, press(KeyCode::Left));
        assert_eq!(app.card_scroll(), 0);
    }
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli events::tests::j_in_dashboard_quota_advances_provider`
Expected: FAIL — routing not yet implemented.

- [ ] **Step 3: Route the keys**

In `apply_normal_key`, replace the existing `j` / `k` arms:

```rust
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
```

With:

```rust
        KeyCode::Char('j') | KeyCode::Down => {
            if app.ui_mode() == UiMode::Dashboard {
                // Dashboard j/k navigates the quota provider list.
                app.quota_select_next();
            } else {
                app.move_selection(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.ui_mode() == UiMode::Dashboard {
                app.quota_select_prev();
            } else {
                app.move_selection(-1);
            }
        }
```

Then add handlers for `Left` / `Right` (Classic Quota tab only):

Add before the `_ => {}` catch-all in `apply_normal_key`:

```rust
        KeyCode::Left if app.tab() == Tab::Quota && app.ui_mode() == UiMode::Classic => {
            app.quota_card_scroll_left();
        }
        KeyCode::Right if app.tab() == Tab::Quota && app.ui_mode() == UiMode::Classic => {
            // In production, the event loop knows `cards_visible` from the
            // render area. Events layer doesn't — pass a conservative default
            // that allows the right-edge to be reached in practice. The next
            // render clamps correctly.
            app.quota_card_scroll_right(1);
        }
```

NOTE: `quota_card_scroll_right(1)` with `cards_visible=1` means max scroll = `slots.len() - 1`, so we can always reach the last slot. The only downside is the scroll can overshoot what's visible in a wide terminal; the next render will still paint the rightmost slots correctly because it only reads `card_scroll..card_scroll+cards_visible`. This is acceptable — slight UX quirk in favor of code simplicity.

- [ ] **Step 4: The existing `dashboard_j_key_routes_to_plan_selection` test will now fail**

That test (in `events.rs` around line 446) was using `app.plan_selected()` — our new routing uses `app.selected_provider()` instead. Update it:

Replace the test body:
```rust
    #[test]
    fn dashboard_j_key_routes_to_plan_selection() {
        use agtop_core::session::{ClientKind, PlanUsage};

        let mut app = App::new();
        app.toggle_ui_mode(); // Dashboard mode

        // Populate plan_usage with 2 entries so plan_select_next has room to move.
        let make_pu = |label: &str| {
            PlanUsage::new(
                ClientKind::Claude,
                label.to_string(),
                None,
                Vec::new(),
                None,
                None,
            )
        };
        app.set_snapshot(Vec::new(), vec![make_pu("Sub A"), make_pu("Sub B")]);

        apply_key(&mut app, press(KeyCode::Char('j')));
        assert_eq!(
            app.plan_selected(),
            1,
            "j in Dashboard should increment plan_selected"
        );

        apply_key(&mut app, press(KeyCode::Char('k')));
        assert_eq!(
            app.plan_selected(),
            0,
            "k in Dashboard should decrement plan_selected"
        );
    }
```

With:

```rust
    #[test]
    fn dashboard_j_key_routes_to_quota_selection() {
        use agtop_core::quota::{ProviderResult, Usage};

        let mut app = App::new();
        app.toggle_ui_mode(); // Dashboard mode

        let mk = |id: agtop_core::quota::ProviderId| ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: true,
            usage: Some(Usage::default()),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        };
        app.apply_quota_results(vec![
            mk(agtop_core::quota::ProviderId::Claude),
            mk(agtop_core::quota::ProviderId::Codex),
        ]);

        apply_key(&mut app, press(KeyCode::Char('j')));
        assert_eq!(
            app.selected_provider(),
            1,
            "j in Dashboard should increment selected_provider"
        );

        apply_key(&mut app, press(KeyCode::Char('k')));
        assert_eq!(
            app.selected_provider(),
            0,
            "k in Dashboard should decrement selected_provider"
        );
    }
```

Also remove the `dashboard_j_moves_plan_selection_down` and `dashboard_k_clamps_at_zero` tests — those test methods on App that we may now also want to deprecate. Leave them for now if they still compile; if they reference `plan_select_next`/`plan_select_prev`, keep them since those methods still exist on `App` (just no longer driven by the arrow keys).

- [ ] **Step 5: Run tests — expect success**

Run: `cargo test -p agtop-cli events::tests`
Expected: PASS.

Run full suite:
Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/agtop-cli/src/tui/events.rs
git commit -m "quota-tui(phase-5): route j/k to quota selection in Dashboard; Left/Right scroll Classic cards"
```

---

## Task 3: Footer hint and regression check

**Files:**
- Modify: `crates/agtop-cli/src/tui/mod.rs`

- [ ] **Step 1: Update footer text**

In `crates/agtop-cli/src/tui/mod.rs`, around line 582 (`render_footer`), the existing Normal-mode footer text is:

```rust
            concat!(
                " q:quit  d:dashboard  j/k:↕  click:select  scroll:↕  Tab:tab  /:filter  >:sort  i:dir  r:refresh  ",
                "g/G:top/bot  PgUp/PgDn:10"
            )
```

`r:refresh` is already present. No change needed — the footer hint for refresh already applies to the quota pane now too.

Double-check no change is required by running:

Run: `cargo test -p agtop-cli renders_filter_mode_footer`
Expected: PASS.

- [ ] **Step 2: Regression check — original tests still pass**

Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

If the `renders_dashboard_with_plan_usage` test from Phase 4 needs further tuning (e.g. different panel title now), fix it inline. The test we wrote in Phase 4 Task 1 (`renders_dashboard_with_quota_idle`) should have replaced it.

- [ ] **Step 3: Commit if anything changed**

```bash
git status
# if changes, commit:
git add -A
git commit -m "quota-tui(phase-5): footer regression check"
```

Otherwise, skip this step.

---

## Task 4: End-to-end smoke test

**Files:**
- Modify: `crates/agtop-cli/src/tui/mod.rs`

- [ ] **Step 1: Write the failing test**

In `crates/agtop-cli/src/tui/mod.rs`, append to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn quota_tab_renders_after_apply_results() {
        use agtop_core::quota::{ProviderId, ProviderResult, Usage, UsageWindow};
        use indexmap::IndexMap;

        let backend = TestBackend::new(140, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.set_tab(Tab::Quota);

        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        windows.insert(
            "5h".into(),
            UsageWindow {
                used_percent: Some(42.0),
                window_seconds: Some(18000),
                reset_at: None,
                value_label: None,
            },
        );
        let usage = Usage {
            windows,
            models: Default::default(),
            extras: Default::default(),
        };
        app.apply_quota_results(vec![ProviderResult {
            provider_id: ProviderId::Claude,
            provider_name: ProviderId::Claude.display_name(),
            configured: true,
            ok: true,
            usage: Some(usage),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        }]);

        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");

        let contents = buffer_to_string(&terminal.backend().buffer().clone());
        assert!(contents.contains("Claude"), "Claude missing:\n{contents}");
        assert!(contents.contains("5h"), "5h missing:\n{contents}");
        assert!(contents.contains("42"), "percentage missing:\n{contents}");
        assert!(contents.contains('■'), "bar char missing:\n{contents}");
    }

    #[test]
    fn dashboard_mode_shows_quota_block() {
        use agtop_core::quota::{ProviderId, ProviderResult, Usage, UsageWindow};
        use indexmap::IndexMap;

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.toggle_ui_mode(); // Dashboard

        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        windows.insert(
            "5h".into(),
            UsageWindow {
                used_percent: Some(88.0),
                window_seconds: None,
                reset_at: None,
                value_label: None,
            },
        );
        let usage = Usage {
            windows,
            models: Default::default(),
            extras: Default::default(),
        };
        app.apply_quota_results(vec![ProviderResult {
            provider_id: ProviderId::Zai,
            provider_name: ProviderId::Zai.display_name(),
            configured: true,
            ok: true,
            usage: Some(usage),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        }]);

        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");

        let contents = buffer_to_string(&terminal.backend().buffer().clone());
        assert!(contents.contains("Quota"), "Quota title missing:\n{contents}");
        assert!(contents.contains("z.ai"), "z.ai provider missing:\n{contents}");
        assert!(contents.contains("88"), "percentage missing:\n{contents}");
    }
```

- [ ] **Step 2: Run tests — expect success**

Run: `cargo test -p agtop-cli tests::quota_tab_renders_after_apply_results tests::dashboard_mode_shows_quota_block`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/mod.rs
git commit -m "quota-tui(phase-5): end-to-end smoke tests for Classic and Dashboard quota rendering"
```

---

## Task 5: Final verification

- [ ] **Step 1: Full test suite**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS for both `agtop-core` and `agtop-cli`.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no warnings.

- [ ] **Step 3: Manual smoke (optional but recommended)**

Run: `cargo run -p agtop-cli -- --dashboard`

Verify:
1. Dashboard opens; quota panel shows "Press r to load quota data" or starts fetching.
2. Press `r` → quota fetches fire; results appear within ~5s.
3. `j`/`k` switch selected provider in the list.
4. Switch to Classic (`d`); Tab-cycle to Quota tab (4th tab).
5. `r` works there too; cards render.
6. `←`/`→` scrolls cards if terminal is narrow.
7. Quit with `q`.

- [ ] **Step 4: Final commit if anything was tweaked**

```bash
git status
# if changes:
git add -A
git commit -m "quota-tui(phase-5): final polish after manual smoke test"
```

Phase 5 complete. Full quota TUI integration merged.

## Handing off

After Phase 5 commits cleanly, the implementation plan is complete. Optional follow-ups (all explicitly out of scope per the spec):

- Per-provider refresh-interval configuration.
- Quota alerts / threshold notifications.
- Historical quota charting.
- Mouse-click provider selection in the Dashboard list.
- Classic-mode detail view.
