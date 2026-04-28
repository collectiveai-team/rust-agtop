# TUI v2 Bugfix & Feature Design — 2026-04-27

## Scope

This spec covers five work items for the `feat/tui-redesign` branch of `rust-agtop`:

1. **Session state derivation fix** (Issues 2 & 4 — shared root cause)
2. **Mouse click consistency** across all interactive UI elements (Issue 1, Groups A/B/C)
3. **Quota Long panel: 2-column layout + fixed height + scroll** (Issue 3)
4. **Subagent tree view** in the sessions table (Issue 5, user addition)
5. **Config screen mouse support** — deferred to a separate ticket (Group D)

Worktree: `/home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign`  
Branch: `feat/tui-redesign`  
Run: `cargo run -p agtop-cli -- tui`  
Tests: `rtk cargo test --workspace -- --test-threads=1` (945 pass, 6 ignored — must stay green)

---

## Issue 1 (partial): Session State Derivation Fix

### Problem

`HeaderModel.sessions_active` and `sessions_idle` both show 0 even when live sessions exist. State dots in the sessions table show wrong colors (grey/blank for live sessions).

### Root Cause

In `refresh_adapter.rs::apply_analyses`, the counting of active/idle sessions may happen before or independent of `normalize_analysis`. The `normalize_analysis` function re-derives `session_state` from `liveness` when `liveness.is_some()`, but if counting runs on the raw analyses, it sees `session_state = None` or stale values.

### Fix

- In `apply_analyses`, ensure `normalize_analysis(&mut a)` is called on each analysis **before** counting states.
- Count `sessions_active` = rows where `session_state == Some(SessionState::Running)`.
- Count `sessions_idle` = rows where `session_state == Some(SessionState::Idle)`.
- The `SessionRow.analysis` stored in `sessions.rows` must hold the already-normalized analysis.
- Confirm `is_muted_row` behavior: currently mutes only when `Closed` AND `pid.is_none()`. A live-process session should never be muted.

### Test

Add unit test in `refresh_adapter.rs` (or `tests/`):
- Given `liveness = Some(Liveness::Live)` + `summary.state = "running"`, assert `session_state == Some(SessionState::Running)` and is counted as active.
- Given `liveness = Some(Liveness::Live)` + `summary.state = "idle"`, assert `session_state == Some(SessionState::Idle)` and is counted as idle.
- Given `liveness = Some(Liveness::Stopped)`, assert `session_state == Some(SessionState::Closed)`.

---

## Issue 2: Mouse Click Consistency — Groups A, B, C

### Context

The audit found the following elements already mouse-clickable:
- Quota `[u]sage` button (`dashboard/quota.rs`)
- Info drawer tabs `[1]–[4]` (`dashboard/info_drawer.rs`)
- Session table row selection + scroll (`dashboard/sessions.rs`)

The following need mouse support added (in this ticket):

### Group A — Global Tab Bar

**File:** `crates/agtop-cli/src/tui/widgets/tab_bar.rs` + `crates/agtop-cli/src/tui/app_v2.rs`

**Design:**
- Add `tab_rects: Vec<(Screen, Rect)>` to the `TabBar` widget (or store on `app_v2::App`).
- During `TabBar::render`, after drawing each tab span, compute and store its `Rect` from x-offset (accumulated span widths), y-position (`area.y`), and string width.
- In `app_v2::App::handle_event`, match `AppEvent::Mouse(MouseEvent { kind: Down(Left), column, row, .. })` and call `tab_bar.hit_test(column, row)` → `Option<Screen>` → dispatch `Msg::SwitchScreen(s)`.

**Tabs affected:** `[d]ashboard`, `[a]ggregation`, `[c]onfig`

### Group B — Aggregation Controls

**File:** `crates/agtop-cli/src/tui/screens/aggregation/controls.rs` + `crates/agtop-cli/src/tui/screens/aggregation/mod.rs`

