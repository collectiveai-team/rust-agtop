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

/// Render the session table into `area`. Takes a `TableState` so scroll
/// offset survives redraws — ratatui doesn't maintain it internally.
pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App, state: &mut TableState) {
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

    let header_cells = [
        header_with_marker("PROVIDER", SortColumn::Provider, app, arrow),
        header_cell("SESSION"),
        header_cell("STARTED"),
        header_with_marker("AGE", SortColumn::LastActive, app, arrow),
        header_with_marker("MODEL", SortColumn::Model, app, arrow),
        header_cell("CWD"),
        header_with_marker("TOK", SortColumn::Tokens, app, arrow),
        header_cell("OUT"),
        header_cell("CACHE"),
        header_with_marker("COST$", SortColumn::Cost, app, arrow),
    ];
    let header = Row::new(header_cells).style(header_style).height(1);

    let now = Utc::now();
    let view = app.view();
    let rows: Vec<Row> = view.iter().map(|a| row_for(a, now)).collect();

    let widths = [
        Constraint::Length(8),  // PROVIDER
        Constraint::Length(12), // SESSION
        Constraint::Length(16), // STARTED
        Constraint::Length(5),  // AGE
        Constraint::Length(24), // MODEL
        Constraint::Min(16),    // CWD (flexes)
        Constraint::Length(8),  // TOK (grand total)
        Constraint::Length(8),  // OUT
        Constraint::Length(8),  // CACHE
        Constraint::Length(10), // COST$
    ];

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
