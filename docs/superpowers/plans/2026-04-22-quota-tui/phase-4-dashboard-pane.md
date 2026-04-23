# Phase 4 — Dashboard Quota pane

> **For agentic workers:** Phases 1–3 must be committed first.

**Goal:** Rewrite `widgets/dashboard_plan.rs` to render the quota data: left-panel compact short view (one line per provider), right-panel full detail for selected provider.

**Spec sections covered:** "Dashboard mode — Quota pane", left/right layout, extras, Google per-model, stale warning.

---

## Task 1: Replace `dashboard_plan.rs` with a quota-driven skeleton

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

- [ ] **Step 1: Snapshot the existing tests that must continue to hold**

The existing `dashboard_plan.rs` has tests for `canonical_name`, `merge_plans`, `bar_style`, `bar_spans`. These test PlanUsage-based logic that will be removed. Most of them will be deleted; if you want to preserve any logic for a future fallback, do so now. For this plan: **delete all existing tests in the file**. The quota pane does not need name-canonicalization or plan-usage merging.

- [ ] **Step 2: Write the first failing render test**

Overwrite `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs` with a skeleton that renders a placeholder so we can add tests incrementally:

```rust
//! Dashboard mode — Quota pane.
//!
//! Replaces the previous local-estimate Subscription Details pane. Driven
//! by `App::quota_slots` (populated by the refresh worker from
//! `agtop_core::quota::fetch_all`). 40 % left (compact list) / 60 % right
//! (full detail) split. See the 2026-04-22-quota-tui design spec.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::app::{App, QuotaState};
use crate::tui::theme as th;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(" Quota ");
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    match app.quota_state() {
        QuotaState::Idle => {
            render_centered(frame, inner, "Press r to load quota data");
            return;
        }
        QuotaState::Loading => {
            render_centered(frame, inner, "Fetching quota data…");
            return;
        }
        QuotaState::Error(msg) => {
            render_centered(frame, inner, &format!("Error: {msg}"));
            return;
        }
        QuotaState::Ready => {}
    }

    if app.quota_slots().is_empty() {
        render_centered(frame, inner, "No quota data");
        return;
    }

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(inner);

    render_list(frame, panes[0], app);
    render_details(frame, panes[1], app);
}

fn render_centered(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let p = Paragraph::new(msg)
        .style(th::QUOTA_TITLE)
        .alignment(Alignment::Center);
    frame.render_widget(p, area);
}

fn render_list(_frame: &mut Frame<'_>, _area: Rect, _app: &App) {
    // filled in by Task 2
}

fn render_details(_frame: &mut Frame<'_>, _area: Rect, _app: &App) {
    // filled in by Task 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::quota::{ProviderId, ProviderResult, Usage, UsageWindow};
    use indexmap::IndexMap;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    pub(super) fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
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

    pub(super) fn ok_result(id: ProviderId, usage: Usage) -> ProviderResult {
        ProviderResult {
            provider_id: id,
            provider_name: id.display_name(),
            configured: true,
            ok: true,
            usage: Some(usage),
            error: None,
            fetched_at: 0,
            meta: Default::default(),
        }
    }

    pub(super) fn make_usage(pairs: &[(&str, f64)]) -> Usage {
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

    #[test]
    fn idle_state_shows_placeholder() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new();
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Press r"),
            "idle placeholder missing:\n{contents}"
        );
    }

    #[test]
    fn ready_state_renders_block_title() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let r = ok_result(ProviderId::Claude, make_usage(&[("5h", 72.0)]));
        app.apply_quota_results(vec![r]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Quota"), "block title missing");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p agtop-cli widgets::dashboard_plan`
Expected: PASS (2 tests).

Previous dashboard_plan tests should be GONE — they tested PlanUsage-based helpers that no longer exist.

The existing `renders_dashboard_with_plan_usage` test in `tui/mod.rs` (line 910) will break because it expects the panel title "Subscription Details". **Update that test** to match the new title and expectations — or more cleanly, replace it with a quota-based test. Do it now in the same commit:

Edit `crates/agtop-cli/src/tui/mod.rs` around line 910. Replace the whole `renders_dashboard_with_plan_usage` test with:

