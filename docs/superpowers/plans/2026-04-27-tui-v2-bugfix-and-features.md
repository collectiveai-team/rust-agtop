# TUI v2 Bugfix & Features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix session state counting, add mouse click support to all interactive UI elements, redesign the quota Long panel with 2-column layout + scroll, and add a toggleable subagent tree view to the sessions table.

**Architecture:** Four independent areas of work touching `refresh_adapter.rs` (state fix), `tab_bar.rs`/`app_v2.rs`/`aggregation/`/`drilldown.rs` (mouse clicks), `quota.rs` (panel redesign), and `sessions.rs`/`refresh_adapter.rs` (tree view). All changes are within `crates/agtop-cli/src/tui/`. No core data model changes needed.

**Tech Stack:** Rust, ratatui 0.29, crossterm, chrono, `std::collections::HashSet`

---

## File Map

| File | What changes |
|------|-------------|
| `crates/agtop-cli/src/tui/refresh_adapter.rs` | Task 1: add tests for state derivation (verify already correct); Task 5: insert child rows into flat session list |
| `crates/agtop-cli/src/tui/widgets/tab_bar.rs` | Task 2: change to stateful struct, store tab rects, expose `hit_test` |
| `crates/agtop-cli/src/tui/app_v2.rs` | Task 2: own `TabBar` instance, route mouse click to `hit_test` |
| `crates/agtop-cli/src/tui/screens/aggregation/controls.rs` | Task 3: add `chip_rects`, `handle_event` |
| `crates/agtop-cli/src/tui/screens/aggregation/mod.rs` | Task 3: route mouse events to `controls.handle_event` |
| `crates/agtop-cli/src/tui/screens/aggregation/drilldown.rs` | Task 4: add `last_area`, mouse click to close |
| `crates/agtop-cli/src/tui/screens/dashboard/quota.rs` | Task 6: add `scroll_offset`, 2-col layout, scroll events |
| `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs` | Task 5: add `depth`, `parent_session_id` to `SessionRow`; add `collapsed` to `SessionsTable`; render toggle + indent |

---

## Task 1: Verify Session State Derivation (tests already present)

**Files:**
- Verify: `crates/agtop-cli/src/tui/refresh_adapter.rs`

**Context:** `refresh_adapter.rs` already has `normalize_analysis` called before counting in `apply_analyses` (lines 29, 48-63). The existing test at line 200 (`live_process_without_parser_state_is_active_running`) already covers the main case. This task adds the missing test cases and verifies nothing is broken.

- [ ] **Step 1: Run the existing refresh_adapter tests**

```bash
rtk cargo test -p agtop-cli refresh_adapter -- --test-threads=1
```

Expected: all pass. If any fail, stop and investigate before continuing.

- [ ] **Step 2: Add missing test cases for `Liveness::Live + state = "idle"` and `Liveness::Stopped`**

In `crates/agtop-cli/src/tui/refresh_adapter.rs`, add inside the `#[cfg(test)] mod tests` block (after the `historical_session_without_parser_state_stays_closed` test at line 248):

```rust
    #[test]
    fn live_process_with_idle_state_counted_as_idle() {
        use agtop_core::process::{Confidence, Liveness, ProcessMetrics};
        let mut a = analysis("idle-session");
        a.pid = Some(5678);
        a.liveness = Some(Liveness::Live);
        a.match_confidence = Some(Confidence::Medium);
        a.process_metrics = Some(ProcessMetrics {
            cpu_percent: 0.0,
            memory_bytes: 512,
            virtual_memory_bytes: 1024,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
        });
        a.summary.state = Some("idle".to_string());

        let (header, sessions) = apply_one(a);

        // is_active() is true for Idle, so it counts as active too.
        assert_eq!(header.sessions_active, 1, "idle sessions are active");
        assert_eq!(header.sessions_idle, 1, "idle sessions are also idle");
        assert!(matches!(
            sessions.rows[0].analysis.session_state,
            Some(SessionState::Idle)
        ));
    }

    #[test]
    fn stopped_process_is_closed_not_counted() {
        use agtop_core::process::{Confidence, Liveness};
        let mut a = analysis("stopped-session");
        a.pid = Some(9999);
        a.liveness = Some(Liveness::Stopped);
        a.match_confidence = Some(Confidence::Medium);

        let (header, sessions) = apply_one(a);

        assert_eq!(header.sessions_active, 0);
        assert_eq!(header.sessions_idle, 0);
        assert!(matches!(
            sessions.rows[0].analysis.session_state,
            Some(SessionState::Closed)
        ));
    }
```

- [ ] **Step 3: Run all tests to confirm green**

```bash
rtk cargo test -p agtop-cli refresh_adapter -- --test-threads=1
```

Expected: 5 tests pass (including the 2 new ones).

- [ ] **Step 4: Commit**

```bash
cd /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign
git add crates/agtop-cli/src/tui/refresh_adapter.rs
git commit -m "test(refresh_adapter): add idle and stopped liveness state tests"
```

---

## Task 2: Tab Bar Mouse Click Support

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/tab_bar.rs`
- Modify: `crates/agtop-cli/src/tui/app_v2.rs`

**Context:** `tab_bar.rs` is currently a free function `render(...)`. We need to convert it to a stateful struct so it can store tab rects between render and event handling. `app_v2.rs` owns the tab bar and routes events.

- [ ] **Step 1: Write the failing test for `TabBar::hit_test`**

At the bottom of `crates/agtop-cli/src/tui/widgets/tab_bar.rs`, inside the existing `#[cfg(test)] mod tests` block, add:

```rust
    #[test]
    fn hit_test_returns_correct_screen_for_each_tab() {
        let backend = TestBackend::new(120, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 120, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
        // The dashboard tab starts at column 9 (" agtop │ " = 9 chars).
        // "[d]ashboard" = 11 chars.
        // Check middle of [d]ashboard tab.
        assert_eq!(bar.hit_test(14, 0), Some(ScreenId::Dashboard));
        // Check a column before any tab (the "agtop" logo area).
        assert_eq!(bar.hit_test(2, 0), None);
    }

    #[test]
    fn hit_test_wrong_row_returns_none() {
        let backend = TestBackend::new(120, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 120, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
        // Wrong row.
        assert_eq!(bar.hit_test(14, 1), None);
    }
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
rtk cargo test -p agtop-cli tab_bar -- --test-threads=1
```

Expected: compile error (no `TabBar` struct or `hit_test` method yet).

- [ ] **Step 3: Refactor `tab_bar.rs` to stateful struct**

Replace the entire content of `crates/agtop-cli/src/tui/widgets/tab_bar.rs` with:

