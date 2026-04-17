//! "Chart" tab: rolling 1-hour usage sparklines for token throughput and
//! cumulative cost across all sessions.
//!
//! Layout (inside the bottom-panel area):
//!
//! ```text
//! ┌─ Usage — last 60 min ─────────────────────────────────────────────┐
//! │  Tokens/min  [bar chart — 60 bars]                                 │
//! │  Cost/min    [sparkline]                                           │
//! │  <axis labels>                                                     │
//! └───────────────────────────────────────────────────────────────────┘
//! ```

use chrono::Utc;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    prelude::*,
    style::{Color, Modifier, Style},
    widgets::{BarChart, Block, Borders, Paragraph, Sparkline},
};

use crate::tui::app::{App, CHART_WINDOW_MINS};

/// Number of time buckets rendered on the chart.
/// One bucket ≈ 1 minute when `CHART_WINDOW_MINS == 60`.
const N_BUCKETS: usize = 60;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Usage — last {CHART_WINDOW_MINS} min "));
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    if inner.height < 4 {
        // Not enough vertical space — show a short notice instead.
        let msg = Paragraph::new("(terminal too small for chart)")
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let now = Utc::now();
    let (tok_buckets, cost_buckets) = app.history.buckets(now, N_BUCKETS);

    // Build BarChart data: ("", value) pairs; labels are empty to keep
    // the bars dense. The ratatui BarChart API requires `&[(&str, u64)]`.
    // We build owned strings (empty) and zip them with the values.
    let bar_data: Vec<u64> = tok_buckets.clone();

    // Sparkline data for cost (scaled to u64 micro-dollars for resolution).
    let sparkline_data: Vec<u64> = cost_buckets
        .iter()
        .map(|&c| (c * 1_000_000.0) as u64)
        .collect();

    let max_tokens = *tok_buckets.iter().max().unwrap_or(&1);
    let max_cost_microdollars = *sparkline_data.iter().max().unwrap_or(&1);

    // Summary line below the charts.
    let total_tokens_now: u64 = app.history.points().back().map(|p| p.tokens).unwrap_or(0);
    let total_cost_now: f64 = app.history.points().back().map(|p| p.cost).unwrap_or(0.0);
    let n_points = app.history.points().len();

    // Vertical split:
    //   row 0 (label, 1)  — "Tokens" heading
    //   row 1 (bars, flex) — BarChart
    //   row 2 (label, 1)  — "Cost" heading
    //   row 3 (spark, 3)  — Sparkline
    //   row 4 (footer, 1) — summary text
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tokens label
            Constraint::Min(2),    // bar chart (takes remaining space)
            Constraint::Length(1), // cost label
            Constraint::Length(3), // sparkline
            Constraint::Length(1), // summary footer
        ])
        .split(inner);

    // ── Tokens label ─────────────────────────────────────────────────────
    let tok_label = format!(" Tokens (peak/min, max={}) ", format_tokens(max_tokens));
    frame.render_widget(
        Paragraph::new(tok_label).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
    );

    // ── Token bar chart ──────────────────────────────────────────────────
    // ratatui BarChart takes `&[Bar]` (ratatui 0.29 API).
    // We need at most `inner_width / 2` bars to avoid overlap.
    let max_bars = (rows[1].width as usize).saturating_sub(2).max(1);
    let n = N_BUCKETS.min(max_bars);
    let skip = N_BUCKETS.saturating_sub(n);

    // Build the bar data as owned strings — BarChart in ratatui 0.29 uses
    // the older `(&str, u64)` tuple API.
    // We keep the label empty ("") so bars are dense.
    let bar_tuples: Vec<(&str, u64)> = bar_data[skip..].iter().map(|&v| ("", v)).collect();

    let barchart = BarChart::default()
        .data(&bar_tuples)
        .bar_width(1)
        .bar_gap(0)
        .bar_style(Style::default().fg(Color::Blue))
        .value_style(Style::default().fg(Color::Reset)) // hide numeric labels
        .max(max_tokens.max(1));
    frame.render_widget(barchart, rows[1]);

    // ── Cost label ───────────────────────────────────────────────────────
    let max_cost_dollars = max_cost_microdollars as f64 / 1_000_000.0;
    let cost_label = format!(" Cost $/min (max=${:.4}) ", max_cost_dollars);
    frame.render_widget(
        Paragraph::new(cost_label).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        rows[2],
    );

    // ── Cost sparkline ───────────────────────────────────────────────────
    let spark_data: Vec<u64> = sparkline_data[skip..].to_vec();
    let sparkline = Sparkline::default()
        .data(&spark_data)
        .max(max_cost_microdollars.max(1))
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(sparkline, rows[3]);

    // ── Summary footer ───────────────────────────────────────────────────
    let summary = format!(
        " now: {} tokens  ${:.4} total  ({} snapshots) ",
        format_tokens(total_tokens_now),
        total_cost_now,
        n_points,
    );
    frame.render_widget(
        Paragraph::new(summary).style(Style::default().fg(Color::Gray)),
        rows[4],
    );
}

fn format_tokens(n: u64) -> String {
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
