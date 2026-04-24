//! Dashboard mode — Quota pane.
//!
//! Replaces the previous local-estimate Subscription Details pane. Driven
//! by `App::quota_slots` (populated by the refresh worker from
//! `agtop_core::quota::fetch_all`). 40 % left (compact list) / 60 % right
//! (full detail) split. See the 2026-04-22-quota-tui design spec.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::app::{App, QuotaState};
use crate::tui::theme as th;
use crate::tui::widgets::quota_bar::provider_short_name;
use agtop_core::quota::UsageWindow;
use indexmap::IndexMap;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let outer_block = Block::default().borders(Borders::ALL).title(" Quota ");
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    match app.quota_state() {
        QuotaState::Idle => {
            render_centered(frame, inner, "Press r to load quota data");
            return;
        }
        QuotaState::Loading => {
            render_centered(frame, inner, "Fetching quota data\u{2026}");
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
    // Add a 2-column left margin to the right pane so there is clear blank
    // space separating the provider list from the detail content.
    let detail_area = panes[1].inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    render_details(frame, detail_area, app);
}

fn render_centered(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let p = Paragraph::new(msg)
        .style(th::QUOTA_TITLE)
        .alignment(Alignment::Center);
    frame.render_widget(p, area);
}

fn render_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    use crate::tui::widgets::quota_bar::{error_token, provider_short_name, status_glyph};

    let slots = app.quota_slots();
    let selected = app.selected_provider();

    let mut lines: Vec<Line<'_>> = Vec::with_capacity(slots.len());
    for (i, slot) in slots.iter().enumerate() {
        let is_selected = i == selected;
        let stale = !slot.current.ok && slot.last_good.is_some();
        let errored = !slot.current.ok && slot.last_good.is_none();
        let loading = slot.current.usage.is_none() && slot.current.ok;

        let glyph = status_glyph(slot.current.ok, slot.last_good.is_some(), loading);
        let name_suffix = if stale { " \u{2020}" } else { "" };
        let name = slot
            .current
            .meta
            .get("plan")
            .map(String::as_str)
            .unwrap_or_else(|| provider_short_name(slot.current.provider_id));

        // Left column shows only the status glyph + plan name (no usage bar).
        // The bar/percentage live exclusively in the right detail pane.
        let status_text: Option<String> = if errored {
            let token = slot
                .current
                .error
                .as_ref()
                .map(error_token)
                .unwrap_or_else(|| "err".into());
            Some(format!(" — {token}"))
        } else if loading {
            Some(" — loading\u{2026}".into())
        } else {
            None
        };

        let line_text = format!(
            "{glyph} {name}{name_suffix}{}",
            status_text.as_deref().unwrap_or("")
        );
        let line = Line::from(Span::raw(line_text));

        let line = if is_selected {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|s| Span::styled(s.content.to_string(), s.style.patch(th::QUOTA_SELECTED)))
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

fn render_details(frame: &mut Frame<'_>, area: Rect, app: &App) {
    const BAR_WIDTH: usize = 10;
    const LABEL_WIDTH: usize = 16;

    let slots = app.quota_slots();
    let sel = app.selected_provider();
    let slot = match slots.get(sel) {
        Some(s) => s,
        None => return,
    };

    let stale = !slot.current.ok && slot.last_good.is_some();
    let error_only = !slot.current.ok && slot.last_good.is_none();

    // Reserve the last row for the "fetched at" footer (always pinned to bottom).
    // If the area is too small to split, render everything in place.
    let (content_area, footer_area) = if area.height >= 2 {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        (split[0], Some(split[1]))
    } else {
        (area, None)
    };

    let mut lines: Vec<Line<'_>> = Vec::new();

    // Header: subscription name (meta["plan"]) + login if available.
    // meta["plan"] is the full human-readable subscription label produced
    // by subscription.rs (e.g. "Max 5x", "GitHub Copilot · Individual").
    // Fall back to the short provider name when no plan is known yet.
    let plan_label = slot
        .current
        .meta
        .get("plan")
        .cloned()
        .unwrap_or_else(|| provider_short_name(slot.current.provider_id).to_string());
    let mut header_parts = vec![plan_label];
    if let Some(login) = slot.current.meta.get("login") {
        header_parts.push(login.clone());
    }
    lines.push(Line::from(Span::styled(
        header_parts.join(" \u{00b7} "),
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
            format!("! Stale — data from {fetched_at_str} \u{00b7} last error: {err_label}"),
            th::QUOTA_BAR_STALE,
        )));
    }

    lines.push(Line::from(""));

    // Effective usage (stale → last_good).
    let effective = if stale {
        slot.last_good.as_ref().unwrap_or(&slot.current)
    } else {
        &slot.current
    };

    let mut any_preview = false;

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
        frame.render_widget(p, content_area);
    } else {
        if let Some(usage) = effective.usage.as_ref() {
            for (label, w) in &usage.windows {
                lines.push(window_line(label, w, BAR_WIDTH, LABEL_WIDTH, stale));
            }

            if !usage.models.is_empty() {
                lines.push(Line::from(""));
                let (model_lines, preview) =
                    google_model_lines(&usage.models, BAR_WIDTH, LABEL_WIDTH, stale);
                any_preview = preview;
                lines.extend(model_lines);
            }

            // Extras.
            if !usage.extras.is_empty() {
                lines.push(Line::from(""));
                for (name, extra) in &usage.extras {
                    lines.push(extra_line(name, extra));
                }
            }
        }
        let visible_height = content_area.height as usize;
        let total = lines.len();
        let scroll = app.model_scroll().min(total.saturating_sub(visible_height));
        let has_overflow = total > visible_height;

        let visible: Vec<Line<'_>> = if has_overflow {
            lines
                .into_iter()
                .skip(scroll)
                .take(visible_height.saturating_sub(1))
                .collect()
        } else {
            lines
        };

        let mut render_lines = visible;
        if has_overflow {
            let can_up = scroll > 0;
            let can_down = scroll + visible_height.saturating_sub(1) < total;
            let up = if can_up { "\u{25b2}" } else { " " };
            let down = if can_down { "\u{25bc}" } else { " " };
            render_lines.push(Line::from(Span::raw(format!("{up} {down}"))));
        }

        let p = Paragraph::new(render_lines).wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(p, content_area);
    }

    if let Some(footer_rect) = footer_area {
        let fetched = format!("fetched at {}", format_epoch_ms(effective.fetched_at));
        let footnote = if any_preview { "* preview" } else { "" };
        let footer_text = format!("{footnote:<20}{fetched:>}");
        let footer_p = Paragraph::new(Line::from(Span::styled(footer_text, th::PLAN_NOTE)))
            .alignment(Alignment::Left);
        frame.render_widget(footer_p, footer_rect);
    }
}

