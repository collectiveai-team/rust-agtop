# Quota TUI — Design

**Date:** 2026-04-22
**Status:** Approved
**Owner:** jedzill4
**Context:** Follow-up to `2026-04-21-quota-provider-fetchers-design.md`. That spec
delivers `agtop-core::quota` with a CLI subcommand. This spec wires the quota data
into the TUI: replacing the existing local-estimate Subscription Details pane in
Dashboard mode, and adding a compact Quota tab to Classic mode's bottom panel.

---

## Summary

The existing **Subscription Details** pane in Dashboard mode is driven by
locally-estimated `PlanUsage` data derived from session analysis. It is replaced
entirely by a new pane driven by `ProviderResult` data from the quota subsystem.

A new **Quota tab** is added to Classic mode's bottom panel (alongside Info, Cost,
Config) showing a compact per-provider/per-window summary.

Quota fetches are **on-demand**: the first fetch fires when the user navigates to
the quota pane. While the pane is active, auto-refresh runs every 60 s. When the
user leaves, the refresh loop pauses.

The implementation extends the existing refresh worker (Option A) — same tokio
runtime, new inner loop, new `watch` channel for pane focus commands.

---

## Architecture

### Crate responsibilities

| Crate | Responsibility |
|-------|---------------|
| `agtop-core` | `quota::fetch_all`, `ProviderResult`, `ProviderSlot` logic — already specified in the fetchers spec |
| `agtop-cli` | `ProviderSlot` bookkeeping, refresh worker extension, `App` state, all rendering |

`agtop-core` remains stateless. `agtop-cli` owns slot bookkeeping and the refresh
loop, exactly as described in the fetchers spec's "Consumer integration" section.

### New / changed files

| File | Change |
|------|--------|
| `crates/agtop-cli/src/app/mod.rs` | Add `ProviderSlot`, `QuotaState`, `quota_slots`, `selected_provider`, `apply_quota_results`, `set_quota_error`, `set_quota_loading`; add `Tab::Quota` |
| `crates/agtop-cli/src/tui/refresh.rs` | Add `QuotaCmd`, extend `RefreshMsg`, add quota inner loop |
| `crates/agtop-cli/src/tui/mod.rs` | Wire `QuotaCmd` on tab/mode switches; handle new `RefreshMsg` variants; register `Tab::Quota` in bottom panel dispatch |
| `crates/agtop-cli/src/tui/widgets/quota_tab.rs` | **New** — Classic mode compact quota tab |
| `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs` | **Rewrite** — Dashboard quota pane (replaces plan-usage rendering) |

---

## Data model

### `ProviderSlot` (in `app/mod.rs`)

```rust
pub struct ProviderSlot {
    pub last_good: Option<ProviderResult>,  // most recent ok=true result
    pub current:   ProviderResult,           // latest attempt (ok or err)
}
```

Render policy (from fetchers spec):

| last_good | current.ok | Render |
|-----------|------------|--------|
| None      | true       | Normal |
| None      | false      | Error row, no gauges |
| Some      | true       | Normal |
| Some      | false      | Stale gauges + inline warning |

### `QuotaState` (in `app/mod.rs`)

```rust
pub enum QuotaState {
    Idle,           // pane never opened
    Loading,        // first fetch in-flight, no results yet
    Ready,          // at least one fetch completed (slots populated)
    Error(String),  // fetch_all returned error before any result arrived
}
```

### `App` additions

```rust
pub quota_slots:       Vec<ProviderSlot>,
pub quota_state:       QuotaState,          // default: Idle
pub selected_provider: usize,               // list selection in Dashboard pane
pub model_scroll:      usize,               // scroll offset for Google per-model list; reset to 0 on provider change
pub card_scroll:       usize,               // horizontal scroll offset for Classic Quota tab card row
```

### `App` methods

```rust
/// Upsert results into quota_slots; update QuotaState → Ready.
pub fn apply_quota_results(&mut self, results: Vec<ProviderResult>);

/// Called on QuotaError before any Ready state.
pub fn set_quota_error(&mut self, message: String);

/// Called when first fetch fires; transitions Idle → Loading.
pub fn set_quota_loading(&mut self);
```

