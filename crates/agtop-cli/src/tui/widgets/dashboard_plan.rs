use chrono::{DateTime, Local, Utc};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::tui::app::App;
use crate::tui::theme as th;
use agtop_core::session::PlanUsage;

// ---------------------------------------------------------------------------
// Merged subscription data
// ---------------------------------------------------------------------------

/// One subscription entry after deduplication across providers/agents.
struct MergedPlan<'a> {
    /// Human-readable subscription name ("Max 5x", "ChatGPT Plus", …).
    subscription_name: String,
    /// Windows from all sources, deduplicated by label (most-recent reset_at wins).
    windows: Vec<&'a agtop_core::session::PlanWindow>,
    /// Most recent limit-hit moment across all merged sources.
    last_limit_hit: Option<DateTime<Utc>>,
    /// Notes from all sources, deduplicated.
    notes: Vec<String>,
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
        let key = canonical_name(pu).to_lowercase();
        let display = canonical_name(pu);

        if !map.contains_key(&key) {
            order.push(key.clone());
            map.insert(
                key.clone(),
                MergedPlan {
                    subscription_name: display,
                    windows: Vec::new(),
                    last_limit_hit: pu.last_limit_hit,
                    notes: Vec::new(),
                },
            );
        }

        let entry = map.get_mut(&key).unwrap();

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

    render_list(frame, panes[0], &merged, app.plan_selected());
    render_details(frame, panes[1], &merged, app.plan_selected());
}

// ---------------------------------------------------------------------------
// Left pane: subscription list
// ---------------------------------------------------------------------------

fn render_list(frame: &mut Frame<'_>, area: Rect, merged: &[MergedPlan<'_>], selected: usize) {
    // Fixed 20-char bar in the list pane.
    const BAR_WIDTH: usize = 20;

    let items: Vec<ListItem> = merged
        .iter()
        .enumerate()
        .map(|(i, mp)| {
            // Pick the window with the nearest reset_at for the list bar.
            let util = mp
                .windows
                .iter()
                .filter(|w| w.reset_at.is_some())
                .min_by_key(|w| w.reset_at.unwrap())
                .and_then(|w| w.utilization)
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
            let bar_line = Line::from(vec![
                Span::raw("  "),
                filled_span,
                empty_span,
                Span::raw(format!(" {pct_str}")),
            ]);

            ListItem::new(vec![
                Line::from(Span::styled(mp.subscription_name.clone(), name_style)),
                bar_line,
            ])
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

// ---------------------------------------------------------------------------
// Right pane: details
// ---------------------------------------------------------------------------

fn render_details(frame: &mut Frame<'_>, area: Rect, merged: &[MergedPlan<'_>], selected: usize) {
    let now = Utc::now();
    let mp = match merged.get(selected) {
        Some(m) => m,
        None => return,
    };

    // Bar width: full inner width minus 4 (2 left indent + 2 right margin).
    let bar_width = (area.width as usize).saturating_sub(4).max(4);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header.
    lines.push(Line::from(Span::styled(
        mp.subscription_name.clone(),
        th::PLAN_LABEL,
    )));
    lines.push(Line::from(""));

    for w in &mp.windows {
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
