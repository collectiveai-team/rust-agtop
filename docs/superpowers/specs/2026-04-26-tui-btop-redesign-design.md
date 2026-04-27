# TUI btop-style redesign

**Date:** 2026-04-26
**Status:** Design — awaiting user review
**Scope:** Full restructure of the agtop TUI: 3 top-level views, new Dashboard layout, Aggregation analytics view, full-page Config, theme system, incremental architecture refactor.

---

## Goals

- Reposition agtop as a **lightweight agent-observability tool** with a btop-inspired visual identity.
- Replace the current single-screen tabbed UI with **three top-level views**: Dashboard (default), Aggregation, Config.
- Introduce a **theme system** with semantic color tokens, true-color support, and a polished VS Code Dark+ default.
- Add **braille-based density visuals** (sparklines, gradient bars) wherever they earn their pixels.
- Refactor incrementally toward a **Model-View-Update + Component** architecture without big-bang rewrites; the app stays runnable at every step.
- Keep `agtop-core` largely untouched. Allowed additive changes: aggregation helpers, an `Error` variant on `SessionState`, exposing the stalled-threshold as runtime config, and a small `current_action()` helper if not already present.

## Non-goals

- Customizable keybindings (deferred — v1 ships a read-only reference).
- Multiple themes shipped (only VS Code Dark+; theme loading from TOML deferred).
- Theme editor UI.
- Opening sessions in their client app from the dashboard (separate TODO item).
- Rewrites of `agtop-core`.
- Mouse-driven panel resizing.

---

## 1. View architecture & navigation

agtop becomes a 3-view application with single-key mnemonic switching:

| Key | View         | Purpose                                       |
|-----|--------------|-----------------------------------------------|
| `d` | Dashboard    | Live ops view (default on startup)            |
| `a` | Aggregation  | Group-by analytics across time ranges         |
| `c` | Config       | Full-page settings (replaces in-dashboard tab)|
| `q` | Quit         | Exit                                          |
| `?` | Help overlay | Context-sensitive keymap                      |

Reserved global keys: `d / a / c / q / ?`. All other keys are scoped to the active view.

A slim tab bar at the top of every view shows the active view (highlighted via `accent.primary`) and the version badge on the far right. Mouse-clickable.

Architecturally:

```rust
enum Screen {
    Dashboard(DashboardState),
    Aggregation(AggregationState),
    Config(ConfigState),
}
```

Top-level `App` owns `Screen` + global state. Events route first to global keymap, then to the active screen. See §6 for the full architecture.

---

## 2. Dashboard

### Layout

Three regions, top-to-bottom:

1. **Header** — 3 fixed rows
2. **Sessions table** — flex (fills remaining space)
3. **Usage Quota panel** — toggleable bottom region (short / long / hidden)

Mockup:

```
┌─ agtop ──────────────── [d]ashboard [a]ggregation [c]onfig  q=quit ?=help  v0.4.2 ─┐
│ Procs 12   CPU  ⡀⣀⣤⣶⣿⣷⣶⣤⣀⡀⣀⣤⣶⣿  34%                              ⟳ 2s · 14:25:49 │
│ Mem ████████████░░░░░░░░ 12.4G/16G               Sessions: 8 active · 3 idle · 47 today │
│ Sessions ────────────────────────────────────────────────────────────────────────────  │
├────────────────────────────────────────────────────────────────────────────────────────┤
│      SESSION    AGE  ACTION                  ACTIVITY    CLIENT         SUBSCRIPTION       MODEL              CPU   MEM    TOKENS    COST    PROJECT       SESSION_NAME │
│ ●    a3f2c1de    3m  bash: cargo test        ⡀⣀⣤⣶⣿⣷⣶   claude-code    Claude Max 5x      sonnet-4.5         12%   184M  12.4k    $0.18    rust-agtop    refactor tui layout… │
│ ●    b81e9402   12m  permission: bash        ⡀⡀⣀⣤⣀⡀     claude-code    Claude Max 5x      sonnet-4.5          0%   142M   8.1k    $0.09    dotfiles      shell init cleanup    │
│ ●    c2d4f6a1    1m  edit: src/lib.rs        ⣀⣤⣶⣤⣀⡀     codex          ChatGPT Plus       gpt-5              23%   210M   4.2k    $0.07    webapp        api endpoint design   │
│ ●    e1f5c2d8    8m  responding…             ⡀⣀⡀⣀⡀⣀     copilot        GitHub Copilot     gpt-4o              5%   124M   1.8k    —        frontend      style guide audit     │
│ ●    f7a3b210   12m  —                       ⡀⡀⡀⡀⡀      gemini-cli     Google AI Pro      gemini-2.5-pro      0%    98M   2.1k    $0.02    research      lit review            │
│      d9e8a7b3   2h   —                                   claude-code    Claude Max 5x      sonnet-4.5          —     —     45.2k   $0.62    research      data analysis pass    │
├─ Usage Quota (short) ──────────────────────────────────────────────────── [u]sage ────┤
│ claude-code  ████████████░░░ 78% 5h  ·  codex  █████░░░░░░░░░ 31% weekly  ·  …       │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

Dot legend in this mockup:
- Row 1 dot: yellow `running` (running tool: bash)
- Row 2 dot: violet pulsating `waiting` (permission requested — also colored in ACTION)
- Row 3 dot: yellow `running` (edit tool)
- Row 4 dot: yellow `running` (response streaming)
- Row 5 dot: green `idle` (no current action; live)
- Row 6: no dot (closed) — entire row in `fg.muted`

### Header (3 rows, fixed)

| Row | Left                              | Center / Right                                |
|-----|-----------------------------------|-----------------------------------------------|
| 1   | `Procs N` + CPU braille sparkline | CPU % · refresh interval · clock              |
| 2   | Mem gradient block bar + G used/total | `Sessions: X active · Y idle · Z today`   |
| 3   | Section divider rule with the embedded title `Sessions` (thin horizontal line, title rendered in `fg.emphasis` on a small offset) |  |

Notes:
- Sparkline uses braille (`⡀⣀⣤⣶⣿⣷⣶⣤⣀`); gradient color follows the urgency palette (green→yellow→orange→red).
- Memory bar uses block gradient (`░▒▓█`).
- Version badge `v0.x.y` is rendered inside the top tab bar (right edge), reading from `crates/agtop-cli/src/version.rs`.

### Sessions table

**Default visible columns (left → right):**

| # | Column         | Notes |
|---|----------------|-------|
| 1 | `●` (state dot)| 1-cell colored dot encoding session state. Empty cell for `closed`. See §2.5 for color mapping. |
| 2 | `SESSION`      | Truncated session id |
| 3 | `AGE`          | Right-aligned, compact |
| 4 | `ACTION`       | NEW — current agent action (tool call / response status). `—` when not running. ~18 chars, truncates with `…`. |
| 5 | `ACTIVITY`     | NEW — braille sparkline of token rate (last N samples) |
| 6 | `CLIENT`       | Full name, rendered in client theme color |
| 7 | `SUBSCRIPTION` | e.g. `Claude Max 5x`, `ChatGPT Plus` |
| 8 | `MODEL`        | Full model name |
| 9 | `CPU`          | Right-align, % |
| 10| `MEM`          | Right-align, compact |
| 11| `TOKENS`       | Right-align, compact |
| 12| `COST`         | Right-align, `$x.xx` or `—` |
| 13| `PROJECT`      | Flex, basename |
| 14| `SESSION_NAME` | Flex, truncated with `…` |

The textual `STATE` column is **not** in the default visible set (the dot replaces it). It remains available via Config → Columns for users who want both.

**Hidden by default but available** (via Config → Columns):
`STATE` (textual), `EFFORT`, `PID`, `STARTED`, `LAST_ACTIVE`, `DURATION`, `OUT`, `CACHE`, `TOOLS`, `AGENT`, `USER`, `CONTEXT`, `VSZ`, `DISKR`, `DISKW`, `CWD`. The existing `column_config.rs` enum is preserved; we add one new id (`Action`) for the new column.

**Sort:**
- Default: `AGE` ascending (newest first).
- `s` cycles sort key through visible columns; `S` reverses direction.
- Click on column header sorts by that column; click again reverses.

### 2.5 Session state model

The redesign **normalizes session state to 6 canonical variants in `agtop-core`**, replacing the legacy 7-variant flat enum. This is a deliberate (breaking) refactor — see §6.1. State is a **domain concept owned by core**, not a TUI display concern. The TUI renders state directly without a mapping layer; only color and animation choices live in the TUI.

#### Canonical core state

```rust
#[non_exhaustive]
pub enum SessionState {
    /// Agent is actively producing output or executing a tool call.
    Running,
    /// Agent has paused and requires user response. The reason carries
    /// the actionable detail (input vs permission vs custom).
    Waiting { reason: WaitReason },
    /// Live session that hasn't shown progress past the configured threshold,
    /// or another non-fatal anomaly.
    Warning { reason: WarningReason },
    /// Session ended in an explicit error condition.
    Error { reason: ErrorReason },
    /// Live session, ready for input, not currently working.
    Idle,
    /// No live process; historical/archival.
    Closed,
}

