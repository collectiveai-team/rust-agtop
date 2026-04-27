//! General tab: tabular key/value listing of all session metadata.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use agtop_core::session::SessionAnalysis;
use crate::tui::theme_v2::Theme;

pub fn render(frame: &mut Frame<'_>, area: Rect, a: &SessionAnalysis, theme: &Theme) {
    let kv = |key: &'static str, val: String| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {key:>16}  "), Style::default().fg(theme.fg_muted)),
            Span::styled(val, Style::default().fg(theme.fg_default)),
        ])
    };
    let lines = vec![
        kv("Client", a.summary.client.as_str().to_string()),
        kv("Subscription", a.summary.subscription.clone().unwrap_or_default()),
        kv("Model", a.summary.model.clone().unwrap_or_default()),
        kv("Effort", a.summary.model_effort.clone().unwrap_or_default()),
        kv("Project", a.summary.cwd.clone().unwrap_or_default()),
        kv("Started", a.summary.started_at.map(|t| t.to_rfc3339()).unwrap_or_default()),
        kv("Last active", a.summary.last_active.map(|t| t.to_rfc3339()).unwrap_or_default()),
        kv("PID", a.pid.map(|p| p.to_string()).unwrap_or_default()),
        kv("Session id", a.summary.session_id.clone()),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}