`apply_quota_results` upserts by `provider_id`: sets `current`; if `current.ok`,
also sets `last_good`. Always transitions `QuotaState` to `Ready`.

---

## Refresh worker extension (`refresh.rs`)

### New channel

```rust
// Added to RefreshHandle:
pub quota_trigger_tx: watch::Sender<QuotaCmd>,

pub enum QuotaCmd {
    Start,  // user entered quota pane
    Stop,   // user left quota pane
}
```

`RefreshHandle::drop` sends `Stop` before setting the shutdown flag, so the quota
loop exits cleanly.

### `RefreshMsg` additions

```rust
pub enum RefreshMsg {
    Snapshot { generation: u64, analyses: Vec<SessionAnalysis>, plan_usage: Vec<PlanUsage> },
    Error    { generation: u64, message: String },
    // new:
    QuotaSnapshot { generation: u64, results: Vec<ProviderResult> },
    QuotaError    { generation: u64, message: String },
}
```

`RefreshMsg::Snapshot` retains its `plan_usage` field — the session refresh loop
still produces `PlanUsage` for any consumers that may exist. The Dashboard quota
pane no longer reads `app.plan_usage()`; it reads `app.quota_slots` instead.
The existing `plan_usage` field in `App` and the `set_snapshot` path are
unchanged.

### Quota inner loop (pseudocode)

```
loop:
    wait for quota_trigger_rx.changed() with value Start
    app.set_quota_loading()  ← sent via existing msg_tx channel

    loop:
        results = spawn_blocking(|| fetch_all(auth, http))
        publish QuotaSnapshot or QuotaError on msg_tx

        select!:
            _ = sleep(refresh_interval_secs) => continue inner loop
            _ = quota_trigger_rx.changed()   =>
                if Stop: break inner loop
                else (Start again): immediate re-fetch, then continue
            _ = manual_rx.changed()          => immediate re-fetch
            _ = shutdown                     => return
```