#[non_exhaustive]
pub enum WaitReason {
    /// Agent is asking the user a question (free-form input expected).
    Input,
    /// Agent is requesting permission to run a tool / sandbox / filesystem op.
    Permission,
    /// Other client-specific waiting condition; payload is a short label.
    Other(String),
}

#[non_exhaustive]
pub enum WarningReason {
    /// Live session has had no observable activity since `since` past the
    /// configured threshold (default 5 minutes; configurable per call).
    Stalled { since: chrono::DateTime<chrono::Utc> },
    /// Other client-specific warning condition; payload is a short label.
    Other(String),
}

#[non_exhaustive]
pub enum ErrorReason {
    /// Underlying process exited with a non-zero exit code.
    ExitCode(i32),
    /// Process crashed (signal, OOM, etc.).
    Crash,
    /// Parser detected an error event in the session log.
    ParserDetected(String),
}
```

This is **strictly more expressive** than the old flat 7-variant enum: consumers that only care about coarse state use `matches!(state, SessionState::Waiting { .. })`; consumers that need detail destructure the reason. No `Unknown` variant — absence of state is `Option<SessionState>` at the producer side; parsers return `Result<SessionState, ParseError>` when relevant.

#### Why this lives in core

- **Single source of truth.** `agtop-core` is the library; programmatic consumers (a future `agtop --json` export, an MCP server, observability exporters) must see the same state vocabulary the TUI sees.
- **Threshold logic stays in core.** The `Running → Warning::Stalled` transition is computed inside core's `state_resolution`, owning the threshold parameter. The TUI reads the result; it never re-runs the arithmetic.
- **No translation layer.** The TUI has no `DisplayState` enum, no `from_core` mapping. State is state.

#### TUI rendering of state

State is presentation-agnostic; the TUI maps it to colors and animation in `tui::widgets::state_style`:

| State                         | Dot color (theme slot)            | Style modifiers                          |
|-------------------------------|-----------------------------------|------------------------------------------|
| `Running`                     | `status.warning`   (`#CCA700`)    | bold                                     |
| `Waiting { .. }`              | `accent.secondary` (`#C586C0`)    | **pulsating** brightness, ~800ms cycle   |
| `Warning { .. }`              | `status.attention` (`#D89614`)    | none                                     |
| `Error { .. }`                | `status.error`     (`#F48771`)    | bold                                     |
| `Idle`                        | `status.success`   (`#89D185`)    | none                                     |
| `Closed`                      | none — empty cell                  | row text in `fg.muted` (dim)             |

Implementation:

```rust
pub fn dot_color(state: &SessionState, theme: &Theme) -> Option<Color> {
    match state {
        SessionState::Running       => Some(theme.status_warning),
        SessionState::Waiting { .. }=> Some(theme.accent_secondary),
        SessionState::Warning { .. }=> Some(theme.status_attention),
        SessionState::Error { .. }  => Some(theme.status_error),
        SessionState::Idle          => Some(theme.status_success),
        SessionState::Closed        => None, // empty cell
    }
}

pub fn should_pulse(state: &SessionState) -> bool {
    matches!(state, SessionState::Waiting { .. })
}
```

#### Stalled threshold

Configurable, default **5 minutes**. Setting: `refresh.stalled_threshold_secs` (default 300), lives in Config → Refresh. The threshold is consumed inside core's `state_resolution::resolve_state_with_threshold` to construct `Warning { reason: Stalled { since } }` from a `Running` candidate. The TUI passes the configured value when triggering refresh.

#### Pulsation

`Waiting { .. }` dots pulsate brightness between full and ~70% on an 800ms cycle. Implemented via a separate animation tick in the event loop with dirty-region tracking (only the dot cell repaints between full data refreshes).

#### Animation opt-out

`appearance.animations: bool` config (default `true`). When `false`, `Waiting` renders as static violet bold. Auto-disables when frame budget exceeded (see §6.4 frame instrumentation). `NO_COLOR` environment auto-disables animations.

#### Closed-row visual treatment

Entire row text rendered in `fg.muted` (dim). Combined with the empty dot cell, makes live vs historical sessions instantly distinguishable.

#### ACTION column relationship

The Sessions table's `ACTION` column shows the agent's current activity (current tool call, response status). It is **independent** of `SessionState`:

