//! Top half of the TUI: the session list table.
//!
//! Pure rendering — all business logic (sort / filter / selection)
//! lives in [`crate::tui::app`]. This module just turns the app
//! snapshot into ratatui widgets.

use chrono::{DateTime, Local, Utc};
use ratatui::{
    layout::Constraint,
    prelude::*,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};

use crate::tui::app::{App, SortColumn, SortDir};
use crate::tui::column_config::ColumnId;

/// Render the session table into `area`. Takes a `TableState` so scroll
/// offset survives redraws — ratatui doesn't maintain it internally.
///
/// `header_cols` is overwritten with the absolute terminal x-ranges of
/// every sortable header cell so the mouse handler can hit-test clicks.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    state: &mut TableState,
    header_cols: &mut Vec<(u16, u16, SortColumn)>,
) {
    // Sync the widget's idea of selection with the app's.
    state.select(app.selected_idx());

    let header_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // Build a compact arrow so the header advertises the active sort
    // column without stealing a full column of width.
    let arrow = match app.sort_dir() {
        SortDir::Asc => "↑",
        SortDir::Desc => "↓",
    };

    let col_cfg = app.column_config();
    let visible = col_cfg.visible();

    let header_cells: Vec<Cell<'static>> = visible
        .iter()
        .map(|&col_id| {
            let label = col_id.label();
            match col_id.sort_col() {
                Some(sc) => header_with_marker(label, sc, app, arrow),
                None => header_cell(label),
            }
        })
        .collect();
    let header = Row::new(header_cells).style(header_style).height(1);

    // ── Compute absolute x-ranges for sortable header cells ──────────────
    // The table widget draws: left border (1px) + highlight-symbol (2px) +
    // then columns laid out left-to-right with 1px spacing between them.
    header_cols.clear();
    let mut cursor_x = area.x + 1 + 2; // left border + "▶ "

    // Walk all visible columns. Stop tracking when we hit a flexible (CWD)
    // column since we don't know its rendered width here.
    let mut hit_flexible = false;
    for &col_id in &visible {
        if hit_flexible {
            break;
        }
        if col_id.is_flexible() {
            hit_flexible = true;
            continue;
        }
        let w = col_id.fixed_width().unwrap_or(0);
        if let Some(sc) = col_id.sort_col() {
            header_cols.push((cursor_x, cursor_x + w, sc));
        }
        cursor_x += w + 1; // +1 for the inter-column spacing
    }

    // Columns after the flexible CWD column — compute from right edge.
    if hit_flexible {
        let right_edge = area.x + area.width - 1;
        let tail: Vec<ColumnId> = visible
            .iter()
            .rev()
            .take_while(|&&id| !id.is_flexible())
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let mut rx = right_edge;
        for col_id in tail.iter().rev() {
            let w = col_id.fixed_width().unwrap_or(0);
            let x_start = rx.saturating_sub(w);
            if let Some(sc) = col_id.sort_col() {
                header_cols.push((x_start, rx, sc));
            }
            rx = x_start.saturating_sub(1);
        }
    }
    // ─────────────────────────────────────────────────────────────────────

    let widths: Vec<Constraint> = visible
        .iter()
        .map(|&col_id| {
            if col_id.is_flexible() {
                Constraint::Min(16)
            } else {
                Constraint::Length(col_id.fixed_width().unwrap_or(8))
            }
        })
        .collect();

    let title = format!(
        " Sessions ({visible}/{total})  sort:{col}{dir}  filter:\"{f}\" ",
        visible = app.view_len(),
        total = app.total_count(),
        col = app.sort_col().label(),
        dir = match app.sort_dir() {
            SortDir::Asc => "↑",
            SortDir::Desc => "↓",
        },
        f = app.filter(),
    );

    let now = Utc::now();
    let view = app.view();
    let rows: Vec<Row> = view.iter().map(|a| row_for(a, now, &visible)).collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, state);
}

