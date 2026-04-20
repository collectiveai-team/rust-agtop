use chrono::{DateTime, Local, Utc};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::tui::app::App;
use crate::tui::theme as th;
use agtop_core::session::{ClientKind, PlanUsage};

// ---------------------------------------------------------------------------
// Merged subscription data
// ---------------------------------------------------------------------------

/// One subscription entry after deduplication across clients/agents.
struct MergedPlan<'a> {
    subscription_name: String,
    windows: Vec<&'a agtop_core::session::PlanWindow>,
    last_limit_hit: Option<DateTime<Utc>>,
    notes: Vec<String>,
    clients: Vec<ClientKind>,
}

/// Strip known agent-tool suffixes/prefixes to derive a canonical subscription name.
fn canonical_name(pu: &PlanUsage) -> String {
    // Prefer the structured plan_name field.
    if let Some(name) = &pu.plan_name {
        if !name.is_empty() {
            return name.clone();
        }
    }
    // Fall back to stripping known agent labels from `pu.label`.
    let s = pu.label.as_str();
    // Strip " via <agent>" suffix.
    for suffix in &[" via Claude Code", " via OpenCode", " via Codex"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return stripped.trim().to_string();
        }
    }
    // Strip "<agent> · " prefix.
    for prefix in &["Claude Code · ", "OpenCode · ", "Codex · "] {
        if let Some(stripped) = s.strip_prefix(prefix) {
            return stripped.trim().to_string();
        }
    }
    s.trim().to_string()
}