```rust
//! Top-of-screen view-switcher tab bar.
//!
//! Renders: ` agtop ─── [d]ashboard [a]ggregation [c]onfig  q=quit ?=help  vX.Y.Z `
//! Active view is wrapped in `accent.primary` color + bold; inactive views in `fg.muted`.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::msg::ScreenId;
use crate::tui::theme_v2::Theme;

/// Stateful tab bar that records each tab's rendered area for mouse hit-testing.
#[derive(Debug, Default)]
pub struct TabBar {
    /// (screen_id, rect) for each tab, populated by `render()`.
    tab_rects: Vec<(ScreenId, Rect)>,
}

impl TabBar {
    /// Render the tab bar and record tab rects for mouse hit-testing.
    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        current: ScreenId,
        version: &str,
        theme: &Theme,
    ) {
        self.tab_rects.clear();
        let mut spans: Vec<Span> = Vec::with_capacity(16);

        let logo = " agtop ";
        let sep = "│ ";
        spans.push(Span::styled(
            logo,
            Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(sep, Style::default().fg(theme.border_muted)));

        let mut x_offset: u16 = (logo.chars().count() + sep.chars().count()) as u16 + area.x;

        for (id, label) in [
            (ScreenId::Dashboard, "[d]ashboard"),
            (ScreenId::Aggregation, "[a]ggregation"),
            (ScreenId::Config, "[c]onfig"),
        ] {
            let w = label.chars().count() as u16;
            self.tab_rects.push((id, Rect::new(x_offset, area.y, w, 1)));
            x_offset += w;

            let style = if id == current {
                Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_muted)
            };
            spans.push(Span::styled(label, style));

            // Two trailing spaces after each tab.
            spans.push(Span::raw("  "));
            x_offset += 2;
        }

        spans.push(Span::styled("│  ", Style::default().fg(theme.border_muted)));
        spans.push(Span::styled("q=quit  ?=help", Style::default().fg(theme.fg_muted)));

        // Spacer pushing version to the right edge.
        let prefix_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let version_str = format!(" v{version} ");
        let pad = (area.width as usize)
            .saturating_sub(prefix_len)
            .saturating_sub(version_str.chars().count());
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(
            version_str,
            Style::default().fg(theme.fg_muted),
        ));

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Return the `ScreenId` whose tab was clicked, or `None`.
    pub fn hit_test(&self, column: u16, row: u16) -> Option<ScreenId> {
        for &(id, rect) in &self.tab_rects {
            if row == rect.y && column >= rect.x && column < rect.x + rect.width {
                return Some(id);
            }
        }
        None
    }
}

// Keep the old free function as a thin wrapper for any callers not yet migrated.
pub fn render(frame: &mut Frame<'_>, area: Rect, current: ScreenId, version: &str, theme: &Theme) {
    let mut bar = TabBar::default();
    bar.render(frame, area, current, version, theme);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn render_does_not_panic_on_narrow_terminal() {
        let backend = TestBackend::new(40, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 40, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_on_wide_terminal() {
        let backend = TestBackend::new(200, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 200, 1), ScreenId::Aggregation, "0.4.0", &theme))
            .unwrap();
    }

    #[test]
    fn hit_test_returns_correct_screen_for_each_tab() {
        let backend = TestBackend::new(120, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 120, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
        // " agtop " (7) + "│ " (2) = 9 chars before first tab.
        // "[d]ashboard" is 11 chars wide, x = 9..19.
        assert_eq!(bar.hit_test(14, 0), Some(ScreenId::Dashboard));
        // Before any tab.
        assert_eq!(bar.hit_test(2, 0), None);
    }

    #[test]
    fn hit_test_wrong_row_returns_none() {
        let backend = TestBackend::new(120, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 120, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
        assert_eq!(bar.hit_test(14, 1), None);
    }
}
```

- [ ] **Step 4: Update `app_v2.rs` to own a `TabBar` and route mouse clicks**

In `crates/agtop-cli/src/tui/app_v2.rs`:

1. Add `tab_bar: tab_bar::TabBar` to `App` struct (after `running` field):
```rust
pub struct App {
    pub current: ScreenId,
    pub theme: Theme,
    pub show_help: bool,
    pub running: bool,
    pub tab_bar: tab_bar::TabBar,   // ADD THIS

    pub dashboard: DashboardState,
    pub aggregation: AggregationState,
    pub config: ConfigState,
}
```

2. Add `tab_bar: tab_bar::TabBar::default()` to `Default::default()`:
```rust
impl Default for App {
    fn default() -> Self {
        Self {
            current: ScreenId::Dashboard,
            theme: theme_v2::vscode_dark_plus::theme(),
            show_help: false,
            running: true,
            tab_bar: tab_bar::TabBar::default(),    // ADD THIS
            dashboard: DashboardState::default(),
            aggregation: Default::default(),
            config: Default::default(),
        }
    }
}
```

3. In `App::render`, change `tab_bar::render(...)` to `self.tab_bar.render(...)`:
```rust
pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    self.tab_bar.render(frame, layout[0], self.current, env!("CARGO_PKG_VERSION"), &self.theme);  // CHANGED
    match self.current {
        ScreenId::Dashboard => self.dashboard.render(frame, layout[1], &self.theme),
        ScreenId::Aggregation => self.aggregation.render(frame, layout[1], &self.theme),
        ScreenId::Config => self.config.render(frame, layout[1], &self.theme),
    }
}
```

4. In `App::handle_event`, before routing to the active screen, add mouse tab hit-testing:
```rust
pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
    // Global keymap first.
    if let AppEvent::Key(k) = event {
        if let Some(m) = self.global_keymap(*k) { return Some(m); }
    }
    // Tab bar mouse click: switch screen.
    if let AppEvent::Mouse(me) = event {
        use crossterm::event::{MouseButton, MouseEventKind};
        if me.kind == MouseEventKind::Down(MouseButton::Left) {
            if let Some(screen) = self.tab_bar.hit_test(me.column, me.row) {
                return Some(Msg::SwitchScreen(screen));
            }
        }
    }
    // Then route to active screen.
    match self.current {
        ScreenId::Dashboard => self.dashboard.handle_event(event),
        ScreenId::Aggregation => self.aggregation.handle_event(event),
        ScreenId::Config => self.config.handle_event(event),
    }
}
```

- [ ] **Step 5: Run all tests**

```bash
rtk cargo test -p agtop-cli tab_bar app_v2 -- --test-threads=1
```

Expected: all pass including the two new `hit_test` tests.

- [ ] **Step 6: Commit**

```bash
cd /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign
git add crates/agtop-cli/src/tui/widgets/tab_bar.rs crates/agtop-cli/src/tui/app_v2.rs
git commit -m "feat(tab-bar): add mouse click support for screen switching"
```

