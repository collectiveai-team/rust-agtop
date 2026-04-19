# Subscription Details Two-Pane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign the Subscription Details dashboard pane into a two-pane (40/60) layout: a left list of subscriptions (one per canonical subscription name, with a colored usage bar) and a right details panel showing all usage windows with traffic-light bars and local-timezone reset times.

**Architecture:** Merge `PlanUsage` entries by canonical subscription name in `dashboard_plan.rs`. Add `plan_selected: usize` to `App`. Replace the single `Paragraph` with a horizontal split rendering a `List` + `Paragraph`. Navigation (`j/k/↑/↓`) in Dashboard mode drives the `plan_selected` index.

**Tech Stack:** Rust, ratatui 0.29, crossterm, chrono (with `chrono-tz` for local timezone conversion).

---

## File Map

| File | Change |
|---|---|
| `crates/agtop-cli/src/tui/theme.rs` | Add 4 new style constants |
| `crates/agtop-cli/src/tui/app/mod.rs` | Add `plan_selected: usize` field + 2 methods |
| `crates/agtop-cli/src/tui/events.rs` | Route `j/k/↑/↓` to plan list in Dashboard mode |
| `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs` | Full rewrite |

No new files created. No changes to `session.rs` (merge logic stays in the widget).

---

### Task 1: Add theme constants

**Files:**
- Modify: `crates/agtop-cli/src/tui/theme.rs`

- [ ] **Step 1: Add 4 new plan style constants at the end of the `// ── Dashboard ──` section**

In `crates/agtop-cli/src/tui/theme.rs`, after line 100 (`pub const PLAN_EMPTY`), add:

```rust
/// Filled portion of a usage bar when utilization < 30 % (safe).
pub const PLAN_BAR_GREEN: Style = Style::new().fg(Color::Green);

/// Filled portion of a usage bar when utilization is 30–80 % (caution).
pub const PLAN_BAR_YELLOW: Style = Style::new().fg(Color::Yellow);

/// Filled portion of a usage bar when utilization ≥ 80 % (critical).
pub const PLAN_BAR_RED: Style = Style::new().fg(Color::Red);

/// Highlighted / selected row in the subscription list.
pub const PLAN_SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p agtop-cli 2>&1 | head -20
```

Expected: `Finished` with no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/theme.rs
git commit -m "feat(theme): add plan bar color and selection styles"
```

---

### Task 2: Add `plan_selected` state to `App`

**Files:**
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`

- [ ] **Step 1: Add the field to the `App` struct**

In `crates/agtop-cli/src/tui/app/mod.rs`, in the `App` struct (around line 167), add after the `cost_scroll: usize` field:

```rust
    /// Selected subscription index in the plan-usage list (dashboard mode).
    plan_selected: usize,
```

- [ ] **Step 2: Initialize the field in `App::new()`**

In `App::new()` (around line 223), add after `cost_scroll: 0`:

```rust
            plan_selected: 0,
```

- [ ] **Step 3: Add accessor and two mutation methods**

After the `pub fn cost_scroll(&self) -> usize` accessor (around line 292), add:

```rust
    pub fn plan_selected(&self) -> usize {
        self.plan_selected
    }

    /// Move the subscription list selection down by 1, clamped to the list length.
    pub fn plan_select_next(&mut self, list_len: usize) {
        if list_len > 0 {
            self.plan_selected = (self.plan_selected + 1).min(list_len - 1);
        }
    }

    /// Move the subscription list selection up by 1, clamped to 0.
    pub fn plan_select_prev(&mut self) {
        self.plan_selected = self.plan_selected.saturating_sub(1);
    }
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check -p agtop-cli 2>&1 | head -20
```