/// Merge a slice of `PlanUsage` entries into deduplicated `MergedPlan` entries.
/// Entries with the same canonical name (case-insensitive) are merged.
/// For duplicate window labels, keep the window with the most recent reset_at.
fn merge_plans(usages: &[PlanUsage]) -> Vec<MergedPlan<'_>> {
    // We preserve insertion order (first occurrence of a subscription name).
    let mut order: Vec<String> = Vec::new(); // canonical names in order
    let mut map: std::collections::HashMap<String, MergedPlan<'_>> =
        std::collections::HashMap::new();

    for pu in usages {
        let display = canonical_name(pu);
        let key = display.to_lowercase();

        if !map.contains_key(&key) {
            order.push(key.clone());
            map.insert(
                key.clone(),
                MergedPlan {
                    subscription_name: display,
                    windows: Vec::new(),
                    last_limit_hit: pu.last_limit_hit,
                    notes: Vec::new(),
                    clients: vec![pu.client],
                },
            );
        }

        let entry = map.get_mut(&key).unwrap();

        if !entry.clients.contains(&pu.client) {
            entry.clients.push(pu.client);
        }

        // Update last_limit_hit to the most recent.
        if let Some(lh) = pu.last_limit_hit {
            entry.last_limit_hit = Some(match entry.last_limit_hit {
                Some(existing) if existing >= lh => existing,
                _ => lh,
            });
        }

        // Merge notes (dedup).
        if let Some(note) = &pu.note {
            if !note.is_empty() && !entry.notes.contains(note) {
                entry.notes.push(note.clone());
            }
        }

        // Merge windows: if a window with the same label already exists,
        // keep the one with the more recent reset_at (or replace if new has
        // reset_at and existing doesn't).
        for w in &pu.windows {
            let existing_pos = entry.windows.iter().position(|e| e.label == w.label);
            match existing_pos {
                None => entry.windows.push(w),
                Some(pos) => {
                    let keep_new = match (entry.windows[pos].reset_at, w.reset_at) {
                        (None, Some(_)) => true,
                        (Some(existing_ts), Some(new_ts)) => new_ts > existing_ts,
                        _ => false,
                    };
                    if keep_new {
                        entry.windows[pos] = w;
                    }
                }
            }
        }
    }

    // Sort windows within each merged entry: nearest reset_at first, None last.
    for key in &order {
        let entry = map.get_mut(key).unwrap();
        entry
            .windows
            .sort_by(|a, b| match (a.reset_at, b.reset_at) {
                (Some(ta), Some(tb)) => ta.cmp(&tb),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
    }

    order.into_iter().map(|k| map.remove(&k).unwrap()).collect()
}

// ---------------------------------------------------------------------------
// Bar helpers
// ---------------------------------------------------------------------------

/// Choose the style for a bar based on utilization thresholds.
/// green < 0.30, yellow 0.30–0.80, red ≥ 0.80, dim for None.
fn bar_style(util: Option<f64>) -> Style {
    match util {
        None => th::PLAN_NOTE,
        Some(u) if u < 0.30 => th::PLAN_BAR_GREEN,
        Some(u) if u < 0.80 => th::PLAN_BAR_YELLOW,
        _ => th::PLAN_BAR_RED,
    }
}

/// Build a fixed-width bar as two styled `Span`s (filled + empty).
/// Width is the total char width of the bar.
fn bar_spans(util: Option<f64>, width: usize) -> [Span<'static>; 2] {
    let filled = util
        .map(|u| (u.clamp(0.0, 1.0) * width as f64).round() as usize)
        .unwrap_or(0);
    let empty = width.saturating_sub(filled);
    let style = bar_style(util);
    [
        Span::styled("█".repeat(filled), style),
        Span::styled("░".repeat(empty), th::PLAN_NOTE),
    ]
}

/// Format a DateTime<Utc> into local time as "Mon DD, HH:MM AM/PM".
fn format_local(ts: DateTime<Utc>) -> String {
    let local: DateTime<Local> = ts.into();
    local.format("%a %b %-d, %-I:%M %p").to_string()
}

fn relative_since(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

// ---------------------------------------------------------------------------
// Render entry point
// ---------------------------------------------------------------------------

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(" Subscription Details ");
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    let merged = merge_plans(app.plan_usage());

    if merged.is_empty() {
        let p = Paragraph::new(Span::styled("(no subscription data)", th::PLAN_EMPTY));
        frame.render_widget(p, inner);
        return;
    }

    // Split inner area: 40% list | 60% details.
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(inner);

    // Clamp selected against merged list length (raw count may exceed merged count).
    let selected = app.plan_selected().min(merged.len().saturating_sub(1));

    render_list(frame, panes[0], &merged, selected, app);
    render_details(frame, panes[1], &merged, selected, app);
}

// ---------------------------------------------------------------------------
// Left pane: subscription list
// ---------------------------------------------------------------------------

fn render_list(
    frame: &mut Frame<'_>,
    area: Rect,
    merged: &[MergedPlan<'_>],
    selected: usize,
    app: &App,
) {
    const BAR_WIDTH: usize = 20;

    let items: Vec<ListItem> = merged
        .iter()
        .enumerate()
        .map(|(i, mp)| {
            let util = mp
                .windows
                .iter()
                .filter_map(|w| w.reset_at.map(|t| (t, w.utilization)))
                .min_by_key(|(t, _)| *t)
                .and_then(|(_, util)| util)
                .or_else(|| mp.windows.iter().find_map(|w| w.utilization));

            let pct_str = util
                .map(|u| format!("{:>3.0}%", u * 100.0))
                .unwrap_or_else(|| "  - ".to_string());

            let name_style = if i == selected {
                th::PLAN_SELECTED
            } else {
                th::PLAN_LABEL
            };

            let [filled_span, empty_span] = bar_spans(util, BAR_WIDTH);

            let has_logo = mp
                .clients
                .first()
                .map(|c| app.logo(*c).is_some())
                .unwrap_or(false);
            let name_prefix = if has_logo { "   " } else { "  " };

            let bar_line = Line::from(vec![
                Span::raw("  "),
                filled_span,
                empty_span,
                Span::raw(format!(" {pct_str}")),
            ]);

            ListItem::new(vec![
                Line::from(vec![
                    Span::raw(name_prefix),
                    Span::styled(mp.subscription_name.clone(), name_style),
                ]),
                bar_line,
            ])
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);

    for (i, mp) in merged.iter().enumerate() {
        if let Some(client) = mp.clients.first() {
            if let Some(proto) = app.logo(*client) {
                let y = area.y + (i as u16) * 2;
                if y + 1 > area.y + area.height {
                    break;
                }
                let logo_rect = ratatui::layout::Rect {
                    x: area.x + 1,
                    y,
                    width: 1,
                    height: 1,
                };
                let img_widget = ratatui_image::Image::new(proto);
                frame.render_widget(img_widget, logo_rect);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Right pane: details
// ---------------------------------------------------------------------------

fn render_details(
    frame: &mut Frame<'_>,
    area: Rect,
    merged: &[MergedPlan<'_>],
    selected: usize,
    app: &App,
) {
    let now = Utc::now();
    let mp = match merged.get(selected) {
        Some(m) => m,
        None => return,
    };

    let bar_width = (area.width as usize).saturating_sub(4).max(4);

    let mut lines: Vec<Line<'static>> = Vec::new();

    let mut header_spans: Vec<Span<'static>> = Vec::new();
    if let Some(client) = mp.clients.first() {
        if let Some(proto) = app.logo(*client) {
            let logo_rect = ratatui::layout::Rect {
                x: area.x + 1,
                y: area.y,
                width: 1,
                height: 1,
            };
            let img_widget = ratatui_image::Image::new(proto);
            frame.render_widget(img_widget, logo_rect);
            header_spans.push(Span::raw(" "));
        }
    }
    header_spans.push(Span::styled(mp.subscription_name.clone(), th::PLAN_LABEL));
    lines.push(Line::from(header_spans));
    lines.push(Line::from(""));

    for w in &mp.windows {
        // Note: `w.binding` is not displayed here; windows are ordered by reset_at
        // so the binding window typically appears first naturally.
        // Label + percentage on one line, right-aligned pct.
        let pct_str = w
            .utilization
            .map(|u| format!("{:.0}%", u * 100.0))
            .unwrap_or_else(|| "-".to_string());
        let line_width = (area.width as usize).saturating_sub(2);
        let label_width = line_width.saturating_sub(pct_str.len() + 1);
        let label_line = format!(
            "  {:<label_width$} {pct_str}",
            w.label,
            label_width = label_width,
        );
        lines.push(Line::from(Span::styled(label_line, Style::new())));

        // Bar line.
        let [filled_span, empty_span] = bar_spans(w.utilization, bar_width);
        lines.push(Line::from(vec![Span::raw("  "), filled_span, empty_span]));

        // Reset line.
        let reset_text = if let Some(ts) = w.reset_at {
            format!("  Resets: {}", format_local(ts))
        } else if let Some(hint) = &w.reset_hint {
            format!("  {hint}")
        } else {
            "  (no reset time)".to_string()
        };
        lines.push(Line::from(Span::styled(reset_text, th::PLAN_NOTE)));
        lines.push(Line::from(""));
    }

    if let Some(ts) = mp.last_limit_hit {
        lines.push(Line::from(Span::styled(
            format!("  Last limit hit: {} ago", relative_since(ts, now)),
            th::PLAN_NOTE,
        )));
    }

    for note in &mp.notes {
        lines.push(Line::from(Span::styled(format!("  {note}"), th::PLAN_NOTE)));
    }

    let p = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{ClientKind, PlanUsage, PlanWindow};

    fn make_pu(label: &str, plan_name: Option<&str>) -> PlanUsage {
        PlanUsage::new(
            ClientKind::Claude,
            label.to_string(),
            plan_name.map(|s| s.to_string()),
            Vec::new(),
            None,
            None,
        )
    }

    #[test]
    fn canonical_name_prefers_plan_name() {
        let pu = make_pu("Claude Code · Max 5x", Some("max_5x"));
        assert_eq!(canonical_name(&pu), "max_5x");
    }

    #[test]
    fn canonical_name_strips_via_suffix() {
        let pu = make_pu("Max 5x via Claude Code", None);
        assert_eq!(canonical_name(&pu), "Max 5x");
    }

    #[test]
    fn canonical_name_strips_agent_prefix() {
        let pu = make_pu("Claude Code · Max 5x", None);
        assert_eq!(canonical_name(&pu), "Max 5x");
    }

    #[test]
    fn canonical_name_strips_opencode_prefix() {
        let pu = make_pu("OpenCode · Max 5x", None);
        assert_eq!(canonical_name(&pu), "Max 5x");
    }

    #[test]
    fn merge_deduplicates_same_subscription() {
        let pu1 = make_pu("Max 5x via Claude Code", None);
        let pu2 = make_pu("Max 5x via OpenCode", None);
        let plans = [pu1, pu2];
        let merged = merge_plans(&plans);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].subscription_name, "Max 5x");
    }

    #[test]
    fn merge_keeps_two_different_subscriptions() {
        let pu1 = make_pu("Max 5x via Claude Code", None);
        let pu2 = make_pu("ChatGPT Plus", None);
        let plans = [pu1, pu2];
        let merged = merge_plans(&plans);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_window_dedup_keeps_newer_reset() {
        use chrono::TimeZone;

        let t1 = Utc.with_ymd_and_hms(2026, 4, 18, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();

        let mut pu1 = make_pu("Max via Claude Code", None);
        pu1.windows.push(PlanWindow::new(
            "5h".to_string(),
            Some(0.5),
            Some(t1),
            None,
            false,
        ));

        let mut pu2 = make_pu("Max via OpenCode", None);
        pu2.windows.push(PlanWindow::new(
            "5h".to_string(),
            Some(0.7),
            Some(t2),
            None,
            false,
        ));

        let plans = [pu1, pu2];
        let merged = merge_plans(&plans);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].windows.len(), 1);
        // t2 is more recent, so the window with utilization 0.7 should be kept.
        assert!((merged[0].windows[0].utilization.unwrap() - 0.7).abs() < 1e-9);
    }

    #[test]
    fn bar_style_green_below_30() {
        assert_eq!(bar_style(Some(0.29)), th::PLAN_BAR_GREEN);
        assert_eq!(bar_style(Some(0.0)), th::PLAN_BAR_GREEN);
    }

    #[test]
    fn bar_style_yellow_30_to_80() {
        assert_eq!(bar_style(Some(0.30)), th::PLAN_BAR_YELLOW);
        assert_eq!(bar_style(Some(0.79)), th::PLAN_BAR_YELLOW);
    }

    #[test]
    fn bar_style_red_at_or_above_80() {
        assert_eq!(bar_style(Some(0.80)), th::PLAN_BAR_RED);
        assert_eq!(bar_style(Some(1.0)), th::PLAN_BAR_RED);
    }

    #[test]
    fn bar_spans_correct_char_counts() {
        let [filled, empty] = bar_spans(Some(0.5), 10);
        // 50% of 10 = 5 filled, 5 empty.
        assert_eq!(filled.content.chars().count(), 5);
        assert_eq!(empty.content.chars().count(), 5);
    }

    #[test]
    fn bar_style_none_is_dim() {
        assert_eq!(bar_style(None), th::PLAN_NOTE);
    }

    #[test]
    fn canonical_name_passthrough_when_no_prefix_or_suffix() {
        let pu = make_pu("Pro Plan", None);
        assert_eq!(canonical_name(&pu), "Pro Plan");
    }

    #[test]
    fn canonical_name_empty_plan_name_falls_through_to_label() {
        // plan_name is Some("") — should fall through to label stripping.
        let pu = make_pu("Claude Code · Max 5x", Some(""));
        assert_eq!(canonical_name(&pu), "Max 5x");
    }

    #[test]
    fn bar_spans_rounding() {
        // 33% of 10 = 3.3 → rounds to 3 filled, 7 empty.
        let [filled, empty] = bar_spans(Some(0.33), 10);
        assert_eq!(filled.content.chars().count(), 3);
        assert_eq!(empty.content.chars().count(), 7);
    }
}
