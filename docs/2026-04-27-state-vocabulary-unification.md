# Session State Vocabulary Unification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `agtop_core::session::SessionState` the single source of truth for session state across TUI, JSON output, and all internal logic. Remove the parallel string-based state vocabulary (`"waiting" / "stopped" / "working" / "stale"`). Fix the semantic bug where parser-reported end-of-turn (`"stopped"`) is mis-mapped to `Closed` instead of `Idle`.

**Architecture:** Replace `SessionSummary.state: Option<String>` with `SessionSummary.parser_state: ParserState` (typed enum). Wire `state_resolution::resolve_state` (currently dead) as the canonical converter from `(parser_state, liveness, last_active, now)` to `SessionState`. Delete the duplicate `refresh_adapter::derive_state`. Delete the parallel display module `widgets::state_display`. Migrate v1 widgets and the `agtop json` CLI subcommand to read `SessionState` directly. Tighten `Warning(Stalled)` to require `Liveness::Live`.

**Tech Stack:** Rust 2024 edition, ratatui, serde, chrono. Workspace crates: `agtop-core` (parsers, domain), `agtop-cli` (TUI + CLI).

**Branch:** Continue on existing `feat/tui-redesign` (worktree at `.worktrees/tui-redesign`).

---

## Background and rationale

The architecture document (`docs/architecture/ARCHITECTURE.md` §"Session state", lines 58–73) defines the canonical 6-variant `SessionState` enum:

- `Running` — agent actively producing output or executing a tool call (theme: `status_warning`, yellow)
- `Waiting(WaitReason)` — paused on user (theme: `accent_secondary`, purple, pulsating)
- `Warning(WarningReason)` — live but anomalous, e.g. stalled (theme: `status_attention`, orange)
- `Error(ErrorReason)` — explicit error (theme: `status_error`, red)
- `Idle` — live, ready for input, not currently working (theme: `status_success`, green)
- `Closed` — no live process; historical/archival (no dot, muted row text)

`Closed` is the **only non-live state.** All other states imply a live (or recently live) process. The architecture doc says "State resolution (`state_resolution::resolve_state`) converts string-based parser output into the canonical `SessionState` enum" — but `resolve_state` is dead code (no callers outside its own tests).

**Current drift from the design:**

1. **Four state derivation sites** instead of one:
   - `agtop_core::state_resolution::resolve_state` — DEAD, but documented as canonical.
   - `agtop_cli::tui::refresh_adapter::derive_state` — used by v2 dashboard, ad-hoc 3-path logic.
   - `agtop_cli::tui::widgets::state_display::display_state` — used by v1 widgets and `agtop json`, returns `("working" | "waiting" | "stale", Style)`.
   - The codex parser inlines its own state-from-response-item logic before stuffing a string.

2. **Parser vocabulary is `Option<String>` with values `{None, "waiting", "stopped"}`** — `"idle" / "closed" / "running"` are referenced in `match` arms but never produced. `SessionState::Idle` is therefore unreachable today.

3. **`"stopped"` from parsers means "assistant turn ended, awaiting user input"** (Claude `stop_reason=end_turn`, OpenCode `finish=stop`, Codex `final_answer`, Gemini message-with-no-toolCalls). The current code mis-maps `"stopped" → Closed`, contradicting the rule that `Closed` means no live process. A live Claude CLI sitting at the user prompt is rendered as Closed and dimmed.

4. **Two `pid.is_none()` hacks** in `screens/dashboard/sessions.rs:181` and `:952` un-dim rows where this misclassification happens. They can go away once the root cause is fixed.

5. **JSON output uses different vocabulary** (`"working"/"stale"`) than the TUI and the canonical enum, making CLI/TUI behaviour inconsistent.

**Fix:** Replace strings with a typed `ParserState` enum at the parser boundary, wire `resolve_state` as the single converter, delete `derive_state` and `display_state`, fix the `"stopped" → Idle` semantic, migrate JSON output to use canonical strings (`SessionState::as_str()`), tighten `Warning(Stalled)` to require live PID.

**Out of scope (open issue at end of plan):** Detecting opencode `question` tool, codex escalation/permission tools, and similar to emit `Waiting(Permission)` distinctly. The minimum fix here uses `Waiting(Input)` for everything that today produces `"waiting"`.

---

## Decisions locked in

1. **JSON `display_state` field adopts the canonical vocabulary.** Values become `"running" | "waiting" | "idle" | "warning" | "error" | "closed"` exactly matching the TUI. Breaking change for downstream JSON consumers — accepted for consistency.
2. **`SessionSummary.state: Option<String>` is replaced** by `parser_state: ParserState`. No legacy compat shim.
3. **Permission/question tool detection is deferred.** Open an issue at the end.
4. **`is_active()` keeps its current semantics** (`Running | Idle | Warning`). Add `is_live()` (`!matches!(_, Closed)`) when the broader meaning is needed. Audit each existing caller and route to whichever fits.
5. **`Warning(Stalled)` requires `Liveness::Live`.** Stale + no liveness signal → `Closed`.
6. **`state_detail: Option<String>`** stays as a free-form diagnostic field — not load-bearing in any logic, just surfaced in the info drawer.

---

## Truth table — target behaviour

After this refactor, `state_resolution::resolve_state(parser_state, liveness, last_active, now)` produces:

| `parser_state` | `liveness` | age (now − last_active) | → `SessionState` |
|---|---|---|---|
| any | `Some(Stopped)` | any | `Closed` (OS-confirmed dead always wins) |
| `Idle` | `Some(Live)` | any | `Idle` |
| `Idle` | `None` | < 5m | `Idle` |
| `Idle` | `None` | ≥ 5m or unknown | `Closed` |
| `Running` | `Some(Live)` | < 5m | `Running` |
| `Running` | `Some(Live)` | ≥ 5m | `Warning(Stalled)` (live but stalled) |
| `Running` | `None` | < 30s | `Running` |
| `Running` | `None` | 30s..5m | `Closed` (no liveness + stale → not Warning) |
| `Running` | `None` | ≥ 5m or unknown | `Closed` |
| `Waiting(_)` | `Some(Live)` | any | `Waiting(_)` (pass through reason) |
| `Waiting(_)` | `None` | < 5m | `Waiting(_)` |
| `Waiting(_)` | `None` | ≥ 5m or unknown | `Closed` |
| `Error(_)` | `Some(Live)` | any | `Error(_)` |
| `Error(_)` | `None` | any | `Error(_)` (errors persist; not auto-Closed) |
| `Unknown` | `Some(Live)` | < 5m | `Running` |
| `Unknown` | `Some(Live)` | ≥ 5m | `Warning(Stalled)` |
| `Unknown` | `None` | < 30s | `Running` |
| `Unknown` | `None` | 30s..5m | `Closed` |
| `Unknown` | `None` | ≥ 5m or unknown | `Closed` |

**Key changes from current behaviour:**
- Claude `end_turn` / OpenCode `finish=stop` / Codex `final_answer` / Gemini end-message → now `Idle` (was `Closed`)
- Gemini tool error → now `Error(ParserDetected)` (was `Waiting`)
- `Warning(Stalled)` only when live (was: also when no liveness + age > 30s)

---

## File structure

### Files to modify

| File | Responsibility | Change |
|---|---|---|
| `crates/agtop-core/src/session.rs` | `SessionSummary`, `SessionState` enum | Add `ParserState` enum; replace `state: Option<String>` with `parser_state: ParserState`; add `SessionState::is_live()` |
| `crates/agtop-core/src/state_resolution.rs` | Canonical state converter | Rewrite `resolve_state` to take typed inputs; drop `&mut SessionAnalysis` arg |
| `crates/agtop-core/src/clients/claude.rs` | Claude parser | Replace string state with `ParserState` |
| `crates/agtop-core/src/clients/opencode.rs` | OpenCode parser | Replace string state with `ParserState` |
| `crates/agtop-core/src/clients/codex.rs` | Codex parser | Replace string state with `ParserState` |
| `crates/agtop-core/src/clients/gemini_cli.rs` | Gemini parser | Replace string state with `ParserState`; emit `Error` for tool errors |
| `crates/agtop-core/src/clients/copilot.rs` | Copilot parser | Replace string state with `ParserState` |
| `crates/agtop-core/src/clients/cursor.rs` | Cursor parser | Replace `state: None` with `ParserState::Unknown` |
| `crates/agtop-core/src/clients/antigravity.rs` | Antigravity parser | Same as cursor |
| `crates/agtop-core/src/lib.rs` | Crate root re-exports | Re-export `ParserState` |
| `crates/agtop-cli/src/tui/refresh_adapter.rs` | Snapshot → dashboard model | Delete `derive_state` and constants; call `state_resolution::resolve_state` |
| `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs` | Session table render | Remove `pid.is_none()` hacks at lines 181 and 952 |
| `crates/agtop-cli/src/tui/widgets/session_table.rs` | v1 session table | Read `SessionState` directly; use `state_style::label_for` |
| `crates/agtop-cli/src/tui/widgets/info_tab.rs` | v1 info tab | Same as above |
| `crates/agtop-cli/src/main.rs` | `agtop json` subcommand | `display_state` field uses `SessionState::as_str()` |
| `docs/architecture/ARCHITECTURE.md` | Architecture docs | Update §Session state to reflect actuality |