fn window_line<'a>(
    label: &str,
    w: &'a agtop_core::quota::UsageWindow,
    bar_width: usize,
    label_width: usize,
    stale: bool,
) -> Line<'a> {
    use crate::tui::widgets::quota_bar::bar_spans;
    match w.used_percent {
        Some(p) => {
            let [filled, empty] = bar_spans(Some(p), bar_width, stale);
            Line::from(vec![
                Span::raw(format!("{label:<label_width$}")),
                filled,
                empty,
                Span::raw(format!("  {p:>3.0}%  {}", reset_suffix(w))),
            ])
        }
        None => {
            let text = w.value_label.clone().unwrap_or_else(|| "—".into());
            Line::from(Span::raw(format!(
                "{label:<label_width$}  {text}  {}",
                reset_suffix(w)
            )))
        }
    }
}

fn reset_suffix(w: &agtop_core::quota::UsageWindow) -> String {
    use chrono::Utc;
    match w.reset_at {
        None => "".into(),
        Some(ms) => {
            let now_ms = Utc::now().timestamp_millis();
            let delta = ms - now_ms;
            if delta < 0 {
                return "resets in ??h ??m".into();
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
            let pct = utilization
                .map(|u| format!(" ({u:.0}%)"))
                .unwrap_or_default();
            let status = if *enabled {
                format!("enabled \u{00b7} {cur}{used_v:.2} used of {cur}{limit:.2}{pct}")
            } else {
                format!("disabled \u{00b7} limit {cur}{limit:.2}")
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
                .join(" \u{00b7} ");
            Line::from(Span::raw(format!("{name}  {s}")))
        }
    }
}

fn format_epoch_ms(ms: i64) -> String {
    use chrono::{DateTime, Local, Utc};
    let utc: DateTime<Utc> = DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default();
    let local: DateTime<Local> = utc.into();
    local.format("%H:%M").to_string()
}

fn short_model_name(key: &str) -> (String, bool) {
    // Strip scope prefix  ("gemini/gemini-2.5-flash" → "gemini-2.5-flash")
    let s = key.strip_prefix("gemini/").unwrap_or(key);
    let is_preview = s.ends_with("-preview");
    let s = s.strip_suffix("-preview").unwrap_or(s);
    // Strip redundant model-family prefix ("gemini-2.5-flash" → "2.5-flash")
    let s = s.strip_prefix("gemini-").unwrap_or(s);
    // Split into segments and classify: leading numeric/dot segments are the
    // version, the rest are the variant name (Flash, Pro, Lite, ...).
    let segments: Vec<&str> = s.split('-').collect();
    let mut version_parts: Vec<&str> = Vec::new();
    let mut name_parts: Vec<String> = Vec::new();
    let mut past_version = false;
    for seg in &segments {
        if !past_version && seg.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            version_parts.push(seg);
        } else {
            past_version = true;
            // Title-case the segment.
            let mut chars = seg.chars();
            let titled = match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + &chars.as_str().to_lowercase()
                }
            };
            name_parts.push(titled);
        }
    }
    // Reorder: variant first, then version  ("Flash 2.5 Lite" not "2.5 Flash Lite")
    let version = version_parts.join(".");
    let name = if name_parts.is_empty() {
        version.clone()
    } else if version.is_empty() {
        name_parts.join(" ")
    } else {
        // Insert version after the first variant word.
        let first = &name_parts[0];
        let rest = &name_parts[1..];
        let mut out = format!("{first} {version}");
        for r in rest {
            out.push(' ');
            out.push_str(r);
        }
        out
    };
    (name, is_preview)
}