---

## Task 3: Aggregation Controls Mouse Click Support

**Files:**
- Modify: `crates/agtop-cli/src/tui/screens/aggregation/controls.rs`
- Modify: `crates/agtop-cli/src/tui/screens/aggregation/mod.rs`

**Context:** `ControlsModel` is currently a `Copy` struct (no mutable state). We need to add `chip_rects` for hit-testing, which means switching to a non-Copy model. The `render` function needs to become a method on a stateful wrapper, or we pass `chip_rects` out. Simplest: make `ControlsModel` non-Copy and add `chip_rects` + `handle_event`.

- [ ] **Step 1: Write failing test for `ControlsModel::handle_event` on group-by click**

In `crates/agtop-cli/src/tui/screens/aggregation/controls.rs`, add inside the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn click_on_provider_chip_sets_group_by_provider() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;

        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut m = ControlsModel::default(); // starts at GroupBy::Client
        term.draw(|f| m.render(f, Rect::new(0, 0, 140, 2), &theme)).unwrap();

        // Find the "Provider" chip rect and click in the middle of it.
        let provider_rect = m.chip_rects.iter()
            .find(|(label, _)| *label == "Provider")
            .map(|(_, r)| *r)
            .expect("Provider chip rect must exist after render");

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: provider_rect.x + provider_rect.width / 2,
            row: provider_rect.y,
            modifiers: KeyModifiers::NONE,
        });
        m.handle_event(&click);
        assert!(matches!(m.group_by, agtop_core::aggregate::GroupBy::Provider));
    }
```

- [ ] **Step 2: Run to confirm compile error**

```bash
rtk cargo test -p agtop-cli controls -- --test-threads=1 2>&1 | head -30
```

Expected: compile errors (no `chip_rects`, `render` not a method, no `handle_event`).

- [ ] **Step 3: Rewrite `controls.rs` with stateful `ControlsModel`**

Replace the entire content of `crates/agtop-cli/src/tui/screens/aggregation/controls.rs` with:

```rust
//! Top-of-screen pickers: Group by + Range + Sort/Reverse.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use agtop_core::aggregate::{GroupBy, TimeRange};

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone)]
pub struct ControlsModel {
    pub group_by: GroupBy,
    pub range: TimeRange,
    pub sort_label: &'static str,
    pub reverse: bool,
    /// (label, rect) for each rendered chip.  Populated by `render()`.
    pub chip_rects: Vec<(String, Rect)>,
}

impl Default for ControlsModel {
    fn default() -> Self {
        Self {
            group_by: GroupBy::Client,
            range: TimeRange::Today,
            sort_label: "COST",
            reverse: false,
            chip_rects: Vec::new(),
        }
    }
}

impl ControlsModel {
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.chip_rects.clear();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);
        self.render_row1(frame, layout[0], theme);
        self.render_row2(frame, layout[1], theme);
    }

    fn render_row1(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let prefix = " Group by:  ";
        let mut spans = vec![Span::styled(prefix, Style::default().fg(theme.fg_muted))];
        let mut x = area.x + prefix.chars().count() as u16;

        for g in [GroupBy::Client, GroupBy::Provider, GroupBy::Model, GroupBy::Project, GroupBy::Subscription] {
            let label = match g {
                GroupBy::Client => "Client",
                GroupBy::Provider => "Provider",
                GroupBy::Model => "Model",
                GroupBy::Project => "Project",
                GroupBy::Subscription => "Subscription",
            };
            let chip_str = if g == self.group_by {
                format!("‹ {label} › ")
            } else {
                format!("  {label}   ")
            };
            let chip_w = chip_str.chars().count() as u16;
            self.chip_rects.push((label.to_string(), Rect::new(x, area.y, chip_w, 1)));
            x += chip_w;

            let style = if g == self.group_by {
                Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_muted)
            };
            spans.push(Span::styled(chip_str, style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_row2(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let prefix = " Range:     ";
        let mut spans = vec![Span::styled(prefix, Style::default().fg(theme.fg_muted))];
        let mut x = area.x + prefix.chars().count() as u16;

        for r in [TimeRange::Today, TimeRange::Week, TimeRange::Month, TimeRange::All] {
            let label = match r {
                TimeRange::Today => "Today",
                TimeRange::Week => "Week",
                TimeRange::Month => "Month",
                TimeRange::All => "All",
            };
            let chip_str = if r == self.range {
                format!("‹ {label} › ")
            } else {
                format!("  {label}   ")
            };
            let chip_w = chip_str.chars().count() as u16;
            self.chip_rects.push((label.to_string(), Rect::new(x, area.y, chip_w, 1)));
            x += chip_w;

            let style = if r == self.range {
                Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_muted)
            };
            spans.push(Span::styled(chip_str, style));
        }

        // Sort chip
        let sort_sep = "  |  Sort: ";
        spans.push(Span::styled(sort_sep, Style::default().fg(theme.fg_muted)));
        x += sort_sep.chars().count() as u16;
        let sort_str = format!("‹{}›", self.sort_label);
        let sort_w = sort_str.chars().count() as u16;
        self.chip_rects.push(("__sort__".to_string(), Rect::new(x, area.y, sort_w, 1)));
        x += sort_w;
        spans.push(Span::styled(
            sort_str,
            Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD),
        ));

        // Reverse toggle
        let rev_sep = "  Reverse: ";
        spans.push(Span::styled(rev_sep, Style::default().fg(theme.fg_muted)));
        x += rev_sep.chars().count() as u16;
        let rev_str = if self.reverse { "on" } else { "off" };
        let rev_w = rev_str.chars().count() as u16;
        self.chip_rects.push(("__reverse__".to_string(), Rect::new(x, area.y, rev_w, 1)));
        spans.push(Span::styled(
            rev_str,
            Style::default().fg(if self.reverse { theme.accent_primary } else { theme.fg_muted }),
        ));

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Handle an event. Returns `Some(Msg::Noop)` if state changed, `None` otherwise.
    /// **Does not call `recompute()`** — callers (e.g. `AggregationState`) must do that.
    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        if let AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            ..
        }) = event
        {
            for (label, rect) in &self.chip_rects {
                if *row == rect.y && *column >= rect.x && *column < rect.x + rect.width {
                    match label.as_str() {
                        "Client" => self.group_by = GroupBy::Client,
                        "Provider" => self.group_by = GroupBy::Provider,
                        "Model" => self.group_by = GroupBy::Model,
                        "Project" => self.group_by = GroupBy::Project,
                        "Subscription" => self.group_by = GroupBy::Subscription,
                        "Today" => self.range = TimeRange::Today,
                        "Week" => self.range = TimeRange::Week,
                        "Month" => self.range = TimeRange::Month,
                        "All" => self.range = TimeRange::All,
                        "__sort__" => { /* cycle sort — handled by caller */ }
                        "__reverse__" => self.reverse = !self.reverse,
                        _ => {}
                    }
                    return Some(Msg::Noop);
                }
            }
        }
        None
    }
}

