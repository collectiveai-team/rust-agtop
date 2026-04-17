//! "Cost" tab: the selected session's CostBreakdown laid out as a
//! small per-bucket table.

use ratatui::{
    layout::{Alignment, Constraint},
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::tui::app::{cost_rows, App};
use crate::tui::theme as th;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Cost ");

    let Some((_, a)) = app.selected() else {
        let msg = Paragraph::new("(no session selected)")
            .alignment(Alignment::Center)
            .style(th::EMPTY_HINT)
            .block(block);
        frame.render_widget(msg, area);
        return;
    };

    if a.cost.included {
        // Shortcut: included sessions don't have a dollar breakdown to
        // show. Render the bucket counts but suppress per-row dollars.
        let rows: Vec<Row> = cost_rows(&a.tokens, &a.cost)
            .into_iter()
            .map(|(label, tokens, _)| {
                Row::new(vec![
                    Cell::from(label).style(th::COST_BUCKET_LABEL),
                    Cell::from(tokens),
                    Cell::from("incl").style(th::COST_INCL),
                ])
            })
            .collect();
        frame.render_widget(
            build_table(rows, Some("Session is covered by plan — $0.00 billed")),
            area.inner(ratatui::layout::Margin {
                horizontal: 0,
                vertical: 0,
            }),
        );
        return;
    }

    let rows: Vec<Row> = cost_rows(&a.tokens, &a.cost)
        .into_iter()
        .map(|(label, tokens, dollars)| {
            Row::new(vec![
                Cell::from(label).style(th::COST_BUCKET_LABEL),
                Cell::from(tokens),
                Cell::from(dollars),
            ])
        })
        .collect();

    let total = format!("${:.4}", a.cost.total);
    let total_row = Row::new(vec![
        Cell::from("total").style(th::COST_TOTAL),
        Cell::from(""),
        Cell::from(total).style(th::COST_TOTAL),
    ]);
    let mut all_rows = rows;
    all_rows.push(total_row);

    let widths = [
        Constraint::Length(18),
        Constraint::Length(12),
        Constraint::Length(14),
    ];
    let header = Row::new(vec![
        Cell::from("bucket").style(th::HEADER),
        Cell::from("tokens").style(th::HEADER),
        Cell::from("dollars").style(th::HEADER),
    ])
    .height(1);

    let effective = a.effective_model.as_deref().or(a.summary.model.as_deref());
    let title = match effective {
        Some(m) => format!(" Cost — {m} "),
        None => " Cost ".to_string(),
    };
    let table = Table::new(all_rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

    frame.render_widget(table, area);
}

/// Build a bare table for included-session mode.
fn build_table<'a>(rows: Vec<Row<'a>>, title: Option<&'a str>) -> Table<'a> {
    let widths = [
        Constraint::Length(18),
        Constraint::Length(12),
        Constraint::Length(10),
    ];
    let header = Row::new(vec![
        Cell::from("bucket").style(th::HEADER),
        Cell::from("tokens").style(th::HEADER),
        Cell::from("dollars").style(th::HEADER),
    ])
    .height(1);
    let block = match title {
        Some(t) => Block::default()
            .borders(Borders::ALL)
            .title(format!(" {t} ")),
        None => Block::default().borders(Borders::ALL).title(" Cost "),
    };
    Table::new(rows, widths).header(header).block(block)
}
