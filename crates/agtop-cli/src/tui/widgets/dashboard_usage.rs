use chrono::Utc;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::*,
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
};

use agtop_core::session::ClientKind;

use crate::fmt;
use crate::tui::app::{App, CHART_WINDOW_MINS};
use crate::tui::theme as th;

const N_BUCKETS: usize = 60;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(format!(
        " Usage · tokens/min · last {CHART_WINDOW_MINS} min "
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(1)])
        .split(inner);

    let now = Utc::now();
    let claude = app
        .history()
        .buckets_by_client(now, N_BUCKETS, ClientKind::Claude);
    let codex = app
        .history()
        .buckets_by_client(now, N_BUCKETS, ClientKind::Codex);
    let opencode = app
        .history()
        .buckets_by_client(now, N_BUCKETS, ClientKind::OpenCode);

    let pts_claude: Vec<(f64, f64)> = claude
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v as f64))
        .collect();
    let pts_codex: Vec<(f64, f64)> = codex
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v as f64))
        .collect();
    let pts_opencode: Vec<(f64, f64)> = opencode
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v as f64))
        .collect();

    let max_y = *claude
        .iter()
        .chain(codex.iter())
        .chain(opencode.iter())
        .max()
        .unwrap_or(&1) as f64;

    let datasets = vec![
        Dataset::default()
            .name("claude")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Line)
            .style(th::CHART_CLAUDE)
            .data(&pts_claude),
        Dataset::default()
            .name("codex")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Line)
            .style(th::CHART_CODEX)
            .data(&pts_codex),
        Dataset::default()
            .name("opencode")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Line)
            .style(th::CHART_OPENCODE)
            .data(&pts_opencode),
    ];

    let chart = Chart::new(datasets)
        .x_axis(Axis::default().bounds([0.0, N_BUCKETS as f64]).labels(vec![
            Line::from("-60m"),
            Line::from("-30m"),
            Line::from("now"),
        ]))
        .y_axis(Axis::default().bounds([0.0, max_y.max(1.0)]));
    frame.render_widget(chart, rows[0]);

    let current = app
        .history()
        .points()
        .back()
        .map(|p| p.tokens_by_client)
        .unwrap_or([0, 0, 0, 0, 0, 0, 0]);
    let summary = format!(
        " now: claude {} /m · codex {} /m · opencode {} /m ",
        fmt::compact(current[0]),
        fmt::compact(current[1]),
        fmt::compact(current[2])
    );
    frame.render_widget(Paragraph::new(summary).style(th::CHART_SUMMARY), rows[1]);
}