```rust
    #[test]
    fn renders_dashboard_with_quota_idle() {
        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.toggle_ui_mode(); // Dashboard

        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");

        let buffer = terminal.backend().buffer().clone();
        let contents = {
            let mut s = String::new();
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    s.push_str(buffer.cell((x, y)).unwrap().symbol());
                }
                s.push('\n');
            }
            s
        };
        assert!(
            contents.contains("Quota"),
            "Quota panel title missing:\n{contents}"
        );
        assert!(
            contents.contains("Press r"),
            "idle placeholder missing:\n{contents}"
        );
    }
```

- [ ] **Step 4: Run full suite**

Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs crates/agtop-cli/src/tui/mod.rs
git commit -m "quota-tui(phase-4): replace dashboard_plan with quota-driven skeleton"
```

---

## Task 2: Left panel — compact short-view list

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `dashboard_plan.rs`:

```rust
    use agtop_core::quota::{ErrorKind, QuotaError};

    pub(super) fn err_result(id: ProviderId, status: u16) -> ProviderResult {
        ProviderResult {
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
        }
    }

    #[test]
    fn left_list_shows_ok_provider_with_bar() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.apply_quota_results(vec![ok_result(
            ProviderId::Claude,
            make_usage(&[("5h", 72.0)]),
        )]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Claude"));
        assert!(contents.contains("5h"));
        assert!(contents.contains("72%"));
        assert!(contents.contains('■'));
        assert!(contents.contains('●'), "ok glyph missing:\n{contents}");
    }

    #[test]
    fn left_list_shows_error_provider() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.apply_quota_results(vec![err_result(ProviderId::Google, 401)]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Google"));
        assert!(contents.contains("401"));
        assert!(contents.contains('✗'), "error glyph missing:\n{contents}");
    }
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli widgets::dashboard_plan::tests::left_list_shows_ok_provider_with_bar`
Expected: FAIL — list not rendered.

- [ ] **Step 3: Implement `render_list`**

Replace the stub `render_list` in `dashboard_plan.rs` with:

```rust
fn render_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    use crate::tui::app::quota::preferred_window;
    use crate::tui::widgets::quota_bar::{bar_spans, error_token, status_glyph};

    const BAR_WIDTH: usize = 10;

    let slots = app.quota_slots();
    let selected = app.selected_provider();

    // Build one Line per slot.
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(slots.len());
    for (i, slot) in slots.iter().enumerate() {
        let is_selected = i == selected;
        let stale = !slot.current.ok && slot.last_good.is_some();
        let errored = !slot.current.ok && slot.last_good.is_none();
        let loading = slot.current.usage.is_none() && slot.current.ok;

        let glyph = status_glyph(slot.current.ok, slot.last_good.is_some(), loading);
        let name_suffix = if stale { " †" } else { "" };
        let prefix = format!("{glyph} {name}{name_suffix}  ", name = slot.current.provider_name);

        let body: Vec<Span<'_>> = if errored {
            let token = slot
                .current
                .error
                .as_ref()
                .map(error_token)
                .unwrap_or_else(|| "err".into());
            vec![Span::raw(format!("— {token}"))]
        } else if loading {
            vec![Span::raw("— loading…")]
        } else {
            // Either ok, or stale → read effective usage.
            let effective = if stale {
                slot.last_good.as_ref().unwrap_or(&slot.current)
            } else {
                &slot.current
            };
            match effective.usage.as_ref().and_then(|u| preferred_window(effective.provider_id, u)) {
                None => vec![Span::raw("—")],
                Some((label, w)) => match w.used_percent {
                    Some(p) => {
                        let [filled, empty] = bar_spans(Some(p), BAR_WIDTH, stale);
                        vec![
                            Span::raw(format!("{label}  {p:>3.0}%  ")),
                            filled,
                            empty,
                        ]
                    }
                    None => {
                        let label_text = w.value_label.clone().unwrap_or_else(|| "—".into());
                        vec![Span::raw(format!("{label}  {label_text}"))]
                    }
                },
            }
        };

        let mut spans = vec![Span::raw(prefix)];
        spans.extend(body);
        let line = Line::from(spans);

        let line = if is_selected {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|s| Span::styled(s.content, s.style.patch(th::QUOTA_SELECTED)))
                    .collect::<Vec<_>>(),
            )
        } else {
            line
        };
        lines.push(line);
    }

    let p = Paragraph::new(lines);
    frame.render_widget(p, area);
}
```

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli widgets::dashboard_plan`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs
git commit -m "quota-tui(phase-4): implement Dashboard left-panel short view"
```

---

## Task 3: Right panel — full detail for selected provider

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:

```rust
    #[test]
    fn right_pane_lists_all_windows_for_selected_provider() {
        let backend = TestBackend::new(120, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let r = ok_result(
            ProviderId::Claude,
            make_usage(&[("5h", 72.0), ("7d", 45.0), ("7d-sonnet", 12.0)]),
        );
        app.apply_quota_results(vec![r]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        // All three window labels appear.
        assert!(contents.contains("5h"), "5h missing");
        assert!(contents.contains("7d"), "7d missing");
        assert!(contents.contains("sonnet"), "7d-sonnet missing");
        // All three percentages appear somewhere.
        assert!(contents.contains("72"));
        assert!(contents.contains("45"));
        assert!(contents.contains("12"));
    }

    #[test]
    fn right_pane_stale_banner_appears_when_stale() {
        let backend = TestBackend::new(120, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let id = ProviderId::Claude;
        app.apply_quota_results(vec![ok_result(id, make_usage(&[("5h", 72.0)]))]);
        app.apply_quota_results(vec![err_result(id, 503)]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Stale"),
            "stale banner missing:\n{contents}"
        );
    }

    #[test]
    fn right_pane_error_only_state_shows_detail() {
        let backend = TestBackend::new(120, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.apply_quota_results(vec![err_result(ProviderId::Codex, 401)]);
        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Error"),
            "error heading missing:\n{contents}"
        );
        assert!(
            contents.contains("401"),
            "status code missing:\n{contents}"
        );
    }
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli widgets::dashboard_plan::tests::right_pane_lists_all_windows_for_selected_provider`
Expected: FAIL.

- [ ] **Step 3: Implement `render_details`**

Replace the stub `render_details` with:

```rust
fn render_details(frame: &mut Frame<'_>, area: Rect, app: &App) {
    use crate::tui::widgets::quota_bar::bar_spans;

    const BAR_WIDTH: usize = 10;

    let slots = app.quota_slots();
    let sel = app.selected_provider();
    let slot = match slots.get(sel) {
        Some(s) => s,
        None => return,
    };

    let stale = !slot.current.ok && slot.last_good.is_some();
    let error_only = !slot.current.ok && slot.last_good.is_none();

    let mut lines: Vec<Line<'_>> = Vec::new();

    // Header: provider name + plan + login (from meta).
    let mut header_parts = vec![slot.current.provider_name.to_string()];
    if let Some(plan) = slot.current.meta.get("plan") {
        header_parts.push(plan.clone());
    }
    if let Some(login) = slot.current.meta.get("login") {
        header_parts.push(login.clone());
    }
    lines.push(Line::from(Span::styled(
        header_parts.join(" · "),
        th::PLAN_LABEL,
    )));

    // Stale banner.
    if stale {
        let err_label = slot
            .current
            .error
            .as_ref()
            .map(|e| format!("{:?}", e.kind))
            .unwrap_or_else(|| "unknown".into());
        let fetched_at_str = format_epoch_ms(
            slot.last_good
                .as_ref()
                .map(|r| r.fetched_at)
                .unwrap_or(slot.current.fetched_at),
        );
        lines.push(Line::from(Span::styled(
            format!("! Stale — data from {fetched_at_str} · last error: {err_label}"),
            th::QUOTA_BAR_STALE,
        )));
    }

    lines.push(Line::from(""));

    if error_only {
        let err = slot.current.error.as_ref();
        let kind = err
            .map(|e| format!("{:?}", e.kind))
            .unwrap_or_else(|| "unknown".into());
        let detail = err.map(|e| e.detail.clone()).unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!("Error: {kind}"),
            th::QUOTA_BAR_CRIT,
        )));
        if !detail.is_empty() {
            lines.push(Line::from(Span::raw(detail)));
        }
        let p = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(p, area);
        return;
    }

    // Effective usage (stale → last_good).
    let effective = if stale {
        slot.last_good.as_ref().unwrap_or(&slot.current)
    } else {
        &slot.current
    };

    if let Some(usage) = effective.usage.as_ref() {
        for (label, w) in &usage.windows {
            lines.push(window_line(label, w, BAR_WIDTH, stale));
        }

        // Google per-model windows.
        if !usage.models.is_empty() {
            lines.push(Line::from(""));
            for (model, windows) in &usage.models {
                for (wlabel, w) in windows {
                    let label = format!("{model}  {wlabel}");
                    lines.push(window_line(&label, w, BAR_WIDTH, stale));
                }
            }
        }

        // Extras.
        if !usage.extras.is_empty() {
            lines.push(Line::from(""));
            for (name, extra) in &usage.extras {
                lines.push(extra_line(name, extra));
            }
        }
    }

    // Fetched-at footer (right-aligned).
    let footer = format!("fetched at {}", format_epoch_ms(effective.fetched_at));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(footer, th::PLAN_NOTE)).alignment(Alignment::Right));

    let p = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(p, area);
}