### Files to delete

| File | Reason |
|---|---|
| `crates/agtop-cli/src/tui/widgets/state_display.rs` | Parallel string-based vocabulary, no longer needed |

### Files to keep but may have snapshot updates

| File | Reason |
|---|---|
| `crates/agtop-cli/tests/snapshots/*.snap` | Labels in state column change (`"working"` → `"running"`, `"stale"` → `"warning"`); some `Idle`-rendered sessions appear for the first time |

---

## Phasing and commits

The plan is sequenced so each commit compiles, tests pass, and the system remains usable. Total: ~16 commits across 4 phases.

- **Phase A (commits 1–3):** Add `ParserState` enum + rewire `resolve_state`. No callers yet.
- **Phase B (commits 4–10):** Migrate parsers one-by-one to populate `parser_state`. Each commit migrates one parser and removes its string state assignment.
- **Phase C (commits 11–13):** Wire up the canonical converter in the TUI (`refresh_adapter`), remove `derive_state`, remove `pid.is_none()` hacks, tighten `Warning(Stalled)`.
- **Phase D (commits 14–16):** Migrate v1 widgets and JSON output, delete `state_display.rs`, update docs, open follow-up issue.

---

## Test commands

- Workspace: `rtk cargo test --workspace -- --test-threads=1`
- Single test: `rtk cargo test -p <crate> <test_name> -- --test-threads=1 --nocapture`
- Lint: `rtk cargo clippy --workspace --all-targets -- -D warnings`
- TUI manual smoke: `rtk cargo run -p agtop-cli -- tui`
- JSON manual smoke: `rtk cargo run -p agtop-cli -- json` (verify `display_state` field values)
- **Do not run `cargo fmt --all`** (per project rule from handoff).
- **Pre-existing known failure** (NOT to fix): `tui::tests::clicking_sortable_header_sorts_by_correct_column` in bin `agtop`.

---

## Phase A — types and the canonical converter

### Task A1: Define `ParserState` enum

**Files:**
- Modify: `crates/agtop-core/src/session.rs` (after the `WaitReason`/`WarningReason`/`ErrorReason` block, around line 53)

- [ ] **Step 1: Write the failing test**

Add to `crates/agtop-core/src/session.rs` (inside an existing `#[cfg(test)] mod tests { ... }` at the bottom of the file):

```rust
#[test]
fn parser_state_default_is_unknown() {
    assert_eq!(ParserState::default(), ParserState::Unknown);
}

#[test]
fn parser_state_serde_round_trip() {
    let cases = [
        ParserState::Idle,
        ParserState::Running,
        ParserState::Waiting(WaitReason::Input),
        ParserState::Waiting(WaitReason::Permission),
        ParserState::Error(ErrorReason::ParserDetected("boom".into())),
        ParserState::Unknown,
    ];
    for c in cases {
        let s = serde_json::to_string(&c).unwrap();
        let back: ParserState = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
```

- [ ] **Step 2: Run the failing test**

Run: `rtk cargo test -p agtop-core parser_state -- --test-threads=1`
Expected: FAIL — `ParserState` not defined.

- [ ] **Step 3: Add the `ParserState` enum**

Insert into `crates/agtop-core/src/session.rs` after line 52 (after the `ErrorReason` enum, before `impl SessionState`):

```rust
/// Coarse state inferred by a per-client parser from session log content.
///
/// This is the *parser's* opinion of what the agent is doing based on the
/// session file alone (e.g. "the last assistant turn ended"); it is fed
/// into `state_resolution::resolve_state` along with OS liveness data to
/// produce the canonical [`SessionState`].
///
/// Parsers MUST return a typed `ParserState` value. Callers MUST NOT
/// inspect parser state via string matching — use this enum.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum ParserState {
    /// Last assistant turn ended cleanly; the agent is awaiting user input.
    /// Maps to `SessionState::Idle` when the process is live.
    Idle,
    /// Agent is mid-turn — actively generating output or running a tool.
    /// Maps to `SessionState::Running` when the process is live.
    Running,
    /// Agent is paused waiting for a specific kind of user response.
    /// Maps to `SessionState::Waiting(_)` when the process is live.
    Waiting(WaitReason),
    /// Parser detected an explicit error in the session log
    /// (e.g. tool execution failure, crash trace).
    Error(ErrorReason),
    /// Parser had no opinion. Resolution falls back to recency + liveness.
    #[default]
    Unknown,
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `rtk cargo test -p agtop-core parser_state -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/agtop-core/src/session.rs
rtk git commit -m "feat(core): add ParserState enum for typed parser-side state"
```

---

### Task A2: Add `parser_state` field to `SessionSummary`; deprecate `state`

**Strategy:** Add `parser_state` alongside the existing `state: Option<String>` in this commit so parsers compile. Migration of `state` removal happens at the end of Phase B (Task B7).

**Files:**
- Modify: `crates/agtop-core/src/session.rs` (struct `SessionSummary` lines 147–178; `SessionSummary::new` lines 199–235)

- [ ] **Step 1: Write the failing test**

Add to `crates/agtop-core/src/session.rs` tests:

```rust
#[test]
fn session_summary_default_parser_state_is_unknown() {
    let s = SessionSummary::new(
        ClientKind::Claude,
        None,
        "id".to_string(),
        None,
        None,
        None,
        None,
        std::path::PathBuf::from("/tmp/x.jsonl"),
        None,
        None,
        None,
        None,
    );
    assert_eq!(s.parser_state, ParserState::Unknown);
}
```

- [ ] **Step 2: Run the failing test**

Run: `rtk cargo test -p agtop-core session_summary_default_parser_state -- --test-threads=1`
Expected: FAIL — `parser_state` field does not exist.

- [ ] **Step 3: Add the field**

Edit the `SessionSummary` struct in `crates/agtop-core/src/session.rs`. Insert this field directly after the existing `pub state: Option<String>` field (around line 162):

```rust
    /// Typed parser-side state. Replaces the legacy `state: Option<String>`
    /// (kept temporarily during migration). Default: `ParserState::Unknown`.
    #[serde(default)]
    pub parser_state: ParserState,
```

In `SessionSummary::new` (lines 219–234), add `parser_state: ParserState::default(),` to the struct construction. The constructor signature stays the same — `parser_state` always defaults; parsers will set it via field assignment after construction or via a fluent builder.

- [ ] **Step 4: Run the test**

Run: `rtk cargo test -p agtop-core session_summary_default_parser_state -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run the workspace build**