- For `Running` / `Waiting` rows, ACTION shows what the agent is doing (or paused on).
- For `Idle` / `Closed` / `Warning` / `Error` rows, ACTION is `—`.
- When the session is in `Waiting { reason: Permission }`, the action text (e.g. `permission: bash`) is rendered with `status.warning` color modifier — providing a second visual cue alongside the violet pulsing dot.

ACTION text is sourced from `SessionAnalysis::current_action` (a sibling field on the analysis, populated by per-client parsers). It is not derived from `SessionState` itself.

**Activity sparkline column:**
- Braille (`⠀⡀⣀⣤⣶⣿⣷` etc.), 6–8 cells wide.
- Backed by a per-session ring buffer of recent token-rate samples.
- Cell intensity gradient (green → yellow) encodes magnitude.
- Empty cells (`⠀`) for samples before the session started.

**Responsive width strategy:**
- Identity columns (state dot, `SESSION`, `CLIENT`) never truncate. The dot is always visible.
- Flex columns (`PROJECT`, `SESSION_NAME`) absorb slack and truncate with `…`.
- The `ACTION` column truncates at its column width with `…`.
- < 100 cols: hide `SESSION_NAME` first.
- < 90 cols: hide `MEM`, then `CPU`.
- < 80 cols: hide `ACTIVITY` and `SUBSCRIPTION`.
- < 70 cols: hide `ACTION` (state dot still encodes activity status).

### Usage Quota panel (`u` cycles short → long → hidden)

- **Short (~4 rows):** one line per active client showing only the **closest-to-limit window**. Format: `client-name  ████████░░░ 78% 5h-window  ·  …`. Inline `·` separators on wide terminals; wraps to multiple rows on narrow.
- **Long (~10 rows):** current full quota tab content — all windows per client, vertically stacked.
- **Hidden:** panel removed, sessions table fills the freed rows.

Bound to `u` (chosen to avoid colliding with `q`=quit). Footer hint: `[u]sage`.

### Info drawer (`i` toggles)

A floating panel anchored to the **bottom-right corner**. Approximately 50% width × 60% height. Does **not** dim the underlying view. Clear border in `bg.overlay` color. Four tabs along the top: `[1] Summary  [2] General  [3] Costs  [4] Process`. **Summary is the default tab** when the drawer opens.

**Tab keys: `1` / `2` / `3` / `4`** (numerics avoid colliding with the global view-switch mnemonics `d/a/c`). `Tab` / `Shift+Tab` also cycle. Arrow keys / `j/k` / scroll wheel scroll within tab content.

#### Summary tab (default)

Three stacked sections within the tab:

**Hero block (~5 rows):**
- Line 1: session name (or project basename) + state in matching color (running/waiting/warning/error/idle/closed)
- Line 2: project path (with folder icon when `appearance.nerd_font=true`, plain text otherwise)
- Line 3: relative-time line (e.g. `just now · started 3m 41s ago`)
- Line 4: tag chips — small colored pills using `bg.overlay` background, one per tag (client name, subscription, model). Each pill rendered in its theme color.

**Status block (~4 rows, key/value pairs):**
- `State` + dot color + current action text (e.g. `● running  bash: cargo test`)
- `Session` — full session id
- `PID` — pid + parent pid
- `Tokens` + `Cost`

**Recent messages block (flex, scrollable):**
- Last 5–10 message turns from the session log
- Each turn prefixed with role label (`user` / `agent` / `tool`), color-coded
- Each turn truncated to ~2 lines with `…`; full content reachable via scroll
- Tool calls rendered inline beneath the agent turn that issued them: `[tool] bash: cargo test`
- Currently-running tool gets a `▸` marker
- Earlier-turns indicator at the top: `⋮ N earlier turns`
- Auto-scrolls to bottom on update; Page Up/Down navigates pages
- For clients without message-log support: `Recent messages not available for this client.` in `fg.muted`

#### General tab
Client, Subscription, Model + effort, Project, Started, Last message, Interactions counts, PID + parent PID, full Session id, all session metadata in tabular key/value form.

#### Costs tab
Per-model breakdown table, totals, output/cache token split, cost-per-turn average.

#### Process tab
PID tree, CPU/MEM history sparklines, disk I/O, parent process info.

Tracks the currently-selected row in the sessions table — content updates live as you move selection. `i` or `Esc` dismisses.

### Footer

Single row at the bottom: `[i] info  [u] quota  [/] filter  [s] sort  [Enter] open  [?] help`. View-scoped; updates per active screen.

### Mouse interactions (Dashboard)

- Click on `[d]/[a]/[c]` tab in the top bar → switch view.
- Click on a session row → select.
- Double-click on a row → open session (same as `Enter`).
- Click on column header → sort by that column; click again → reverse.
- Scroll wheel on table → scroll rows.
- Scroll wheel on info drawer → scroll tab content.
- Click on quota panel → cycle short → long → hidden (matches `u`).
- Click on `[i]`/`[u]`/`[?]` footer hints → trigger that action.
- Click outside info drawer → dismiss it.
- **Shift+click bypasses mouse capture for native terminal text selection.**

Mouse capture is opt-out via config (`appearance.mouse_capture: true|false`).

---

## 3. Aggregation view (`a`)

A full-screen analytics view with group-by and time-range selectors.

### Layout

```
┌─ agtop ──────────────── [d]ashboard [a]ggregation [c]onfig  q=quit ?=help  v0.4.2 ─┐
│ Group by:  ‹ Client ›  Provider  Model  Project  Subscription                          │
│ Range:     ‹ Today ›  Week  Month  All     |  Sort: ‹COST›  Reverse: off               │
├────────────────────────────────────────────────────────────────────────────────────────┤
│  GROUP            SESSIONS  TOKENS    COST     AVG DUR   LAST ACTIVE   ACTIVITY        │
│  claude-code            12   124.3k   $1.84      4m 22s   2m ago        ⡀⣀⣤⣶⣿⣷⣶⣤⣀⡀⣀⣤  │
│  codex                   7    48.6k   $0.91      2m 18s   1m ago        ⡀⣀⣤⣀⡀⣀⣤⣶⣤⣀⡀⣀  │
│  …                                                                                     │
├────────────────────────────────────────────────────────────────────────────────────────┤
│  TOTAL                  34   214.9k   $2.94                                            │
└────────────────────────────────────────────────────────────────────────────────────────┘
[g] group   [r] range   [s] sort   [/] filter   [Enter] drill into group   [?] help
```

### Controls (rows 1–2)