fn window_line<'a>(
    label: &str,
    w: &'a agtop_core::quota::UsageWindow,
    bar_width: usize,
    stale: bool,
) -> Line<'a> {
    use crate::tui::widgets::quota_bar::bar_spans;
    match w.used_percent {
        Some(p) => {
            let [filled, empty] = bar_spans(Some(p), bar_width, stale);
            Line::from(vec![
                Span::raw(format!("{label:<14}")),
                filled,
                empty,
                Span::raw(format!("  {p:>3.0}%  {}", reset_suffix(w))),
            ])
        }
        None => {
            let text = w.value_label.clone().unwrap_or_else(|| "—".into());
            Line::from(Span::raw(format!(
                "{label:<14}  {text}  {}",
                reset_suffix(w)
            )))
        }
    }
}

fn reset_suffix(w: &agtop_core::quota::UsageWindow) -> String {
    use chrono::{DateTime, Utc};
    match w.reset_at {
        None => "".into(),
        Some(ms) => {
            let now_ms = Utc::now().timestamp_millis();
            let delta = ms - now_ms;
            if delta < 0 {
                return "resets (any moment)".into();
            }
            let secs = delta / 1000;
            if secs < 3600 {
                format!("resets in {}m", secs / 60)
            } else if secs < 86_400 {
                format!("resets in {}h {}m", secs / 3600, (secs % 3600) / 60)
            } else {
                format!("resets in {}d {}h", secs / 86_400, (secs % 86_400) / 3600)
            }
        }
    }
}

