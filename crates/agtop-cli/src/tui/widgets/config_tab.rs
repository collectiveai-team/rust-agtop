//! "Config" tab: provider enable/disable + column visibility/order editor.
//!
//! Two sections, each a Table widget. Cursor navigates both as one
//! virtual list [providers..., columns...]. Keys & mouse both route
//! through App::toggle_cursor_item().

use ratatui::{
    layout::Alignment,
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::tui::app::{App, ConfigSection};
use crate::tui::theme as th;

/// Out-params: caller gives us Vec<(Rect, usize)> so it can be read back
/// for mouse hit-testing.
pub struct ConfigRenderOut<'a> {
    pub provider_rows: &'a mut Vec<(Rect, usize)>,
    pub column_rows: &'a mut Vec<(Rect, usize)>,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App, out: ConfigRenderOut<'_>) {
    out.provider_rows.clear();
    out.column_rows.clear();

    let n_providers = app.column_config().providers.len() as u16;
    // Providers block needs: 2 border rows + 1 header row + n_providers data rows.
    // Reserve at least 5 rows for the Columns block (border+header+2 data rows+border).
    let columns_min: u16 = 5;
    let providers_block_height = n_providers
        .saturating_add(3)
        .min(area.height.saturating_sub(columns_min));
    // Remaining rows go to the Columns block (minimum columns_min).
    let columns_block_height = area
        .height
        .saturating_sub(providers_block_height)
        .max(columns_min);

    let rows_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(providers_block_height),
            Constraint::Length(columns_block_height),
        ])
        .split(area);

    render_providers(frame, rows_layout[0], app, out.provider_rows);
    render_columns(frame, rows_layout[1], app, out.column_rows);
}

fn render_providers(frame: &mut Frame<'_>, area: Rect, app: &App, out: &mut Vec<(Rect, usize)>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Providers — Space/click: toggle ");

    let providers = &app.column_config().providers;
    let cursor = app.config_cursor();

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("PROVIDER"),
        Cell::from("STATUS"),
    ])
    .style(th::HEADER)
    .height(1);

    let rows: Vec<Row> = providers
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let check = if entry.enabled { "[x]" } else { "[ ]" };
            let check_style = if entry.enabled {
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
                Cell::from(entry.kind.as_str()),
                Cell::from(if entry.enabled { "enabled" } else { "disabled" }),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(14),
        Constraint::Min(10),
    ];

    let mut state = TableState::default();
    // Only highlight in this section when cursor is actually here.
    if app.config_section_at(cursor) == ConfigSection::Providers {
        state.select(Some(cursor));
    }

    // Compute row rects BEFORE rendering so we can write them into `out`.
    // Inside `area`, row 0 = top border, row 1 = header, rows 2.. = data.
    let data_y0 = area.y + 2;
    for (i, _) in providers.iter().enumerate() {
        let y = data_y0 + i as u16;
        if y >= area.y + area.height.saturating_sub(1) {
            break; // bottom border
        }
        let rect = Rect {
            x: area.x + 1, // inside left border
            y,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        out.push((rect, i)); // virtual idx == local idx for providers
    }

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(th::SELECTED)
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_columns(frame: &mut Frame<'_>, area: Rect, app: &App, out: &mut Vec<(Rect, usize)>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Columns — Space/click: toggle, Shift+J/K: reorder ");

    let col_cfg = app.column_config();
    let cursor = app.config_cursor();
    let n_providers = col_cfg.providers.len();

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("COLUMN"),
        Cell::from("DESCRIPTION"),
    ])
    .style(th::HEADER)
    .height(1);

    if col_cfg.columns.is_empty() {
        let msg = Paragraph::new("No columns defined.")
            .alignment(Alignment::Center)
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

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
            // Virtual index for this row is (n_providers + i).
            let row_style = if (n_providers + i) == cursor {
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
    if app.config_section_at(cursor) == ConfigSection::Columns {
        state.select(Some(app.config_local_idx(cursor)));
    }

    // Row rects — columns section.
    let data_y0 = area.y + 2;
    for (i, _) in col_cfg.columns.iter().enumerate() {
        let y = data_y0 + i as u16;
        if y >= area.y + area.height.saturating_sub(1) {
            break;
        }
        let rect = Rect {
            x: area.x + 1,
            y,
            width: area.width.saturating_sub(2),
            height: 1,
        };
        out.push((rect, n_providers + i));
    }

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(th::SELECTED)
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, area, &mut state);
}