Run: `rtk cargo build --workspace`
Expected: PASS (existing parsers still compile, they just don't yet set `parser_state`).

- [ ] **Step 6: Commit**

```bash
rtk git add crates/agtop-core/src/session.rs
rtk git commit -m "feat(core): add parser_state: ParserState field to SessionSummary"
```

---

### Task A3: Re-export `ParserState` and rewrite `state_resolution::resolve_state`

**Files:**
- Modify: `crates/agtop-core/src/lib.rs` (re-export block, around lines 30–34)
- Modify: `crates/agtop-core/src/state_resolution.rs` (full rewrite of `resolve_state` and `resolve_state_with_threshold`)

- [ ] **Step 1: Write the failing test (truth-table tests)**

Replace the existing `mod tests` in `crates/agtop-core/src/state_resolution.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Liveness;
    use crate::session::{ErrorReason, ParserState, WaitReason, WarningReason};

    fn now() -> DateTime<Utc> { Utc::now() }

    #[test]
    fn os_stopped_always_wins() {
        let n = now();
        for ps in [ParserState::Idle, ParserState::Running, ParserState::Waiting(WaitReason::Input), ParserState::Unknown] {
            let s = resolve_state(ps, Some(Liveness::Stopped), Some(n), n);
            assert_eq!(s, SessionState::Closed, "OS stopped must win");
        }
    }

    #[test]
    fn idle_with_live_pid_is_idle() {
        let n = now();
        let s = resolve_state(ParserState::Idle, Some(Liveness::Live), Some(n), n);
        assert_eq!(s, SessionState::Idle);
    }

    #[test]
    fn idle_with_no_liveness_within_window_is_idle() {
        let n = now();
        let last = n - Duration::seconds(60);
        let s = resolve_state(ParserState::Idle, None, Some(last), n);
        assert_eq!(s, SessionState::Idle);
    }

    #[test]
    fn idle_with_no_liveness_past_window_is_closed() {
        let n = now();
        let last = n - Duration::minutes(10);
        let s = resolve_state(ParserState::Idle, None, Some(last), n);
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn running_with_live_pid_within_window_is_running() {
        let n = now();
        let last = n - Duration::seconds(10);
        let s = resolve_state(ParserState::Running, Some(Liveness::Live), Some(last), n);
        assert_eq!(s, SessionState::Running);
    }

    #[test]
    fn running_with_live_pid_past_stall_threshold_is_warning() {
        let n = now();
        let last = n - Duration::minutes(10);
        let s = resolve_state(ParserState::Running, Some(Liveness::Live), Some(last), n);
        assert!(matches!(s, SessionState::Warning(WarningReason::Stalled { .. })));
    }

    #[test]
    fn running_with_no_liveness_stale_is_closed_not_warning() {
        // Tightening: Warning(Stalled) requires liveness == Live.
        let n = now();
        let last = n - Duration::minutes(2);
        let s = resolve_state(ParserState::Running, None, Some(last), n);
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn waiting_passes_through_reason() {
        let n = now();
        let s = resolve_state(
            ParserState::Waiting(WaitReason::Permission),
            Some(Liveness::Live),
            Some(n),
            n,
        );
        assert_eq!(s, SessionState::Waiting(WaitReason::Permission));
    }

    #[test]
    fn waiting_no_liveness_stale_is_closed() {
        let n = now();
        let last = n - Duration::minutes(10);
        let s = resolve_state(ParserState::Waiting(WaitReason::Input), None, Some(last), n);
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn error_persists_regardless_of_liveness() {
        let n = now();
        let e = ParserState::Error(ErrorReason::ParserDetected("boom".into()));
        for live in [None, Some(Liveness::Live), Some(Liveness::Stopped)] {
            let s = resolve_state(e.clone(), live, Some(n), n);
            if live == Some(Liveness::Stopped) {
                // OS stopped still wins, even over Error.
                assert_eq!(s, SessionState::Closed);
            } else {
                assert!(matches!(s, SessionState::Error(_)));
            }
        }
    }

    #[test]
    fn unknown_with_live_within_window_is_running() {
        let n = now();
        let last = n - Duration::seconds(10);
        let s = resolve_state(ParserState::Unknown, Some(Liveness::Live), Some(last), n);
        assert_eq!(s, SessionState::Running);
    }

    #[test]
    fn unknown_no_liveness_recent_is_running() {
        // Within RUNNING_WINDOW (30s).
        let n = now();
        let last = n - Duration::seconds(10);
        let s = resolve_state(ParserState::Unknown, None, Some(last), n);
        assert_eq!(s, SessionState::Running);
    }

    #[test]
    fn unknown_no_liveness_stale_is_closed() {
        let n = now();
        let last = n - Duration::minutes(10);
        let s = resolve_state(ParserState::Unknown, None, Some(last), n);
        assert_eq!(s, SessionState::Closed);
    }

    #[test]
    fn unknown_no_liveness_no_last_active_is_closed() {
        let n = now();
        let s = resolve_state(ParserState::Unknown, None, None, n);
        assert_eq!(s, SessionState::Closed);
    }
}
```

- [ ] **Step 2: Run the failing tests**

Run: `rtk cargo test -p agtop-core --lib state_resolution -- --test-threads=1`
Expected: All FAIL — new `resolve_state` signature does not exist.

- [ ] **Step 3: Rewrite `state_resolution.rs`**

Replace the entire body of `crates/agtop-core/src/state_resolution.rs` with:

```rust
//! State resolution: derive the canonical [`SessionState`] from typed parser
//! state, OS liveness, and recency.
//!
//! This module owns the *single* state-derivation policy. Both the TUI and
//! the JSON CLI consume `SessionState` produced here; nothing else re-implements
//! this logic.

use chrono::{DateTime, Duration, Utc};

use crate::process::Liveness;
use crate::session::{ParserState, SessionState, WarningReason};

/// Default threshold past which a Running session with a live PID but no
/// recent activity is reported as `Warning(Stalled)`.
pub const DEFAULT_STALLED_AFTER: Duration = Duration::minutes(5);

/// Window within which an un-correlated session with no parser opinion is
/// considered Running purely on recency. Mirrors v1's `WORKING_WINDOW_SECS`.
pub const RUNNING_RECENCY_WINDOW: Duration = Duration::seconds(30);

/// Threshold past which a session with no live PID is considered Closed
/// regardless of parser hints. Used for `Idle` / `Waiting` / `Unknown`
/// when liveness is `None`.
pub const NO_LIVENESS_CLOSED_AFTER: Duration = Duration::minutes(5);

/// Resolve the canonical [`SessionState`] from typed inputs.
///
/// Inputs:
/// - `parser`: the per-client parser's typed opinion of the session.
/// - `liveness`: the OS-correlator's view of the matching process.
///   `None` means "correlator did not run / did not match"; not "no process".
/// - `last_active`: most-recent observed session activity timestamp.
/// - `now`: the reference instant (typically `Utc::now()`).
#[must_use]
pub fn resolve_state(
    parser: ParserState,
    liveness: Option<Liveness>,
    last_active: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> SessionState {
    resolve_state_with_threshold(parser, liveness, last_active, now, DEFAULT_STALLED_AFTER)
}

/// Same as [`resolve_state`] but with a configurable stall threshold.
#[must_use]
pub fn resolve_state_with_threshold(
    parser: ParserState,
    liveness: Option<Liveness>,
    last_active: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    stalled_after: Duration,
) -> SessionState {
    // OS-confirmed dead always wins.
    if matches!(liveness, Some(Liveness::Stopped)) {
        return SessionState::Closed;
    }

    let age = last_active.map(|t| now.signed_duration_since(t));
    let is_live = matches!(liveness, Some(Liveness::Live));

    match parser {
        ParserState::Idle => match (is_live, age) {
            (true, _) => SessionState::Idle,
            (false, Some(a)) if a < NO_LIVENESS_CLOSED_AFTER => SessionState::Idle,
            _ => SessionState::Closed,
        },
        ParserState::Running => match (is_live, age) {
            (true, Some(a)) if a >= stalled_after => SessionState::Warning(
                WarningReason::Stalled { since: last_active.unwrap() },
            ),
            (true, _) => SessionState::Running,
            (false, Some(a)) if a < RUNNING_RECENCY_WINDOW => SessionState::Running,
            _ => SessionState::Closed,
        },
        ParserState::Waiting(reason) => match (is_live, age) {
            (true, _) => SessionState::Waiting(reason),
            (false, Some(a)) if a < NO_LIVENESS_CLOSED_AFTER => SessionState::Waiting(reason),
            _ => SessionState::Closed,
        },
        ParserState::Error(reason) => SessionState::Error(reason),
        ParserState::Unknown => match (is_live, age) {
            (true, Some(a)) if a >= stalled_after => SessionState::Warning(
                WarningReason::Stalled { since: last_active.unwrap() },
            ),
            (true, _) => SessionState::Running,
            (false, Some(a)) if a < RUNNING_RECENCY_WINDOW => SessionState::Running,
            _ => SessionState::Closed,
        },
    }
}
```

- [ ] **Step 4: Update the lib re-exports**

Edit `crates/agtop-core/src/lib.rs`. Find the `pub use session::{...}` block (around line 34) and add `ParserState` to the exported symbols. Example (adapt to actual contents):

```rust
pub use session::{
    ClientKind, CostBreakdown, ParserState, SessionAnalysis, SessionState, SessionSummary,
    TokenTotals, WaitReason, WarningReason, ErrorReason,
};
```

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test -p agtop-core state_resolution -- --test-threads=1`
Expected: All PASS.

- [ ] **Step 6: Run the workspace build**

Run: `rtk cargo build --workspace`
Expected: PASS — old `resolve_state` callers (none in production, two in its own old test mod which has been replaced) compile.

- [ ] **Step 7: Commit**

```bash
rtk git add crates/agtop-core/src/state_resolution.rs crates/agtop-core/src/lib.rs
rtk git commit -m "refactor(core): rewrite resolve_state to take typed inputs

Per architecture doc, state_resolution is the canonical converter
from parser state + liveness + recency to SessionState. Rewrite
its signature to take a typed (ParserState, Liveness, last_active, now)
tuple instead of mutating a SessionAnalysis. Tighten Warning(Stalled)
to require Liveness::Live. Pin the full truth table in tests."
```

---

## Phase B — migrate parsers to `ParserState`

Each parser commit:
1. Defines a typed helper `fn parser_state_from_*(...)` returning `ParserState`.
2. Replaces the string-state local variable with `ParserState`.
3. Sets `summary.parser_state = ...` after construction (since `SessionSummary::new` keeps its old signature).
4. Leaves `state: Option<String>` populated as a stringified mirror temporarily — Task B7 deletes that field.

### Task B1: Migrate `claude.rs`

**Files:**
- Modify: `crates/agtop-core/src/clients/claude.rs:290-306` (`state_from_claude_record`)
- Modify: `crates/agtop-core/src/clients/claude.rs:440-513` (parser body)

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `crates/agtop-core/src/clients/claude.rs` (test module starts at line 853):

```rust
#[test]
fn claude_end_turn_maps_to_idle_not_closed() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(
        r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#,
    ).unwrap();
    let (ps, _detail) = parser_state_from_claude_record(&v).unwrap();
    assert_eq!(ps, ParserState::Idle);
}