- **Group by** (`g` cycles): `Client / Provider / Model / Project / Subscription`. Active option wrapped in `‹ … ›`, colored `accent.primary`.
  - `Client` = the agentic CLI (claude-code, codex, gemini-cli, copilot, cursor, antigravity, opencode, …).
  - `Provider` = the upstream model API (anthropic, openai, google, …) — distinct from Client only for opencode-style clients that route to multiple providers; otherwise it mostly mirrors Client.
  - `Subscription` = the billing plan (Claude Max 5x, ChatGPT Plus, Google AI Pro, GitHub Copilot, …).
- **Range** (`r` cycles): `Today / Week / Month / All`.
  - Today = since 00:00 local
  - Week = rolling last 7 days
  - Month = rolling last 30 days
  - All = no filter
- **Sort** (`s` cycles visible columns); **Reverse** (`S` toggles).

### Table

Fixed columns (not user-configurable in v1):

| Column        | Description |
|---------------|-------------|
| `GROUP`       | Group label; colored when grouping by `Client` or `Provider` |
| `SESSIONS`    | Count within range |
| `TOKENS`      | Compact total (`124.3k`, `2.1M`) |
| `COST`        | Total; `—` if any session lacks pricing data |
| `AVG DUR`     | Mean session duration |
| `LAST ACTIVE` | Most recent activity timestamp, relative |
| `ACTIVITY`    | Braille sparkline distributed across the selected range (auto-bucketed: hours for Today, days for Week/Month, weeks for All up to 12 buckets) |

### Total row

Sticky at the bottom of the table area, above the keymap footer. Bold values; same column structure with `TOTAL` label.

### Drill-down (`Enter` on a row)

Opens an overlay over the Aggregation view containing the same Sessions table component as the Dashboard, pre-filtered to the selected group + time range. `Esc` closes. Reuses the dashboard sessions component (validates the `Component` trait).

### Filter (`/`)

Inline filter on the `GROUP` column substring. `n/N` next/prev match. `Esc` clears.

### Visual rules

- When grouping by `Client` or `Provider`, the entire `GROUP` cell is rendered in the client/provider theme color.
- When grouping by `Model`, the model name uses `syntax.keyword` styling.
- Cost coloring: `$0.00` → `fg.muted`; normal → `fg.default`; highest-cost row → `accent.secondary`; top-5% outliers → `status.warning`.
- Sparkline cells use the green→yellow gradient.

### Mouse interactions

- Click on group-by pill → switch grouping.
- Click on range pill → switch range.
- Click on column header → sort.
- Click on row → select; double-click → drill in.
- Scroll wheel → scroll table.
- Shift+click → native text selection.

### Empty / loading

- Empty range: centered message `No sessions in selected range. Press [r] to widen the time window.`
- Loading: braille spinner + `Computing aggregates…` if computation exceeds 200ms.

---

## 4. Config view (`c`)

Full-screen page with VS Code Settings layout: section list on the left, settings detail on the right.

### Layout

```
┌─ agtop ──────────────── [d]ashboard [a]ggregation [c]onfig  q=quit ?=help  v0.4.2 ─┐
│ Search: ____________________________________                                            │
├──────────────────────┬─────────────────────────────────────────────────────────────────┤
│  ‹ Appearance ›      │  Appearance                                                      │
│    Columns           │  ─────────                                                       │
│    Refresh           │    Theme              [ vscode-dark+ ▾ ]                         │
│    Clients           │    True color         [ auto ▾ ]   auto / on / off              │
│    Keybinds          │    Mouse capture      [x]   (Shift+click for text selection)   │
│    Data sources      │    Show version badge [x]                                        │
│    About             │    Header density     ( ) compact (•) normal ( ) detailed        │
│                      │  Status colors                                                   │
│                      │    live      ████████  #89D185                                   │
│                      │    waiting   ████████  #CCA700                                   │
│                      │    error     ████████  #F48771                                   │
│                      │    done      ████████  #C586C0                                   │
│                      │  Client colors                                                   │
│                      │    claude-code  ████  #D97757                                    │
│                      │    codex        ████  #00A67E                                    │
│                      │    …                                                             │
├──────────────────────┴─────────────────────────────────────────────────────────────────┤
│ [↑↓] navigate  [Tab] switch pane  [Enter] edit  [/] search  [Esc] back  [?] help        │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### Sections (left sidebar, ~22% width)

| Section        | Contents |
|----------------|----------|
| Appearance     | Theme, true color, mouse capture, version badge toggle, header density, status colors, client colors |
| Columns        | Sessions table column visibility + reorder (existing column editor, relocated) |
| Refresh        | Refresh interval, pause-on-idle, lazy-load on startup |
| Clients        | Per-client enable/disable, custom session paths, parser flags |
| Keybinds       | Read-only reference table |
| Data sources   | Session source paths per client, refresh status |
| About          | Version, build info, repo/docs links, config file path |

### Right pane (detail, ~78% width)

- Section title (bold, `fg.emphasis`) + underline rule
- Subsection headings (smaller underline rule)
- Setting rows: `label  control  helper-text`
- Controls: dropdown `[ value ▾ ]`, checkbox `[x]/[ ]`, radio `(•)/( )`, color swatch `████ #HEX`, text input `[ text________ ]`
- Helper text in `fg.muted`
- Scrollable; sidebar fixed

### Search bar (top, full width)

`/` focuses it; typing filters both sidebar sections and right-pane settings (substring + fuzzy on label, helper, section name). `Esc` clears.

### Interaction

- `Tab` / `Shift+Tab` — switch focus between sidebar and right pane.
- `↑/↓` or `j/k` — move within focused pane.
- `Enter` — open dropdown / toggle / cycle radio / open color picker / open text input.
- `Esc` — close picker; if nothing open, return focus to sidebar.
- `/` — focus search.
- Mouse: click to focus and trigger; scroll wheel scrolls right pane.

### Color picker

`Enter` on a color swatch opens an inline picker with 16 ANSI presets and a hex input field. Live preview updates the swatch as you type. `Enter` confirms; `Esc` cancels.

### Persistence

Existing config file format/location preserved. Changes apply immediately; persist asynchronously. Footer toast: `Saved: appearance.theme = vscode-dark+`. Bad values revert silently with a footer toast.

### Deferred (v2)

- Customizable keybindings (read-only in v1).
- Theme editor / multiple themes.
- Per-section import/export.
- Validation error dialogs.

---

## 5. Theme system

Single shipped theme: **VS Code Dark+**. Theme system is structured to support more themes later via TOML.

### Semantic tokens (palette)