// Keep free function for compile compatibility during migration.
pub fn render(frame: &mut Frame<'_>, area: Rect, m: &ControlsModel, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Static render (no rect recording) — used only by old callers.
    let mut spans = vec![Span::styled(" Group by:  ", Style::default().fg(theme.fg_muted))];
    for g in [GroupBy::Client, GroupBy::Provider, GroupBy::Model, GroupBy::Project, GroupBy::Subscription] {
        let label = match g {
            GroupBy::Client => "Client", GroupBy::Provider => "Provider",
            GroupBy::Model => "Model", GroupBy::Project => "Project",
            GroupBy::Subscription => "Subscription",
        };
        let style = if g == m.group_by {
            Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
        } else { Style::default().fg(theme.fg_muted) };
        let s = if g == m.group_by { format!("‹ {label} › ") } else { format!("  {label}   ") };
        spans.push(Span::styled(s, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), layout[0]);

    let mut spans2 = vec![Span::styled(" Range:     ", Style::default().fg(theme.fg_muted))];
    for r in [TimeRange::Today, TimeRange::Week, TimeRange::Month, TimeRange::All] {
        let label = match r {
            TimeRange::Today => "Today", TimeRange::Week => "Week",
            TimeRange::Month => "Month", TimeRange::All => "All",
        };
        let style = if r == m.range {
            Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
        } else { Style::default().fg(theme.fg_muted) };
        let s = if r == m.range { format!("‹ {label} › ") } else { format!("  {label}   ") };
        spans2.push(Span::styled(s, style));
    }
    spans2.push(Span::styled("  |  Sort: ", Style::default().fg(theme.fg_muted)));
    spans2.push(Span::styled(format!("‹{}›", m.sort_label), Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)));
    spans2.push(Span::styled("  Reverse: ", Style::default().fg(theme.fg_muted)));
    spans2.push(Span::styled(if m.reverse { "on" } else { "off" }, Style::default().fg(if m.reverse { theme.accent_primary } else { theme.fg_muted })));
    frame.render_widget(Paragraph::new(Line::from(spans2)), layout[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn renders_without_panic() {
        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut m = ControlsModel::default();
        term.draw(|f| m.render(f, Rect::new(0, 0, 140, 2), &theme)).unwrap();
    }

    #[test]
    fn click_on_provider_chip_sets_group_by_provider() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;

        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut m = ControlsModel::default();
        term.draw(|f| m.render(f, Rect::new(0, 0, 140, 2), &theme)).unwrap();

        let provider_rect = m.chip_rects.iter()
            .find(|(label, _)| label == "Provider")
            .map(|(_, r)| *r)
            .expect("Provider chip rect must exist after render");

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: provider_rect.x + provider_rect.width / 2,
            row: provider_rect.y,
            modifiers: KeyModifiers::NONE,
        });
        m.handle_event(&click);
        assert!(matches!(m.group_by, agtop_core::aggregate::GroupBy::Provider));
    }

    #[test]
    fn click_on_reverse_toggle_flips_it() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;

        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut m = ControlsModel::default();
        assert!(!m.reverse);
        term.draw(|f| m.render(f, Rect::new(0, 0, 140, 2), &theme)).unwrap();

        let rev_rect = m.chip_rects.iter()
            .find(|(label, _)| label == "__reverse__")
            .map(|(_, r)| *r)
            .expect("__reverse__ chip rect must exist after render");

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rev_rect.x,
            row: rev_rect.y,
            modifiers: KeyModifiers::NONE,
        });
        m.handle_event(&click);
        assert!(m.reverse, "reverse should flip to true");
        m.handle_event(&click);
        assert!(!m.reverse, "reverse should flip back to false");
    }
}
```

- [ ] **Step 4: Update `AggregationState::render` and `handle_event` in `mod.rs`**

In `crates/agtop-cli/src/tui/screens/aggregation/mod.rs`:

Change `controls::render(frame, layout[0], &self.controls, theme)` to `self.controls.render(frame, layout[0], theme)` at line 49.

In `handle_event`, after the `if self.drill.is_open()` guard, add a mouse routing clause before the `KeyCode` match:

```rust
    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        // Drill-down captures events when open.
        if self.drill.is_open() {
            return self.drill.handle_event(event);
        }

        // Route mouse clicks to controls (group-by / range chips).
        if matches!(event, AppEvent::Mouse(_)) {
            if let Some(msg) = self.controls.handle_event(event) {
                self.recompute();
                return Some(msg);
            }
        }

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let AppEvent::Key(KeyEvent { code, modifiers, .. }) = event else { return None };
        // ... rest unchanged
```

- [ ] **Step 5: Run all tests**

```bash
rtk cargo test -p agtop-cli controls aggregation -- --test-threads=1
```

Expected: all pass including the 2 new click tests.

- [ ] **Step 6: Commit**

```bash
cd /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign
git add crates/agtop-cli/src/tui/screens/aggregation/controls.rs crates/agtop-cli/src/tui/screens/aggregation/mod.rs
git commit -m "feat(aggregation): add mouse click support for group-by, range, and reverse controls"
```

---

## Task 4: Drill-down Close Button Mouse Click

**Files:**
- Modify: `crates/agtop-cli/src/tui/screens/aggregation/drilldown.rs`

- [ ] **Step 1: Write failing test**

In `crates/agtop-cli/src/tui/screens/aggregation/drilldown.rs`, add a `#[cfg(test)] mod tests` block at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn click_on_esc_close_button_closes_drilldown() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;

        let backend = TestBackend::new(120, 30);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();

        let mut d = DrillDown::default();
        d.open("Claude".to_string(), &[], GroupBy::Client);
        assert!(d.is_open());

        // Render to populate last_area.
        term.draw(|f| d.render(f, ratatui::layout::Rect::new(0, 0, 120, 30), &theme)).unwrap();

        // The title bar row is y=0. Title is " Drill-down: Claude  [Esc] close ".
        // "[Esc]" starts at offset 22 in the title (after " Drill-down: Claude  ").
        // With a left border at x=0, title chars start at x=1.
        let area = d.last_area.unwrap();
        let title_str = format!(" Drill-down: {}  [Esc] close ", "Claude");
        let esc_offset = title_str.find("[Esc]").unwrap() as u16;
        let click_col = area.x + 1 + esc_offset + 1; // +1 for border, +1 for inside "[Esc]"

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: click_col,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        });
        d.handle_event(&click);
        assert!(!d.is_open(), "clicking [Esc] close must close the drill-down");
    }
}
```

- [ ] **Step 2: Run to confirm compile error / test failure**

```bash
rtk cargo test -p agtop-cli drilldown -- --test-threads=1 2>&1 | head -20
```

Expected: compile error (no `last_area` field).

- [ ] **Step 3: Add `last_area` field and mouse handler to `drilldown.rs`**

In `crates/agtop-cli/src/tui/screens/aggregation/drilldown.rs`:

1. Add `last_area` to `DrillDown`:
```rust
#[derive(Debug, Default)]
pub struct DrillDown {
    open: bool,
    label: String,
    table: SessionsTable,
    last_area: Option<Rect>,   // ADD THIS
}
```

2. Set `last_area` in `render`:
```rust
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        if !self.open { return }
        self.last_area = Some(area);   // ADD THIS
        frame.render_widget(Clear, area);
        // ... rest unchanged
