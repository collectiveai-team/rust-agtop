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

Note: the quota pane occupies a fixed position in Dashboard mode and is always
visible when Dashboard is active. There is no sub-focus concept within Dashboard
— entering Dashboard mode is equivalent to entering the quota pane.
| Any: `r` key while quota pane active | existing `manual_tx.send(...)` |
| TUI quit | `RefreshHandle::drop` sends `Stop` then sets shutdown |

"Quota pane active" = `app.ui_mode == Dashboard` OR `app.tab == Tab::Quota`.

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

Single area. Header row + body.

```
┌─ Quota ──────────────────────────────────────────────────────┐
│ Provider          Window    Used    Bar              Resets  │
│ Claude            5h         72%   [███████░░░]      2h 14m │
│                   7d         45%   [████░░░░░░]      3d 12h │
│ Copilot           premium  Unlimited                         │
│ z.ai †            5h         88%   [████████░░]      1h 02m │
│                   monthly    31%   [███░░░░░░░]      18d    │
│ Codex             — loading…                                 │
│ Google            — token expired (HTTP 401)                 │
└──────────────────────────────────────────────────────────────┘
```

### Rendering rules

**State gates:**
- `QuotaState::Idle` → centered line: `"Press r to load quota data"`
- `QuotaState::Loading` → centered line: `"Fetching quota data…"`
- `QuotaState::Error(msg)` → centered line: `"Error: {msg}"`
- `QuotaState::Ready` → render slot table

**Per-provider grouping:**
- Provider name shown only on first window row; blank on subsequent rows.
- Stale provider (`current.ok=false && last_good=Some`): `†` suffix on provider
  name; window bars rendered in dim colors.
- Error provider (`current.ok=false && last_good=None`): single row
  `— <short ErrorKind description>` in dim red. Full detail in `QuotaError.detail`
  is shown on mouse hover (same UiLayout hover mechanism as existing panels).
- In-flight provider (slot not yet in `quota_slots`): single row `— loading…`

**Per-window rows:**
- `used_percent = Some(p)`:
  - Progress bar (10 chars wide, filled/empty blocks)
  - Percentage text
  - Reset countdown (`resets in Xh Ym` or `Xd Yh`) computed from
    `reset_at` and current time
- `used_percent = None`:
  - `value_label` text in place of bar+percentage (e.g. `Unlimited`,
    `$12.34 remaining`)
  - Reset countdown if `reset_at` present
- Windows where both `used_percent` and `value_label` are `None`: row omitted.

**Color thresholds** (use existing theme constants, or define new ones in `theme.rs`):
- `< 75%` → normal
- `75–90%` → yellow (`theme::WARN`)
- `> 90%` → red (`theme::CRIT`)

**`NotConfigured` providers**: omitted entirely (same as spec's render table).

---

## Dashboard mode — Quota pane (`widgets/dashboard_plan.rs`)

Replaces all existing content. Same file, same `pub fn render(frame, area, app)`
signature. Existing `MergedPlan`, `merge_plans`, and all `PlanUsage`-based logic
are removed.

### Layout

40% left / 60% right split (identical to existing plan pane split).

**State gate:** `QuotaState::Idle` or `Loading` renders a centered message across
the full area instead of the split.

### Left panel — provider list

```
┌─ Quota ──────────┐
│ ● Claude     72% │
│ ● Copilot    ∞   │
│ ▲ z.ai       88% │
│ ✗ Google    401  │
│ ○ Codex      …   │
└──────────────────┘
```

**Glyphs:**
| Glyph | Meaning |
|-------|---------|
| `●` | Ready, ok |
| `▲` | Stale (last_good exists, current failed) |
| `✗` | Error, no last_good |
| `○` | Loading / not yet fetched |

**Summary value** (right-aligned):
- Ok: worst (highest) `used_percent` across all windows, or `∞` if all unlimited,
  or `—` if no windows
- Stale: same as ok, rendered dim
- Error (no last_good): short status code (e.g. `401`, `net`)
- Loading: `…`

Selected provider highlighted with existing theme selection style. `j`/`k` and
arrow keys scroll the list.

### Right panel — detail view

```
┌─ Claude · Pro · user@example.com ───────────────────────────┐
│ ! Stale — data from 14:23 · last error: Transport           │
│                                                              │
│ 5h        [███████░░░]  72%   resets in 2h 14m             │
│ 7d        [████░░░░░░]  45%   resets in 3d 12h             │
│ 7d-sonnet [█░░░░░░░░░]  12%   resets in 3d 12h             │
│                                                              │
│ Overage   disabled · limit $0.00                            │
│                                                              │
│                                  fetched at 14:23:05        │
└──────────────────────────────────────────────────────────────┘
```

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
gemini/gemini-2.5-pro    daily   [███░░░░░░░]  31%   resets in 18h
antigravity/gemini-pro   5h      [██████████]  98%   resets in 0h 12m
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
| `d` | Any | Toggle Dashboard/Classic (existing) |

No new global key bindings.

---

## Testing

### Layer 1 — Render snapshot tests (extend `tui/mod.rs` `TestBackend` tests)

**Classic Quota tab:**
- `QuotaState::Idle` → "Press r" message
- `QuotaState::Loading` → loading message
- Mixed providers: one with windows (some `used_percent`, some `None`), one unlimited,
  one stale (bar dim), one error (no last_good), one not-yet-fetched
- Color threshold boundary: 74%, 75%, 90%, 91%

**Dashboard quota pane:**
- Same state set as Classic tab
- Google provider: per-model windows, no top-level windows
- Extras: `OverageBudget` enabled, `OverageBudget` disabled, `PerToolCounts`
- Absent Claude windows rendered as `——` placeholders
- Stale warning line present/absent

All fixtures built inline from `ProviderResult` structs. No HTTP, no tokio.

### Layer 2 — State machine unit tests (`app/mod.rs`)

- `apply_quota_results`: first ok result → `last_good=None, current=ok, state=Ready`
- `apply_quota_results`: second ok result → `last_good=Some, current=ok`
- `apply_quota_results`: err after ok → `last_good=Some(prev), current=err`
- `set_quota_error` before any Ready → `QuotaState::Error`
- `set_quota_loading` → `QuotaState::Loading`
- `selected_provider` clamps to `quota_slots.len().saturating_sub(1)`

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
