//! Classic mode — Quota tab body.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::*,
    widgets::Paragraph,
};

use crate::tui::app::{quota::preferred_window, App, ProviderSlot, QuotaState};
use crate::tui::theme as th;
use crate::tui::widgets::quota_bar::{bar_spans, error_token};

/// Fixed card slot width (including gutter).
pub const CARD_SLOT_WIDTH: u16 = 20;
/// Width of the bar inside a card, in cells.
const CARD_BAR_WIDTH: usize = 6;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
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

    let mut constraints: Vec<Constraint> = visible
        .iter()
        .map(|_| Constraint::Length(CARD_SLOT_WIDTH))
        .collect();
    constraints.push(Constraint::Min(0));

    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, slot) in visible.iter().enumerate() {
        render_card(frame, cells[i], slot);
    }

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
    let p = Paragraph::new(msg)
        .style(th::QUOTA_TITLE)
        .alignment(Alignment::Center);
    frame.render_widget(p, area);
}

fn render_indicator(frame: &mut Frame<'_>, area: Rect, ch: char, alignment: Alignment) {
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

pub(crate) fn build_card_lines<'a>(slot: &'a ProviderSlot) -> (Line<'a>, Line<'a>, Option<Style>) {
    let provider_name = slot.current.provider_name;
    let stale = !slot.current.ok && slot.last_good.is_some();
    let errored = !slot.current.ok && slot.last_good.is_none();

    let glyph_suffix = if stale {
        " †"
    } else if errored {
        " ✗"
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
                        let label_text = w.value_label.clone().unwrap_or_else(|| "—".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::quota::{
        ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageWindow,
    };
    use indexmap::IndexMap;
    use std::collections::BTreeMap;

    fn make_usage(windows: &[(&str, f64)]) -> Usage {
        let mut map: IndexMap<String, UsageWindow> = IndexMap::new();
        for (k, v) in windows {
            map.insert(
                (*k).to_string(),
                UsageWindow {
                    used_percent: Some(*v),
                    window_seconds: None,
                    reset_at: None,
                    value_label: None,
                },
            );
        }
        Usage {
            windows: map,
            models: Default::default(),
            extras: Default::default(),
        }
    }

    fn ok_slot(provider_id: ProviderId, name: &'static str, usage: Usage) -> ProviderSlot {
        let result = ProviderResult::ok(provider_id, name, usage, BTreeMap::new());
        ProviderSlot::new(result)
    }

    fn err_slot(provider_id: ProviderId, name: &'static str, kind: ErrorKind) -> ProviderSlot {
        let result = ProviderResult::err(
            provider_id,
            name,
            QuotaError {
                kind,
                detail: String::new(),
            },
        );
        ProviderSlot::new(result)
    }

    // Helper to extract plain text from a Line
    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn ok_slot_shows_provider_name() {
        let usage = make_usage(&[("5h", 42.0)]);
        let slot = ok_slot(ProviderId::Claude, "Claude", usage);
        let (name_line, _, _) = build_card_lines(&slot);
        assert!(line_text(&name_line).contains("Claude"));
    }

    #[test]
    fn ok_slot_shows_percentage_in_value_line() {
        let usage = make_usage(&[("5h", 42.0)]);
        let slot = ok_slot(ProviderId::Claude, "Claude", usage);
        let (_, value_line, _) = build_card_lines(&slot);
        let text = line_text(&value_line);
        assert!(text.contains("42"), "expected percentage in: {text}");
    }

    #[test]
    fn errored_slot_shows_error_token() {
        let slot = err_slot(
            ProviderId::Claude,
            "Claude",
            ErrorKind::Http {
                status: 401,
                retry_after: None,
            },
        );
        let (_, value_line, _) = build_card_lines(&slot);
        let text = line_text(&value_line);
        assert!(text.contains("401"), "expected error token in: {text}");
    }

    #[test]
    fn errored_slot_name_has_cross_glyph() {
        let slot = err_slot(ProviderId::Claude, "Claude", ErrorKind::Transport);
        let (name_line, _, _) = build_card_lines(&slot);
        let text = line_text(&name_line);
        assert!(text.contains('✗'), "expected ✗ in: {text}");
    }

    #[test]
    fn stale_slot_name_has_dagger() {
        let usage = make_usage(&[("5h", 10.0)]);
        let mut slot = ok_slot(ProviderId::Claude, "Claude", usage);
        // Make it stale: last_good is Some, current is error
        let err_result = ProviderResult::err(
            ProviderId::Claude,
            "Claude",
            QuotaError {
                kind: ErrorKind::Transport,
                detail: String::new(),
            },
        );
        slot.upsert(err_result);
        let (name_line, _, style) = build_card_lines(&slot);
        let text = line_text(&name_line);
        assert!(text.contains('†'), "expected † in: {text}");
        assert!(style.is_some(), "expected stale style override");
    }

    #[test]
    fn stale_slot_shows_last_good_percentage() {
        let usage = make_usage(&[("5h", 77.0)]);
        let mut slot = ok_slot(ProviderId::Claude, "Claude", usage);
        let err_result = ProviderResult::err(
            ProviderId::Claude,
            "Claude",
            QuotaError {
                kind: ErrorKind::Transport,
                detail: String::new(),
            },
        );
        slot.upsert(err_result);
        let (_, value_line, _) = build_card_lines(&slot);
        let text = line_text(&value_line);
        assert!(text.contains("77"), "expected last-good pct in: {text}");
    }

    #[test]
    fn no_windows_shows_dash() {
        let usage = make_usage(&[]);
        let slot = ok_slot(ProviderId::Claude, "Claude", usage);
        let (_, value_line, _) = build_card_lines(&slot);
        let text = line_text(&value_line);
        assert_eq!(text, "—");
    }

    #[test]
    fn usable_card_count_at_least_one() {
        assert_eq!(usable_card_count(0), 1);
        assert_eq!(usable_card_count(10), 1);
        assert_eq!(usable_card_count(CARD_SLOT_WIDTH), 1);
        assert_eq!(usable_card_count(CARD_SLOT_WIDTH * 3), 3);
    }

    #[test]
    fn transport_error_shows_net_token() {
        let slot = err_slot(ProviderId::Codex, "Codex", ErrorKind::Transport);
        let (_, value_line, _) = build_card_lines(&slot);
        let text = line_text(&value_line);
        assert!(text.contains("net"), "expected 'net' in: {text}");
    }
}