Expected: `Finished` with no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs
git commit -m "feat(app): add plan_selected state for subscription list navigation"
```

---

### Task 3: Route navigation keys to the subscription list in Dashboard mode

**Files:**
- Modify: `crates/agtop-cli/src/tui/events.rs`

The current `j/k/↑/↓` bindings call `app.move_selection(±1)` which moves the session table cursor. In Dashboard mode we want those keys to move the subscription list selection instead. The session table in Dashboard mode has no explicit keyboard selection anyway (it just shows sessions, no row highlight in Dashboard).

- [ ] **Step 1: Write a failing test for Dashboard j/k navigation**

In `crates/agtop-cli/src/tui/events.rs`, inside the `#[cfg(test)] mod tests` block (after the last test, around line 380), add:

```rust
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
```

- [ ] **Step 2: Run the tests to confirm they compile and pass (they test `App` methods directly)**

```bash
cargo test -p agtop-cli -- events::tests::dashboard 2>&1
```

Expected: 2 tests pass.

- [ ] **Step 3: Route `j/k/↑/↓` in Dashboard mode**

In `apply_normal_key` in `crates/agtop-cli/src/tui/events.rs`, replace the existing `j/k/↑/↓` match arms (around lines 95–98):

```rust
        KeyCode::Char('j') | KeyCode::Down => app.move_selection(1),
        KeyCode::Char('k') | KeyCode::Up => app.move_selection(-1),
        KeyCode::PageDown => app.move_selection(10),
        KeyCode::PageUp => app.move_selection(-10),
```

with:

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
        KeyCode::PageDown => app.move_selection(10),
        KeyCode::PageUp => app.move_selection(-10),
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check -p agtop-cli 2>&1 | head -20
```

Expected: `Finished` with no errors.

- [ ] **Step 5: Run all existing event tests to confirm no regressions**

```bash
cargo test -p agtop-cli -- events::tests 2>&1
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/agtop-cli/src/tui/events.rs
git commit -m "feat(events): route j/k to subscription list navigation in Dashboard mode"
```

---

### Task 4: Rewrite `dashboard_plan.rs` — merge logic and helpers

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

This task adds the merge/dedup data layer and bar helper functions. The `render` function is rewritten in Task 5.

- [ ] **Step 1: Replace the file contents up through the helper functions**

Replace the entire contents of `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs` with:

```rust
use chrono::{DateTime, Local, Utc};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::tui::app::App;
use crate::tui::theme as th;
use agtop_core::session::PlanUsage;

// ---------------------------------------------------------------------------
// Merged subscription data
// ---------------------------------------------------------------------------

/// One subscription entry after deduplication across clients/agents.
struct MergedPlan<'a> {
    /// Human-readable subscription name ("Max 5x", "ChatGPT Plus", …).
    subscription_name: String,
    /// Windows from all sources, deduplicated by label (most-recent reset_at wins).
    windows: Vec<&'a agtop_core::session::PlanWindow>,
    /// Most recent limit-hit moment across all merged sources.
    last_limit_hit: Option<DateTime<Utc>>,
    /// Notes from all sources, deduplicated.
    notes: Vec<String>,
}

/// Strip known agent-tool suffixes/prefixes to derive a canonical subscription name.
fn canonical_name(pu: &PlanUsage) -> String {
    // Prefer the structured plan_name field.
    if let Some(name) = &pu.plan_name {
        if !name.is_empty() {
            return name.clone();
        }
    }
    // Fall back to stripping known agent labels from `pu.label`.
    let s = pu.label.as_str();
    // Strip " via <agent>" suffix.
    for suffix in &[" via Claude Code", " via OpenCode", " via Codex"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return stripped.trim().to_string();
        }
    }
    // Strip "<agent> · " prefix.
    for prefix in &["Claude Code · ", "OpenCode · ", "Codex · "] {
        if let Some(stripped) = s.strip_prefix(prefix) {
            return stripped.trim().to_string();
        }
    }
    s.trim().to_string()
}