```

3. Add mouse handling in `handle_event`:
```rust
    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
        // Keyboard Esc closes.
        if let AppEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) = event {
            self.open = false;
            return Some(Msg::Noop);
        }
        // Mouse click on "[Esc] close" in the title bar closes.
        if let AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column, row, ..
        }) = event {
            if let Some(area) = self.last_area {
                if *row == area.y {
                    let title = format!(" Drill-down: {}  [Esc] close ", self.label);
                    let title_start = area.x + 1; // +1 for left border
                    if *column >= title_start {
                        let rel = (*column - title_start) as usize;
                        if let Some(esc_pos) = title.find("[Esc]") {
                            let esc_end = esc_pos + "[Esc] close".len();
                            if rel >= esc_pos && rel < esc_end {
                                self.open = false;
                                return Some(Msg::Noop);
                            }
                        }
                    }
                }
            }
        }
        // Forward to inner table.
        self.table.handle_event(event)
    }
```

Also add these imports at the top of `handle_event` — the function already has them; just make sure `MouseButton`, `MouseEvent`, `MouseEventKind` are included.

- [ ] **Step 4: Also add `last_area` to the `DrillDown::open` constructor (reset it)**

After `self.open = true;` in `DrillDown::open`, add:
```rust
        self.last_area = None;
```

- [ ] **Step 5: Run all tests**

```bash
rtk cargo test -p agtop-cli drilldown -- --test-threads=1
```

Expected: 1 new test passes.

- [ ] **Step 6: Commit**

```bash
cd /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign
git add crates/agtop-cli/src/tui/screens/aggregation/drilldown.rs
git commit -m "feat(drilldown): add mouse click to close button"
```

---

## Task 5: Subagent Tree View in Sessions Table

**Files:**
- Modify: `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs`
- Modify: `crates/agtop-cli/src/tui/refresh_adapter.rs`

**Context:** `SessionAnalysis.children: Vec<SessionAnalysis>` is already populated by the refresh layer. We add `depth`/`parent_session_id` to `SessionRow`, `collapsed` to `SessionsTable`, and build child rows in `apply_analyses`.

- [ ] **Step 1: Write failing tests for tree structure**

In `crates/agtop-cli/src/tui/refresh_adapter.rs`, add inside `mod tests` (after the existing tests):

```rust
    #[test]
    fn children_appear_as_depth_1_rows_after_parent() {
        use agtop_core::session::{SessionAnalysis, SessionSummary, TokenTotals, CostBreakdown, ClientKind};
        use agtop_core::process::Liveness;
        use chrono::Utc;

        let mut parent = analysis("parent-1");
        parent.liveness = Some(Liveness::Live);

        let child_summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "child-1".to_string(),
            None,
            Some(Utc::now()),
            None,
            None,
            std::path::PathBuf::from("/tmp/child.jsonl"),
            None,
            None,
            None,
            None,
        );
        let child = SessionAnalysis::new(
            child_summary,
            TokenTotals::default(),
            CostBreakdown::default(),
            None, 0, None, None, None, None, None,
        );
        parent.children = vec![child];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = crate::tui::screens::dashboard::quota::QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(&[parent], &mut header, &mut sessions, &mut quota, &mut aggregation, 5);

        assert_eq!(sessions.rows.len(), 2, "parent + 1 child = 2 rows");
        assert_eq!(sessions.rows[0].depth, 0, "parent is depth 0");
        assert_eq!(sessions.rows[1].depth, 1, "child is depth 1");
        assert_eq!(
            sessions.rows[1].parent_session_id.as_deref(),
            Some("parent-1"),
            "child parent_session_id must point to parent"
        );
    }

    #[test]
    fn collapsed_parent_hides_children() {
        use agtop_core::session::{SessionAnalysis, SessionSummary, TokenTotals, CostBreakdown, ClientKind};
        use agtop_core::process::Liveness;
        use chrono::Utc;
        use std::collections::HashSet;

        let mut parent = analysis("parent-collapsed");
        parent.liveness = Some(Liveness::Live);
        let child_summary = SessionSummary::new(
            ClientKind::Claude, None, "child-collapsed".to_string(), None,
            Some(Utc::now()), None, None, std::path::PathBuf::from("/tmp/c.jsonl"),
            None, None, None, None,
        );
        let child = SessionAnalysis::new(child_summary, TokenTotals::default(), CostBreakdown::default(), None, 0, None, None, None, None, None);
        parent.children = vec![child];

        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        sessions.collapsed.insert("parent-collapsed".to_string());
        let mut quota = crate::tui::screens::dashboard::quota::QuotaPanel::default();
        let mut aggregation = AggregationState::default();
        apply_analyses(&[parent], &mut header, &mut sessions, &mut quota, &mut aggregation, 5);

        assert_eq!(sessions.rows.len(), 1, "collapsed parent hides children");
    }
