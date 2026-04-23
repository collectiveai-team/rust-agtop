# Design: Version Badge in TUI Status Bar

**Date:** 2026-04-23  
**Scope:** TUI only — no changes to CLI (`--list`, `--json`) output modes.

---

## Goal

Display the current crate version (`v0.1.0`, sourced from `CARGO_PKG_VERSION`) as plain dimmed text on the right side of the TUI's top status bar.

## Current State

`render_status` in `crates/agtop-cli/src/tui/mod.rs:532` renders a single `Paragraph` widget that fills the entire 1-row status area with left-aligned text:

```
 agtop [classic]  refresh#3  [1/2] claude:deadbeef
```

Both Classic and Dashboard layouts allocate `Constraint::Length(1)` as `outer[0]` for this bar.

## Approach: Horizontal Split Within render_status (Option A)

Split the status row into two side-by-side `Paragraph` widgets using a horizontal `Layout`:

- **Left pane** — existing status string, left-aligned, `STATUS_BAR` style (unchanged)
- **Right pane** — `v{CARGO_PKG_VERSION}`, `Alignment::Right`, styled with the existing `DIM` / muted style from the theme

### Layout

```
| agtop [classic]  refresh#3  [1/2] claude:deadbeef  |  v0.1.0 |
 ←————————————————— left pane ————————————————————————  right →
```

The right pane width is fixed at the length of the version string (e.g., `"v0.1.0"` = 6 chars). The left pane gets the remainder via `Constraint::Min(0)` / `Constraint::Length(version_len)`.

### Implementation Details

1. **`render_status` signature stays the same** — no callers change.
2. Inside `render_status`, before rendering:
   ```rust
   let version = concat!("v", env!("CARGO_PKG_VERSION"));
   let [left_area, right_area] = Layout::horizontal([
       Constraint::Min(0),
       Constraint::Length(version.len() as u16 + 1), // +1 for right padding
   ])
   .areas(area);
   ```
3. Left `Paragraph` — current status string, `th::STATUS_BAR` style.
4. Right `Paragraph` — version string, `th::STATUS_BAR` style with `Modifier::DIM`, `Alignment::Right`.

### Version Source

`concat!("v", env!("CARGO_PKG_VERSION"))` — compile-time constant, zero runtime cost, always matches workspace version in `Cargo.toml`.

### Style

Use the existing theme constant `th::STATUS_BAR` as the base style, with `Modifier::DIM` added so the version is visually subordinate to the operational status info.

## Affected Files

| File | Change |
|---|---|
| `crates/agtop-cli/src/tui/mod.rs` | Modify `render_status` to split area and render two paragraphs |

No other files change. No layout changes outside `render_status`.

## Out of Scope

- CLI `--list` / `--json` output — no changes
- `--version` flag — already handled by clap, no changes
- Help overlay — version not added there

## Testing

- Existing `render_status` tests (if any) updated to account for two render calls
- Manual visual verification: run `agtop` and confirm `v0.1.0` appears right-aligned in the status bar in both Classic and Dashboard modes
