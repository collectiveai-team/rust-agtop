use chrono::Utc;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::app::App;
use crate::tui::theme as th;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let now = Utc::now();

    if app.plan_usage().is_empty() {
        lines.push(Line::from(Span::styled(
            "(no plan usage data)",
            th::PLAN_EMPTY,
        )));
    } else {
        for pu in app.plan_usage() {
            lines.push(Line::from(Span::styled(pu.label.clone(), th::PLAN_LABEL)));

            for w in &pu.windows {
                let pct = w.utilization.map(|u| format!("{:>3.0}%", u * 100.0));
                let bar = bar10(w.utilization);
                let reset = w
                    .reset_at
                    .map(|t| relative_until(t, now))
                    .or_else(|| w.reset_hint.clone())
                    .unwrap_or_else(|| "-".to_string());
                let binding = if w.binding { "*" } else { " " };
                let line = format!(
                    "  {} {:<3} {} {}  {}",
                    binding,
                    w.label,
                    bar,
                    pct.unwrap_or_else(|| "  - ".to_string()),
                    reset,
                );
                lines.push(Line::from(line));
            }

            if let Some(ts) = pu.last_limit_hit {
                lines.push(Line::from(format!(
                    "  last limit-hit: {} ago",
                    relative_since(ts, now)
                )));
            }

            if let Some(note) = &pu.note {
                lines.push(Line::from(Span::styled(format!("  {note}"), th::PLAN_NOTE)));
            }

            lines.push(Line::from(""));
        }
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Plan Usage "))
        .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(p, area);
}

fn bar10(util: Option<f64>) -> String {
    let Some(u) = util else {
        return "----------".to_string();
    };
    let filled = (u.clamp(0.0, 1.0) * 10.0).round() as usize;
    let mut s = String::new();
    for i in 0..10 {
        s.push(if i < filled { '■' } else { '·' });
    }
    s
}

fn relative_since(ts: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

fn relative_until(ts: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> String {
    let secs = (ts - now).num_seconds();
    if secs <= 0 {
        "reset".to_string()
    } else if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}
