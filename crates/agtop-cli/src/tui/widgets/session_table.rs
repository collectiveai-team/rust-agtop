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

/// A column descriptor: (header label, optional sort column, pixel width).
/// `None` for `sort_col` means the column is not sortable.
struct ColDef {
    label: &'static str,
    sort_col: Option<SortColumn>,
    width: u16,
}

/// Column definitions: label, optional sort column, and fixed pixel width.
/// `Constraint::Min` columns (CWD) use width=0 as a sentinel — they are
/// not sortable so they don't need an exact width for hit-testing.
fn col_defs() -> [ColDef; 10] {
    [
        ColDef {
            label: "PROVIDER",
            sort_col: Some(SortColumn::Provider),
            width: 8,
        },
        ColDef {
            label: "SESSION",
            sort_col: None,
            width: 12,
        },
        ColDef {
            label: "STARTED",
            sort_col: Some(SortColumn::Started),
            width: 16,
        },
        ColDef {
            label: "AGE",
            sort_col: Some(SortColumn::LastActive),
            width: 5,
        },
        ColDef {
            label: "MODEL",
            sort_col: Some(SortColumn::Model),
            width: 24,
        },
        ColDef {
            label: "CWD",
            sort_col: None,
            width: 0,
        }, // Min(16)
        ColDef {
            label: "TOK",
            sort_col: Some(SortColumn::Tokens),
            width: 8,
        },
        ColDef {
            label: "OUT",
            sort_col: Some(SortColumn::OutputTokens),
            width: 8,
        },
        ColDef {
            label: "CACHE",
            sort_col: Some(SortColumn::CacheTokens),
            width: 8,
        },
        ColDef {
            label: "COST$",
            sort_col: Some(SortColumn::Cost),
            width: 10,
        },
    ]
}

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

    let defs = col_defs();

    let header_cells: Vec<Cell<'static>> = defs
        .iter()
        .map(|d| match d.sort_col {
            Some(sc) => header_with_marker(d.label, sc, app, arrow),
            None => header_cell(d.label),
        })
        .collect();
    let header = Row::new(header_cells).style(header_style).height(1);

    // ── Compute absolute x-ranges for sortable header cells ──────────────
    // The table widget draws: left border (1px) + highlight-symbol (2px) +
    // then columns laid out left-to-right with 1px spacing between them.
    // We replicate that math here so we can map a clicked x-coordinate to
    // a SortColumn without re-running ratatui's layout engine.
    //
    // area.x + 1 (border) + 2 (highlight symbol "▶ ") = first column origin.
    // Each column occupies exactly `width` chars; Min(16) columns (CWD,
    // width==0 in our defs) occupy whatever is left — we skip them since
    // they are not sortable.
    header_cols.clear();
    let mut cursor_x = area.x + 1 + 2; // left border + "▶ "
    for d in &defs {
        if d.width == 0 {
            // Variable-width (CWD): not sortable — skip the hit-test entry
            // but we still need to advance past it. Since we don't know its
            // rendered width here, we just stop tracking after this point;
            // columns after CWD are tracked by computing from the right edge.
            break;
        }
        if let Some(sc) = d.sort_col {
            header_cols.push((cursor_x, cursor_x + d.width, sc));
        }
        cursor_x += d.width + 1; // +1 for the inter-column spacing
    }
    // Columns after CWD (TOK, OUT, CACHE, COST$) — compute from right edge.
    // area.x + area.width - 1 (right border) = first char past the last col.
    let right_edge = area.x + area.width - 1;
    // Walk the tail cols in reverse.
    let tail_defs: Vec<&ColDef> = defs
        .iter()
        .rev()
        .take_while(|d| d.width != 0)
        .collect::<Vec<_>>();
    // They were reversed, so flip them back for left→right ordering.
    let tail_defs: Vec<&ColDef> = tail_defs.into_iter().rev().collect();
    let mut rx = right_edge;
    for d in tail_defs.iter().rev() {
        let x_start = rx.saturating_sub(d.width);
        if let Some(sc) = d.sort_col {
            header_cols.push((x_start, rx, sc));
        }
        rx = x_start.saturating_sub(1); // -1 for inter-column spacing
    }
    // ─────────────────────────────────────────────────────────────────────

    let widths = defs.iter().map(|d| {
        if d.width == 0 {
            Constraint::Min(16)
        } else {
            Constraint::Length(d.width)
        }
    });

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
    let rows: Vec<Row> = view.iter().map(|a| row_for(a, now)).collect();

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

fn row_for<'a>(a: &'a agtop_core::session::SessionAnalysis, now: DateTime<Utc>) -> Row<'a> {
    let s = &a.summary;
    let t = &a.tokens;
    let c = &a.cost;

    let started = s
        .started_at
        .map(format_local_datetime)
        .unwrap_or_else(|| "-".into());
    let age = s
        .last_active
        .map(|t| relative_age(t, now))
        .unwrap_or_else(|| "-".into());
    let model = s.model.clone().unwrap_or_else(|| "?".into());
    let cwd = shorten_path(s.cwd.as_deref().unwrap_or("-"));
    let cost = if c.included {
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

    // Color the cost cell for quick at-a-glance reading:
    // - included sessions: dim green
    // - $0.00 real: default
    // - anything else: yellow (warn) above $5, white (default) otherwise.
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
    };

    let cache_total = t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input;

    Row::new(vec![
        Cell::from(s.provider.as_str()).style(provider_style),
        Cell::from(short),
        Cell::from(started),
        Cell::from(age),
        Cell::from(model),
        Cell::from(cwd),
        Cell::from(compact(t.input + t.output + cache_total)),
        Cell::from(compact(t.output)),
        Cell::from(compact(cache_total)),
        Cell::from(cost).style(cost_style),
    ])
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
