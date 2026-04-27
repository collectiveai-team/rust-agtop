#![allow(dead_code, unused)]
use std::collections::BTreeMap;

use chrono::{Datelike, Local};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Tabs},
};

use agtop_core::session::{ClientKind, SessionAnalysis};

use crate::tui::app::{App, CostPeriod, CostTab};
use crate::tui::theme as th;

/// Output params for mouse hit-testing, written by [`render`] every frame.
pub struct CostRenderOut<'a> {
    /// The one-row tab bar area for the Cost Summary sub-tab.
    pub tab_bar_area: &'a mut Rect,
    /// Absolute x-ranges for each sub-tab button: `(x_start, x_end_exclusive, CostTab)`.
    pub tab_cells: &'a mut Vec<(u16, u16, CostTab)>,
    /// The single-row area that holds the period toggle ("total" / "month").
    pub period_row_area: &'a mut Rect,
    /// Click ranges for the period toggle labels: `(x_start, x_end_exclusive, CostPeriod)`.
    pub period_cells: &'a mut Vec<(u16, u16, CostPeriod)>,
    /// Full area of the Cost Summary panel (for scroll-wheel hit-testing).
    pub cost_panel_area: &'a mut Rect,
    /// Number of data rows in the current breakdown (for scroll clamping).
    pub cost_row_count: &'a mut usize,
    /// Number of visible data rows in the current breakdown (for scroll clamping).
    pub cost_visible_rows: &'a mut usize,
}

/// Render the Cost Summary panel.
///
/// Layout (all inside one bordered block):
///   row 0 : period toggle  — "total $X  N sess  │  month $X  N sess"
///   row 1 : group-by tabs  — "Client | Subscription | Model | Project"
///   rows 2..h-2 : scrollable data rows
///   row h-1 : pinned totals row
pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App, out: CostRenderOut<'_>) {
    let CostRenderOut {
        tab_bar_area: tab_bar_area_out,
        tab_cells: tab_cells_out,
        period_row_area: period_row_area_out,
        period_cells: period_cells_out,
        cost_panel_area: cost_panel_area_out,
        cost_row_count: cost_row_count_out,
        cost_visible_rows: cost_visible_rows_out,
    } = out;
    *cost_panel_area_out = area;
    *cost_visible_rows_out = 0;
    *period_row_area_out = Rect::default();

    // ── Compute aggregates ──────────────────────────────────────────────────
    let now_local = Local::now();
    let cur_year = now_local.year();
    let cur_month = now_local.month();

    let mut total_cost = 0.0f64;
    let mut total_sessions = 0usize;
    let mut month_cost = 0.0f64;
    let mut month_sessions = 0usize;

    for s in app.sessions() {
        total_sessions += 1;
        total_cost += s.cost.total;
        if is_current_month(s, cur_year, cur_month) {
            month_sessions += 1;
            month_cost += s.cost.total;
        }
    }

    let active_period = app.cost_period();

    // ── Build data rows (sorted descending by cost) ─────────────────────────
    let rows = build_rows(app, active_period, cur_year, cur_month);
    let period_total = match active_period {
        CostPeriod::Total => total_cost,
        CostPeriod::Month => month_cost,
    };
    *cost_row_count_out = rows.len();

    // ── Inner layout (inside a single block border) ─────────────────────────
    // We render the block first as a frame, then fill the inner area manually.
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Cost Summary ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 4 {
        // Not enough space to show anything useful.
        return;
    }

    // inner rows:
    //   [0]   period toggle
    //   [1]   tab bar
    //   [2..h-1]  data rows (scrollable)
    //   [h-1] totals row (pinned)
    let inner_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // period toggle
            Constraint::Length(1), // tab bar
            Constraint::Min(1),    // data rows + totals
        ])
        .split(inner);

    let period_row = inner_rows[0];
    let tab_row = inner_rows[1];
    let data_area = inner_rows[2];

    *period_row_area_out = period_row;

    // ── Period toggle row ───────────────────────────────────────────────────
    let total_text = format!("total ${:>9.4}  {:>4} sess", total_cost, total_sessions);
    let month_text = format!("month ${:>9.4}  {:>4} sess", month_cost, month_sessions);
    let (total_style, month_style) = match active_period {
        CostPeriod::Total => (th::PLAN_LABEL, th::PLAN_NOTE),
        CostPeriod::Month => (th::PLAN_NOTE, th::PLAN_LABEL),
    };
    let period_line = Line::from(vec![
        Span::styled(total_text.clone(), total_style),
        Span::raw("  "),
        Span::styled(month_text.clone(), month_style),
    ]);
    frame.render_widget(Paragraph::new(period_line), period_row);

    // Write period click ranges.
    period_cells_out.clear();
    {
        let x0 = period_row.x;
        let total_start = x0;
        let total_end = total_start + total_text.chars().count() as u16;
        period_cells_out.push((total_start, total_end, CostPeriod::Total));
        let month_start = total_end + 2;
        let month_end = month_start + month_text.chars().count() as u16;
        period_cells_out.push((month_start, month_end, CostPeriod::Month));
    }

    // ── Tab bar ─────────────────────────────────────────────────────────────
    *tab_bar_area_out = tab_row;
    tab_cells_out.clear();
    {
        let mut x = tab_row.x;
        for (i, &tab) in CostTab::all().iter().enumerate() {
            let w = tab.title().chars().count() as u16;
            let cell_width = w + 2;
            tab_cells_out.push((x, x + cell_width, tab));
            x += cell_width;
            if i + 1 < CostTab::all().len() {
                x += 1; // divider
            }
        }
    }
    let active_idx = CostTab::all()
        .iter()
        .position(|t| *t == app.cost_tab())
        .unwrap_or(0);
    let tabs_widget = Tabs::new(
        CostTab::all()
            .iter()
            .map(|t| Line::from(t.title()))
            .collect::<Vec<_>>(),
    )
    .select(active_idx)
    .highlight_style(th::PLAN_LABEL)
    .divider("|");
    frame.render_widget(tabs_widget, tab_row);

    // ── Data area: scrollable rows + pinned totals ──────────────────────────
    if data_area.height < 2 {
        return;
    }

    // Reserve 1 row for the pinned totals line at the bottom.
    let visible_data_rows = data_area.height.saturating_sub(1) as usize;
    *cost_visible_rows_out = visible_data_rows;
    let scroll = app
        .cost_scroll()
        .min(rows.len().saturating_sub(visible_data_rows));

    // Clamp label width to fit in the available area (leave room for "$XXXX.XXXX").
    let cost_col_width: usize = 10; // "$XXXX.XXXX"
    let label_max = (data_area.width as usize).saturating_sub(cost_col_width + 2);

    // Render visible data rows.
    let visible_rows = &rows[scroll..rows.len().min(scroll + visible_data_rows)];
    let mut data_lines: Vec<Line<'static>> = visible_rows
        .iter()
        .map(|(label, cost)| {
            let label_trimmed = if label.len() > label_max {
                format!("{}…", &label[..label_max.saturating_sub(1)])
            } else {
                label.clone()
            };
            let cost_str = format!("${:>9.4}", cost);
            // Pad label left, right-align cost to fill the width.
            let padding =
                (data_area.width as usize).saturating_sub(label_trimmed.len() + cost_str.len() + 1);
            Line::from(format!(
                " {}{:>pad$}{}",
                label_trimmed,
                "",
                cost_str,
                pad = padding
            ))
        })
        .collect();

    // Pad with empty lines to fill up to visible_data_rows so the totals line
    // stays at the very bottom.
    while data_lines.len() < visible_data_rows {
        data_lines.push(Line::from(""));
    }

    // Pinned totals row.
    let totals_label = "total";
    let totals_cost_str = format!("${:>9.4}", period_total);
    let totals_padding =
        (data_area.width as usize).saturating_sub(totals_label.len() + totals_cost_str.len() + 1);
    data_lines.push(Line::from(vec![Span::styled(
        format!(
            " {}{:>pad$}{}",
            totals_label,
            "",
            totals_cost_str,
            pad = totals_padding
        ),
        th::COST_TOTAL,
    )]));

    frame.render_widget(Paragraph::new(data_lines), data_area);
}