| Slot                | Color (true-color) | Use |
|---------------------|--------------------|-----|
| `bg.base`           | `#1E1E1E`          | main background |
| `bg.surface`        | `#252526`          | panels |
| `bg.overlay`        | `#2D2D30`          | drawer / popups |
| `bg.selection`      | `#264F78`          | row highlight |
| `fg.default`        | `#D4D4D4`          | body text |
| `fg.muted`          | `#858585`          | metadata |
| `fg.emphasis`       | `#FFFFFF`          | headers, focused |
| `border.muted`      | `#3C3C3C`          | unfocused borders |
| `border.focused`    | `#007ACC`          | focused border |
| `accent.primary`    | `#007ACC`          | links, focus |
| `accent.secondary`  | `#C586C0`          | secondary highlights |
| `status.error`      | `#F48771`          | errors, error state dot |
| `status.warning`    | `#CCA700`          | running state dot, in-progress signals |
| `status.attention`  | `#D89614`          | warning (stalled) state dot, attention-grabbing accents |
| `status.success`    | `#89D185`          | idle state dot, success indicators |
| `status.info`       | `#4FC1FF`          | info |
| `syntax.string`     | `#CE9178`          | project paths |
| `syntax.keyword`    | `#569CD6`          | model names, keys |

### Per-client palette

| Client        | Color     |
|---------------|-----------|
| claude-code   | `#D97757` |
| codex         | `#00A67E` |
| gemini-cli    | `#4285F4` |
| copilot       | `#FFD43B` |
| cursor        | `#A78BFA` |
| antigravity   | `#22D3EE` |
| opencode      | `#F472B6` |
| unknown       | `#6B7280` |

Used to render full client names in colored text wherever they appear (Sessions table, Aggregation table, Usage Quota labels, Info drawer header).

### True-color support

Config setting `appearance.true_color: auto | on | off`:
- `auto` (default): detect via `$COLORTERM=truecolor|24bit`; fall back to 256-color, then 16.
- `on`: force true color.
- `off`: force 256-color downsampling.
- `$NO_COLOR` always disables color regardless of setting.

The renderer holds RGB values in the theme; downsampling happens at emit time.

### Visual hierarchy techniques

- **Bold** — section titles, focused selection labels, `live` state, `error` state.
- **Dim** — metadata, timestamps, `idle` state, `fg.muted`.
- **Reverse** — currently focused row when row colors don't suffice.
- Background layering (base → surface → overlay) creates depth without heavy borders.
- Color is paired with text/position — no color-only meaning; passes WCAG AA contrast in dark mode.

### Iconography & visual character system

agtop **never uses emoji**. The visual vocabulary is organized into three tiers, with strict rules about when each is used. This catalog is the canonical source — widget code references semantic ids, never raw codepoints.

#### Three-tier system

| Tier | What | When | Fallback |
|---|---|---|---|
| **T0 — Universal Unicode** | Box-drawing, braille, basic geometric shapes (`●`, `▾`, `▲▼`, `▸`, `⋮`, `‖`, `⟳`, etc.) | Always rendered. Primary visual vocabulary. | None needed (Unicode 1.1–6.0) |
| **T1 — Bitmap logos** | Client/provider brand logos via `ratatui-image` | Existing — Usage Quota long mode. Don't add new T1 sites in this redesign. | Auto-detect terminal graphics support; collapse to 0-width when absent (existing pattern) |
| **T2 — Nerd Font icons** | Material Design Icons via Nerd Font Private Use Area codepoints | Enhancement-only. Every T2 site has a working T0 or text fallback. | Plain text or omitted (per catalog below) |

Config setting: `appearance.nerd_font: bool` (default `false`). When `false`, no T2 icons render anywhere — UI is fully complete with T0 + T1.

#### Design rules

1. **T0 is primary.** Box-drawing, braille, and geometric shapes define the look. Most of the UI uses only T0.
2. **T1 (bitmap logos) stays where it is.** Quota panel only. No new T1 sites.
3. **T2 is enhancement only.** Every T2 site must produce a clean, intentional UI when disabled.
4. **No emojis anywhere, ever.** Material/Nerd Font codepoints are not emojis.
5. **One canonical glyph per concept.** Widget code uses `Icon::Folder` (a semantic enum), never raw codepoints.

#### T0 — Universal Unicode catalog (always renders)

| Glyph | Codepoint | Use site(s) |
|---|---|---|
| `●` | U+25CF | State dot in Sessions table; pulsates for `waiting` |
| `○` | U+25CB | Reserved for future hollow-dot uses |
| `▌` | U+258C | Focused-row marker (left edge) |
| `▾` | U+25BE | Dropdown indicator (Config controls); active-pill marker (Aggregation controls) |
| `▸` | U+25B8 | Active in-flight tool marker (Info drawer Summary); drill-down row cursor (Aggregation) |
| `▲` / `▼` | U+25B2 / U+25BC | Sort direction on column headers |
| `‹ … ›` | U+2039 / U+203A | Active-pill brackets (Aggregation Group-by, Range; Config sidebar selection) |
| `⋮` | U+22EE | Truncation indicator (earlier rows / earlier turns) |
| `‖` | U+2016 | Pause indicator (refresh paused) |
| `⟳` | U+27F3 | Refresh indicator |
| `✓` | U+2713 | Success toast |
| `✗` | U+2717 | Error toast |
| `!` | U+0021 | Warning glyph (in `status.attention` color) |
| `…` | U+2026 | Text truncation (universal) |
| `▔` | U+2594 | Active-tab underline marker |
| `░▒▓█` | U+2591–U+2588 | Block-gradient bars (memory, quota, color swatches) |
| Braille range | U+2800–U+28FF | Sparklines, loading spinner |

#### T2 — Nerd Font catalog (opt-in)

Each entry: site / Nerd Font codepoint / fallback.

**Header & status:**

| Site | Nerd Font | T0/text fallback |
|---|---|---|
| Refresh indicator (header) | `nf-md-refresh` `󰑐` | T0 `⟳` |
| Pause indicator | `nf-md-pause` `󰏤` | T0 `‖` |
| Clock prefix | `nf-md-clock_outline` `󰥔` | (omitted — text suffices) |
| Process counter prefix | `nf-md-cog_outline` `󰢻` | (omitted — `Procs` text) |

**Info drawer Summary tab (hero block only):**

| Site | Nerd Font | Fallback |
|---|---|---|
| Project path | `nf-md-folder` `󰉋` | (omitted — bare path) |
| Started/last-active timestamp | `nf-md-clock_outline` `󰥔` | (omitted) |

(Info drawer tabs themselves are **always text-only**, never icons.)

**Config view section titles** (icons render when `nerd_font=true`, text-only otherwise):

| Section | Nerd Font |
|---|---|
| Appearance | `nf-md-palette` `󰏘` |
| Columns | `nf-md-table_column` `󱏷` |
| Refresh | `nf-md-refresh` `󰑐` |
| Clients | `nf-md-account_multiple_outline` `󰣉` |
| Keybinds | `nf-md-keyboard_outline` `󰥻` |
| Data sources | `nf-md-database_outline` `󰆼` |
| About | `nf-md-information_outline` `󰋽` |