`fetch_all` uses `UreqClient` constructed from `QuotaConfig` (already wired in
Phase 6's public API). `QuotaConfig` is read once at worker spawn and is immutable
for the session.

### `event_loop` handling

```rust
QuotaSnapshot { results, .. } => app.apply_quota_results(results),
QuotaError    { message, .. } => app.set_quota_error(message),
```

---

## Pane focus wiring (`tui/mod.rs`)

### Entering/leaving quota pane

| Event | Action |
|-------|--------|
| Classic: switch to `Tab::Quota` | `handle.quota_trigger_tx.send(QuotaCmd::Start)` |
| Classic: switch away from `Tab::Quota` | `handle.quota_trigger_tx.send(QuotaCmd::Stop)` |
| Dashboard: enter Dashboard mode (`d` key) | `handle.quota_trigger_tx.send(QuotaCmd::Start)` |
| Dashboard: leave Dashboard mode | `handle.quota_trigger_tx.send(QuotaCmd::Stop)` |
| Any: `r` key while quota pane active | existing `manual_tx.send(...)` |
| TUI quit | `RefreshHandle::drop` sends `Stop` then sets shutdown |

"Quota pane active" = `app.ui_mode == Dashboard` OR `app.tab == Tab::Quota`.

Note: the quota pane occupies a fixed position in Dashboard mode and is always
visible when Dashboard is active. There is no sub-focus concept within Dashboard
— entering Dashboard mode is equivalent to entering the quota pane.

### Footer hint

When quota pane is active, `[r] refresh` appears in the footer alongside existing
hints. No new global key bindings.

---

## Classic mode — Quota tab (`widgets/quota_tab.rs`)

### Tab enum

```rust
// app/mod.rs
pub enum Tab {
    Info,
    Cost,
    Config,
    Quota,   // new
}
```

Cycles with existing Tab / Shift-Tab bindings.

### Layout

A **single wide panel** with one card per configured provider, laid out
**horizontally** (side-by-side, not a vertical list). Each card shows the
provider's **preferred window** as a short view (name + bar + %). Only one
window per provider — full details are Dashboard-only.

Content within each card is **horizontally centered** in its allocated width
(both the name line and the value line).

```
┌─ Quota ───────────────────────────────────────────────────────────────────┐
│    Claude          z.ai †         Copilot         Codex ○       Google ✗ ›│
│  5h 72% ■■■■       5h 88% ■■■■■    premium ∞       loading…        401    │
└───────────────────────────────────────────────────────────────────────────┘
```

Each card occupies two lines:
- Line 1: provider name + status glyph (if non-ok) — centered
- Line 2: preferred-window label + % + bar (or `value_label` when `used_percent` is None) — centered

### Preferred window table

Per-provider preferred-window label, used for the short view:

| Provider | Preferred label | Fallback if absent |
|----------|----------------|---------------------|
| Claude | `5h` | first window in `IndexMap` |
| Codex | `5h` | `weekly` → first window |
| Copilot | `premium` | first window |
| CopilotAddon | `premium` | first window |
| z.ai | `5h` | `monthly` → first window |
| Google | first model's `5h` or `daily` | first model's first window |

If the preferred label is not present in the slot, fall back as listed.
If no windows at all, show `—` in the short view.

### Card dimensions

- Fixed card slot width: **20 columns** (including 2-column gutter between cards).
- Content width: 18 columns — content is centered within these 18 columns.
- Bar width: **6 cells** fixed when shown (each cell = 1 `■` or 1 space).
- Provider name line truncated with `…` if it exceeds 18 columns.
- Card count computed from available area width: `cards_visible = area.width / 20`.

### Horizontal scrolling

When `quota_slots.len() > cards_visible`:
- `card_scroll: usize` (new field in `App`) tracks the leftmost visible card index.
- `←` / `→` arrow keys (when Classic + Quota tab active) scroll the row by 1.
- Indicators: `‹` in the top-right of the panel when `card_scroll > 0`; `›` when
  `card_scroll + cards_visible < quota_slots.len()`.
- Scroll clamps to `[0, quota_slots.len().saturating_sub(cards_visible)]`.

### Rendering rules

**State gates:**
- `QuotaState::Idle` → centered line: `"Press r to load quota data"`
- `QuotaState::Loading` → centered line: `"Fetching quota data…"`
- `QuotaState::Error(msg)` → centered line: `"Error: {msg}"`
- `QuotaState::Ready` → render card row

**Per-card rendering** (all lines centered within the 18-col content area):
- Name line: `{provider_name}{glyph}` where glyph is one of:
  - (nothing) — ok
  - ` †` — stale (`current.ok=false && last_good=Some`), entire card rendered dim
  - ` ✗` — error (`current.ok=false && last_good=None`), bar replaced by short error token (`401`, `net`, `parse`, etc.)
  - ` ○` — loading (slot not yet populated), bar replaced by `loading…`
- Value line:
  - `used_percent = Some(p)`: `{label} {p}% {bar}` (bar 6 cells of `■`/space)
  - `used_percent = None, value_label = Some(s)`: `{label} {s}` truncated to fit
  - both None: `{label} —`

### Bar characters and colors

- Filled cell: `■` (U+25A0 BLACK SQUARE)
- Empty cell: ` ` (space) rendered with default/white foreground to leave visual
  whitespace — no character drawn, the column is blank
- Fill count: `round((p / 100.0) * bar_width)`, clamped to `[0, bar_width]`
- Bar color (applied to the `■` characters only):
  - `< 75%` → green (`theme::OK`)
  - `75–90%` → yellow (`theme::WARN`)
  - `> 90%` → red (`theme::CRIT`)
- Percentage text color: same threshold coloring as the bar.
- Stale cards: entire card rendered in a dim variant (dim green/yellow/red)
  regardless of threshold.
- The existing `theme.rs` `OK` / `WARN` / `CRIT` constants are used; define them
  if not already present as green/yellow/red.

**`NotConfigured` providers**: omitted entirely (no card).

---

## Dashboard mode — Quota pane (`widgets/dashboard_plan.rs`)

Replaces all existing content. Same file, same `pub fn render(frame, area, app)`
signature. Existing `MergedPlan`, `merge_plans`, and all `PlanUsage`-based logic
are removed.

### Layout

40% left / 60% right split (identical to existing plan pane split).

**State gate:** `QuotaState::Idle` or `Loading` renders a centered message across
the full area instead of the split.

### Left panel — provider list (compact short view)

One line per provider. Each line shows: status glyph + provider name + the
provider's **preferred window** (same table as Classic tab) with bar + %.
The bar uses `■` cells colored by threshold, same as the Classic tab cards.

```
┌─ Quota ────────────────────────────────┐
│ ● Claude    5h       72%  ■■■■■■■      │
│ ● Copilot   premium       Unlimited    │
│ ▲ z.ai †    5h       88%  ■■■■■■■■■    │
│ ✗ Google    — 401                      │
│ ○ Codex     — loading…                 │
└────────────────────────────────────────┘
```

**Glyphs:**
| Glyph | Meaning |
|-------|---------|
| `●` | Ready, ok |
| `▲` | Stale (last_good exists, current failed) |
| `✗` | Error, no last_good |
| `○` | Loading / not yet fetched |

**Line content** (preferred-window short view):
- Ok: `{glyph} {provider}  {label} {p}%  {bar}` (bar 10 cells of `■`/space)
- Ok, unlimited: `{glyph} {provider}  {label}  Unlimited` (no bar)
- Ok, value_label without %: `{glyph} {provider}  {label}  {value_label}`
- Stale: same as ok, dim colors, ` †` after provider name
- Error: `{glyph} {provider}  — {short_error_token}` (e.g. `— 401`)
- Loading: `{glyph} {provider}  — loading…`

Bar colors (green/yellow/red) and stale dimming rules: identical to the
Classic tab cards (see "Bar characters and colors" above).

Preferred window table: same as the Classic tab (see above).

Selected provider highlighted with existing theme selection style. `j`/`k` and
arrow keys scroll the list. Selection persists across refreshes; when a
provider disappears from the slot list (e.g. deconfigured), selection clamps to
`quota_slots.len().saturating_sub(1)`.

### Right panel — detail view

```
┌─ Claude · Pro · user@example.com ───────────────────────────┐
│ ! Stale — data from 14:23 · last error: Transport           │
│                                                              │
│ 5h        ■■■■■■■        72%   resets in 2h 14m            │
│ 7d        ■■■■           45%   resets in 3d 12h            │
│ 7d-sonnet ■              12%   resets in 3d 12h            │
│                                                              │
│ Overage   disabled · limit $0.00                            │
│                                                              │
│                                  fetched at 14:23:05        │
└──────────────────────────────────────────────────────────────┘
```

Right-panel bar width: 10 cells. Same `■`/space characters and same
green/yellow/red coloring rules as the Classic tab.

**Block title:** `provider_name · meta["plan"] · meta["login"]` (omit absent keys).

**Stale warning line:** shown only when `current.ok=false && last_good=Some`.
Single line, dim yellow: `"! Stale — data from HH:MM · last error: <ErrorKind>"`.
`ErrorKind` → human text mapping from fetchers spec (§ QuotaError).

**Error-only state** (`last_good=None && current.ok=false`):
Full-area error text, no gauges:
```
Error: <ErrorKind human text>
<detail (up to 500 bytes, word-wrapped)>
```

**Windows section:**
- Same bar/label rules as Classic tab.
- Absent windows (key not in `IndexMap`) rendered as `——` dim line — shows the
  user the window type exists but wasn't returned by the provider this cycle.
  Only emit absent-window placeholders for the known label set of the selected
  provider (Claude: 5h/7d/7d-sonnet/7d-opus/7d-oauth-apps/7d-cowork/7d-omelette;
  others: only emit what was returned).

**Extras section** (below windows, separated by blank line):
- `OverageBudget`:
  `"Overage   enabled · $X.XX used of $Y.YY (Z%)"` or `"Overage   disabled · limit $Y.YY"`
- `PerToolCounts` (z.ai web-tools):
  Compact two-column table: `tool-name   count / cap   resets in …`
- `KeyValue`: one `key   value` line per entry.
- Absent extras: not shown.

**Google — per-model section:**
Top-level `windows` is empty for Google (per fetchers spec). Instead, detail area
shows a scrollable sub-list of models:
```
gemini/gemini-2.5-pro    daily   ■■■         31%   resets in 18h
antigravity/gemini-pro   5h      ■■■■■■■■■■  98%   resets in 0h 12m
```
`selected_provider` list navigation drives model scroll position via a secondary
`model_scroll: usize` in `App` (reset to 0 when selected provider changes).

**Fetched-at timestamp:** dim, right-aligned in the last line of the detail area.
Source: `current.fetched_at` (epoch ms → local time HH:MM:SS).

---

## Key bindings summary

| Key | Mode | Action |
|-----|------|--------|
| `Tab` / `Shift-Tab` | Classic | Cycle tabs (now includes Quota) |
| `r` | Quota pane active | Manual refresh |
| `j` / `↓` | Dashboard quota pane | Next provider |
| `k` / `↑` | Dashboard quota pane | Previous provider |
| `←` / `→` | Classic Quota tab | Scroll card row when overflowing |
| `d` | Any | Toggle Dashboard/Classic (existing) |

No new global key bindings.

---

## Testing

### Layer 1 — Render snapshot tests (extend `tui/mod.rs` `TestBackend` tests)

**Classic Quota tab (horizontal card row):**
- `QuotaState::Idle` → "Press r" message
- `QuotaState::Loading` → loading message
- Mixed providers (one card each): one with preferred window present, one
  unlimited, one stale (` †`, dim), one error (` ✗`, short token), one loading
  (` ○`, `loading…`)
- Preferred-window fallback: provider whose preferred label is absent — verify
  correct fallback window is shown
- Color threshold boundary: 74%, 75%, 90%, 91%
- Horizontal overflow: more providers than fit — verify `›` indicator and
  `card_scroll` offset shifts visible cards

**Dashboard quota pane:**
- State gates: `Idle`, `Loading`, `Ready`, `Error`
- Left panel short view: mixed ok / stale / error / loading providers; verify
  preferred window is the one shown; verify `†` / `✗` / `○` glyphs
- Right panel detail: selected provider with multiple windows, with extras,
  with absent Claude windows as `——` placeholders
- Google provider: per-model windows in right panel, no top-level windows
- Extras: `OverageBudget` enabled, `OverageBudget` disabled, `PerToolCounts`
- Stale warning line present/absent
- Error-only state (no last_good): full-area error text, no gauges
- Selection change: right panel updates, `model_scroll` resets to 0

All fixtures built inline from `ProviderResult` structs. No HTTP, no tokio.

### Layer 2 — State machine unit tests (`app/mod.rs`)

- `apply_quota_results`: first ok result → `last_good=None, current=ok, state=Ready`
- `apply_quota_results`: second ok result → `last_good=Some, current=ok`
- `apply_quota_results`: err after ok → `last_good=Some(prev), current=err`
- `set_quota_error` before any Ready → `QuotaState::Error`
- `set_quota_loading` → `QuotaState::Loading`
- `selected_provider` clamps to `quota_slots.len().saturating_sub(1)`
- `card_scroll` clamps to `[0, quota_slots.len().saturating_sub(cards_visible)]`
- Preferred-window resolution: given a slot, returns correct window per
  per-provider preferred-label table, with fallback when absent

### Layer 3 — Worker integration tests (extend or add to `refresh.rs` tests)

Using `FakeHttp` from `agtop-core`'s `test-util` feature:
- `QuotaCmd::Start` → `QuotaSnapshot` published on `msg_tx`
- `QuotaCmd::Stop` → no further messages after stop
- `manual_tx` trigger while quota loop active → immediate re-fetch

---

## Non-goals (this spec)

- No per-provider refresh interval configuration (uniform 60 s).
- No quota alerts or threshold notifications.
- No historical quota data or charting.
- No mouse-click provider selection in the Dashboard list (keyboard only).
- No Classic-mode detail view (compact tab only; full details are Dashboard-only).
- No changes to `agtop-core` — all new code is in `agtop-cli`.

---

## Dependencies on prior work

This spec depends on Phase 6 of the quota fetchers plan being complete:
- `agtop_core::quota::{fetch_all, fetch_one, list_providers}` must be public.
- `ProviderResult`, `UsageWindow`, `Usage`, `UsageExtra`, `QuotaError`,
  `ErrorKind`, `ProviderId` must be re-exported from `agtop_core`.
- `QuotaConfig` must be constructible from `agtop-cli`.
- `agtop-core` must be compiled with `features = ["test-util"]` in dev/test
  for `FakeHttp` access.