fn header_cell(s: &'static str) -> Cell<'static> {
    Cell::from(s)
}

/// Append a direction arrow to the column header when it matches the
/// app's active sort column. This is what htop does with F6.
fn header_with_marker(
    label: &'static str,
    col: SortColumn,
    app: &App,
    arrow: &'static str,
) -> Cell<'static> {
    if app.sort_col() == col {
        Cell::from(format!("{label}{arrow}"))
    } else {
        Cell::from(label)
    }
}

fn row_for<'a>(
    a: &'a agtop_core::session::SessionAnalysis,
    now: DateTime<Utc>,
    visible: &[ColumnId],
) -> Row<'a> {
    let s = &a.summary;
    let t = &a.tokens;
    let c = &a.cost;

    let started = s
        .started_at
        .map(format_local_datetime)
        .unwrap_or_else(|| "-".into());
    let age = s
        .last_active
        .map(|ts| relative_age(ts, now))
        .unwrap_or_else(|| "-".into());
    let model = s.model.clone().unwrap_or_else(|| "?".into());
    let subscription = s.subscription.clone().unwrap_or_else(|| "-".into());
    let cwd = shorten_path(s.cwd.as_deref().unwrap_or("-"));
    let cost_str = if c.included {
        "incl".to_string()
    } else {
        format!("{:.4}", c.total)
    };
    let short = {
        let mut id = short_id(&s.session_id);
        if a.subagent_file_count > 0 {
            id.push_str(&format!("+{}", a.subagent_file_count));
        }
        id
    };
    let cache_total = t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input;

    // Color the cost cell for quick at-a-glance reading:
    let cost_style = if c.included {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::DIM)
    } else if c.total >= 5.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    // Provider color: cheap "tag" for eye-tracking.
    let provider_style = match s.provider {
        agtop_core::session::ProviderKind::Claude => Style::default().fg(Color::Magenta),
        agtop_core::session::ProviderKind::Codex => Style::default().fg(Color::Cyan),
        agtop_core::session::ProviderKind::OpenCode => Style::default().fg(Color::Green),
        _ => Style::default(),
    };

    let cells: Vec<Cell<'a>> = visible
        .iter()
        .map(|&col_id| match col_id {
            ColumnId::Provider => Cell::from(s.provider.as_str()).style(provider_style),
            ColumnId::Subscription => Cell::from(subscription.clone()),
            ColumnId::Session => Cell::from(short.clone()),
            ColumnId::Started => Cell::from(started.clone()),
            ColumnId::Age => Cell::from(age.clone()),
            ColumnId::Model => Cell::from(model.clone()),
            ColumnId::Cwd => Cell::from(cwd.clone()),
            ColumnId::Tokens => Cell::from(compact(t.input + t.output + cache_total)),
            ColumnId::OutputTokens => Cell::from(compact(t.output)),
            ColumnId::CacheTokens => Cell::from(compact(cache_total)),
            ColumnId::Cost => Cell::from(cost_str.clone()).style(cost_style),
            ColumnId::ToolCalls => Cell::from(
                a.tool_call_count
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
            ),
            ColumnId::Duration => Cell::from(
                a.duration_secs
                    .map(format_duration_compact)
                    .unwrap_or_else(|| "-".into()),
            ),
        })
        .collect();

    Row::new(cells)
}

fn format_local_datetime(ts: DateTime<Utc>) -> String {
    ts.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

fn relative_age(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    if secs < 60 {
        return "now".into();
    }
    if secs < 3600 {
        return format!("{}m", secs / 60);
    }
    if secs < 86_400 {
        return format!("{}h", secs / 3600);
    }
    if secs < 604_800 {
        return format!("{}d", secs / 86_400);
    }
    if secs < 2_592_000 {
        return format!("{}w", secs / 604_800);
    }
    if secs < 31_536_000 {
        return format!("{}mo", secs / 2_592_000);
    }
    format!("{}y", secs / 31_536_000)
}

fn compact(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}G", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn format_duration_compact(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

fn short_id(id: &str) -> String {
    if id.starts_with("ses_") {
        return id[..id.len().min(10)].to_string();
    }
    id.chars().take(8).collect()
}

fn shorten_path(p: &str) -> String {
    if let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) {
        if let Some(rest) = p.strip_prefix(&home) {
            return format!("~{}", rest);
        }
    }
    p.to_string()
}