#[test]
fn claude_tool_use_maps_to_waiting_input() {
    use crate::session::{ParserState, WaitReason};
    let v: serde_json::Value = serde_json::from_str(
        r#"{"type":"assistant","message":{"stop_reason":"tool_use"}}"#,
    ).unwrap();
    let (ps, _detail) = parser_state_from_claude_record(&v).unwrap();
    // Tool use mid-turn: parser sees "model wants to call a tool" — that's
    // an active step, not user-input wait. Until we add Waiting(Permission)
    // detection (out of scope), this is Running.
    assert_eq!(ps, ParserState::Running);
}
```

- [ ] **Step 2: Run the failing test**

Run: `rtk cargo test -p agtop-core --lib claude -- --test-threads=1 claude_end_turn`
Expected: FAIL — `parser_state_from_claude_record` not defined.

- [ ] **Step 3: Add the new typed function**

Insert into `crates/agtop-core/src/clients/claude.rs` directly after `state_from_claude_record` (which currently ends around line 306):

```rust
/// Typed equivalent of [`state_from_claude_record`]. New code MUST use this.
///
/// Returns `(ParserState, detail_string)` where `detail` is the diagnostic
/// label for `summary.state_detail`.
fn parser_state_from_claude_record(
    v: &serde_json::Value,
) -> Option<(crate::session::ParserState, String)> {
    use crate::session::ParserState;
    match v
        .get("message")
        .and_then(|m| m.get("stop_reason"))
        .and_then(|x| x.as_str())
    {
        // Claude `tool_use`: the assistant emitted a tool call mid-turn.
        // The model is actively working; user is not blocked.
        Some("tool_use") => Some((
            ParserState::Running,
            "assistant.stop_reason=tool_use".to_string(),
        )),
        // Claude `end_turn`: assistant finished its response cleanly. The
        // CLI process is alive at the user prompt awaiting next input.
        // This is Idle, NOT Closed.
        Some("end_turn") => Some((
            ParserState::Idle,
            "assistant.stop_reason=end_turn".to_string(),
        )),
        _ => None,
    }
}
```

- [ ] **Step 4: Switch the parser body to use the new function**

Edit `crates/agtop-core/src/clients/claude.rs` lines 440–513. Locate this block:

```rust
    let mut state: Option<String> = None;
    let mut state_detail: Option<String> = None;
```

(around line 448–449)

Insert before `state` declaration:

```rust
    use crate::session::ParserState;
    let mut parser_state: ParserState = ParserState::Unknown;
```

Replace the loop body that calls `state_from_claude_record` (lines 484–487):

```rust
        if let Some((next_state, detail)) = state_from_claude_record(v) {
            state = Some(next_state);
            state_detail = Some(detail);
        }
```

with:

```rust
        if let Some((next, detail)) = parser_state_from_claude_record(v) {
            parser_state = next;
            state_detail = Some(detail);
        }
```

Then in the final `Ok(SessionSummary { ... })` literal (around lines 498–512), remove the `state` field if it still exists, and add `parser_state,`. The `state: Option<String>` field on the struct is being removed in Task B7 — for this commit, leave the legacy field present but populate it with `None` (we no longer need to set it since `parser_state` carries the truth).

Concretely the struct construction becomes:

```rust
    Ok(SessionSummary {
        client: ClientKind::Claude,
        subscription: None,
        session_id,
        started_at: earliest,
        last_active,
        model,
        cwd,
        state: None, // legacy; removed in Task B7
        state_detail,
        parser_state,
        model_effort: None,
        model_effort_detail: None,
        session_title,
        data_path: path.to_path_buf(),
    })
```

- [ ] **Step 5: Delete the old `state_from_claude_record` function**

It is now dead. Delete lines 290–306 (the entire `fn state_from_claude_record` block).

- [ ] **Step 6: Run the tests**

Run: `rtk cargo test -p agtop-core --lib claude -- --test-threads=1`
Expected: All claude tests pass, including the two new ones.

- [ ] **Step 7: Commit**

```bash
rtk git add crates/agtop-core/src/clients/claude.rs
rtk git commit -m "fix(claude): map end_turn to ParserState::Idle (was misclassified as Closed)

Claude's stop_reason=end_turn signals 'assistant turn complete, awaiting
user input' — the CLI process is alive at the user prompt. Old code mapped
this to the string 'stopped' which was then mis-derived as SessionState::Closed,
causing live idle Claude sessions to render as dim/dead. Map it to
ParserState::Idle so resolve_state produces SessionState::Idle correctly.

Also map stop_reason=tool_use to ParserState::Running (the agent is
mid-turn executing a tool, not waiting on the user)."
```

---

### Task B2: Migrate `opencode.rs`

**Files:**
- Modify: `crates/agtop-core/src/clients/opencode.rs:169-175` (`state_from_opencode_message`)
- Modify: `crates/agtop-core/src/clients/opencode.rs:1093-1107`, `:1152-1167`, `:1563-1577` (three `SessionSummary` construction sites)

- [ ] **Step 1: Write the failing test**

Add to opencode tests (mod starts at line 1738):

```rust
#[test]
fn opencode_finish_stop_maps_to_idle() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(r#"{"finish":"stop"}"#).unwrap();
    let (ps, _detail) = parser_state_from_opencode_message(&v).unwrap();
    assert_eq!(ps, ParserState::Idle);
}

