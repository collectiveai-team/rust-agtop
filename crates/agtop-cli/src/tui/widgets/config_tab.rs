//! "Config" tab: column visibility and order editor.
//!
//! Keys (when Config tab is active):
//!   j / ↓       — move cursor down
//!   k / ↑       — move cursor up
//!   Space / Enter — toggle column visibility
//!   Shift+↑ / K  — move column up in order
//!   Shift+↓ / J  — move column down in order
//!
//! Changes are persisted immediately to ~/.config/agtop/columns.json.

use ratatui::{
    layout::Alignment,
    prelude::*,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};

use crate::tui::app::App;
use crate::tui::theme as th;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Config — Columns (Space:toggle, K/J:reorder) ");

    let col_cfg = app.column_config();
    let cursor = app.config_cursor();

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("COLUMN"),
        Cell::from("DESCRIPTION"),
    ])
    .style(th::HEADER)
    .height(1);

    let rows: Vec<Row> = col_cfg
        .columns
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let check = if entry.visible { "[x]" } else { "[ ]" };
            let check_style = if entry.visible {
                th::CONFIG_CHECKED
            } else {
                th::CONFIG_UNCHECKED
            };
            let row_style = if i == cursor {
                th::SELECTED
            } else {
                ratatui::style::Style::new()
            };
            Row::new(vec![
                Cell::from(check).style(check_style),
                Cell::from(entry.id.label()),
                Cell::from(entry.id.description()),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(12),
        Constraint::Min(20),
    ];

    let mut state = TableState::default();
    state.select(Some(cursor));

    if col_cfg.columns.is_empty() {
        let msg = Paragraph::new("No columns defined.")
            .alignment(Alignment::Center)
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(th::SELECTED)
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, &mut state);
}

// Bring Paragraph into scope for the empty-state fallback.
use ratatui::widgets::Paragraph;
