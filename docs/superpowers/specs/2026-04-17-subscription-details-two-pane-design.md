# Subscription Details Two-Pane Redesign

**Date:** 2026-04-17
**Status:** Approved

## Overview

Redesign the Subscription Details pane in the agtop TUI dashboard from a single scrolling list of provider cards to a two-pane interactive view: a left subscription list with usage bars and a right details panel showing all usage windows with colored progress bars and reset times.

## Goals

- Show one card per subscription (e.g. "Claude Max", "ChatGPT Plus"), not one per agent/provider combination.
- Show all usage windows (5-Hour, 7-Day, 7-Day Sonnet, Weekly, etc.) in the details pane with colored bars.
- Show next reset datetime (local timezone) per window.
- Use traffic-light coloring for bars: green <30%, yellow 30–80%, red ≥80%.
- Do not expose which CLI tool (Claude Code, OpenCode, Codex) the data came from.

## Non-Goals

- Predicting future usage.
- Showing cost information (that is the Cost Summary pane's job).
- Changing the data collection layer or provider integrations.

## Layout

The existing `dashboard_plan` area is split horizontally **40% left / 60% right** within the `Subscription Details` border block.

```
┌─ Subscription Details ─────────────────────────────────────────────┐
│ ┌── List ───────────────┐ ┌── Details ──────────────────────────┐  │
│ │ > Claude Max          │ │ Claude Max                          │  │
│ │   ■■■■■■■·····  71%   │ │                                     │  │
│ │   ChatGPT Plus        │ │   5-Hour                       71%  │  │
│ │   ■·········  11%     │ │   ████████████████████░░░░░░░░░░░   │  │
│ │   Codex               │ │   Resets: Sat Apr 18, 1:00 AM       │  │
│ │   ──────────   0%     │ │                                     │  │
│ └───────────────────────┘ │   7-Day Limit                  18%  │  │
│                           │   ████░░░░░░░░░░░░░░░░░░░░░░░░░░░   │  │
│                           │   Resets: Fri Apr 24, 10:00 AM      │  │
│                           │                                     │  │
│                           │   7-Day Sonnet Limit             9% │  │
│                           │   ██░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   │  │
│                           │   Resets: Fri Apr 24, 12:00 PM      │  │
│                           │                                     │  │
│                           │   Last limit hit: 18d ago           │  │
│                           └─────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────┘
```

## Data Layer

### Subscription Name Normalization

A `canonical_subscription_name(pu: &PlanUsage) -> String` helper:
1. Use `pu.plan_name` if set.
2. Otherwise, strip known agent suffixes from `pu.label`: ` via Claude Code`, ` via OpenCode`, ` via Codex`; strip ` · ` provider prefixes.
3. Resulting name must not contain agent/tool names.

Examples:
- `"Claude Code · Max 5x"` → `"Max 5x"`
- `"Max 5x via Claude Code"` → `"Max 5x"`
- `"OpenCode · Anthropic (Max)"` → `"Anthropic (Max)"`
- `plan_name: Some("max")` → `"max"` (used as-is, no capitalization applied)

### Merging Multiple Sources

`merged_plan_usages(plan_usages: &[PlanUsage]) -> Vec<MergedPlanUsage>`:
- Group by canonical subscription name (case-insensitive).
- For windows with the same `label` from multiple sources, keep the one with the most recent `reset_at` (most up-to-date data).
- `last_limit_hit` = most recent across all merged entries.
- `notes` = union of all notes, deduplicated.
- Order output: by subscription name alphabetically (or preserve insertion order from `app.plan_usage()`).

### `MergedPlanUsage` Struct

```rust
pub struct MergedPlanUsage {
    pub subscription_name: String,
    pub windows: Vec<PlanWindow>,       // deduplicated, ordered: shortest reset_at first, None last
    pub last_limit_hit: Option<DateTime<Utc>>,
    pub notes: Vec<String>,
}
```

Location: inline in `dashboard_plan.rs` (private) or in `session.rs` (if reused elsewhere). Prefer inline initially.

## UI Components

### Left Pane — Subscription List

- Ratatui `List` widget.
- One `ListItem` per `MergedPlanUsage`.
- Each item is two lines:
  - Line 1: subscription name (highlighted when selected).
  - Line 2: `  bar20 XX%` — 20-char ASCII bar + percentage.
- Bar data source: utilization from the window with the **nearest (smallest) `reset_at`** value (i.e. soonest to reset). If no window has `reset_at`, fall back to the first window with non-None utilization.
- Bar color: green if util <30%, yellow if 30–80%, red if ≥80%.
- Selected item uses `PLAN_SELECTED` style (reversed/highlighted).
- Navigation: `↑`/`↓` arrow keys and `j`/`k` vi-keys cycle through subscriptions. Selection wraps (or clamps — clamping preferred for simplicity).
- Selection state stored as `pub plan_selected: usize` on `App`.

### Right Pane — Details

- Ratatui `Paragraph` widget (with scroll if content exceeds area).
- Header: subscription name, `PLAN_LABEL` style (cyan bold).
- Blank line.
- For each window in order (shortest reset first):
  - Label line: `"  {window.label:<20}{pct:>4}%"` — label left-padded 20 chars, pct right-aligned.
  - Bar line: full-width styled bar using `Span`s (`█` filled, `░` empty), color applied to filled chars.
  - Reset line: `"  Resets: {local_datetime}"` or `"  {reset_hint}"` if no `reset_at`.
  - Blank line separator.
- If `last_limit_hit` is set: `"  Last limit hit: {relative_since}"`.
- Notes (if any): dimmed, one line each.

### Bar Width Calculation

- Left-pane list bar: fixed 20 chars (`bar20()`).
- Right-pane details bar: `area.width - 4` (2 chars left indent + 2 chars right margin), computed at render time.
- Bar chars: `█` (U+2588) for filled, `░` (U+2591) for empty.

### Color Thresholds

| Utilization | Color |
|---|---|
| `< 0.30` | Green |
| `0.30 – 0.80` | Yellow |
| `≥ 0.80` | Red |
| `None` | `DIM` gray |

## Theme Changes (`theme.rs`)

Add:
```rust
pub const PLAN_BAR_GREEN:  Style = Style::new().fg(Color::Green);
pub const PLAN_BAR_YELLOW: Style = Style::new().fg(Color::Yellow);
pub const PLAN_BAR_RED:    Style = Style::new().fg(Color::Red);
pub const PLAN_SELECTED:   Style = Style::new().add_modifier(Modifier::REVERSED);
```

## Files Changed

| File | Change |
|---|---|
| `crates/agtop-cli/src/tui/app/mod.rs` | Add `plan_selected: usize` field; key handlers `↑`/`↓`/`j`/`k` |
| `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs` | Full rewrite: merge logic, two-pane layout, List + Paragraph, colored bars |
| `crates/agtop-cli/src/tui/theme.rs` | Add 4 new style constants |
| `crates/agtop-core/src/session.rs` | Add `MergedPlanUsage` struct (if shared) |

## Key Handling

- Arrow `↑`/`↓` and `j`/`k` route to subscription list navigation when Dashboard tab is active.
- Verify how focus currently works in `tui/mod.rs` event handler to ensure keys don't conflict with existing bindings.
- Selection clamps to `[0, len-1]`; no wrap-around.

## Out of Scope

- Keyboard scrolling of the details pane.
- Mouse support.
- Filtering the subscription list.