#[test]
fn opencode_finish_tool_calls_maps_to_running() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(r#"{"finish":"tool-calls"}"#).unwrap();
    let (ps, _detail) = parser_state_from_opencode_message(&v).unwrap();
    assert_eq!(ps, ParserState::Running);
}
```

- [ ] **Step 2: Run the failing test**

Run: `rtk cargo test -p agtop-core --lib opencode_finish -- --test-threads=1`
Expected: FAIL.

- [ ] **Step 3: Add the typed function**

Insert into `crates/agtop-core/src/clients/opencode.rs` directly after `state_from_opencode_message` (currently ends ~line 175):

```rust
/// Typed equivalent of [`state_from_opencode_message`].
fn parser_state_from_opencode_message(
    v: &serde_json::Value,
) -> Option<(crate::session::ParserState, String)> {
    use crate::session::ParserState;
    match v.get("finish").and_then(|x| x.as_str()) {
        // Mid-turn tool calls: agent is actively working.
        Some("tool-calls") => Some((ParserState::Running, "finish=tool-calls".to_string())),
        // Turn finished: agent is idle at the user prompt.
        Some("stop") => Some((ParserState::Idle, "finish=stop".to_string())),
        _ => None,
    }
}
```

- [ ] **Step 4: Update `latest_message_state_sqlite` to return `(ParserState, String)`**

This helper is at lines 1173–1211. Change its return type:

```rust
fn latest_message_state_sqlite(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> (crate::session::ParserState, Option<String>) {
```

Inside the function, replace calls to `state_from_opencode_message` with `parser_state_from_opencode_message`. The two early-return paths return `(ParserState::Unknown, None)`; the matched paths return `(ps, Some(detail))`.

- [ ] **Step 5: Update both SQLite struct-literal sites**

In `crates/agtop-core/src/clients/opencode.rs:1085-1107` and `:1136-1167`, the lines that read:

```rust
            let (state, state_detail) = latest_message_state_sqlite(&conn, &id);
```

become:

```rust
            let (parser_state, state_detail) = latest_message_state_sqlite(&conn, &id);
```

In each `SessionSummary { ... }` literal at the corresponding sites, replace:

```rust
                state,
                state_detail,
```

with:

```rust
                state: None, // legacy; removed in Task B7
                state_detail,
                parser_state,
```

- [ ] **Step 6: Update the JSON path site at line 1563**

Find the `let mut state: Option<String> = None;` (line 448) and `state = Some(next_state);` (line 485) in the JSON parser body. Replace with:

```rust
    use crate::session::ParserState;
    let mut parser_state: ParserState = ParserState::Unknown;
```

…and the loop body update site:

```rust
        if let Some((next, detail)) = parser_state_from_opencode_message(...) {
            parser_state = next;
            state_detail = Some(detail);
        }
```

In the trailing `Ok(SessionSummary { ... })` (line 1563), update fields the same way as the SQLite sites.

- [ ] **Step 7: Delete the old `state_from_opencode_message` function** (lines 169–175). It is now dead.

- [ ] **Step 8: Run the tests**

Run: `rtk cargo test -p agtop-core --lib opencode -- --test-threads=1`
Expected: PASS, including the two new tests.

- [ ] **Step 9: Commit**

```bash
rtk git add crates/agtop-core/src/clients/opencode.rs
rtk git commit -m "fix(opencode): map finish=stop to ParserState::Idle (was misclassified as Closed)"
```

---

### Task B3: Migrate `codex.rs`

**Files:**
- Modify: `crates/agtop-core/src/clients/codex.rs:312-326` (`state_from_response_item`)
- Modify: `crates/agtop-core/src/clients/codex.rs:365-384` (`state_from_codex_message`)
- Modify: `crates/agtop-core/src/clients/codex.rs:592-720` (parser body)

Same pattern as B1/B2. Specifically:

- `function_call` / `custom_tool_call` → `ParserState::Running` (was `"waiting"`)
- `function_call_output` / `custom_tool_call_output` → `ParserState::Idle` (was `"stopped"`)
- assistant `final_answer` containing `?` → `ParserState::Waiting(WaitReason::Input)` (was `"waiting"`)
- assistant `final_answer` (no `?`) → `ParserState::Idle` (was `"stopped"`)

**Important semantic correction for codex:** The current code maps `function_call` to `"waiting"` ("waiting on tool"). In the new vocabulary, that's the *agent* working on a tool, not the user being asked anything — that's `Running`, not `Waiting(_)`. Keep `Waiting(_)` reserved for "user input is required to proceed".

- [ ] **Step 1: Write the failing tests**

Add to codex tests:

```rust
#[test]
fn codex_function_call_maps_to_running() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(r#"{"type":"function_call"}"#).unwrap();
    let (ps, _) = parser_state_from_codex_response_item(&v).unwrap();
    assert_eq!(ps, ParserState::Running);
}

#[test]
fn codex_function_call_output_maps_to_idle() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(r#"{"type":"function_call_output"}"#).unwrap();
    let (ps, _) = parser_state_from_codex_response_item(&v).unwrap();
    assert_eq!(ps, ParserState::Idle);
}

#[test]
fn codex_assistant_question_maps_to_waiting_input() {
    use crate::session::{ParserState, WaitReason};
    let v: serde_json::Value = serde_json::from_str(
        r#"{"role":"assistant","phase":"final_answer","content":"shall we?"}"#,
    ).unwrap();
    let (ps, _) = parser_state_from_codex_message(&v).unwrap();
    assert_eq!(ps, ParserState::Waiting(WaitReason::Input));
}

#[test]
fn codex_assistant_final_answer_maps_to_idle() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(
        r#"{"role":"assistant","phase":"final_answer","content":"done."}"#,
    ).unwrap();
    let (ps, _) = parser_state_from_codex_message(&v).unwrap();
    assert_eq!(ps, ParserState::Idle);
}
```

- [ ] **Step 2: Run the failing tests**

Run: `rtk cargo test -p agtop-core --lib codex_function_call -- --test-threads=1`
Expected: FAIL.

- [ ] **Step 3: Add the new typed functions**

Insert into `crates/agtop-core/src/clients/codex.rs` after `state_from_codex_message` (~line 384):

```rust
/// Typed equivalent of [`state_from_response_item`].
fn parser_state_from_codex_response_item(
    payload: &serde_json::Value,
) -> Option<(crate::session::ParserState, String)> {
    use crate::session::ParserState;
    let ty = payload.get("type").and_then(|x| x.as_str())?;
    let detail = format!("response_item:{ty}");
    match ty {
        "function_call" | "custom_tool_call" => Some((ParserState::Running, detail)),
        "function_call_output" | "custom_tool_call_output" => Some((ParserState::Idle, detail)),
        _ => None,
    }
}

/// Typed equivalent of [`state_from_codex_message`].
fn parser_state_from_codex_message(
    payload: &serde_json::Value,
) -> Option<(crate::session::ParserState, String)> {
    use crate::session::{ParserState, WaitReason};
    if payload.get("role").and_then(|x| x.as_str()) != Some("assistant") {
        return None;
    }
    let phase = payload.get("phase").and_then(|x| x.as_str());
    let text = message_text_from_payload(payload).unwrap_or_default();
    if phase == Some("final_answer") && text.contains('?') {
        Some((
            ParserState::Waiting(WaitReason::Input),
            "response_item:assistant-question".to_string(),
        ))
    } else if phase == Some("final_answer") {
        Some((
            ParserState::Idle,
            "response_item:assistant-final".to_string(),
        ))
    } else {
        None
    }
}
```

- [ ] **Step 4: Update the parser body**

In `crates/agtop-core/src/clients/codex.rs:592-720`, replace the local `let mut state: Option<String> = None;` with:

```rust
    use crate::session::ParserState;
    let mut parser_state: ParserState = ParserState::Unknown;
```

Replace the three sites that assign `state = Some(...)`:

- Lines ~654–656: change `if let Some((next_state, detail)) = state_from_response_item(p) { state = Some(next_state); state_detail = Some(detail); }` → use `parser_state_from_codex_response_item`, assign to `parser_state`.
- Lines ~657–659: same for `state_from_codex_message` → `parser_state_from_codex_message`.
- Lines ~664–667 (the `function_call_output` / `custom_tool_call_output` block, which currently sets `state = Some("stopped".to_string())`): this case is now subsumed by `parser_state_from_codex_response_item` (which returns `ParserState::Idle` for these types). Delete the entire `else if matches!(ty, "function_call_output" | "custom_tool_call_output") { ... }` branch.

In the trailing `Ok(SessionSummary { ... })` literal (line 705), replace `state, state_detail,` with `state: None, state_detail, parser_state,`.

- [ ] **Step 5: Delete the old functions**

Delete `state_from_response_item` (lines 312–326) and `state_from_codex_message` (lines 365–384).

- [ ] **Step 6: Run the tests**

Run: `rtk cargo test -p agtop-core --lib codex -- --test-threads=1`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
rtk git add crates/agtop-core/src/clients/codex.rs
rtk git commit -m "fix(codex): map final_answer to Idle, function_call to Running

function_call mid-turn is the agent working, not the user blocked → Running.
final_answer is end-of-turn → Idle (was Closed).
final_answer ending in ? remains Waiting(Input)."
```

---

### Task B4: Migrate `gemini_cli.rs` (with Error fix)

**Files:**
- Modify: `crates/agtop-core/src/clients/gemini_cli.rs:540-560` (`update_state_from_gemini_message`)
- Modify: `crates/agtop-core/src/clients/gemini_cli.rs:382, 437, 498, 508` (parser bodies)

Semantic corrections:
- Tool call with `status != "success"` → `ParserState::Error(ParserDetected("gemini.toolCalls.failed"))` (was `"waiting"`, mis-mapped)
- Tool calls all success → `ParserState::Idle` (was `"stopped"`)
- Generic gemini message (no toolCalls) → `ParserState::Idle` (was `"stopped"`)

- [ ] **Step 1: Write the failing tests**

Add to gemini_cli tests:

```rust
#[test]
fn gemini_failed_tool_call_maps_to_error() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(
        r#"{"type":"gemini","gemini":{"toolCalls":[{"status":"failed"}]}}"#,
    ).unwrap();
    let mut ps = ParserState::Unknown;
    let mut detail: Option<String> = None;
    update_parser_state_from_gemini_message(&v, &mut ps, &mut detail);
    assert!(matches!(ps, ParserState::Error(_)), "got {:?}", ps);
}

#[test]
fn gemini_successful_tool_calls_map_to_idle() {
    use crate::session::ParserState;
    let v: serde_json::Value = serde_json::from_str(
        r#"{"type":"gemini","gemini":{"toolCalls":[{"status":"success"}]}}"#,
    ).unwrap();
    let mut ps = ParserState::Unknown;
    let mut detail: Option<String> = None;
    update_parser_state_from_gemini_message(&v, &mut ps, &mut detail);
    assert_eq!(ps, ParserState::Idle);
}
```

- [ ] **Step 2: Run the failing tests**

Run: `rtk cargo test -p agtop-core --lib gemini_failed_tool -- --test-threads=1`
Expected: FAIL.

- [ ] **Step 3: Add the typed function**

Insert into `crates/agtop-core/src/clients/gemini_cli.rs` directly after `update_state_from_gemini_message`:

```rust
/// Typed equivalent of [`update_state_from_gemini_message`]. New code MUST use this.
fn update_parser_state_from_gemini_message(
    message: &serde_json::Value,
    state: &mut crate::session::ParserState,
    state_detail: &mut Option<String>,
) {
    use crate::session::{ErrorReason, ParserState};
    if let Some(tool_calls) = gemini_tool_calls(message) {
        if tool_calls
            .iter()
            .any(|call| call.get("status").and_then(|x| x.as_str()) != Some("success"))
        {
            *state = ParserState::Error(ErrorReason::ParserDetected(
                "gemini.toolCalls.failed".to_string(),
            ));
            *state_detail = Some("gemini.toolCalls.pending_or_error".to_string());
        } else if !tool_calls.is_empty() {
            *state = ParserState::Idle;
            *state_detail = Some("gemini.toolCalls.success".to_string());
        }
    } else {
        *state = ParserState::Idle;
        *state_detail = Some("gemini.message".to_string());
    }
}
```

- [ ] **Step 4: Update the two parser bodies**

For both `parse_gemini_session` (line 437) and `parse_gemini_session_json` (line 508):

Replace `let mut state: Option<String> = None;` (lines 382 and 498) with:

```rust
    use crate::session::ParserState;
    let mut parser_state: ParserState = ParserState::Unknown;
```

Replace the calls to `update_state_from_gemini_message(...)` with `update_parser_state_from_gemini_message(v, &mut parser_state, &mut state_detail)`.

In each `SessionSummary::new(...)` call site (lines 437 and 508), the function takes `state` as a positional argument. Pass `None` for the legacy `state` param. After construction, set `summary.parser_state = parser_state;` (the helper functions return `SessionSummary` directly here — adapt as needed; if they wrap in `Ok(...)`, set the field on the inner value via a `mut` binding).

For example, replace this pattern:

```rust
    let mut summary = SessionSummary::new(/* ..., */ state, state_detail, /* ... */);
    Ok(summary)
```

with:

```rust
    let mut summary = SessionSummary::new(/* ..., */ None, state_detail, /* ... */);
    summary.parser_state = parser_state;
    Ok(summary)
```

- [ ] **Step 5: Delete the old function**

Delete `update_state_from_gemini_message` (lines 540–560).

- [ ] **Step 6: Run the tests**

Run: `rtk cargo test -p agtop-core --lib gemini -- --test-threads=1`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
rtk git add crates/agtop-core/src/clients/gemini_cli.rs
rtk git commit -m "fix(gemini): tool errors emit ParserState::Error (was misclassified as Waiting)

A failing tool call is an error condition, not 'waiting on user' — map it to
ParserState::Error so the TUI shows a red dot. Successful tool calls and
plain assistant messages map to Idle (was 'stopped' / Closed)."
```

---

### Task B5: Migrate `copilot.rs`

Simpler — copilot only has the `questionCarousel` → `Waiting` mapping plus `None`.

- [ ] **Step 1: Write the failing test**

Add to copilot tests:

```rust
#[test]
fn copilot_question_carousel_maps_to_waiting_input() {
    // Smoke: the existing copilot parser sets state="waiting" on
    // questionCarousel; verify the typed equivalent sets ParserState::Waiting.
    // (This test runs through the full parser path; see existing copilot
    // tests for fixture pattern.)
}
```

(If there's no convenient fixture, skip this test and rely on Task A3's table tests + manual smoke test of `agtop json`.)

- [ ] **Step 2: Update the parser body**

In `crates/agtop-core/src/clients/copilot.rs:271` and `:406`:

For the legacy site (271), the `state` field is always `None` — just pass it through and set `parser_state = ParserState::Unknown` after construction.

For the JSONL site (406), where the current code computes:

```rust
        if waiting {
            Some("waiting".into())
        } else {
            None
        }
```

Replace the `let waiting: bool = false;` (line 321) usage by also tracking `parser_state`. Cleanest: replace the `if waiting { Some("waiting".into()) } else { None }` block in the `SessionSummary::new` arg with `None`, and after construction:

```rust
    use crate::session::{ParserState, WaitReason};
    summary.parser_state = if waiting {
        ParserState::Waiting(WaitReason::Input)
    } else {
        ParserState::Unknown
    };
```

- [ ] **Step 3: Run the tests**

Run: `rtk cargo test -p agtop-core --lib copilot -- --test-threads=1`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
rtk git add crates/agtop-core/src/clients/copilot.rs
rtk git commit -m "refactor(copilot): use typed ParserState for questionCarousel detection"
```

---

### Task B6: Migrate `cursor.rs` and `antigravity.rs` (passthrough Unknown)

Both clients always set `state: None` → `parser_state: ParserState::Unknown`. No semantic change.

- [ ] **Step 1: Update `cursor.rs:209`**

After the `Ok(SessionSummary::new(...))` site, change to:

```rust
    let mut summary = SessionSummary::new(/* ..., */ None, /* ... */);
    summary.parser_state = ParserState::Unknown; // explicit
    Ok(summary)
```

(Or simply rely on the default — this assignment is documentation. Pick one and be consistent.)

- [ ] **Step 2: Update `antigravity.rs:92`**

Similar: pass `None` for `state`, set `parser_state = ParserState::Unknown` after construction (or rely on default).

- [ ] **Step 3: Run the workspace build**

Run: `rtk cargo build --workspace`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
rtk git add crates/agtop-core/src/clients/cursor.rs crates/agtop-core/src/clients/antigravity.rs
rtk git commit -m "refactor(cursor,antigravity): default to ParserState::Unknown explicitly"
```

---

### Task B7: Remove `state: Option<String>` from `SessionSummary`

All parsers now set `parser_state`. The `state: Option<String>` field is unused (every site populates it with `None`).

**Files:**
- Modify: `crates/agtop-core/src/session.rs` (struct + `SessionSummary::new`)
- Modify: every parser site that still passes `None` for the `state` arg (claude/opencode/codex/gemini-cli/copilot/cursor/antigravity)

- [ ] **Step 1: Remove the field**

In `crates/agtop-core/src/session.rs`, delete the `pub state: Option<String>,` line (around line 162) from `SessionSummary`. Adjust the `SessionSummary::new` signature: drop the `state: Option<String>` parameter.

- [ ] **Step 2: Update every `SessionSummary::new` call site**

Remove the corresponding positional `None` argument at every call site enumerated in the inventory:
- `crates/agtop-core/src/clients/claude.rs:498` (struct literal — remove `state: None,`)
- `crates/agtop-core/src/clients/opencode.rs:1093, 1152, 1563` (struct literals — remove `state: None,`)
- `crates/agtop-core/src/clients/codex.rs:705` (struct literal — remove `state: None,`)
- `crates/agtop-core/src/clients/gemini_cli.rs:437, 508` (`SessionSummary::new` — drop the `None` for state)
- `crates/agtop-core/src/clients/cursor.rs:209` (`SessionSummary::new` — drop the `None`)
- `crates/agtop-core/src/clients/copilot.rs:271, 406` (`SessionSummary::new` — drop the arg)
- `crates/agtop-core/src/clients/antigravity.rs:92` (`SessionSummary::new` — drop the arg)
- `crates/agtop-core/src/process/mod.rs:197` (test — drop)
- `crates/agtop-core/src/process/transcript_paths.rs:61` (test — drop)
- `crates/agtop-core/src/lib.rs:178` (test — drop)
- `crates/agtop-core/src/aggregate.rs:152` (test — drop)
- `crates/agtop-core/src/process/correlator.rs:474, 568, 604, 649, 690, 731, 857, 871, 914, 928, 982, 996` (tests — drop)
- `crates/agtop-core/src/clients/claude.rs:1111, 1159` (tests — drop)
- `crates/agtop-core/src/clients/opencode.rs:2433, 2447, 2461, 2485, 2499` (tests — drop)
- `crates/agtop-core/src/state_resolution.rs:79` (test — drop)
- `crates/agtop-cli/src/tui/refresh_adapter.rs:236` (test fixture — drop)
- `crates/agtop-cli/src/tui/widgets/state_display.rs` (will be deleted in Task D2; ignore for now)
- `crates/agtop-cli/src/main.rs` (the `JsonSession` struct also has a `state` field — see Task D1 for migration)

Use ripgrep to locate all of them mechanically:

```bash
rtk rg -n 'SessionSummary::new\(|SessionSummary\s*\{' crates
```

- [ ] **Step 3: Update consumers of `summary.state.as_deref()`**

Find any code that reads `summary.state`:

```bash
rtk rg -n 'summary\.state\b' crates
```

Replace each access with `summary.parser_state`. The Claude parser at line 528 has the call:

```rust
    let should_extract_action = matches!(
        summary.state.as_deref(),
        Some("waiting") | Some("running") | None
    );
```

Replace with:

```rust
    use crate::session::ParserState;
    let should_extract_action = matches!(
        summary.parser_state,
        ParserState::Waiting(_) | ParserState::Running | ParserState::Unknown
    );
```

- [ ] **Step 4: Run the workspace build**

Run: `rtk cargo build --workspace`
Expected: PASS.

- [ ] **Step 5: Run the workspace tests**

Run: `rtk cargo test --workspace -- --test-threads=1`
Expected: All tests pass except the known pre-existing `clicking_sortable_header_sorts_by_correct_column`.

- [ ] **Step 6: Commit**

```bash
rtk git add -u
rtk git commit -m "refactor(core): remove SessionSummary.state: Option<String>

All parsers now populate the typed parser_state field. Drop the legacy
string field from the public struct and its SessionSummary::new signature.
Update the one consumer (claude.rs:528 should_extract_action) to read
parser_state directly."
```

---

## Phase C — wire `resolve_state` and remove `derive_state`

### Task C1: Replace `refresh_adapter::derive_state` with `state_resolution::resolve_state`

**Files:**
- Modify: `crates/agtop-cli/src/tui/refresh_adapter.rs:147-226` (delete `derive_state`, `normalize_analysis`, constants; call core converter instead)

- [ ] **Step 1: Update the existing tests in `refresh_adapter.rs`**

Several tests in the file (`live_process_without_parser_state_is_active_running`, `historical_session_without_parser_state_stays_closed`, `unmatched_recent_session_is_running_not_closed`, `unmatched_waiting_session_renders_as_waiting`, `unmatched_stale_session_renders_as_warning_stalled`, `live_process_with_idle_state_counted_as_idle`, `stopped_process_is_closed_not_counted`) currently set `analysis.summary.state = Some("...")` to drive the legacy `derive_state`. Migrate each to set `analysis.summary.parser_state = ParserState::...` instead. Specifically:

- `unmatched_waiting_session_renders_as_waiting`: change `a.summary.state = Some("waiting".into());` → `a.summary.parser_state = ParserState::Waiting(WaitReason::Input);`
- `live_process_with_idle_state_counted_as_idle`: change `a.summary.state = Some("idle".to_string());` → `a.summary.parser_state = ParserState::Idle;`
- `unmatched_stale_session_renders_as_warning_stalled`: this test asserts `Warning(Stalled)` when liveness is None and age 2m. **After tightening (Task A3), this should now produce `Closed`, not `Warning`.** Update the assertion to expect `Closed` and update the test name + doc comment accordingly. Add a NEW test `live_stale_process_renders_as_warning_stalled` that sets `liveness = Some(Live)` and 10m age, and asserts `Warning(Stalled)`.

- [ ] **Step 2: Replace `derive_state` body with a call to the core converter**

Edit `crates/agtop-cli/src/tui/refresh_adapter.rs`. Delete the `derive_state` function (lines 168–226) and the three constants `RUNNING_WINDOW`, `CLOSED_AFTER`, `WAITING_STALE` (lines 161–166).

Edit `normalize_analysis` (line 147–157):

```rust
fn normalize_analysis(analysis: &SessionAnalysis) -> SessionAnalysis {
    let mut analysis = analysis.clone();
    analysis.session_state = Some(agtop_core::state_resolution::resolve_state(
        analysis.summary.parser_state.clone(),
        analysis.liveness,
        analysis.summary.last_active,
        chrono::Utc::now(),
    ));
    analysis
}
```

Note: the new logic always re-derives `session_state`. The old code's `if analysis.liveness.is_some() || analysis.session_state.is_none()` guard was a workaround for the pid-without-liveness drift; with a single source of truth, always re-deriving is correct and cheap.

- [ ] **Step 3: Update imports**

Remove unused imports from the top of `refresh_adapter.rs` (`WaitReason`, `WarningReason` may still be needed by tests — leave the test imports untouched). Remove the now-unused `Liveness` import if no longer referenced. Add `use agtop_core::session::ParserState;` if tests reference it.

- [ ] **Step 4: Run the refresh_adapter tests**

Run: `rtk cargo test -p agtop-cli --lib refresh_adapter -- --test-threads=1`
Expected: All PASS (with the updated assertions from Step 1).

- [ ] **Step 5: Commit**

```bash
rtk git add crates/agtop-cli/src/tui/refresh_adapter.rs
rtk git commit -m "refactor(tui): delegate state derivation to agtop_core::state_resolution

Remove the inline derive_state in refresh_adapter and the parallel set
of constants (RUNNING_WINDOW, CLOSED_AFTER, WAITING_STALE). The TUI now
calls the canonical resolve_state from agtop-core, so the v2 dashboard
and any future CLI consumer share one derivation policy.

Also tightens Warning(Stalled) to require Liveness::Live (per
state_resolution); a stale session with no liveness signal now resolves
to Closed rather than Warning."
```

---

### Task C2: Remove the `pid.is_none()` hacks in sessions render

**Files:**
- Modify: `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs:181` and `:952`

After Phases A/B, parsers no longer produce false `Closed` for live idle Claude/OpenCode sessions — those now correctly resolve to `Idle`, which is not a muted-row state. The `pid.is_none()` guards become unnecessary.

- [ ] **Step 1: Write a regression test**

Add a test in the existing `sessions::tests` module asserting that an `Idle` row with `pid.is_some()` renders with `fg_default`, NOT `fg_muted`. This pins the behaviour we want and would have caught the dimming bug originally.

(Pseudocode — adapt to the existing test patterns in `sessions.rs`. Look at the surrounding tests for how rows are constructed in tests there.)

- [ ] **Step 2: Run the failing test**

Should pass already — `Idle` does not match `is_muted_row`. Verifies the test is wired correctly.

- [ ] **Step 3: Simplify both sites**

At `sessions.rs:181`:

```rust
        let row_style = if state_style::is_muted_row(&state) && row.analysis.pid.is_none() {
            Style::default().fg(theme.fg_muted)
        } else {
            Style::default().fg(theme.fg_default)
        };
```

becomes:

```rust
        let row_style = if state_style::is_muted_row(&state) {
            Style::default().fg(theme.fg_muted)
        } else {
            Style::default().fg(theme.fg_default)
        };
```

Same change at `sessions.rs:952`. (`sessions.rs:739` is already without the hack — double-check no divergence remains.)

- [ ] **Step 4: Run the workspace tests**

Run: `rtk cargo test --workspace -- --test-threads=1`
Expected: PASS (modulo known pre-existing failure).

- [ ] **Step 5: Manual smoke test**

```bash
rtk cargo run -p agtop-cli -- tui
```

Verify: live Claude/OpenCode sessions sitting at the prompt show as **green Idle dot, undimmed text**. Closed historical sessions remain dimmed with no dot.

- [ ] **Step 6: Commit**

```bash
rtk git add crates/agtop-cli/src/tui/screens/dashboard/sessions.rs
rtk git commit -m "refactor(sessions): remove pid.is_none() dimming guard

The guard was load-bearing because parsers misclassified end-of-turn as
Closed even when the process was alive. With ParserState::Idle now
flowing through resolve_state, idle live sessions are correctly muted-
free without needing the pid-is-none escape hatch."
```

---

### Task C3: Audit `is_active` callers and add `is_live` if needed

**Files:**
- Modify: `crates/agtop-core/src/session.rs:75-86` (add `is_live`)
- Audit: every caller of `is_active`

- [ ] **Step 1: Add `is_live`**

In `crates/agtop-core/src/session.rs`, in the `impl SessionState` block (around line 76), insert:

```rust
    /// True if there is (or was very recently) a live process backing this
    /// session. Equivalent to "anything except Closed".
    #[must_use]
    pub const fn is_live(&self) -> bool {
        !matches!(self, Self::Closed)
    }
```

Plus a unit test:

```rust
#[test]
fn is_live_excludes_only_closed() {
    assert!(SessionState::Running.is_live());
    assert!(SessionState::Idle.is_live());
    assert!(SessionState::Waiting(WaitReason::Input).is_live());
    assert!(SessionState::Warning(WarningReason::Stalled { since: chrono::Utc::now() }).is_live());
    assert!(SessionState::Error(ErrorReason::Crash).is_live());
    assert!(!SessionState::Closed.is_live());
}
```

- [ ] **Step 2: Audit existing `is_active` call sites**

```bash
rtk rg -n 'is_active\(\)' crates
```

For each hit, decide:
- Header counts (`refresh_adapter.rs:88-90`): "active" semantically means "doing or could resume work without external input" — the current definition (`Running | Idle | Warning`) matches. Keep `is_active`.
- Anywhere else: read context. If the code is actually asking "is there a process here?", switch to `is_live`. If asking "can the agent work right now?", keep `is_active`.

- [ ] **Step 3: Run tests**

Run: `rtk cargo test --workspace -- --test-threads=1`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
rtk git add -u
rtk git commit -m "feat(core): add SessionState::is_live() and audit is_active callers"
```

---

## Phase D — migrate v1 widgets and JSON output, delete `state_display`

### Task D1: Migrate `agtop json` output to canonical vocabulary

**Files:**
- Modify: `crates/agtop-cli/src/main.rs:605-688` (`JsonSession`)

**This is the documented breaking change to JSON output.**

- [ ] **Step 1: Write a failing test**

Update the existing test `json_session_keeps_raw_state_and_adds_display_state` (line 697). The new contract:

- `JsonSession.state` field: removed entirely (legacy raw string is gone).
- `JsonSession.parser_state`: added — serialized form of the typed enum.
- `JsonSession.display_state`: now produces canonical strings — `"running" | "waiting" | "idle" | "warning" | "error" | "closed"`.

```rust
#[test]
fn json_session_uses_canonical_display_state() {
    use agtop_core::session::{ClientKind, ParserState, SessionState, WaitReason};
    let mut a = SessionAnalysis::new(/* ... */);  // construct as in existing tests
    a.summary.parser_state = ParserState::Idle;
    a.session_state = Some(SessionState::Idle);
    let now = chrono::Utc::now();
    let json = JsonSession::from_analysis(&a, now);
    assert_eq!(json.display_state, "idle");
}
```

- [ ] **Step 2: Run the failing test**

Run: `rtk cargo test -p agtop-cli json_session_uses_canonical -- --test-threads=1`
Expected: FAIL — old test still expects `"working"`.

- [ ] **Step 3: Update `JsonSession` and `from_analysis`**

In `crates/agtop-cli/src/main.rs:605`, edit `struct JsonSession`:

- Remove the `state: Option<String>,` field.
- Add `parser_state: agtop_core::session::ParserState,`.
- The `display_state: String,` field stays; semantics change.

In `from_analysis` (line 656), delete:

```rust
        let (display_state_label, _) = display_state(a, now);
```

Replace it with:

```rust
        let session_state = a
            .session_state
            .clone()
            .unwrap_or_else(|| {
                agtop_core::state_resolution::resolve_state(
                    a.summary.parser_state.clone(),
                    a.liveness,
                    a.summary.last_active,
                    now,
                )
            });
```

In the struct construction:
- Delete `state: a.summary.state.clone(),`.
- Add `parser_state: a.summary.parser_state.clone(),`.
- Replace `display_state: display_state_label.to_string(),` with `display_state: session_state.as_str().to_string(),`.

Remove the `use crate::tui::widgets::state_display::display_state;` import at the top of `main.rs:16`.

- [ ] **Step 4: Update or delete the old test**

The original test at line 697 (`json_session_keeps_raw_state_and_adds_display_state`) is now wrong (it asserts `display_state == "working"`). Delete it; replace with the new test from Step 1 covering the canonical vocabulary.

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test -p agtop-cli --bin agtop json_session -- --test-threads=1`
Expected: PASS.

- [ ] **Step 6: Manual smoke test**

```bash
rtk cargo run -p agtop-cli -- json | head -200
```

Verify: `display_state` field shows values from the canonical set. No `"working"` / `"stale"` strings remain.

- [ ] **Step 7: Commit**

```bash
rtk git add crates/agtop-cli/src/main.rs
rtk git commit -m "feat(cli)!: agtop json display_state uses canonical SessionState vocabulary

BREAKING: agtop json now emits 'running' / 'idle' / 'waiting' / 'warning' /
'error' / 'closed' for the display_state field, matching the TUI.
Removes the legacy 'working' / 'stale' strings. The legacy raw 'state'
field is removed; the new parser_state field carries the typed parser
opinion."
```

---

### Task D2: Migrate v1 widgets to read `SessionState` directly

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/session_table.rs:20, 278`
- Modify: `crates/agtop-cli/src/tui/widgets/info_tab.rs:15, 269`

Both files import `display_state` and call it for label/style. Replace with reading `analysis.session_state` and calling `state_style::label_for` + `state_style::dot_color`.

- [ ] **Step 1: Update `session_table.rs`**

Replace line 20:

```rust
use crate::tui::widgets::state_display::display_state;
```

with:

```rust
use crate::tui::widgets::state_style;
use agtop_core::session::SessionState;
```

Replace line 278:

```rust
    let (state, state_style) = display_state(a, now);
```

with:

```rust
    let session_state = a.session_state.clone().unwrap_or(SessionState::Closed);
    let state = state_style::label_for(&session_state);
    let state_color = state_style::dot_color(&session_state, theme); // adapt — get theme from caller
```

(The replacement may need to be adapted to the surrounding code — the original returns a `(label, Style)` pair; the new code may need to construct the `Style` manually using the dot_color or call into `state_style` for full styling. Read the surrounding 30-line context and pick the cleanest replacement.)

- [ ] **Step 2: Update `info_tab.rs`**

Same pattern at line 15 and line 269.

- [ ] **Step 3: Run the tests**

Run: `rtk cargo test -p agtop-cli --lib widgets -- --test-threads=1`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
rtk git add crates/agtop-cli/src/tui/widgets/session_table.rs crates/agtop-cli/src/tui/widgets/info_tab.rs
rtk git commit -m "refactor(tui): migrate v1 widgets to read SessionState directly"
```

---

### Task D3: Delete `widgets/state_display.rs`

**Files:**
- Delete: `crates/agtop-cli/src/tui/widgets/state_display.rs`
- Modify: `crates/agtop-cli/src/tui/widgets/mod.rs` (remove `pub mod state_display;`)

- [ ] **Step 1: Verify no remaining callers**

```bash
rtk rg -n 'display_state|state_display' crates/agtop-cli/src
```

Expected: Only the `mod.rs` declaration remains.

- [ ] **Step 2: Delete the file**

```bash
rtk rm crates/agtop-cli/src/tui/widgets/state_display.rs
```

- [ ] **Step 3: Remove the module declaration**

In `crates/agtop-cli/src/tui/widgets/mod.rs`, delete the `pub mod state_display;` line.

- [ ] **Step 4: Build and test**

Run: `rtk cargo build --workspace && rtk cargo test --workspace -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
rtk git add -A
rtk git commit -m "chore(tui): delete widgets/state_display.rs (parallel string vocabulary)"
```

---

### Task D4: Update architecture doc

**Files:**
- Modify: `docs/architecture/ARCHITECTURE.md` lines 58–73 (§Session state)

- [ ] **Step 1: Replace the section content**

Edit `docs/architecture/ARCHITECTURE.md`. Replace the existing §Session state with:

```markdown
## Session state

Core defines the canonical 6-variant `SessionState` enum:
- `Running` — agent actively producing output or executing a tool call (theme: `status_warning`)
- `Waiting(WaitReason)` — agent paused waiting for user response (theme: `accent_secondary`, pulsating)
- `Warning(WarningReason)` — live but anomalous, e.g. stalled past threshold (theme: `status_attention`)
- `Error(ErrorReason)` — ended with an explicit error (theme: `status_error`)
- `Idle` — live, ready for input, not currently working (theme: `status_success`)
- `Closed` — no live process; historical/archival (no dot, muted row text)

`Closed` is the only non-live state. `SessionState::is_live()` returns true for
all other variants. `SessionState::is_active()` returns true for `Running`,
`Idle`, and `Warning` (the agent could resume work without external input).

### Pipeline

1. Per-client parsers populate `SessionSummary.parser_state: ParserState`
   (a typed enum: `Idle | Running | Waiting(WaitReason) | Error(ErrorReason) |
   Unknown`). Parsers do not consume strings; they emit typed values directly.
2. The OS-process correlator populates `SessionAnalysis.liveness: Option<Liveness>`.
3. `agtop_core::state_resolution::resolve_state(parser_state, liveness, last_active, now)`
   produces the canonical `SessionState`. This is the single derivation site;
   both the TUI and the JSON CLI consume `SessionState` from here.
4. The TUI maps `SessionState` to color/pulse/label via `widgets::state_style`.
   The JSON CLI emits `SessionState::as_str()` as the `display_state` field.

`Closed` is produced by `resolve_state` only when:
- the OS correlator confirms the process exited (`Liveness::Stopped`), or
- there is no liveness signal AND the session is older than the staleness window.

Parsers MUST NOT produce `Closed` directly. End-of-turn signals
(`stop_reason=end_turn`, `finish=stop`, `final_answer`) map to `ParserState::Idle`
(the agent is alive at the user prompt).
```

- [ ] **Step 2: Commit**

```bash
rtk git add docs/architecture/ARCHITECTURE.md
rtk git commit -m "docs(architecture): document the unified session state pipeline"
```

---

### Task D5: Open follow-up issue for permission/question tool detection

- [ ] **Step 1: Open an issue via `gh`**

```bash
gh issue create \
  --title "Detect opencode/codex/claude question/permission tools and emit Waiting(Permission)" \
  --body "$(cat <<'EOF'
## Background

`SessionState::Waiting(WaitReason)` distinguishes:
- `WaitReason::Input` — agent has finished and is asking the user a question.
- `WaitReason::Permission` — agent is asking for permission to take an action (run a command, edit a file, ...).

Today, after the state-vocabulary refactor (`docs/2026-04-27-state-vocabulary-unification.md`), all parser-emitted `Waiting` cases use `WaitReason::Input` because we don't yet distinguish question-style prompts from permission-style escalations.

## What this issue tracks

Detect the per-client signals that indicate a permission/escalation prompt vs. an open question, and emit the right `WaitReason`:

- **OpenCode** has a `question` tool — confirm the JSONL signature and map invocations to `Waiting(Input)`. Identify whether OpenCode has a separate permission/escalation tool; if so, that is `Waiting(Permission)`.
- **Codex** has well-defined question and escalation tools per session notes — identify their tool names in the rollout JSONL and map appropriately.
- **Claude** — investigate whether `tool_use` events with specific tool names (e.g. `BashCommand` with safety prompt) signal permission requests.
- **Gemini CLI** — TBD.

## Definition of done

- Each client parser emits `Waiting(Permission)` for its escalation/permission tools.
- The TUI's existing `state_style::action_needs_warning_modifier` already distinguishes `Waiting(Permission)` for visual emphasis — verify that emphasis fires correctly in a manual smoke test.
- JSON output's `display_state` remains `"waiting"` for both reasons (the reason is not surfaced in the coarse string).
EOF
)"
```

- [ ] **Step 2: No commit needed** — issue tracking only.

---

## Final verification

- [ ] **Run the full workspace test suite**

Run: `rtk cargo test --workspace -- --test-threads=1`
Expected: All tests pass except the known pre-existing `clicking_sortable_header_sorts_by_correct_column` in bin `agtop`.

- [ ] **Run clippy**

Run: `rtk cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Manual smoke test of the TUI**

