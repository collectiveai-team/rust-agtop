//! Aggregation table: the main GROUP / SESSIONS / TOKENS / COST / AVG DUR / LAST ACTIVE / ACTIVITY rows.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

use agtop_core::aggregate::AggregateGroup;

use crate::tui::theme_v2::Theme;
use crate::tui::widgets::sparkline_braille;

#[derive(Debug, Default)]
pub struct AggregationTable {
    pub groups: Vec<AggregateGroup>,
    pub state: TableState,
}

impl AggregationTable {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let header = Row::new(vec![
            "GROUP",
            "SESSIONS",
            "TOKENS",
            "COST",
            "AVG DUR",
            "LAST ACTIVE",
            "ACTIVITY",
        ])
        .style(
            Style::default()
                .fg(theme.fg_emphasis)
                .add_modifier(Modifier::BOLD),
        );
        let mut rows: Vec<Row> = self
            .groups
            .iter()
            .map(|g| Self::row_for(g, theme))
            .collect();

        // Total row.
        if !self.groups.is_empty() {
            let total_sessions: usize = self.groups.iter().map(|g| g.session_count).sum();
            let total_tokens: u64 = self.groups.iter().map(|g| g.total_tokens).sum();
            let total_cost: Option<f64> = self
                .groups
                .iter()
                .try_fold(0.0_f64, |acc, g| g.total_cost.map(|c| acc + c));
            rows.push(
                Row::new(vec![
                    Cell::from(Span::styled(
                        "TOTAL",
                        Style::default()
                            .fg(theme.fg_emphasis)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(format!("{total_sessions}")),
                    Cell::from(format_tokens(total_tokens)),
                    Cell::from(
                        total_cost
                            .map(|c| format!("${c:.2}"))
                            .unwrap_or_else(|| "—".into()),
                    ),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                ])
                .style(
                    Style::default()
                        .fg(theme.fg_emphasis)
                        .add_modifier(Modifier::BOLD),
                ),
            );
        }

        let widths = [
            Constraint::Length(18),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(13),
            Constraint::Length(14),
        ];
        let table = Table::new(rows, widths).header(header);
        frame.render_widget(table, area);
    }

    fn row_for<'a>(g: &'a AggregateGroup, theme: &Theme) -> Row<'a> {
        let activity = sparkline_braille::render_braille(&g.activity, 12, max_or_one(&g.activity));
        let cost = g
            .total_cost
            .map(|c| format!("${c:.2}"))
            .unwrap_or_else(|| "—".into());
        let last = g.last_active.map(format_relative).unwrap_or_default();
        let dur = format!(
            "{}m {}s",
            g.avg_duration_secs / 60,
            g.avg_duration_secs % 60
        );
        Row::new(vec![
            Cell::from(g.label.clone()),
            Cell::from(format!("{}", g.session_count)),
            Cell::from(format_tokens(g.total_tokens)),
            Cell::from(cost),
            Cell::from(dur),
            Cell::from(last),
            Cell::from(Line::from(Span::styled(
                activity,
                Style::default().fg(theme.status_success),
            ))),
        ])
        .style(Style::default().fg(theme.fg_default))
    }
}

fn max_or_one(v: &[f32]) -> f32 {
    v.iter().copied().fold(0.0_f32, f32::max).max(1.0)
}

fn format_tokens(t: u64) -> String {
    let f = t as f32;
    if f >= 1_000_000.0 {
        format!("{:.1}M", f / 1_000_000.0)
    } else if f >= 1_000.0 {
        format!("{:.1}k", f / 1_000.0)
    } else {
        format!("{t}")
    }
}

fn format_relative(t: chrono::DateTime<chrono::Utc>) -> String {
    format_relative_from(t, chrono::Utc::now())
}

pub fn format_relative_from(
    t: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let d = now - t;
    if d.num_seconds() < 60 {
        "just now".into()
    } else if d.num_minutes() < 60 {
        format!("{}m ago", d.num_minutes())
    } else if d.num_hours() < 24 {
        format!("{}h ago", d.num_hours())
    } else {
        format!("{}d ago", d.num_days())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tokens_compact() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(12_500), "12.5k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }
}