**Design:**
- Add `chip_rects: Vec<(ControlTarget, Rect)>` to `ControlsModel` (where `ControlTarget = GroupBy | Range | Sort | Reverse`).
- During `ControlsModel::render`, store each chip's `Rect` (x from accumulated offset, y from row position, width = chip string width).
- Add `ControlsModel::handle_event(&mut self, event) -> bool` that hit-tests on `MouseEvent::Down(Left)` and cycles the clicked control (same mutations as `'g'`/`'r'` keyboard handlers).
- Route from `AggregationState::handle_event` to `ControlsModel::handle_event`.

**Controls affected:** GroupBy selector (5 chips), Range selector (4 chips), Sort chip, Reverse on/off toggle.

### Group C — Aggregation Drill-down Close Button

**File:** `crates/agtop-cli/src/tui/screens/aggregation/drilldown.rs`

**Design:**
- Add `last_area: Option<Rect>` to `DrillDown`.
- Set `last_area` in `render()`.
- In `DrillDown::handle_event`, add `MouseEventKind::Down(Left)` arm:
  - Check if `row == area.y` (title bar row).
  - Compute start offset of `[Esc]` substring in the title string; check `column` is within that range.
  - If hit, close the drill-down (same action as `KeyCode::Esc`).

---

## Issue 3: Quota Long Panel — 2-Column Layout + Fixed Height + Scroll

**File:** `crates/agtop-cli/src/tui/screens/dashboard/quota.rs`

### Struct Changes

```rust
pub struct QuotaPanel {
    pub mode: QuotaMode,
    pub cards: Vec<QuotaCardModel>,
    pub last_area: Option<Rect>,
    pub scroll_offset: usize,   // NEW
}
```

### Height

- `QuotaMode::rows_needed()` for Long mode: change from `12` to `10`.
- The layout constraint in `DashboardState::render` that sizes the quota panel changes to `Constraint::Length(10)`.
- Inner usable height = 8 rows (after 1-row border top + 1-row border bottom).

### `render_long` Redesign

1. Build `all_lines: Vec<Line>` — the full content (provider card headers + per-window rows), same logic as today.
2. **2-column layout:** If `area.width > 80`:
   - Split providers into two halves: left = `cards[0..ceil(n/2)]`, right = `cards[ceil(n/2)..]`.
   - Build `left_lines` and `right_lines` separately.
   - Render using a horizontal sub-layout: `[left_half | right_half]` split at `area.width / 2`.
   - Apply `scroll_offset` to both halves independently (same offset, so they scroll in sync).
3. **Single-column fallback:** If `area.width <= 80`, use the existing single-column rendering with `scroll_offset` applied.
4. **Pagination:**
   - Visible window: `lines[scroll_offset .. scroll_offset + inner_height]`.
   - If `scroll_offset > 0`: show `↑ N lines above` at the top of the inner area.
   - If `scroll_offset + inner_height < total_lines`: show `↓ N more lines` at the bottom.

### Scroll Events

In `QuotaPanel::handle_event`:
- `MouseEventKind::ScrollDown` within `last_area`: `scroll_offset = (scroll_offset + 1).min(max_offset)`.
- `MouseEventKind::ScrollUp` within `last_area`: `scroll_offset = scroll_offset.saturating_sub(1)`.
- Key `'j'` (when mode == Long): same as scroll down.
- Key `'k'` (when mode == Long): same as scroll up.
- Reset `scroll_offset = 0` when cycling to Short or Hidden mode.

### Max Offset

`max_offset = total_lines.saturating_sub(inner_height)`

---

## Issue 4 (deferred — config editing): Group D

Config section checkboxes, radio buttons, dropdowns, and text inputs have no keyboard editing either. Making them mouse-clickable without keyboard support creates an inconsistent experience. This is deferred to a dedicated config-editing feature ticket.

---

## Issue 5: Subagent Tree View

**Files:** `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs`, `crates/agtop-cli/src/tui/refresh_adapter.rs`

### Data Model Changes

```rust
// In sessions.rs
pub struct SessionRow {
    pub analysis: SessionAnalysis,
    pub client_kind: ClientKind,
    pub client_label: String,
    pub activity_samples: Vec<f32>,
    pub depth: u8,                          // NEW: 0 = top-level, 1 = child subagent
    pub parent_session_id: Option<String>,  // NEW: Some(id) for depth=1 rows
}

pub struct SessionsTable {
    pub rows: Vec<SessionRow>,
    pub state: TableState,
    pub pulse: PulseClock,
    pub animations_enabled: bool,
    pub sort_key: SessionSortKey,
    pub sort_dir: SortDir,
    pub table_area: Rect,
    pub collapsed: HashSet<String>,         // NEW: session_ids of collapsed parents
}
```