// ── Data aggregation ────────────────────────────────────────────────────────

fn is_current_month(s: &SessionAnalysis, cur_year: i32, cur_month: u32) -> bool {
    s.summary
        .started_at
        .map(|t| {
            let local = t.with_timezone(&Local);
            local.year() == cur_year && local.month() == cur_month
        })
        .unwrap_or(false)
}

/// Build `(label, cost)` rows for the active tab+period, sorted by cost desc.
fn build_rows(app: &App, period: CostPeriod, cur_year: i32, cur_month: u32) -> Vec<(String, f64)> {
    let mut map: BTreeMap<String, f64> = BTreeMap::new();

    for s in app.sessions() {
        if period == CostPeriod::Month && !is_current_month(s, cur_year, cur_month) {
            continue;
        }
        let key = match app.cost_tab() {
            CostTab::Client => client_label(s.summary.client),
            CostTab::Subscription => s
                .summary
                .subscription
                .clone()
                .unwrap_or_else(|| "(no subscription)".to_string()),
            CostTab::Model => s
                .effective_model
                .clone()
                .or_else(|| s.summary.model.clone())
                .unwrap_or_else(|| "(unknown)".to_string()),
            CostTab::Project => s
                .project_name
                .clone()
                .or_else(|| s.summary.cwd.clone().map(|p| shorten_path(&p)))
                .unwrap_or_else(|| "(unknown)".to_string()),
        };
        *map.entry(key).or_insert(0.0) += s.cost.total;
    }

    let mut rows: Vec<(String, f64)> = map.into_iter().collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

fn client_label(kind: ClientKind) -> String {
    kind.as_str().to_string()
}

/// Shorten a path like `/home/user/projects/foo` → `~/projects/foo`,
/// and trim long paths to fit.
fn shorten_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path.starts_with(home_str.as_ref()) {
            return format!("~{}", &path[home_str.len()..]);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_label_uses_core_client_name_for_all_clients() {
        for client in ClientKind::all() {
            assert_eq!(client_label(*client), client.as_str());
        }
    }
}