(Active section in the sidebar is also wrapped with T0 `‹ … ›` brackets — independent of icon presence.)

**Config view controls:**

| Site | Nerd Font | Fallback |
|---|---|---|
| Search input prefix | `nf-md-magnify` `󰍉` | T0 `/` |

**Empty states:**

| Site | Nerd Font | Fallback |
|---|---|---|
| Empty sessions list | `nf-md-tray` `󰷹` | (omitted — text only) |
| Empty aggregation results | `nf-md-database_off_outline` `󰪎` | (omitted) |
| Connection / data source error | `nf-md-alert_circle_outline` `󰅚` | (omitted) |

#### What deliberately does NOT get icons

These sites stay text-only at all times:
- Top tab bar (`[d]ashboard [a]ggregation [c]onfig`) — text only
- Info drawer tabs (`Summary` / `General` / `Costs` / `Process`)
- Tag chips in Info drawer hero (text in colored pills)
- Sessions table column headers
- Aggregation table column headers
- Status block key/value rows in Info drawer
- Help overlay key caps (`[d]`, `[/]`)
- Recent-messages role labels (`user`/`agent`/`tool`)
- Checkboxes / radio buttons / color swatches (use T0 text-glyphs only)

#### Implementation pattern

A single `widgets::icon` module exposes a semantic enum + render function:

```rust
pub enum Icon {
    Folder, Clock, Refresh, Pause, Search,
    Palette, TableColumn, AccountMultiple, KeyboardOutline,
    DatabaseOutline, DatabaseOffOutline, InformationOutline,
    Tray, AlertCircleOutline, CogOutline,
    // …
}

impl Icon {
    /// Returns rendered string honoring `appearance.nerd_font`.
    /// When NF disabled, returns either the T0 fallback or "" per catalog.
    pub fn render(self, theme: &Theme) -> &'static str;
}
```

Adding a new icon means: add an enum variant + codepoint + fallback. One place.

#### Test matrix

Every TUI snapshot test runs in **two flavors**:
- `nerd_font=false` (the universally-compatible default look)
- `nerd_font=true` (with Material icons)

Both must look intentional and complete. This is enforced via parameterized `insta` snapshot tests in Phase 1.

---

## 6. Architecture & migration

### 6.1 Required `agtop-core` changes

This redesign treats session state as a **core domain concern** and normalizes it. Some changes are additive; the state-enum refactor is a deliberate, breaking change to a pre-stable public API.

#### 1. **Replace `SessionState` with the canonical 6-variant form** (breaking)

The legacy 7-variant enum (`Running / Idle / AwaitingInput / AwaitingPermission / Stalled / Closed / Unknown`) is replaced with the 6-variant `SessionState` defined in §2.5, with `WaitReason / WarningReason / ErrorReason` sub-enums.

Migration path:
- Update `SessionState` definition + `as_str` + `compact_label` + serde reps.
- Update `state_resolution::resolve_state` to produce the new variants. The threshold-aware `Running → Warning::Stalled { since }` transition stays in this module; `since` carries the `last_active` timestamp.
- Update every client parser (`clients/claude.rs`, `clients/codex.rs`, `clients/gemini.rs`, `clients/copilot.rs`, `clients/cursor.rs`, `clients/antigravity.rs`, `clients/opencode.rs`) to produce the new variants. Mechanical mapping:
  - existing `AwaitingInput` → `Waiting { reason: WaitReason::Input }`
  - existing `AwaitingPermission` → `Waiting { reason: WaitReason::Permission }`
  - existing `Stalled` → `Warning { reason: WarningReason::Stalled { since } }`
  - existing `Unknown` → producer returns `None` (callers treat absence as Closed if no PID, error otherwise)
- Update all serde-tagged JSON fixtures and tests to match the new shape.

The serde representation uses `#[serde(tag = "kind", content = "reason")]` for tagged variants:
```json
{"kind": "running"}
{"kind": "waiting", "reason": {"input": null}}
{"kind": "waiting", "reason": {"permission": null}}
{"kind": "warning", "reason": {"stalled": {"since": "2026-04-26T10:00:00Z"}}}
{"kind": "error",   "reason": {"exit_code": 1}}
{"kind": "idle"}
{"kind": "closed"}
```

`#[non_exhaustive]` is preserved on the enum + each Reason sub-enum to allow future variants without breaking matches.

#### 2. **Add `SessionAnalysis::current_action: Option<String>`** (additive)

Extract the latest in-flight tool/action descriptor from a session log if available. Per-client extraction; falls back to `None` for clients that don't expose this. Independent of `SessionState`.

#### 3. **Make stalled threshold configurable per call** (additive)

The existing `pub const DEFAULT_STALLED_AFTER: Duration = Duration::minutes(5)` is preserved as the default. A new `pub fn resolve_state_with_threshold(...)` accepts an explicit `Duration`. The existing `resolve_state` becomes a thin wrapper that passes `DEFAULT_STALLED_AFTER`. The TUI passes its configured value.

#### 4. **Aggregation helpers** (additive, Phase 3)

`crate::aggregate::{GroupBy, TimeRange, aggregate}` — group sessions by dimension over a time range, compute per-bucket activity series. No state-enum impact.

#### Why breaking is okay here

`agtop-core` has no external consumers yet (workspace-internal only, no `crates.io` release). The state enum is a pre-stable API. Doing this now — once — is far cheaper than living with two divergent state models for the lifetime of the project. See `docs/architecture/ARCHITECTURE.md` (introduced by Plan 1) for the rationale captured permanently.

### Target abstractions

```rust
pub enum Screen {
    Dashboard(DashboardState),
    Aggregation(AggregationState),
    Config(ConfigState),
}

pub trait Component {
    fn render(&self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme);
    fn handle_event(&mut self, event: &AppEvent) -> Option<Msg>;
}

pub enum Msg {
    SwitchScreen(ScreenId),
    ToggleInfoDrawer,
    CycleQuotaMode,
    SelectSession(SessionId),
    OpenSession(SessionId),
    SetGroupBy(GroupDimension),
    SetTimeRange(TimeRange),
    SaveConfig(ConfigDelta),
    Quit,
    // …extend as needed
}

// State is owned by core. The TUI maps state → color/animation in
// `tui::widgets::state_style` (no enum, no mapping layer):
pub fn dot_color(state: &SessionState, theme: &Theme) -> Option<Color>;
pub fn should_pulse(state: &SessionState) -> bool;
```

Top-level loop (TEA shape):

