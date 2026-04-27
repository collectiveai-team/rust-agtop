# agtop TUI architecture

This document describes the architecture introduced by the btop-style TUI redesign (April 2026).

## Layers

agtop is split into two crates:

- `agtop-core` — session parsing, state derivation, quota fetching. No TUI.
- `agtop-cli` — the binary, including the TUI.

The TUI uses a Model-View-Update + Component-trait architecture.

## Components

A `Component` is any UI unit with the trait:

```rust
trait Component {
    fn render(&self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme);
    fn handle_event(&mut self, event: &AppEvent) -> Option<Msg>;
}
```

Components compose into Screens (Dashboard / Aggregation / Config). Each screen owns its
state and child components.

## Message flow

1. `crossterm::Event` arrives at the App's event loop.
2. `input::AppEvent::from_crossterm` translates it to a domain event.
3. App routes the event:
   - First to global keymap (`d`/`a`/`c`/`q`/`?`).
   - Then to the focused screen, which routes to the focused component.
4. The component returns `Option<Msg>`.
5. App's `update()` consumes the `Msg` and mutates state.
6. Next frame renders from the mutated state.

No rendering decisions inside event handlers; no event handling inside render.

## Theme tokens

Colors live in `Theme` (semantic slots) — never as raw `Color::*` in widget code:

| Slot | Purpose |
|---|---|
| `bg.base / surface / overlay / selection` | Layered backgrounds for depth |
| `fg.default / muted / emphasis` | Body / metadata / headers |
| `border.muted / focused` | Panel borders |
| `accent.primary / secondary` | Interactive accents |
| `status.error / warning / attention / success / info` | Status colors |
| `syntax.string / keyword` | Project paths, model names |

The default palette is `theme_v2::vscode_dark_plus::theme()`.

Per-client brand colors live in `theme_v2::client_palette`.

## Session state

Core defines a 6-variant `SessionState` enum:
- `Running` — agent actively producing output or executing a tool call
- `Waiting(WaitReason)` — agent paused waiting for user response
- `Warning(WarningReason)` — live but anomalous (stalled past threshold)
- `Error(ErrorReason)` — ended with an explicit error
- `Idle` — live, ready for input, not currently working
- `Closed` — no live process; historical/archival

The TUI maps state → style via `widgets::state_style` (color, pulse, label).
No `DisplayState` enum exists — the TUI reads `&SessionState` directly.

State resolution (`state_resolution::resolve_state`) converts string-based
parser output into the canonical `SessionState` enum with configurable
stall threshold via `resolve_state_with_threshold`.

## Iconography (T0/T1/T2)

- **T0 — Universal Unicode**: braille, box-drawing, geometric shapes. Always renders.
- **T1 — Bitmap logos** (`ratatui-image`): client/provider logos in Quota panel only.
- **T2 — Nerd Font Material Design Icons**: opt-in via `appearance.nerd_font`.

Single source of truth: `widgets::icon::Icon` enum. No raw codepoints in widget code.

## How to add a new panel

1. Decide which Screen the panel belongs to.
2. Create a struct + `impl Component`.
3. Add a `Msg` variant if the panel emits new domain events.
4. Add a `FocusId` constant if the panel is focusable.
5. Add to the parent screen's render/event-routing logic.
6. Write a snapshot test under `tests/`.

## How to add a new screen

1. Add a `ScreenId` variant to `tui::msg::ScreenId`.
2. Add a state struct + `impl Component`.
3. Wire view-switch keymap in the App's global handler.
4. Add a section to this document.

## Testing

- Unit tests live next to code.
- Integration tests under `crates/agtop-cli/tests/`.
- Snapshot tests use `insta` against `ratatui::backend::TestBackend`.
- Every TUI snapshot test runs in two flavors: `nerd_font=false` and `nerd_font=true`.
