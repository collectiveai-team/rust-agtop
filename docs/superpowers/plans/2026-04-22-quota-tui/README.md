# Quota TUI — Implementation Plan Index

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Load only the phase you are currently executing plus this README. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the `agtop-core::quota` subsystem into the TUI — replacing the existing Dashboard Subscription Details pane and adding a new compact Quota tab to Classic mode.

**Spec:** `docs/superpowers/specs/2026-04-22-quota-tui-design.md`

**Prerequisite:** Phase 6 of `docs/superpowers/plans/2026-04-21-quota-provider-fetchers/` must be complete (quota core + CLI).

## Architecture (2-sentence summary)

Refresh worker in `agtop-cli` extended with a second inner loop that runs `quota::fetch_all` every 60 s while the quota pane is active, publishing `QuotaSnapshot` on the existing `watch` channel. Two rendering surfaces consume `App::quota_slots`: a horizontal card row in Classic mode (new `Tab::Quota`), and a list/detail split in Dashboard mode (rewriting `dashboard_plan.rs`).

## Tech stack

Rust 2021, ratatui, crossterm, tokio (existing `RefreshHandle`), `agtop_core::quota` (Phase 6). No new crates.

## Phase order and dependencies

Each phase leaves the codebase compiling with green tests and its own git commit.

| # | File | Content | Est. lines |
|---|------|---------|------------|
| 1 | [phase-1-app-state.md](./phase-1-app-state.md) | `ProviderSlot`, `QuotaState`, `App` fields and methods, `Tab::Quota` enum variant, unit tests. | ~400 |
| 2 | [phase-2-refresh-worker.md](./phase-2-refresh-worker.md) | `QuotaCmd`, `RefreshMsg` variants, quota inner loop in `refresh.rs`, worker integration tests with `FakeHttp`. | ~500 |
| 3 | [phase-3-classic-tab.md](./phase-3-classic-tab.md) | New `widgets/quota_tab.rs` — horizontal card row with centered content, `■` bars, horizontal scroll. Render snapshot tests. | ~600 |
| 4 | [phase-4-dashboard-pane.md](./phase-4-dashboard-pane.md) | Rewrite `widgets/dashboard_plan.rs` — compact left list (short view) + full-detail right pane. Render snapshot tests. | ~700 |
| 5 | [phase-5-wire-up.md](./phase-5-wire-up.md) | `QuotaCmd::Start/Stop` dispatch in `tui/mod.rs`, bottom-panel Quota tab registration, footer hint, key bindings, theme constants. End-to-end snapshot tests. | ~400 |

Phases are strictly sequential — each depends on the previous. No parallelism.

## What's done after each phase

**After Phase 1:**
- `App` gains `quota_slots: Vec<ProviderSlot>`, `quota_state: QuotaState`, `selected_provider: usize`, `model_scroll: usize`, `card_scroll: usize`.
- `App` gains `apply_quota_results`, `set_quota_error`, `set_quota_loading`, `quota_select_next`, `quota_select_prev`, `quota_card_scroll_left`, `quota_card_scroll_right`.
- `Tab::Quota` variant exists and cycles correctly.
- Preferred-window resolver function with per-provider fallback table.
- Unit tests pass: state transitions, upsert semantics, selection clamping, preferred-window lookup.

**After Phase 2:**
- `RefreshMsg::QuotaSnapshot` and `RefreshMsg::QuotaError` exist.
- `RefreshHandle` has `quota_trigger_tx: watch::Sender<QuotaCmd>`.
- Worker has a second inner loop that responds to `Start`/`Stop`, auto-refreshes every 60 s, honors `manual_tx` for ad-hoc refreshes.
- `refresh.rs` test suite extended with quota-loop tests using `FakeHttp`.

**After Phase 3:**
- `widgets/quota_tab.rs` exists with horizontal card layout.
- Bar rendering helpers using `■` (U+25A0) and threshold colors.
- Render snapshot tests: Idle/Loading/Ready states, mixed providers, stale/error/loading cards, overflow scrolling.

**After Phase 4:**
- `widgets/dashboard_plan.rs` fully rewritten; no `PlanUsage` code remains inside.
- Left pane: one-line short view per provider, sortable order from `quota_slots`.
- Right pane: full detail for selected provider (windows, extras, Google per-model, stale banner, error-only state).
- Render snapshot tests pass for all documented states.

**After Phase 5:**
- `tui::mod.rs` dispatches `QuotaCmd::Start/Stop` on tab switch and mode toggle.
- `render_bottom_panel` routes `Tab::Quota` to the new widget.
- Footer hint `[r] refresh` appears when quota pane is active.
- End-to-end TestBackend test: switch to Quota tab → QuotaCmd::Start sent → simulated QuotaSnapshot arrives → slots populated → render matches expected output.
- All existing TUI tests still pass (regression check).

## Key design decisions locked in from the spec

- **Option A (spec § Architecture)**: extend existing refresh worker, no new tokio runtime.
- **On-demand fetch**: no fetch on TUI startup; first fetch fires on quota pane entry.
- **`■` (U+25A0) for bars**; green/yellow/red at 75/90 thresholds; dim variant for stale.
- **Short view window**: preferred label per provider with fallback chain (spec § Preferred window table).
- **No changes to `agtop-core`.** All new code is in `agtop-cli`.
- **Existing `plan_usage` field in `App`/`RefreshMsg::Snapshot` is left in place** (unused by the quota pane but kept for future consumers). The Dashboard pane switches from `app.plan_usage()` to `app.quota_slots`.

## Execution handoff

Pick one:

1. **Subagent-Driven (recommended).** Fresh subagent per task, two-stage review. Use `superpowers:subagent-driven-development`.
2. **Inline execution.** Batch execution with checkpoints. Use `superpowers:executing-plans`.

Either way: process one phase at a time. Commit at the end of each phase before starting the next.

## Handoff to new session

When starting a fresh session to execute a phase:

```
I'm executing an implementation plan.

Plan index: docs/superpowers/plans/2026-04-22-quota-tui/README.md
Phase to execute: <phase-file-name>.md
Spec: docs/superpowers/specs/2026-04-22-quota-tui-design.md

Load the index and the phase file only. Use the executing-plans skill.
```