```rust
loop {
    let event = next_event().await;
    let msg_opt = app.handle_event(&event);
    if let Some(msg) = msg_opt { app.update(msg); }
    terminal.draw(|f| app.render(f))?;
}
```

State is owned by `App` and its child `Screen`s. Events translate to `Msg`s. `update()` mutates state. Render reads state. No rendering decisions inside event handlers; no event handling inside render.

### Target module layout

```
crates/agtop-cli/src/tui/
├── mod.rs                    # entry, terminal init/restore
├── app.rs                    # App struct, loop, dispatch
├── msg.rs                    # Msg enum + dispatch helpers
├── component.rs              # Component trait
├── theme/
│   ├── mod.rs                # Theme struct, semantic tokens
│   ├── vscode_dark_plus.rs   # default palette
│   └── color.rs              # true-color detection, downsampling
├── layout/
│   ├── mod.rs                # named regions
│   └── splits.rs             # reusable split helpers
├── screens/
│   ├── mod.rs
│   ├── dashboard/
│   │   ├── mod.rs
│   │   ├── header.rs
│   │   ├── sessions.rs
│   │   ├── quota.rs
│   │   └── info_drawer.rs
│   ├── aggregation/
│   │   ├── mod.rs
│   │   ├── controls.rs
│   │   ├── table.rs
│   │   └── drilldown.rs
│   └── config/
│       ├── mod.rs
│       ├── sidebar.rs
│       ├── detail.rs
│       └── sections/
├── widgets/
│   ├── sparkline_braille.rs
│   ├── gradient_bar.rs
│   ├── colored_label.rs
│   ├── tab_bar.rs
│   ├── drawer.rs
│   └── help_overlay.rs
├── focus.rs                  # FocusManager
├── input.rs                  # crossterm event → AppEvent
├── column_config.rs          # KEEP — already good
├── refresh.rs                # KEEP — already good
└── events.rs                 # folds into input.rs / msg.rs over time
```

### Migration plan — 4 sequenced phases

Each phase is its own implementation plan, independently mergeable. App stays runnable throughout.

#### Phase 1 — Foundation (no user-visible change)

1. **Core: normalize `SessionState`** (per §6.1.1) — refactor to the canonical 6-variant enum with Reason sub-enums. Update `state_resolution`, all 7 client parsers, and all serde fixtures. Breaking change to `agtop-core`'s pre-stable API; no external consumers exist yet.
2. **Core additions** (per §6.1.2–4): `SessionAnalysis::current_action`, configurable stalled threshold, leave aggregation helpers for Phase 3.
3. Create `theme/` module with `Theme` struct + semantic tokens + VS Code Dark+ palette (incl. `status.attention`).
4. Add true-color detection with config override.
5. Define `Component` trait, `Msg` enum, `AppEvent` type. **No `DisplayState` enum** — state styling lives in `widgets::state_style` directly off `SessionState`.
6. Add `FocusManager`.
7. Add reusable widgets: `sparkline_braille`, `gradient_bar`, `colored_label`, `state_dot` (consumes `&SessionState`, with pulse animation hook), `icon` (semantic enum with full T2 catalog + T0/text fallback registry, reads `appearance.nerd_font`) — all unit-tested with both NF flavors.
8. Add animation tick infrastructure (separate timer, dirty-region tracking) + `appearance.animations` config flag.
9. Add snapshot test harness with `ratatui::backend::TestBackend` + `insta`.
10. Mechanical refactor: existing rendering reads colors from `Theme` instead of hardcoded `Color::*`. No visual behavior change beyond what the state refactor implies.
11. Commit `docs/architecture/ARCHITECTURE.md` with initial component model, theme tokens, and the state model rationale.

**Exit criteria:** all tests pass (including the rewritten state tests); `cargo run` works (visual differences limited to state-derived rendering, which is part of this refactor); new modules unit-tested; pulsing widget verified in isolation.

#### Phase 2 — Dashboard redesign