```

- [ ] **Step 2: Run to confirm compile errors**

```bash
rtk cargo test -p agtop-cli refresh_adapter -- --test-threads=1 2>&1 | head -30
```

Expected: compile errors (no `depth`, `parent_session_id` on `SessionRow`, no `collapsed` on `SessionsTable`).

- [ ] **Step 3: Add `depth` and `parent_session_id` to `SessionRow`**

In `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs`, change `SessionRow`:

```rust
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub analysis: SessionAnalysis,
    pub client_kind: ClientKind,
    pub client_label: String,
    /// Recent token-rate samples (oldest → newest), used for ACTIVITY sparkline.
    pub activity_samples: Vec<f32>,
    /// 0 = top-level session; 1 = child subagent.
    pub depth: u8,
    /// Session ID of the parent, if this is a child row (depth == 1).
    pub parent_session_id: Option<String>,
}
```

- [ ] **Step 4: Add `collapsed` to `SessionsTable`**

In `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs`, add import at top:
```rust
use std::collections::HashSet;
```

Change `SessionsTable`:
```rust
pub struct SessionsTable {
    pub rows: Vec<SessionRow>,
    pub state: TableState,
    pub pulse: PulseClock,
    pub animations_enabled: bool,
    pub sort_key: SessionSortKey,
    pub sort_dir: SortDir,
    /// Rect of the last-rendered table widget, set by `render()`.
    pub table_area: Rect,
    /// Session IDs of collapsed parent rows (children not shown).
    pub collapsed: HashSet<String>,
}
```

Add `collapsed: HashSet::new()` to `Default::default()`:
```rust
impl Default for SessionsTable {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            state: TableState::default(),
            pulse: PulseClock::default(),
            animations_enabled: true,
            sort_key: SessionSortKey::Age,
            sort_dir: SortDir::Desc,
            table_area: Rect::default(),
            collapsed: HashSet::new(),
        }
    }
}
```

- [ ] **Step 5: Fix all `SessionRow` construction sites to add `depth: 0, parent_session_id: None`**

Search for all `SessionRow {` constructions:
```bash
rtk grep -n "SessionRow {" /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign/crates/agtop-cli/src/tui/
```

Add `depth: 0, parent_session_id: None,` to every existing `SessionRow { ... }` literal in:
- `refresh_adapter.rs` lines ~36-43
- `drilldown.rs` lines ~33-39
- `sessions.rs` test helper `mock_row`

- [ ] **Step 6: Update `apply_analyses` to insert child rows**

In `crates/agtop-cli/src/tui/refresh_adapter.rs`, in `apply_analyses`, replace the `sessions.rows = ...` block with:

```rust
    // --- Sessions ---
    let mut flat_rows: Vec<SessionRow> = Vec::new();
    for a in &normalized {
        let kind = a.summary.client;
        let label = kind.as_str().to_string();
        flat_rows.push(SessionRow {
            analysis: a.clone(),
            client_kind: kind,
            client_label: label.clone(),
            activity_samples: vec![],
            depth: 0,
            parent_session_id: None,
        });
        // Insert children unless this parent is collapsed.
        if !a.children.is_empty() && !sessions.collapsed.contains(&a.summary.session_id) {
            let mut children: Vec<&SessionAnalysis> = a.children.iter().collect();
            // Sort children by started_at descending (newest first).
            children.sort_by(|x, y| y.summary.started_at.cmp(&x.summary.started_at));
            for child in children {
                let child_kind = child.summary.client;
                flat_rows.push(SessionRow {
                    analysis: child.clone(),
                    client_kind: child_kind,
                    client_label: child_kind.as_str().to_string(),
                    activity_samples: vec![],
                    depth: 1,
                    parent_session_id: Some(a.summary.session_id.clone()),
                });
            }
        }
    }
    sessions.rows = flat_rows;
    sessions.apply_sort();
```

- [ ] **Step 7: Update `apply_sort` to preserve child ordering**

In `sessions.rs`, replace the entire `apply_sort` function (currently lines 251-334) with a version that:
1. Partitions rows into top-level (depth=0) and children (depth=1).
2. Sorts only the top-level rows.
3. Rebuilds the flat list with children anchored under their parent.

First, add a free function `sort_cmp` before `apply_sort` (after `project_basename` at line 543):

```rust
fn sort_cmp(a: &SessionRow, b: &SessionRow, key: SessionSortKey) -> std::cmp::Ordering {
    match key {
        SessionSortKey::Age => a
            .analysis.summary.last_active
            .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
            .cmp(&b.analysis.summary.last_active.unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)),
        SessionSortKey::Session => a.analysis.summary.session_id.cmp(&b.analysis.summary.session_id),
        SessionSortKey::Client => a.client_label.cmp(&b.client_label),
        SessionSortKey::Cost => a.analysis.cost.total
            .partial_cmp(&b.analysis.cost.total)
            .unwrap_or(std::cmp::Ordering::Equal),
        SessionSortKey::Tokens => a.analysis.tokens.grand_total().cmp(&b.analysis.tokens.grand_total()),
        SessionSortKey::Cpu => {
            let ca = a.analysis.process_metrics.as_ref().map(|m| m.cpu_percent).unwrap_or(0.0);
            let cb = b.analysis.process_metrics.as_ref().map(|m| m.cpu_percent).unwrap_or(0.0);
            ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
        }
        SessionSortKey::Memory => {
            let ma = a.analysis.process_metrics.as_ref().map(|m| m.memory_bytes).unwrap_or(0);
            let mb = b.analysis.process_metrics.as_ref().map(|m| m.memory_bytes).unwrap_or(0);
            ma.cmp(&mb)
        }
        _ => std::cmp::Ordering::Equal,
    }
}
```

Then replace the `apply_sort` method body:

```rust
    pub fn apply_sort(&mut self) {
        let (mut tops, child_rows): (Vec<_>, Vec<_>) = std::mem::take(&mut self.rows)
            .into_iter()
            .partition(|r| r.depth == 0);

        // Build map: parent_session_id -> children (order preserved from apply_analyses).
        let mut children_map: std::collections::HashMap<String, Vec<SessionRow>> = std::collections::HashMap::new();
        for c in child_rows {
            if let Some(ref pid) = c.parent_session_id {
                children_map.entry(pid.clone()).or_default().push(c);
            }
        }

        // Sort top-level rows.
        let key = self.sort_key;
        let dir = self.sort_dir;
        tops.sort_by(|a, b| {
            let ord = sort_cmp(a, b, key);
            if dir == SortDir::Desc { ord.reverse() } else { ord }
        });

        // Rebuild: each top-level row followed by its children.
        for row in tops {
            let session_id = row.analysis.summary.session_id.clone();
            self.rows.push(row);
            if let Some(mut kids) = children_map.remove(&session_id) {
                self.rows.append(&mut kids);
            }
        }
    }