fn extra_line<'a>(name: &'a str, extra: &'a agtop_core::quota::UsageExtra) -> Line<'a> {
    use agtop_core::quota::UsageExtra;
    match extra {
        UsageExtra::OverageBudget {
            monthly_limit,
            used,
            utilization,
            currency,
            enabled,
        } => {
            let cur = currency.clone().unwrap_or_else(|| "$".into());
            let limit = monthly_limit.unwrap_or(0.0);
            let used_v = used.unwrap_or(0.0);
            let pct = utilization.map(|u| format!(" ({u:.0}%)")).unwrap_or_default();
            let status = if *enabled {
                format!("enabled · {cur}{used_v:.2} used of {cur}{limit:.2}{pct}")
            } else {
                format!("disabled · limit {cur}{limit:.2}")
            };
            Line::from(Span::raw(format!("Overage  {status}")))
        }
        UsageExtra::PerToolCounts {
            items,
            total_cap,
            reset_at: _,
        } => {
            let total = items.iter().map(|(_, n)| *n).sum::<u64>();
            let cap = total_cap.map(|c| format!(" / {c}")).unwrap_or_default();
            Line::from(Span::raw(format!("{name}  {total}{cap}")))
        }
        UsageExtra::KeyValue(kv) => {
            let s = kv
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" · ");
            Line::from(Span::raw(format!("{name}  {s}")))
        }
    }
}

fn format_epoch_ms(ms: i64) -> String {
    use chrono::{DateTime, Local, Utc};
    let utc: DateTime<Utc> = DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default();
    let local: DateTime<Local> = utc.into();
    local.format("%H:%M:%S").to_string()
}
```

Note: the `bar_spans` import at the top of `render_details` is unused now (it's used inside `window_line`). Remove the unused `use` inside the function body.

The `extra_line` function matches against the `UsageExtra` variants as declared in `agtop-core/src/quota/types.rs` (confirmed at plan-writing time):
- `OverageBudget { monthly_limit: Option<f64>, used: Option<f64>, utilization: Option<f64>, currency: Option<String>, enabled: bool }`
- `PerToolCounts { items: Vec<(String, u64)>, total_cap: Option<u64>, reset_at: Option<i64> }`
- `KeyValue(IndexMap<String, String>)` — tuple variant

If the types have drifted, update the pattern accordingly before running tests.

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli widgets::dashboard_plan`
Expected: PASS.