1. Add `Screen` enum + view-switching keymap (`d/a/c/q/?`); `a`/`c` go to placeholder screens.
2. Build `screens/dashboard/header.rs` (3-row header with sparkline, mem bar, version badge, session counts).
3. Migrate sessions table → `screens/dashboard/sessions.rs` as a `Component`. Apply:
   - New default column set (14 columns including new state dot column #1 and `ACTION` column #4)
   - Colored client names (no leading dot label — the actual state dot is the dot column)
   - Default sort: `AGE` asc
   - State dot rendering directly off `SessionState` via `widgets::state_style`; pulsation for `Waiting { .. }`
   - `ACTION` column rendering with truncation; rows in `Waiting { reason: Permission }` get `status.warning` color modifier on the action text
4. Add `ACTIVITY` column with per-session ring buffer of token-rate samples.
5. Build `screens/dashboard/quota.rs` (short/long/hidden), bind to `u`.
6. Build `screens/dashboard/info_drawer.rs` (bottom-right overlay, tabs Summary/General/Costs/Process; tab keys 1/2/3/4; Summary as default), bind to `i`.
   - Summary tab: hero block, status block, recent messages block (with per-client message-log adapter; graceful fallback for unsupported clients)
7. Mouse: click on tab bar, rows, column headers; scroll wheel; Shift+click bypass.
8. Snapshot tests: header, sessions table (each display state visible), quota each mode, info drawer (each tab), narrow-mode (80×24) snapshots.
9. Remove old dashboard rendering paths.

**Exit criteria:** Dashboard fully on new architecture. Aggregation/Config = placeholder screens. State dots and pulsation working. Summary tab populated for at least claude-code and codex; graceful fallback for others.

#### Phase 3 — Aggregation view

1. Build `screens/aggregation/{mod,controls,table,drilldown}.rs`.
2. Add aggregation helpers in `agtop-core` if not present: group by dimension, time-range filter, per-bucket activity series.
3. Wire `g/r/s/S/Enter/Esc//` keymap and click handlers.
4. Drill-down overlay reuses dashboard sessions component.
5. Snapshot tests for each (group, range) combination on a fixture dataset.

**Exit criteria:** `a` opens working Aggregation view; old Cost Summary tab removed.

#### Phase 4 — Config view

1. Build `screens/config/{mod,sidebar,detail}.rs` and one file per section under `sections/`.
2. Migrate column editor from old Config tab → `sections/columns.rs`.
3. Implement controls (dropdown, checkbox, radio, color swatch with picker, text input).
4. Search bar wires to a filter masking sidebar + detail.
5. Persist through existing config layer.
6. Snapshot tests per section default render.
7. Remove old `Config` tab and `UiMode::Classic`.

**Exit criteria:** `c` opens working Config view; all settings reachable; old code paths gone.

### Cross-cutting concerns

- **Keep app compiling at every commit.** Each phase ends in green-CI state.
- **No big-bang `App` rewrite.** Add new fields; remove old when last consumer is gone.
- **Snapshot tests anchor visual contracts.** Every new component lands with at least one `insta` snapshot.
- **Frame budget instrumentation:** debug-build timer per component render; log frames > 16ms.
- **Documentation:** `docs/architecture/ARCHITECTURE.md` committed Phase 1, updated each phase end.

### "Extract, don't rewrite" rule

The current `app/mod.rs` is 1849 lines. In Phase 2, when migrating the sessions table:
1. First commit: copy logic to `screens/dashboard/sessions.rs` unchanged, route to it.
2. Subsequent commits: refactor inside the new file, small steps.

This avoids the trap of mixing migration with rewrite and keeps reviews small.

### Risks

| Risk | Mitigation |
|------|------------|
| Phase 2 stalls on the 1849-line `app/mod.rs` | Strict "extract, don't rewrite" rule; small commits |
| Snapshot tests noisy with terminal-size variance | Lock `TestBackend` to (140, 40) for default snapshots; (80, 24) for narrow-mode tests |
| True-color detection differs across terminals | Explicit `auto/on/off` config; document gotchas in ARCHITECTURE.md |
| Mouse capture breaks tmux/SSH text selection | Default on, but document `Shift+click`; config toggle to disable |

---

## 7. Out of scope (explicit)

- Open-session-in-client integration (separate TODO).
- Rewrites of `agtop-core` parser logic. (The `SessionState` enum normalization in §6.1.1 is in-scope, but the parsers themselves are only updated to produce the new variants — no parser rewrites.)
- Theme editor UI / multiple themes shipped.
- Customizable keybindings.
- Per-section config import/export.
- Mouse-driven panel resizing.
- Removal of the `Unknown` state semantics across all consumers. The state enum no longer has `Unknown`, but a follow-up may be needed to tighten any remaining `Option<SessionState>` call sites.

---

## 8. Acceptance criteria

The redesign is complete when:

- [x] `d/a/c` keys switch between three full-screen views; `q` quits; `?` shows help overlay.
- [x] Dashboard header shows CPU sparkline, memory bar, session counts, version badge, all using theme tokens.
- [x] Sessions table defaults to the 14-column set in the documented order (state dot column first), sorted `AGE asc`. Client names rendered in client-theme colors.
- [ ] **`agtop-core::SessionState` is the canonical 6-variant enum** (`Running / Waiting{reason} / Warning{reason} / Error{reason} / Idle / Closed`) with Reason sub-enums per §6.1.1. All 7 client parsers produce the new variants. Serde fixtures updated.
  - MISSING: All 7 parsers output `session_state: None`; `state_resolution::resolve_state` derives the variant at runtime but parsers never assign a concrete `SessionState`. The AC requires parsers to produce the variants directly.
- [x] State dot column renders correct color directly from `SessionState` via `widgets::state_style` (yellow `Running`, violet pulsating `Waiting`, orange `Warning`, red `Error`, green `Idle`, empty for `Closed`). No `DisplayState` enum exists. Closed rows render in `fg.muted` (dim).
- [x] `ACTION` column displays current tool/action for active sessions; `—` when `Idle` / `Closed` / `Warning` / `Error`; rows in `Waiting{reason: Permission}` get `status.warning` styling on the action text.
- [ ] Stalled threshold (`refresh.stalled_threshold_secs`) is configurable; default 300 (5 minutes). Threshold is consumed inside core `state_resolution` to construct `Warning{reason: Stalled{since}}`.
  - MISSING: Config field and core API exist (`resolve_state_with_threshold`), but runtime wiring from persisted config value to the state resolution call is not present in the v2 app layer.
- [ ] `appearance.animations` flag controls `Waiting{..}`-state pulsation; auto-disabled when frame budget exceeded; disabled when `NO_COLOR` set.
  - MISSING: `animations_enabled` flag controls pulsation and `NO_COLOR` disables colour depth, but auto-disable on frame budget exceeded is not implemented, and `NO_COLOR` does not explicitly set `animations_enabled = false`.
- [x] `i` toggles a bottom-right Info drawer with **Summary / General / Costs / Process** tabs (tab keys `1/2/3/4`); Summary is default; tabs track the selected row.
- [x] Summary tab renders hero block, status block, and recent messages block; gracefully shows fallback for clients without message-log support.
- [x] `u` cycles Usage Quota panel through short / long / hidden.
- [x] Aggregation view supports `Client/Provider/Model/Project/Subscription` group-by and `Today/Week/Month/All` ranges, with totals row and `Enter` drill-down.
- [x] Config view is a full-page sidebar+detail layout with 7 sections, search bar, immediate persistence.
- [x] VS Code Dark+ palette applied throughout via semantic tokens. True-color config setting works in `auto/on/off`.
- [x] No emojis anywhere in the UI. The three-tier visual system (T0 universal Unicode, T1 bitmap logos, T2 Nerd Font icons) is implemented per the catalog in §5.
- [x] All T2 (Nerd Font) sites have a clean fallback when `appearance.nerd_font=false`. Disabling produces a complete, intentional UI.
- [x] The `widgets::icon` module is the single source of truth — no raw codepoints in widget code.
- [ ] All snapshot tests run in both `nerd_font=false` and `nerd_font=true` flavors.
  - MISSING: Only the Appearance config section has both `nf_off` and `nf_on` snapshot flavors. Sessions table, aggregation, header, quota, tab-bar snapshots are single-flavor only.
- [ ] Mouse interactions work as specified; Shift+click bypasses capture.
  - MISSING: Shift+click bypass is not implemented; sessions table mouse handler does not check for SHIFT modifier on mouse-down events.
- [ ] Snapshot tests cover all new components at (140, 40) and key narrow-mode breakpoints (80×24, 100×30).
  - MISSING: Tests use 140×12, 140×20, 140×30, 140×3, 80×3 — no 140×40, 80×24, or 100×30 snapshots exist.
- [x] `docs/architecture/ARCHITECTURE.md` documents the component model, message flow, theme tokens, state model rationale, and how to add a new panel/screen.
- [x] `SessionAnalysis::current_action: Option<String>` field added; populated by claude-code parser at minimum.
- [x] All existing functionality preserved (no regressions in client support, refresh, column config persistence).

---

## 9. Implementation plans

This design will be split into four sequenced implementation plans, written via the `writing-plans` skill once this design is approved:

1. **Plan 1 — Foundation** (Phase 1)
2. **Plan 2 — Dashboard redesign** (Phase 2)
3. **Plan 3 — Aggregation view** (Phase 3)
4. **Plan 4 — Config view** (Phase 4)

Each plan stands alone and can be merged independently.