```

- [ ] **Step 8: Add toggle cell rendering for parent rows with children**

In `sessions.rs`, in the `render_row` function (around line 139), modify the state dot cell (first column) for depth=0 rows that have children:

Find the existing state dot cell construction:
```rust
cells.push(Cell::from(Line::from(state_dot::render(
    &state, &self.pulse, self.animations_enabled, theme,
))));
```

Replace it with:
```rust
// For depth=0 rows with children, show collapse toggle before dot.
let dot_span = state_dot::render(&state, &self.pulse, self.animations_enabled, theme);
let first_cell = if row.depth == 0 && !row.analysis.children.is_empty() {
    let toggle = if self.collapsed.contains(&row.analysis.summary.session_id) {
        Span::raw("▶ ")
    } else {
        Span::raw("▼ ")
    };
    Cell::from(Line::from(vec![toggle, dot_span]))
} else if row.depth == 1 {
    // Indent child rows.
    Cell::from(Line::from(vec![Span::raw("  "), dot_span]))
} else {
    Cell::from(Line::from(vec![dot_span]))
};
cells.push(first_cell);
```

- [ ] **Step 9: Add collapse toggle keyboard handler**

In `SessionsTable::handle_event`, in the `KeyCode::Enter` arm (or add a new `KeyCode::Char(' ')` arm), add toggle logic:

```rust
KeyCode::Enter | KeyCode::Char(' ') => {
    if let Some(idx) = self.state.selected() {
        if let Some(row) = self.rows.get(idx) {
            if row.depth == 0 && !row.analysis.children.is_empty() {
                let sid = row.analysis.summary.session_id.clone();
                if self.collapsed.contains(&sid) {
                    self.collapsed.remove(&sid);
                } else {
                    self.collapsed.insert(sid);
                }
                return Some(Msg::Noop);
            }
        }
    }
    None
}
```

- [ ] **Step 10: Run all tests**

```bash
rtk cargo test --workspace -- --test-threads=1
```

Expected: 947+ tests pass (original 945 + 2 new tree tests).

- [ ] **Step 11: Commit**

```bash
cd /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign
git add crates/agtop-cli/src/tui/screens/dashboard/sessions.rs crates/agtop-cli/src/tui/refresh_adapter.rs
git commit -m "feat(sessions): add toggleable subagent tree view with indent and collapse"
```

---

## Task 6: Quota Long Panel — 2-Column Layout + Fixed Height + Scroll

**Files:**
- Modify: `crates/agtop-cli/src/tui/screens/dashboard/quota.rs`

- [ ] **Step 1: Write failing tests**

In `quota.rs`, add to the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn scroll_offset_advances_on_scroll_down_within_area() {
        use crossterm::event::{MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;
        use ratatui::layout::Rect;

        let mut panel = QuotaPanel::default();
        panel.mode = QuotaMode::Long;
        panel.last_area = Some(Rect::new(0, 10, 80, 10));
        // Add enough cards to overflow.
        panel.cards = (0..4).map(|i| QuotaCardModel {
            client_kind: agtop_core::session::ClientKind::Claude,
            client_label: format!("Provider {i}"),
            closest: WindowModel { label: "5h".into(), used_pct: 0.5, note: None, reset_in: None },
            all_windows: vec![
                WindowModel { label: "5h".into(), used_pct: 0.5, note: None, reset_in: None },
                WindowModel { label: "1d".into(), used_pct: 0.3, note: None, reset_in: None },
            ],
        }).collect();

        let scroll_down = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 12, // within last_area (y=10, height=10)
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(panel.scroll_offset, 0);
        panel.handle_event(&scroll_down);
        assert_eq!(panel.scroll_offset, 1);
    }

    #[test]
    fn scroll_up_does_not_go_below_zero() {
        use crossterm::event::{MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;
        use ratatui::layout::Rect;

        let mut panel = QuotaPanel::default();
        panel.mode = QuotaMode::Long;
        panel.last_area = Some(Rect::new(0, 0, 80, 10));
        assert_eq!(panel.scroll_offset, 0);

        let scroll_up = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        panel.handle_event(&scroll_up);
        assert_eq!(panel.scroll_offset, 0, "scroll_offset must not go below 0");
    }

    #[test]
    fn mode_cycle_resets_scroll_offset() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use crate::tui::input::AppEvent;

        let mut panel = QuotaPanel::default();
        panel.mode = QuotaMode::Long;
        panel.scroll_offset = 5;

        let u_key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        panel.handle_event(&u_key);
        assert_eq!(panel.scroll_offset, 0, "cycling mode must reset scroll offset");
    }
```

- [ ] **Step 2: Run to confirm compile errors**

```bash
rtk cargo test -p agtop-cli quota -- --test-threads=1 2>&1 | head -20
```

Expected: compile errors (no `scroll_offset` field).

- [ ] **Step 3: Add `scroll_offset` to `QuotaPanel`**

In `quota.rs`, change `QuotaPanel`:
```rust
#[derive(Debug, Default)]
pub struct QuotaPanel {
    pub mode: QuotaMode,
    pub cards: Vec<QuotaCardModel>,
    /// Last render area; used to hit-test mouse clicks on the `[u]` title button.
    pub last_area: Option<Rect>,
    /// Scroll offset for Long mode (lines scrolled from top).
    pub scroll_offset: usize,
}
```

- [ ] **Step 4: Update `rows_needed` for Long mode**

Change line 41:
```rust
Self::Long => 10,
```

- [ ] **Step 5: Rewrite `render_long` with 2-column layout and scroll**

Replace the `render_long` function body (lines 125-197) with:

```rust
    fn render_long(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Usage Quota (long)  [u]sage ",
                Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(theme.border_muted));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let label_width = self.cards.iter()
            .flat_map(|c| c.all_windows.iter().map(|w| w.label.len()))
            .max()
            .unwrap_or(4)
            .max(4);

        let build_card_lines = |card: &QuotaCardModel| -> Vec<Line<'static>> {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                card.client_label.clone(),
                Style::default()
                    .fg(client_palette::color_for(card.client_kind))
                    .add_modifier(Modifier::BOLD),
            )));
            for w in &card.all_windows {
                let (filled, color, empty) = gradient_bar::render_bar(w.used_pct, 18, theme);
                let mut spans: Vec<Span<'static>> = vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{:>width$}", w.label, width = label_width),
                        Style::default().fg(theme.fg_muted),
                    ),
                    Span::raw("  "),
                    Span::styled(filled, Style::default().fg(color)),
                    Span::styled(empty, Style::default().fg(theme.border_muted)),
                    Span::raw("  "),
                    Span::styled(
                        format!("{:>3.0}%", w.used_pct * 100.0),
                        Style::default().fg(theme.fg_default),
                    ),
                ];
                if let Some(note) = &w.note {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(note.clone(), Style::default().fg(theme.fg_muted)));
                }
                if let Some(reset) = &w.reset_in {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(reset.clone(), Style::default().fg(theme.fg_muted)));
                }
                lines.push(Line::from(spans));
            }
            lines.push(Line::from(""));
            lines
        };

        let inner_h = inner.height as usize;
        // Reserve 1 row for overflow indicators if needed.
        let view_h = inner_h.saturating_sub(1).max(1);

        if area.width > 80 && self.cards.len() >= 2 {
            // 2-column layout.
            use ratatui::layout::{Direction, Layout, Constraint};
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let mid = (self.cards.len() + 1) / 2;
            let left_lines: Vec<Line<'_>> = self.cards[..mid].iter()
                .flat_map(|c| build_card_lines(c))
                .collect();
            let right_lines: Vec<Line<'_>> = self.cards[mid..].iter()
                .flat_map(|c| build_card_lines(c))
                .collect();

            let total = left_lines.len().max(right_lines.len());
            let offset = self.scroll_offset.min(total.saturating_sub(view_h));

            let render_col = |lines: Vec<Line<'_>>, col_rect: Rect| {
                let mut visible: Vec<Line<'_>> = lines.into_iter().skip(offset).take(view_h).collect();
                if offset > 0 {
                    // prepend scroll-up indicator if possible
                    if visible.len() == view_h {
                        visible.insert(0, Line::from(Span::styled(
                            format!("  ↑ {offset} lines above"),
                            Style::default().fg(theme.fg_muted),
                        )));
                        visible.truncate(view_h);
                    }
                }
                frame.render_widget(Paragraph::new(visible), col_rect);
            };

            // Show overflow hint on last row of inner area.
            let overflow_below = total.saturating_sub(offset + view_h);
            if overflow_below > 0 {
                let hint = Line::from(Span::styled(
                    format!("  ↓ {overflow_below} more lines — press [u] for short view"),
                    Style::default().fg(theme.fg_muted),
                ));
                let hint_rect = Rect::new(inner.x, inner.y + inner_h as u16 - 1, inner.width, 1);
                frame.render_widget(Paragraph::new(hint), hint_rect);
            }

            render_col(left_lines, cols[0]);
            render_col(right_lines, cols[1]);
        } else {
            // Single-column layout.
            let all_lines: Vec<Line<'_>> = self.cards.iter()
                .flat_map(|c| build_card_lines(c))
                .collect();
            let total = all_lines.len();
            let offset = self.scroll_offset.min(total.saturating_sub(view_h));
            let mut visible: Vec<Line<'_>> = all_lines.into_iter().skip(offset).take(view_h).collect();

            let overflow_below = total.saturating_sub(offset + view_h);
            if overflow_below > 0 {
                visible.push(Line::from(Span::styled(
                    format!("  ↓ {overflow_below} more lines — press [u] for short view"),
                    Style::default().fg(theme.fg_muted),
                )));
            }
            frame.render_widget(Paragraph::new(visible), inner);
        }
    }
```

**Note:** The closure `build_card_lines` borrows `self` and `theme` immutably, which is fine since `render_long` takes `&self`. However, `Line<'_>` lifetime must be compatible. If there are lifetime issues with `build_card_lines` as a closure, convert it to an inner free function taking `card: &QuotaCardModel, label_width: usize, theme: &Theme`.

- [ ] **Step 6: Update `handle_event` to handle scroll and reset on mode cycle**

In `QuotaPanel::handle_event`, update the `'u'` key arm to reset `scroll_offset`:
```rust
AppEvent::Key(KeyEvent { code: KeyCode::Char('u'), modifiers, .. })
    if modifiers.is_empty() || *modifiers == KeyModifiers::SHIFT =>
{
    self.mode = self.mode.cycle();
    self.scroll_offset = 0;   // ADD THIS
    Some(Msg::Noop)
}
```

Also update the mouse click arm for the `[u]` button:
```rust
// In the existing mouse handler, after `self.mode = self.mode.cycle();`:
self.scroll_offset = 0;  // ADD after mode cycle in the mouse handler too
```

Add scroll event handling. Add a new arm before the final `_ => None`:
```rust
AppEvent::Mouse(MouseEvent {
    kind: kind @ (MouseEventKind::ScrollDown | MouseEventKind::ScrollUp),
    column,
    row,
    ..
}) => {
    if let Some(area) = self.last_area {
        // Only handle scroll when in Long mode and within the panel area.
        if self.mode == QuotaMode::Long
            && *row >= area.y
            && *row < area.y + area.height
            && *column >= area.x
            && *column < area.x + area.width
        {
            if *kind == MouseEventKind::ScrollDown {
                self.scroll_offset += 1;
                // max_offset is clamped in render; here we just increment
                // (render clamps it anyway, no UB if over-incremented).
            } else {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            return Some(Msg::Noop);
        }
    }
    None
}
```

Also add `'j'`/`'k'` keys for scroll when in Long mode:
```rust
AppEvent::Key(KeyEvent { code: KeyCode::Char('j'), modifiers, .. })
    if (modifiers.is_empty()) && self.mode == QuotaMode::Long =>
{
    self.scroll_offset += 1;
    Some(Msg::Noop)
}
AppEvent::Key(KeyEvent { code: KeyCode::Char('k'), modifiers, .. })
    if (modifiers.is_empty()) && self.mode == QuotaMode::Long =>
{
    self.scroll_offset = self.scroll_offset.saturating_sub(1);
    Some(Msg::Noop)
}
```

- [ ] **Step 7: Run all quota tests**

```bash
rtk cargo test -p agtop-cli quota -- --test-threads=1
```

Expected: 6+ tests pass including the 3 new scroll tests.

- [ ] **Step 8: Run full test suite**

```bash
rtk cargo test --workspace -- --test-threads=1
```

Expected: 950+ tests pass, 6 ignored.

- [ ] **Step 9: Commit**

```bash
cd /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign
git add crates/agtop-cli/src/tui/screens/dashboard/quota.rs
git commit -m "feat(quota): 2-column layout, fixed height, scroll in Long mode"
```

---

## Task 7: Final Integration Check

- [ ] **Step 1: Run complete test suite**

```bash
rtk cargo test --workspace -- --test-threads=1
```

Expected: all original 945 tests pass + new tests added in Tasks 1-6, 6 ignored unchanged.

- [ ] **Step 2: Build release binary**

```bash
rtk cargo build -p agtop-cli --release 2>&1 | tail -5
```

Expected: no errors, only warnings (existing `#[allow(dead_code)]` suppresses most).

- [ ] **Step 3: Smoke-test the TUI manually**

```bash
cargo run -p agtop-cli -- tui
```

Verify:
- Clicking `[d]ashboard`, `[a]ggregation`, `[c]onfig` in the tab bar switches screens
- In Aggregation: clicking `‹ Provider ›`, `‹ Week ›`, `Reverse: on/off` chips works
- In Aggregation drill-down: clicking `[Esc] close` closes the overlay
- Sessions table: parent rows with children show `▼`/`▶`; `Space`/`Enter` collapses/expands
- Quota Long mode: scrolls with `j`/`k` and mouse wheel; 2-column layout on wide terminals
- Session counter in header shows non-zero when live sessions exist

- [ ] **Step 4: Commit if any fixes needed**

Fix any issues found in smoke test, commit individually.