Run full suite:
Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs
git commit -m "quota-tui(phase-4): implement Dashboard right-panel detail view"
```

---

## Task 4: Google per-model rendering test

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module:

```rust
    #[test]
    fn google_provider_renders_per_model() {
        use agtop_core::quota::{Usage, UsageWindow};
        use indexmap::IndexMap;

        let backend = TestBackend::new(120, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        let mut m1: IndexMap<String, UsageWindow> = IndexMap::new();
        m1.insert(
            "daily".into(),
            UsageWindow {
                used_percent: Some(31.0),
                window_seconds: Some(86400),
                reset_at: None,
                value_label: None,
            },
        );
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        models.insert("gemini/gemini-2.5-pro".into(), m1);

        let usage = Usage {
            windows: Default::default(),
            models,
            extras: Default::default(),
        };
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);

        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("gemini-2.5-pro"),
            "model name missing:\n{contents}"
        );
        assert!(contents.contains("31"), "percentage missing:\n{contents}");
    }
```

- [ ] **Step 2: Run test — expect success**

The Google per-model logic was added in Task 3. This test just confirms it renders.

Run: `cargo test -p agtop-cli widgets::dashboard_plan::tests::google_provider_renders_per_model`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs
git commit -m "quota-tui(phase-4): test Google per-model rendering"
```

---

## Task 5: Overage / extras rendering test

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module:

```rust
    #[test]
    fn overage_budget_disabled_renders_correctly() {
        use agtop_core::quota::{Usage, UsageExtra};
        use indexmap::IndexMap;

        let backend = TestBackend::new(120, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        let mut extras: IndexMap<String, UsageExtra> = IndexMap::new();
        extras.insert(
            "extra_usage".into(),
            UsageExtra::OverageBudget {
                monthly_limit: Some(0.0),
                used: Some(0.0),
                utilization: Some(0.0),
                currency: Some("$".into()),
                enabled: false,
            },
        );
        let usage = Usage {
            windows: make_usage(&[("5h", 50.0)]).windows,
            models: Default::default(),
            extras,
        };
        let r = ok_result(ProviderId::Claude, usage);
        app.apply_quota_results(vec![r]);

        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Overage"), "overage line missing");
        assert!(contents.contains("disabled"), "disabled status missing");
    }

    #[test]
    fn overage_budget_enabled_shows_used_value() {
        use agtop_core::quota::{Usage, UsageExtra};
        use indexmap::IndexMap;

        let backend = TestBackend::new(120, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        let mut extras: IndexMap<String, UsageExtra> = IndexMap::new();
        extras.insert(
            "extra_usage".into(),
            UsageExtra::OverageBudget {
                monthly_limit: Some(50.0),
                used: Some(12.34),
                utilization: Some(24.0),
                currency: Some("$".into()),
                enabled: true,
            },
        );
        let usage = Usage {
            windows: make_usage(&[("5h", 30.0)]).windows,
            models: Default::default(),
            extras,
        };
        let r = ok_result(ProviderId::Claude, usage);
        app.apply_quota_results(vec![r]);

        terminal
            .draw(|f| render(f, f.area(), &app))
            .expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("12.34"), "used value missing:\n{contents}");
        assert!(contents.contains("50.00"), "limit value missing:\n{contents}");
    }
```

- [ ] **Step 2: Run tests — expect success**

Run: `cargo test -p agtop-cli widgets::dashboard_plan::tests::overage_budget_disabled_renders_correctly widgets::dashboard_plan::tests::overage_budget_enabled_shows_used_value`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs
git commit -m "quota-tui(phase-4): test OverageBudget rendering (enabled/disabled)"
```

---

## Task 6: Clippy + final verification

- [ ] **Step 1: Run clippy**

Run: `cargo clippy -p agtop-cli -- -D warnings`
Expected: no warnings.

- [ ] **Step 2: Run full suite**

Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS (everything: Phase 1-4 tests, existing TUI tests).

- [ ] **Step 3: Commit any fixes**

```bash
git add -A
git commit -m "quota-tui(phase-4): final clippy pass"
```

Phase 4 complete. Dashboard pane now renders quota data with full detail on the right. Key bindings and pane-focus dispatch (Start/Stop commands) are wired up in Phase 5.