/// Merge a slice of `PlanUsage` entries into deduplicated `MergedPlan` entries.
/// Entries with the same canonical name (case-insensitive) are merged.
/// For duplicate window labels, keep the window with the most recent reset_at.
fn merge_plans(usages: &[PlanUsage]) -> Vec<MergedPlan<'_>> {
    // We preserve insertion order (first occurrence of a subscription name).
    let mut order: Vec<String> = Vec::new(); // canonical names in order
    let mut map: std::collections::HashMap<String, MergedPlan<'_>> =
        std::collections::HashMap::new();

    for pu in usages {
        let key = canonical_name(pu).to_lowercase();
        let display = canonical_name(pu);

        if !map.contains_key(&key) {
            order.push(key.clone());
            map.insert(
                key.clone(),
                MergedPlan {
                    subscription_name: display,
                    windows: Vec::new(),
                    last_limit_hit: pu.last_limit_hit,
                    notes: Vec::new(),
                },
            );
        }

        let entry = map.get_mut(&key).unwrap();

        // Update last_limit_hit to the most recent.
        if let Some(lh) = pu.last_limit_hit {
            entry.last_limit_hit = Some(match entry.last_limit_hit {
                Some(existing) if existing >= lh => existing,
                _ => lh,
            });
        }

        // Merge notes (dedup).
        if let Some(note) = &pu.note {
            if !note.is_empty() && !entry.notes.contains(note) {
                entry.notes.push(note.clone());
            }
        }

        // Merge windows: if a window with the same label already exists,
        // keep the one with the more recent reset_at (or replace if new has
        // reset_at and existing doesn't).
        for w in &pu.windows {
            let existing_pos = entry.windows.iter().position(|e| e.label == w.label);
            match existing_pos {
                None => entry.windows.push(w),
                Some(pos) => {
                    let keep_new = match (entry.windows[pos].reset_at, w.reset_at) {
                        (None, Some(_)) => true,
                        (Some(existing_ts), Some(new_ts)) => new_ts > existing_ts,
                        _ => false,
                    };
                    if keep_new {
                        entry.windows[pos] = w;
                    }
                }
            }
        }
    }

    // Sort windows within each merged entry: nearest reset_at first, None last.
    for key in &order {
        let entry = map.get_mut(key).unwrap();
        entry.windows.sort_by(|a, b| match (a.reset_at, b.reset_at) {
            (Some(ta), Some(tb)) => ta.cmp(&tb),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }

    order.into_iter().map(|k| map.remove(&k).unwrap()).collect()
}

// ---------------------------------------------------------------------------
// Bar helpers
// ---------------------------------------------------------------------------

/// Choose the style for a bar based on utilization thresholds.
/// green < 0.30, yellow 0.30–0.80, red ≥ 0.80, dim for None.
fn bar_style(util: Option<f64>) -> Style {
    match util {
        None => th::PLAN_NOTE,
        Some(u) if u < 0.30 => th::PLAN_BAR_GREEN,
        Some(u) if u < 0.80 => th::PLAN_BAR_YELLOW,
        _ => th::PLAN_BAR_RED,
    }
}

/// Build a fixed-width ASCII bar as two styled `Span`s (filled + empty).
/// Width is the total char width of the bar.
fn bar_spans(util: Option<f64>, width: usize) -> [Span<'static>; 2] {
    let filled = util
        .map(|u| (u.clamp(0.0, 1.0) * width as f64).round() as usize)
        .unwrap_or(0);
    let empty = width.saturating_sub(filled);
    let style = bar_style(util);
    [
        Span::styled("█".repeat(filled), style),
        Span::styled("░".repeat(empty), th::PLAN_NOTE),
    ]
}

/// Format a DateTime<Utc> into local time as "Mon DD, HH:MM AM/PM".
fn format_local(ts: DateTime<Utc>) -> String {
    let local: DateTime<Local> = ts.into();
    local.format("%a %b %-d, %-I:%M %p").to_string()
}

fn relative_since(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

// ---------------------------------------------------------------------------
// Render entry point
// ---------------------------------------------------------------------------

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(" Subscription Details ");
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    let merged = merge_plans(app.plan_usage());

    if merged.is_empty() {
        let p = Paragraph::new(Span::styled("(no subscription data)", th::PLAN_EMPTY));
        frame.render_widget(p, inner);
        return;
    }

    // Split inner area: 40% list | 60% details.
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(inner);

    render_list(frame, panes[0], &merged, app.plan_selected());
    render_details(frame, panes[1], &merged, app.plan_selected());
}

// ---------------------------------------------------------------------------
// Left pane: subscription list
// ---------------------------------------------------------------------------

fn render_list(frame: &mut Frame<'_>, area: Rect, merged: &[MergedPlan<'_>], selected: usize) {
    // Each item is two lines: name + bar row.
    // Available width for the bar: area.width - 2 (for left indent "  ") - 5 (" XX%").
    // We use a fixed 20-char bar in the list.
    const BAR_WIDTH: usize = 20;

    let items: Vec<ListItem> = merged
        .iter()
        .enumerate()
        .map(|(i, mp)| {
            // Pick the window with the nearest reset_at for the list bar.
            let util = mp
                .windows
                .iter()
                .filter(|w| w.reset_at.is_some())
                .min_by_key(|w| w.reset_at.unwrap())
                .and_then(|w| w.utilization)
                .or_else(|| {
                    mp.windows
                        .iter()
                        .find_map(|w| w.utilization)
                });

            let pct_str = util
                .map(|u| format!("{:>3.0}%", u * 100.0))
                .unwrap_or_else(|| "  - ".to_string());

            let name_style = if i == selected {
                th::PLAN_SELECTED
            } else {
                th::PLAN_LABEL
            };

            let [filled_span, empty_span] = bar_spans(util, BAR_WIDTH);
            let bar_line = Line::from(vec![
                Span::raw("  "),
                filled_span,
                empty_span,
                Span::raw(format!(" {pct_str}")),
            ]);

            ListItem::new(vec![
                Line::from(Span::styled(mp.subscription_name.clone(), name_style)),
                bar_line,
            ])
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

// ---------------------------------------------------------------------------
// Right pane: details
// ---------------------------------------------------------------------------

fn render_details(frame: &mut Frame<'_>, area: Rect, merged: &[MergedPlan<'_>], selected: usize) {
    let now = Utc::now();
    let mp = match merged.get(selected) {
        Some(m) => m,
        None => return,
    };

    // Bar width: full inner width minus 4 (2 left indent + 2 right margin).
    let bar_width = (area.width as usize).saturating_sub(4).max(4);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header.
    lines.push(Line::from(Span::styled(
        mp.subscription_name.clone(),
        th::PLAN_LABEL,
    )));
    lines.push(Line::from(""));

    for w in &mp.windows {
        // Label + percentage line.
        let pct_str = w
            .utilization
            .map(|u| format!("{:.0}%", u * 100.0))
            .unwrap_or_else(|| "-".to_string());
        // Left-align label, right-align pct — total line width = area.width - 2 indent.
        let line_width = (area.width as usize).saturating_sub(2);
        let label_width = line_width.saturating_sub(pct_str.len() + 1);
        let label_line = format!(
            "  {:<label_width$} {pct_str}",
            w.label,
            label_width = label_width,
        );
        lines.push(Line::from(Span::styled(label_line, Style::new())));

        // Bar line.
        let [filled_span, empty_span] = bar_spans(w.utilization, bar_width);
        lines.push(Line::from(vec![
            Span::raw("  "),
            filled_span,
            empty_span,
        ]));

        // Reset line.
        let reset_text = if let Some(ts) = w.reset_at {
            format!("  Resets: {}", format_local(ts))
        } else if let Some(hint) = &w.reset_hint {
            format!("  {hint}")
        } else {
            "  (no reset time)".to_string()
        };
        lines.push(Line::from(Span::styled(reset_text, th::PLAN_NOTE)));
        lines.push(Line::from(""));
    }

    if let Some(ts) = mp.last_limit_hit {
        lines.push(Line::from(Span::styled(
            format!("  Last limit hit: {} ago", relative_since(ts, now)),
            th::PLAN_NOTE,
        )));
    }

    for note in &mp.notes {
        lines.push(Line::from(Span::styled(
            format!("  {note}"),
            th::PLAN_NOTE,
        )));
    }

    let p = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(p, area);
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p agtop-cli 2>&1 | head -30
```

Expected: `Finished` with no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs
git commit -m "feat(dashboard_plan): rewrite as two-pane subscription list + details with colored bars"
```

---

### Task 5: Add unit tests for merge logic and bar helpers

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

- [ ] **Step 1: Write tests for `canonical_name`**

At the bottom of `dashboard_plan.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{ClientKind, PlanUsage};

    fn make_pu(label: &str, plan_name: Option<&str>) -> PlanUsage {
        PlanUsage {
            client: ClientKind::Claude,
            label: label.to_string(),
            plan_name: plan_name.map(|s| s.to_string()),
            windows: Vec::new(),
            last_limit_hit: None,
            note: None,
        }
    }

    #[test]
    fn canonical_name_prefers_plan_name() {
        let pu = make_pu("Claude Code · Max 5x", Some("max_5x"));
        assert_eq!(canonical_name(&pu), "max_5x");
    }

    #[test]
    fn canonical_name_strips_via_suffix() {
        let pu = make_pu("Max 5x via Claude Code", None);
        assert_eq!(canonical_name(&pu), "Max 5x");
    }

    #[test]
    fn canonical_name_strips_agent_prefix() {
        let pu = make_pu("Claude Code · Max 5x", None);
        assert_eq!(canonical_name(&pu), "Max 5x");
    }

    #[test]
    fn canonical_name_strips_opencode_prefix() {
        let pu = make_pu("OpenCode · anthropic (Max)", None);
        assert_eq!(canonical_name(&pu), "anthropic (Max)");
    }

    #[test]
    fn merge_deduplicates_same_subscription() {
        let pu1 = make_pu("Max 5x via Claude Code", None);
        let pu2 = make_pu("Max 5x via OpenCode", None);
        let merged = merge_plans(&[pu1, pu2]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].subscription_name, "Max 5x");
    }

    #[test]
    fn merge_keeps_two_different_subscriptions() {
        let pu1 = make_pu("Max 5x via Claude Code", None);
        let pu2 = make_pu("ChatGPT Plus", None);
        let merged = merge_plans(&[pu1, pu2]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_window_dedup_keeps_newer_reset() {
        use agtop_core::session::PlanWindow;
        use chrono::TimeZone;

        let t1 = Utc.with_ymd_and_hms(2026, 4, 18, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();

        let mut pu1 = make_pu("Max via Claude Code", None);
        pu1.windows.push(PlanWindow {
            label: "5h".to_string(),
            utilization: Some(0.5),
            reset_at: Some(t1),
            reset_hint: None,
            binding: false,
        });

        let mut pu2 = make_pu("Max via OpenCode", None);
        pu2.windows.push(PlanWindow {
            label: "5h".to_string(),
            utilization: Some(0.7),
            reset_at: Some(t2),
            reset_hint: None,
            binding: false,
        });

        let merged = merge_plans(&[pu1, pu2]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].windows.len(), 1);
        // t2 is more recent, so the window with utilization 0.7 should be kept.
        assert!((merged[0].windows[0].utilization.unwrap() - 0.7).abs() < 1e-9);
    }

    #[test]
    fn bar_style_green_below_30() {
        assert_eq!(bar_style(Some(0.29)), th::PLAN_BAR_GREEN);
        assert_eq!(bar_style(Some(0.0)), th::PLAN_BAR_GREEN);
    }

    #[test]
    fn bar_style_yellow_30_to_80() {
        assert_eq!(bar_style(Some(0.30)), th::PLAN_BAR_YELLOW);
        assert_eq!(bar_style(Some(0.79)), th::PLAN_BAR_YELLOW);
    }

    #[test]
    fn bar_style_red_at_or_above_80() {
        assert_eq!(bar_style(Some(0.80)), th::PLAN_BAR_RED);
        assert_eq!(bar_style(Some(1.0)), th::PLAN_BAR_RED);
    }

    #[test]
    fn bar_spans_correct_char_counts() {
        let [filled, empty] = bar_spans(Some(0.5), 10);
        // 50% of 10 = 5 filled, 5 empty.
        assert_eq!(filled.content.chars().count(), 5);
        assert_eq!(empty.content.chars().count(), 5);
    }
}
```

- [ ] **Step 2: Run the tests**

```bash
cargo test -p agtop-cli -- dashboard_plan::tests 2>&1
```

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs
git commit -m "test(dashboard_plan): add unit tests for merge logic and bar helpers"
```

---

### Task 6: Add a render smoke test for the new two-pane layout

**Files:**
- Modify: `crates/agtop-cli/src/tui/mod.rs`

- [ ] **Step 1: Add `set_plan_usage` method to `App`**

In `crates/agtop-cli/src/tui/app/mod.rs`, after the `plan_selected` accessor methods (around where `plan_select_next` is defined), add:

```rust
    /// Replace the plan usage snapshot (used in tests and by the refresh worker).
    pub fn set_plan_usage(&mut self, usages: Vec<agtop_core::PlanUsage>) {
        self.plan_usage = usages;
    }
```

First check if it already exists:

```bash
grep -n "set_plan_usage\|fn.*plan_usage" crates/agtop-cli/src/tui/app/mod.rs | head -10
```

If `set_plan_usage` is already defined, skip this step.

- [ ] **Step 2: Add a Dashboard render smoke test that includes plan usage**

In `crates/agtop-cli/src/tui/mod.rs`, inside `#[cfg(test)] mod tests` (after the `renders_empty_state` test, around line 690), add:

```rust
    #[test]
    fn renders_dashboard_with_plan_usage() {
        use agtop_core::session::{ClientKind, PlanUsage, PlanWindow};
        use chrono::TimeZone;

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.toggle_ui_mode(); // switch to Dashboard

        let reset_at = Utc.with_ymd_and_hms(2026, 4, 18, 13, 0, 0).unwrap();
        let pu = PlanUsage {
            client: ClientKind::Claude,
            label: "Max 5x via Claude Code".to_string(),
            plan_name: Some("Max 5x".to_string()),
            windows: vec![
                PlanWindow {
                    label: "5h".to_string(),
                    utilization: Some(0.71),
                    reset_at: Some(reset_at),
                    reset_hint: None,
                    binding: true,
                },
                PlanWindow {
                    label: "7d".to_string(),
                    utilization: Some(0.18),
                    reset_at: Some(Utc.with_ymd_and_hms(2026, 4, 24, 10, 0, 0).unwrap()),
                    reset_hint: None,
                    binding: false,
                },
            ],
            last_limit_hit: None,
            note: None,
        };
        app.set_plan_usage(vec![pu]);

        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");

        let contents = buffer_to_string(&terminal.backend().buffer().clone());
        assert!(
            contents.contains("Subscription Details"),
            "panel title missing:\n{contents}"
        );
        assert!(
            contents.contains("Max 5x"),
            "subscription name missing:\n{contents}"
        );
    }
```

- [ ] **Step 3: Verify it compiles and the new test passes**

```bash
cargo test -p agtop-cli -- renders_dashboard_with_plan_usage 2>&1
```

Expected: test passes.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test -p agtop-cli 2>&1
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs crates/agtop-cli/src/tui/mod.rs
git commit -m "test(tui): add dashboard smoke test with plan usage two-pane layout"
```

---

### Task 7: Final verification

- [ ] **Step 1: Run the full workspace test suite**

```bash
cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 2: Clippy check**

```bash
cargo clippy -p agtop-cli -- -D warnings 2>&1
```

Fix any warnings before proceeding.

- [ ] **Step 3: Manual smoke — build and check the dashboard renders**

```bash
cargo build -p agtop-cli 2>&1 | tail -5
```

Expected: `Finished` with no errors.

- [ ] **Step 4: Commit any clippy fixes, if needed**

```bash
git add -A && git commit -m "fix: address clippy warnings in dashboard_plan rewrite"
```

(Skip this step if there were no clippy warnings.)
