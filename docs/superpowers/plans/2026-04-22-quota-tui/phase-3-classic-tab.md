# Phase 3 — Classic mode Quota tab widget

> **For agentic workers:** Phases 1 and 2 must be committed first.

**Goal:** Create `widgets/quota_tab.rs` — a horizontal card row with centered content, `■` bars colored green/yellow/red, and horizontal scroll when providers overflow.

**Spec sections covered:** "Classic mode — Quota tab", "Bar characters and colors".

---

## Task 1: Add quota-specific theme constants

**Files:**
- Modify: `crates/agtop-cli/src/tui/theme.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/agtop-cli/src/tui/theme.rs` (the file has no tests today, so add a `#[cfg(test)] mod tests` at the bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_thresholds_exist() {
        let _ = QUOTA_BAR_OK;
        let _ = QUOTA_BAR_WARN;
        let _ = QUOTA_BAR_CRIT;
        let _ = QUOTA_BAR_STALE;
        let _ = QUOTA_EMPTY;
        let _ = QUOTA_SELECTED;
        let _ = QUOTA_TITLE;
    }
}
```

- [ ] **Step 2: Run test — expect failure**

Run: `cargo test -p agtop-cli theme::tests::quota_thresholds_exist`
Expected: FAIL — constants not defined.

- [ ] **Step 3: Add the constants**

Append to `crates/agtop-cli/src/tui/theme.rs` (before the `#[cfg(test)]` block):

```rust
// ── Quota pane ────────────────────────────────────────────────────────────

/// Bar fill when used_percent < 75 %.
pub const QUOTA_BAR_OK: Style = Style::new().fg(Color::Green);

/// Bar fill when used_percent is in [75, 90).
pub const QUOTA_BAR_WARN: Style = Style::new().fg(Color::Yellow);

/// Bar fill when used_percent >= 90.
pub const QUOTA_BAR_CRIT: Style = Style::new().fg(Color::Red);

/// Card rendered dim when the last fetch failed but a prior good result exists.
pub const QUOTA_BAR_STALE: Style = Style::new()
    .fg(Color::DarkGray)
    .add_modifier(Modifier::DIM);

/// Empty column of a bar — not rendered, but the style is kept for callers
/// that want to paint a background.
pub const QUOTA_EMPTY: Style = Style::new();

/// Highlighted provider row in the Dashboard list.
pub const QUOTA_SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

/// Title for centered placeholder messages (Idle / Loading / Error).
pub const QUOTA_TITLE: Style = Style::new()
    .fg(Color::Gray)
    .add_modifier(Modifier::BOLD);
```

- [ ] **Step 4: Run test — expect success**

Run: `cargo test -p agtop-cli theme::tests::quota_thresholds_exist`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/theme.rs
git commit -m "quota-tui(phase-3): add quota theme constants"
```

---

## Task 2: Bar-rendering helper

**Files:**
- Create: `crates/agtop-cli/src/tui/widgets/quota_bar.rs`
- Modify: `crates/agtop-cli/src/tui/widgets/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/agtop-cli/src/tui/widgets/quota_bar.rs`:

```rust
//! Shared bar-rendering helpers for the quota pane.
//!
//! The bar is a sequence of `■` (U+25A0) cells for the used portion and
//! spaces for the empty portion, colored per threshold.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::tui::theme as th;

/// Unicode "BLACK SQUARE" — used for filled bar cells.
pub const BAR_FILLED: char = '■';

/// Resolve a style for a bar based on `used_percent` (0..100).
/// `stale=true` forces the dim/gray variant regardless of threshold.
pub fn bar_style(used_percent: Option<f64>, stale: bool) -> Style {
    if stale {
        return th::QUOTA_BAR_STALE;
    }
    match used_percent {
        None => th::QUOTA_EMPTY,
        Some(p) if p < 75.0 => th::QUOTA_BAR_OK,
        Some(p) if p < 90.0 => th::QUOTA_BAR_WARN,
        Some(_) => th::QUOTA_BAR_CRIT,
    }
}