fn google_model_lines<'a>(
    models: &'a IndexMap<String, IndexMap<String, UsageWindow>>,
    bar_width: usize,
    label_width: usize,
    stale: bool,
) -> (Vec<Line<'a>>, bool) {
    let mut any_preview = false;
    let mut rows = Vec::new();

    for (model_key, windows) in models {
        let (short, is_preview) = short_model_name(model_key);
        if is_preview {
            any_preview = true;
        }
        let display = if is_preview {
            format!("{short} *")
        } else {
            short
        };
        for (_wlabel, w) in windows {
            rows.push(window_line(&display, w, bar_width, label_width, stale));
        }
    }

    (rows, any_preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::quota::{ProviderId, ProviderResult, Usage, UsageWindow};
    use indexmap::IndexMap;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn reset_time_str(w: &agtop_core::quota::UsageWindow) -> String {
        use chrono::{Local, TimeZone, Utc};
        match w.reset_at {
            None => String::new(),
            Some(0) | Some(1) => "\u{2014}".into(),
            Some(ms) => {
                let utc = Utc.timestamp_millis_opt(ms).single();
                match utc {
                    None => "\u{2014}".into(),
                    Some(dt) => {
                        let local = dt.with_timezone(&Local);
                        let time = local.format("%-I:%M %p").to_string();
                        let now_ms = Utc::now().timestamp_millis();
                        let delta_secs = (ms - now_ms) / 1000;
                        let countdown = if delta_secs <= 0 {
                            "any moment".to_string()
                        } else {
                            let h = delta_secs / 3600;
                            let m = (delta_secs % 3600) / 60;
                            if h == 0 {
                                format!("{m}m")
                            } else {
                                format!("{h}h {m}m")
                            }
                        };
                        let day_note = {
                            let now_local = Local::now();
                            let local_date = local.date_naive();
                            let today = now_local.date_naive();
                            let diff = (local_date - today).num_days();
                            if diff == 0 {
                                String::new()
                            } else if diff == 1 {
                                " tomorrow".to_string()
                            } else {
                                format!(" {}d", diff)
                            }
                        };
                        format!("{time} ({countdown}{})", day_note.trim_start())
                    }
                }
            }
        }
    }

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
    fn short_model_name_known_keys() {
        let cases = vec![
            ("gemini/gemini-2.5-flash", "Flash 2.5", false),
            ("gemini/gemini-2.5-flash-lite", "Flash 2.5 Lite", false),
            ("gemini/gemini-2.5-pro", "Pro 2.5", false),
            ("gemini/gemini-3-flash-preview", "Flash 3", true),
            ("gemini/gemini-3-pro-preview", "Pro 3", true),
            (
                "gemini/gemini-3.1-flash-lite-preview",
                "Flash 3.1 Lite",
                true,
            ),
            ("gemini/gemini-3.1-pro-preview", "Pro 3.1", true),
        ];
        for (input, expected_name, expected_preview) in cases {
            let (name, is_preview) = short_model_name(input);
            assert_eq!(name, expected_name, "name for {input}");
            assert_eq!(is_preview, expected_preview, "preview for {input}");
        }
    }

    #[test]
    fn short_model_name_unknown_fallback() {
        let (name, is_preview) = short_model_name("gemini/gemini-4-nano-finetune");
        assert_eq!(name, "Nano 4 Finetune");
        assert!(!is_preview);
    }

    #[test]
    fn short_model_name_unprefixed() {
        let (name, _) = short_model_name("gemini-2.5-flash");
        assert_eq!(name, "Flash 2.5");
    }

    #[test]
    fn short_model_name_already_bare() {
        let (name, _) = short_model_name("custom-model");
        assert_eq!(name, "Custom Model");
    }

    #[test]
    fn reset_time_str_epoch_zero_is_dash() {
        let w = UsageWindow {
            used_percent: Some(0.0),
            window_seconds: None,
            reset_at: Some(0),
            value_label: None,
        };
        assert_eq!(reset_time_str(&w), "—");
    }

    #[test]
    fn reset_time_str_none_is_empty() {
        let w = UsageWindow {
            used_percent: None,
            window_seconds: None,
            reset_at: None,
            value_label: None,
        };
        assert_eq!(reset_time_str(&w), "");
    }

    #[test]
    fn reset_time_str_future_shows_time_and_countdown() {
        let now = chrono::Utc::now().timestamp_millis();
        let in_2h = now + 2 * 3600 * 1000 + 45 * 60 * 1000;
        let w = UsageWindow {
            used_percent: Some(50.0),
            window_seconds: None,
            reset_at: Some(in_2h),
            value_label: None,
        };
        let s = reset_time_str(&w);
        assert!(
            s.contains('(') && s.contains(')'),
            "expected time + countdown in parens, got: {s}"
        );
        assert!(s.contains("2h"), "expected '2h' in countdown, got: {s}");
    }

    #[test]
    fn loading_state_shows_placeholder() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.set_quota_loading();
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Fetching"),
            "loading placeholder missing:\n{contents}"
        );
    }

    #[test]
    fn error_state_shows_error_message() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.set_quota_error("connection refused".into());
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("connection refused"),
            "error message missing:\n{contents}"
        );
    }

    #[test]
    fn idle_state_shows_placeholder() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new();
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
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
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Quota"), "block title missing");
    }

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
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Claude"));
        assert!(contents.contains("5h"));
        assert!(contents.contains("72"));
        assert!(contents.contains('\u{25a0}'));
        assert!(
            contents.contains('\u{25cf}'),
            "ok glyph missing:\n{contents}"
        );
    }

    #[test]
    fn left_list_shows_error_provider() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.apply_quota_results(vec![err_result(ProviderId::Google, 401)]);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Google"));
        assert!(contents.contains("401"));
        assert!(
            contents.contains('\u{2717}'),
            "error glyph missing:\n{contents}"
        );
    }

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
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("5h"), "5h missing");
        assert!(contents.contains("7d"), "7d missing");
        assert!(contents.contains("sonnet"), "7d-sonnet missing");
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
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
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
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Error"),
            "error heading missing:\n{contents}"
        );
        assert!(contents.contains("401"), "status code missing:\n{contents}");
    }

    #[test]
    fn google_provider_renders_per_model() {
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

        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Pro 2.5"),
            "short model name missing:\n{contents}"
        );
        assert!(contents.contains("31"), "percentage missing:\n{contents}");
    }

    #[test]
    fn google_table_has_no_column_header() {
        // Google detail pane should use the same window_line format as all
        // other providers -- no separate column headers.
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let usage = make_google_table_usage();
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        // The old column-header row ("Model  Usage  Resets") should not appear.
        // Note: "Resets" could only have come from the old header since
        // window_line uses "resets in ..." (lowercase) for the countdown.
        assert!(
            !contents.contains("Resets"),
            "column header row should not appear:\n{contents}"
        );
    }

    #[test]
    fn google_table_shows_short_names() {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let usage = make_google_table_usage();
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Flash 2.5"),
            "short name missing:\n{contents}"
        );
        assert!(
            contents.contains("Pro 2.5"),
            "short name missing:\n{contents}"
        );
        assert!(
            !contents.contains("gemini/gemini-"),
            "raw scoped keys should not appear:\n{contents}"
        );
    }

    #[test]
    fn google_table_shows_preview_asterisk() {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let usage = make_google_table_usage();
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("Flash 3 *"),
            "preview asterisk missing:\n{contents}"
        );
        assert!(
            contents.contains("Pro 3 *"),
            "preview asterisk missing:\n{contents}"
        );
    }

    #[test]
    fn google_table_shows_preview_footnote() {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let usage = make_google_table_usage();
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("preview"),
            "footnote missing:\n{contents}"
        );
    }

    #[test]
    fn google_table_no_footnote_when_no_previews() {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        let mut m1: IndexMap<String, UsageWindow> = IndexMap::new();
        m1.insert(
            "daily".into(),
            UsageWindow {
                used_percent: Some(50.0),
                window_seconds: Some(86400),
                reset_at: None,
                value_label: None,
            },
        );
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        models.insert("gemini/gemini-2.5-flash".into(), m1);

        let usage = Usage {
            windows: Default::default(),
            models,
            extras: Default::default(),
        };
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            !contents.contains("* preview"),
            "footnote should not appear without preview models:\n{contents}"
        );
    }

    fn make_google_table_usage() -> Usage {
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        let entries = vec![
            ("gemini/gemini-2.5-flash", 7.0),
            ("gemini/gemini-2.5-pro", 100.0),
            ("gemini/gemini-3-flash-preview", 7.0),
            ("gemini/gemini-3-pro-preview", 100.0),
        ];
        for (key, pct) in entries {
            let mut wins: IndexMap<String, UsageWindow> = IndexMap::new();
            wins.insert(
                "daily".into(),
                UsageWindow {
                    used_percent: Some(pct),
                    window_seconds: Some(86400),
                    reset_at: None,
                    value_label: None,
                },
            );
            models.insert(key.into(), wins);
        }
        Usage {
            windows: Default::default(),
            models,
            extras: Default::default(),
        }
    }

    #[test]
    fn google_table_scroll_hides_overflow_rows() {
        let backend = TestBackend::new(120, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let usage = make_google_table_usage();
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);
        app.set_model_scroll_for_test(2);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            !contents.contains("Flash 2.5")
                || contents.lines().take(4).any(|l| !l.contains("Flash 2.5")),
            "scrolled rows should not be visible at top:\n{contents}"
        );
    }

    #[test]
    fn google_table_scroll_shows_indicators() {
        let backend = TestBackend::new(120, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        let usage = make_google_table_usage();
        let r = ok_result(ProviderId::Google, usage);
        app.apply_quota_results(vec![r]);
        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains('\u{25bc}') || contents.contains('\u{25b2}'),
            "scroll indicators missing when models overflow:\n{contents}"
        );
    }

    #[test]
    fn overage_budget_disabled_renders_correctly() {
        use agtop_core::quota::UsageExtra;

        let backend = TestBackend::new(120, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        let mut extras: IndexMap<String, agtop_core::quota::UsageExtra> = IndexMap::new();
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

        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(contents.contains("Overage"), "overage line missing");
        assert!(contents.contains("disabled"), "disabled status missing");
    }

    #[test]
    fn overage_budget_enabled_shows_used_value() {
        use agtop_core::quota::UsageExtra;

        let backend = TestBackend::new(120, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();

        let mut extras: IndexMap<String, agtop_core::quota::UsageExtra> = IndexMap::new();
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

        terminal.draw(|f| render(f, f.area(), &app)).expect("draw");
        let contents = buffer_to_string(terminal.backend().buffer());
        assert!(
            contents.contains("12.34"),
            "used value missing:\n{contents}"
        );
        assert!(
            contents.contains("50.00"),
            "limit value missing:\n{contents}"
        );
    }
}