### Row Building in `refresh_adapter.rs`

In `apply_analyses`, after building the top-level rows:
1. For each top-level row in order:
   a. Add the parent row (depth=0) to the flat output.
   b. If `analysis.children` is non-empty AND `session_id` NOT in `collapsed`:
      - Add each child as a `SessionRow` with `depth=1` and `parent_session_id = Some(parent_session_id)`.
      - Children are sorted by age (descending), same as parent sort default.

### Rendering in `sessions.rs`

**Toggle cell (first column, parent rows with children):**
- If `depth == 0` and `analysis.children.len() > 0`:
  - Show `▼ ` if expanded (not in `collapsed`).
  - Show `▶ ` if collapsed (in `collapsed`).
- Depth=0 rows with no children: render first column normally.

**Child row indentation:**
- Depth=1 rows: prepend `  ` (2 spaces) to the session-id/name cell content.
- Child rows use the same columns as parent rows; process metrics (CPU/mem) are inherited from the parent process since subagents run in-process.

**Visual example:**
```
▼ ● ses_2303  Claude  claude-opus-4   ...  running
    ● child_001 Claude  claude-opus-4  ...  running
▶ ● ses_2302  Claude  claude-haiku-3  ...  idle
  ● ses_2301  Codex   gpt-4o          ...  running
```

### Toggle Interaction

**Mouse:** Click on the toggle cell (column 0, first 2 chars of a parent row's rect) → toggle `collapsed`.
- Store each parent row's `Rect` in `table_area` hit-testing. The toggle hit area is `[row_rect.x .. row_rect.x + 2]` × `row_rect.y`.

**Keyboard:** `Enter` or `Space` on a selected row that has children → toggle `collapsed`.

**Default:** All parents start expanded (empty `collapsed` set).

### Sorting

- Sort is applied to top-level rows only.
- Child rows remain anchored immediately after their parent, sorted by age within the parent.
- When the table is re-sorted, rebuild the flat row list from sorted top-level + their children.

### No Grandchildren

The data model is two-level only (`SessionAnalysis.children` is one level deep). No recursive tree rendering needed.

---

## Key Files Reference

```
crates/agtop-cli/src/tui/
  app_v2.rs                           # global event loop, Msg dispatch
  widgets/tab_bar.rs                  # global tab bar rendering
  refresh_adapter.rs                  # apply_analyses, normalize_analysis, derive_state
  screens/dashboard/
    mod.rs                            # DashboardState render + event routing
    header.rs                         # HeaderModel
    sessions.rs                       # SessionsTable, SessionRow
    quota.rs                          # QuotaPanel
    info_drawer.rs                    # InfoDrawer
  screens/aggregation/
    mod.rs                            # AggregationState
    controls.rs                       # ControlsModel (GroupBy/Range chips)
    drilldown.rs                      # DrillDown overlay
  screens/config/
    mod.rs                            # ConfigState (Group D, deferred)
  widgets/
    state_dot.rs
    state_style.rs
crates/agtop-core/src/
  process/mod.rs                      # attach_process_info, Liveness enum
```

---

## Patterns to Reuse

- **Button click hit-testing:** Store `last_area: Option<Rect>` (or `Vec<(T, Rect)>`) on widget. Set in `render(&mut self, ...)`. In `handle_event`, match `AppEvent::Mouse(MouseEvent { kind: Down(Left), row, column, .. })` and check rect bounds.
- **Scroll gating:** Compare `(column, row)` against stored `Rect` before acting on `ScrollUp`/`ScrollDown`.
- **State re-derivation:** `normalize_analysis` in `refresh_adapter.rs` — override `session_state` from `liveness` when `liveness.is_some()`.

---

## Out of Scope (Deferred)

- Config screen control editing (checkboxes, radio buttons, dropdowns, text inputs) — deferred to separate config-editing feature.
- Grandchild subagent rows — data model is two-level only; no deeper nesting needed.