/// Build a pair of spans `[filled, empty]` of total width `width`.
/// `used_percent` clamped to [0, 100]. `None` → zero fill.
pub fn bar_spans(used_percent: Option<f64>, width: usize, stale: bool) -> [Span<'static>; 2] {
    let width = width.max(1);
    let fill = used_percent
        .map(|p| {
            let clamped = p.clamp(0.0, 100.0);
            ((clamped / 100.0) * width as f64).round() as usize
        })
        .unwrap_or(0)
        .min(width);
    let empty = width - fill;
    let style = bar_style(used_percent, stale);
    [
        Span::styled(BAR_FILLED.to_string().repeat(fill), style),
        Span::raw(" ".repeat(empty)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_below_75_is_ok() {
        assert_eq!(bar_style(Some(74.9), false), th::QUOTA_BAR_OK);
    }

    #[test]
    fn style_75_to_90_is_warn() {
        assert_eq!(bar_style(Some(75.0), false), th::QUOTA_BAR_WARN);
        assert_eq!(bar_style(Some(89.9), false), th::QUOTA_BAR_WARN);
    }

    #[test]
    fn style_at_or_above_90_is_crit() {
        assert_eq!(bar_style(Some(90.0), false), th::QUOTA_BAR_CRIT);
        assert_eq!(bar_style(Some(100.0), false), th::QUOTA_BAR_CRIT);
    }

    #[test]
    fn style_stale_overrides_threshold() {
        assert_eq!(bar_style(Some(50.0), true), th::QUOTA_BAR_STALE);
    }

    #[test]
    fn spans_fill_calculation() {
        let [filled, empty] = bar_spans(Some(50.0), 10, false);
        assert_eq!(filled.content.chars().count(), 5);
        assert_eq!(empty.content.chars().count(), 5);
    }

    #[test]
    fn spans_fill_rounds_to_nearest() {
        // 33 % of 10 = 3.3 → 3 filled.
        let [filled, _] = bar_spans(Some(33.0), 10, false);
        assert_eq!(filled.content.chars().count(), 3);
        // 35 % of 10 = 3.5 → 4 filled (banker's rounding n/a: .round() rounds half up).
        let [filled, _] = bar_spans(Some(35.0), 10, false);
        assert_eq!(filled.content.chars().count(), 4);
    }

    #[test]
    fn spans_none_is_all_empty() {
        let [filled, empty] = bar_spans(None, 6, false);
        assert_eq!(filled.content.chars().count(), 0);
        assert_eq!(empty.content.chars().count(), 6);
    }

    #[test]
    fn spans_total_width_always_matches() {
        for pct in [0.0, 10.0, 50.0, 99.0, 100.0] {
            let [f, e] = bar_spans(Some(pct), 20, false);
            assert_eq!(f.content.chars().count() + e.content.chars().count(), 20);
        }
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/agtop-cli/src/tui/widgets/mod.rs`, append:

```rust
pub mod quota_bar;
```

(If the file doesn't exist, or has a different structure, check existing sibling modules like `session_table` to follow the pattern.)

- [ ] **Step 3: Run tests**

Run: `cargo test -p agtop-cli widgets::quota_bar`
Expected: PASS (8 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/quota_bar.rs crates/agtop-cli/src/tui/widgets/mod.rs
git commit -m "quota-tui(phase-3): add quota_bar helper with ■ chars and threshold colors"
```

---

## Task 3: Error-token and status-glyph helpers

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/quota_bar.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `quota_bar.rs`:

```rust
    #[test]
    fn error_token_401() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Http {
                status: 401,
                retry_after: None,
            },
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "401");
    }

    #[test]
    fn error_token_429() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Http {
                status: 429,
                retry_after: Some(30),
            },
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "429");
    }

    #[test]
    fn error_token_transport() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Transport,
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "net");
    }

    #[test]
    fn error_token_parse() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Parse,
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "parse");
    }

    #[test]
    fn error_token_provider() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Provider {
                code: Some("E001".into()),
            },
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "E001");
        let e2 = QuotaError {
            kind: ErrorKind::Provider { code: None },
            detail: "".into(),
        };
        assert_eq!(error_token(&e2), "err");
    }
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli widgets::quota_bar::tests::error_token_401`
Expected: FAIL — `error_token` not defined.

- [ ] **Step 3: Add the helper**

In `crates/agtop-cli/src/tui/widgets/quota_bar.rs`, append to the module (before the `#[cfg(test)]`):

```rust
use agtop_core::quota::QuotaError;

/// Short (≤ 5 char) identifier used in the card's bar slot when the
/// current fetch failed and no last-good result is available.
pub fn error_token(err: &QuotaError) -> String {
    use agtop_core::quota::ErrorKind;
    match &err.kind {
        ErrorKind::NotConfigured => "n/c".to_string(),
        ErrorKind::Http { status, .. } => status.to_string(),
        ErrorKind::Transport => "net".to_string(),
        ErrorKind::Parse => "parse".to_string(),
        ErrorKind::Provider { code } => code.clone().unwrap_or_else(|| "err".to_string()),
    }
}

/// Status glyph for the Dashboard list column:
/// - ● ok
/// - ▲ stale
/// - ✗ error (no last_good)
/// - ○ loading
pub fn status_glyph(current_ok: bool, last_good_some: bool, loading: bool) -> char {
    if loading {
        '○'
    } else if current_ok {
        '●'
    } else if last_good_some {
        '▲'
    } else {
        '✗'
    }
}
```

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli widgets::quota_bar`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/quota_bar.rs
git commit -m "quota-tui(phase-3): add error_token and status_glyph helpers"
```

---

## Task 4: Classic tab body — card row rendering

**Files:**
- Create: `crates/agtop-cli/src/tui/widgets/quota_tab.rs`
- Modify: `crates/agtop-cli/src/tui/widgets/mod.rs`

- [ ] **Step 1: Write the first failing test**

Create `crates/agtop-cli/src/tui/widgets/quota_tab.rs`:

```rust
//! Classic mode — Quota tab body.
//!
//! A single wide panel with one card per configured provider, laid out
//! horizontally. Each card shows the provider's preferred window as a
//! short view (name + bar + %). Full details live in the Dashboard pane.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::*,
    widgets::Paragraph,
};

use crate::tui::app::{quota::preferred_window, App, ProviderSlot, QuotaState};
use crate::tui::theme as th;
use crate::tui::widgets::quota_bar::{bar_spans, error_token, status_glyph};

/// Fixed card slot width (including gutter).
pub const CARD_SLOT_WIDTH: u16 = 20;
/// Width of the bar inside a card, in cells.
const CARD_BAR_WIDTH: usize = 6;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    // Full-area placeholders for non-Ready states.
    match app.quota_state() {
        QuotaState::Idle => {
            render_centered(frame, area, "Press r to load quota data");
            return;
        }
        QuotaState::Loading => {
            render_centered(frame, area, "Fetching quota data…");
            return;
        }
        QuotaState::Error(msg) => {
            render_centered(frame, area, &format!("Error: {msg}"));
            return;
        }
        QuotaState::Ready => {}
    }

    let slots = app.quota_slots();
    if slots.is_empty() {
        render_centered(frame, area, "No quota data");
        return;
    }

    let cards_visible = usable_card_count(area.width);
    let scroll = app.card_scroll().min(slots.len().saturating_sub(1));
    let end = (scroll + cards_visible).min(slots.len());
    let visible = &slots[scroll..end];

    // Layout: one slot per card, fixed-width constraints, rest goes to padding.
    let mut constraints: Vec<Constraint> = visible
        .iter()
        .map(|_| Constraint::Length(CARD_SLOT_WIDTH))
        .collect();
    // Final "Min(0)" absorbs unused columns so cards stay left-aligned.
    constraints.push(Constraint::Min(0));

    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, slot) in visible.iter().enumerate() {
        render_card(frame, cells[i], slot);
    }

    // Scroll indicators: ‹ top-right if scrolled from start, › if more exist.
    if scroll > 0 {
        render_indicator(frame, area, '‹', Alignment::Left);
    }
    if end < slots.len() {
        render_indicator(frame, area, '›', Alignment::Right);
    }
}

fn usable_card_count(width: u16) -> usize {
    ((width / CARD_SLOT_WIDTH) as usize).max(1)
}

fn render_centered(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let p = Paragraph::new(msg).style(th::QUOTA_TITLE).alignment(Alignment::Center);
    frame.render_widget(p, area);
}

fn render_indicator(frame: &mut Frame<'_>, area: Rect, ch: char, alignment: Alignment) {
    // One-char paragraph pinned to the top row of the panel.
    let indicator_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let p = Paragraph::new(ch.to_string())
        .style(th::QUOTA_TITLE)
        .alignment(alignment);
    frame.render_widget(p, indicator_area);
}

fn render_card(frame: &mut Frame<'_>, area: Rect, slot: &ProviderSlot) {
    // Two rows per card: name line + value line.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let (name_line, value_line, card_style_override) = build_card_lines(slot);

    let mut name_p = Paragraph::new(name_line).alignment(Alignment::Center);
    if let Some(style) = card_style_override {
        name_p = name_p.style(style);
    }
    frame.render_widget(name_p, rows[0]);

    let mut val_p = Paragraph::new(value_line).alignment(Alignment::Center);
    if let Some(style) = card_style_override {
        val_p = val_p.style(style);
    }
    frame.render_widget(val_p, rows[1]);
}

/// Produce the two lines of a card.
///
/// Return value is `(name_line, value_line, overall_style_override)`.
/// The override is `Some(..)` for stale cards (dim) so the whole card is rendered dim.
pub(crate) fn build_card_lines<'a>(
    slot: &'a ProviderSlot,
) -> (Line<'a>, Line<'a>, Option<Style>) {
    let provider_name = slot.current.provider_name;
    let stale = !slot.current.ok && slot.last_good.is_some();
    let errored = !slot.current.ok && slot.last_good.is_none();

    let glyph_suffix = if stale {
        " †"
    } else if errored {
        " ✗"
    } else if slot.current.usage.is_none() && slot.current.ok {
        // Unusual: ok=true with no usage. Still show name only.
        ""
    } else {
        ""
    };

    let name_line = Line::from(vec![Span::raw(format!("{provider_name}{glyph_suffix}"))]);

    let value_line = if errored {
        let token = slot
            .current
            .error
            .as_ref()
            .map(error_token)
            .unwrap_or_else(|| "err".into());
        Line::from(vec![Span::raw(format!("— {token}"))])
    } else {
        // Use `current` for shape; if current.ok=false and last_good=Some, display
        // last_good values but retain the dim style via `stale` flag (applied in bar_spans).
        let effective = if stale {
            slot.last_good.as_ref().unwrap_or(&slot.current)
        } else {
            &slot.current
        };
        match effective.usage.as_ref() {
            None => Line::from(vec![Span::raw("— loading…")]),
            Some(u) => match preferred_window(effective.provider_id, u) {
                None => Line::from(vec![Span::raw("—")]),
                Some((label, w)) => match w.used_percent {
                    Some(p) => {
                        let pct_text = format!("{label} {p:.0}% ");
                        let [filled, empty] = bar_spans(Some(p), CARD_BAR_WIDTH, stale);
                        Line::from(vec![Span::raw(pct_text), filled, empty])
                    }
                    None => {
                        let label_text = w
                            .value_label
                            .clone()
                            .unwrap_or_else(|| "—".into());
                        Line::from(vec![Span::raw(format!("{label} {label_text}"))])
                    }
                },
            },
        }
    };

    let overall_style = if stale {
        Some(th::QUOTA_BAR_STALE)
    } else {
        None
    };

    (name_line, value_line, overall_style)
}
```

- [ ] **Step 2: Register the module**

In `crates/agtop-cli/src/tui/widgets/mod.rs`, append:

```rust
pub mod quota_tab;
```

- [ ] **Step 3: Write the snapshot tests**

Append to `quota_tab.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{App, ProviderSlot};
    use agtop_core::quota::{
        ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageWindow,
    };
    use indexmap::IndexMap;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_usage(pairs: &[(&str, f64)]) -> Usage {
        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        for (k, p) in pairs {
            windows.insert(
                (*k).to_string(),
                UsageWindow {
                    used_percent: Some(*p),
                    window_seconds: None,
                    reset_at: None,
                    value_label: None,
                },
            );
        }
        Usage {
            windows,
            models: Default::default(),
            extras: Default::default(),
        }
    }

    fn ok_slot(id: ProviderId, usage: Usage) -> ProviderSlot {
        ProviderSlot::new(ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: true,
            usage: Some(usage),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        })
    }

    fn err_slot(id: ProviderId, status: u16) -> ProviderSlot {
        ProviderSlot::new(ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: false,
            usage: None,
            error: Some(QuotaError {
                kind: ErrorKind::Http {
                    status,
                    retry_after: None,
                },
                detail: "".into(),
            }),
            fetched_at: 0,
            meta: Default::default(),
        })
    }

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        let area = buf.area;
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = buf.cell((x, y)).expect("cell in bounds");
                out.push_str(cell.symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn idle_state_shows_press_r() {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new();
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Press r to load quota data"),
            "idle placeholder missing:\n{contents}"
        );
    }

    #[test]
    fn loading_state_shows_fetching() {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.set_quota_loading();
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Fetching"),
            "loading placeholder missing:\n{contents}"
        );
    }

    #[test]
    fn ready_state_renders_card_with_bar() {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let slot = ok_slot(ProviderId::Claude, make_usage(&[("5h", 72.0)]));
        app.apply_quota_results(vec![slot.current.clone()]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Claude"), "name missing:\n{contents}");
        assert!(contents.contains("5h"), "label missing:\n{contents}");
        assert!(contents.contains("72%"), "percentage missing:\n{contents}");
        // Bar char present (at least one ■).
        assert!(contents.contains('■'), "bar char missing:\n{contents}");
    }

    #[test]
    fn error_card_shows_short_token() {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let slot = err_slot(ProviderId::Google, 401);
        app.apply_quota_results(vec![slot.current.clone()]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Google"), "name missing:\n{contents}");
        assert!(contents.contains("401"), "error token missing:\n{contents}");
        assert!(contents.contains('✗'), "error glyph missing:\n{contents}");
    }

    #[test]
    fn stale_card_has_dagger_and_dim_bar() {
        // First an ok result, then an error → stale.
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let id = ProviderId::Zai;
        let ok = ok_slot(id, make_usage(&[("5h", 88.0)]));
        app.apply_quota_results(vec![ok.current.clone()]);
        let err = err_slot(id, 429);
        app.apply_quota_results(vec![err.current.clone()]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("z.ai"), "name missing:\n{contents}");
        assert!(contents.contains('†'), "stale dagger missing:\n{contents}");
        assert!(contents.contains('■'), "stale bar still rendered:\n{contents}");
    }

    #[test]
    fn unlimited_card_shows_text_no_bar() {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        windows.insert(
            "premium".into(),
            UsageWindow {
                used_percent: None,
                window_seconds: None,
                reset_at: None,
                value_label: Some("Unlimited".into()),
            },
        );
        let usage = Usage {
            windows,
            models: Default::default(),
            extras: Default::default(),
        };
        let slot = ok_slot(ProviderId::Copilot, usage);
        app.apply_quota_results(vec![slot.current.clone()]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Copilot"), "name missing:\n{contents}");
        assert!(
            contents.contains("Unlimited"),
            "Unlimited text missing:\n{contents}"
        );
    }

    #[test]
    fn overflow_shows_scroll_indicator() {
        // 80 cols / 20 card width = 4 cards visible. Five providers → › shows.
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        for id in [
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::Copilot,
            ProviderId::CopilotAddon,
            ProviderId::Zai,
        ] {
            let slot = ok_slot(id, make_usage(&[("5h", 10.0)]));
            app.apply_quota_results(vec![slot.current.clone()]);
        }
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains('›'), "overflow indicator missing:\n{contents}");
    }

    #[test]
    fn scrolled_row_shows_left_indicator() {
        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        for id in [
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::Copilot,
            ProviderId::CopilotAddon,
            ProviderId::Zai,
        ] {
            let slot = ok_slot(id, make_usage(&[("5h", 10.0)]));
            app.apply_quota_results(vec![slot.current.clone()]);
        }
        // Scroll right by 1 to force the left indicator.
        app.quota_card_scroll_right(4);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains('‹'), "left indicator missing:\n{contents}");
    }
}
```

- [ ] **Step 4: Wire the new widget into the bottom-panel dispatch**

In `crates/agtop-cli/src/tui/mod.rs` around line 562, replace the placeholder added in Phase 1:

```rust
        // Phase 3 will wire this up to widgets::quota_tab::render.
        // For now, render a placeholder so the build stays green.
        Tab::Quota => {
            use ratatui::widgets::Paragraph;
            let p = Paragraph::new("(quota tab not yet implemented)");
            frame.render_widget(p, rows[1]);
        }
```

With:

```rust
        Tab::Quota => widgets::quota_tab::render(frame, rows[1], app),
```

- [ ] **Step 5: Run tests — expect success**

Run: `cargo test -p agtop-cli widgets::quota_tab`
Expected: PASS (8 render tests).

Run full suite:
Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/quota_tab.rs crates/agtop-cli/src/tui/widgets/mod.rs crates/agtop-cli/src/tui/mod.rs
git commit -m "quota-tui(phase-3): add Classic Quota tab with horizontal cards"
```

---

## Task 5: Clippy pass

- [ ] **Step 1: Run clippy**

Run: `cargo clippy -p agtop-cli -- -D warnings`
Expected: no warnings.

Likely items:
- Unused `QUOTA_SELECTED` and `QUOTA_EMPTY` (Phase 4 will use them) → keep as `pub const`, no warning generated for public constants.
- Unused `Stylize` import if added accidentally → remove.

- [ ] **Step 2: Commit fixes if needed**

```bash
git add -A
git commit -m "quota-tui(phase-3): clippy fixes"
```

Phase 3 complete. The Classic Quota tab renders horizontal cards with centered, colored `■` bars. No arrow-key handling yet — that's Phase 5.
