use chrono::{Datelike, Local};
use ratatui::{
    prelude::*,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
};

use agtop_core::session::ProviderKind;

use crate::tui::app::App;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut total_cost = 0.0f64;
    let mut total_sessions = 0usize;
    let mut month_cost = 0.0f64;
    let mut month_sessions = 0usize;
    let mut per_provider = [0.0f64; 3];

    let now_local = Local::now();
    let cur_year = now_local.year();
    let cur_month = now_local.month();

    for s in app.sessions() {
        total_sessions += 1;
        total_cost += s.cost.total;
        per_provider[idx(s.summary.provider)] += s.cost.total;

        if let Some(started) = s.summary.started_at {
            let local = started.with_timezone(&Local);
            if local.year() == cur_year && local.month() == cur_month {
                month_sessions += 1;
                month_cost += s.cost.total;
            }
        }
    }

    let lines = vec![
        Line::from(format!(
            " total   ${:>9.4}   {:>5} sessions",
            total_cost, total_sessions
        )),
        Line::from(format!(
            " month   ${:>9.4}   {:>5} sessions",
            month_cost, month_sessions
        )),
        Line::from(""),
        Line::from(Span::styled(
            " per agentic provider (retail estimate)",
            Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
        )),
        Line::from(format!(" claude    ${:>9.4}", per_provider[0])),
        Line::from(format!(" codex     ${:>9.4}", per_provider[1])),
        Line::from(format!(" opencode  ${:>9.4}", per_provider[2])),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Cost Summary "),
        ),
        area,
    );
}

fn idx(kind: ProviderKind) -> usize {
    match kind {
        ProviderKind::Claude => 0,
        ProviderKind::Codex => 1,
        ProviderKind::OpenCode => 2,
    }
}