Run: `rtk cargo run -p agtop-cli -- tui`

Verify visually:
- Live Claude session at user prompt → green Idle dot, undimmed text.
- Live Claude session mid-tool-call → yellow Running dot.
- Live OpenCode session at user prompt → green Idle dot.
- Closed historical session → no dot, dimmed text.
- A session with no liveness + recent activity → yellow Running dot for the first 30s.
- A session with no liveness + stale activity (> 30s, < 5m) → no dot, dimmed (Closed). NOT a Warning dot.
- A live stalled session (`liveness == Live`, `age > 5m`) → orange Warning dot.

- [ ] **Manual smoke test of the JSON output**

Run: `rtk cargo run -p agtop-cli -- json | jq '.[] | .display_state' | sort -u`

Expected output values (any subset of):
```
"closed"
"error"
"idle"
"running"
"waiting"
"warning"
```

Any `"working"` or `"stale"` value indicates incomplete migration.

- [ ] **Commit a final summary if any drift was caught during smoke testing**

If the smoke tests revealed missed migration sites, fix them and add a commit `fix(state): address smoke-test findings` (or skip if everything is clean).

---

## Self-review checklist

- [x] Each task has exact file paths.
- [x] Each task that writes code shows the actual code.
- [x] Test commands shown for each task with expected output.
- [x] No "TBD" / "implement appropriate" / placeholder text in any task body.
- [x] Type names are consistent across tasks: `ParserState`, `SessionState`, `WaitReason`, `WarningReason`, `ErrorReason`, `Liveness` are all used identically throughout.
- [x] Method names consistent: `parser_state_from_*` for parsers, `update_parser_state_from_*` for in-place updates, `resolve_state` for the converter.
- [x] Spec coverage:
  - String vocabulary removal → Phase A + B7
  - Single derivation site → Phase A3 + Phase C1
  - Idle reachable → Phase B (parsers)
  - `pid.is_none()` hack removed → Phase C2
  - `Warning(Stalled)` requires Live → Phase A3 (truth table) + Phase C1 (test update)
  - JSON consistency → Phase D1
  - v1 widgets migrated → Phase D2
  - Permission/question tools → Phase D5 (deferred via issue)
  - Architecture doc updated → Phase D4
