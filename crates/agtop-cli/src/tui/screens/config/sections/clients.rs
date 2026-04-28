//! Clients section: per-client enable/disable + custom session paths.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use agtop_core::session::ClientKind;

use crate::tui::screens::config::controls;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone, Default)]
pub struct ClientsModel {
    pub enabled: std::collections::HashMap<ClientKind, bool>,
    pub custom_paths: std::collections::HashMap<ClientKind, String>,
}

const ALL_CLIENTS: [ClientKind; 7] = [
    ClientKind::Claude,
    ClientKind::Codex,
    ClientKind::GeminiCli,
    ClientKind::Copilot,
    ClientKind::Cursor,
    ClientKind::Antigravity,
    ClientKind::OpenCode,
];

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &ClientsModel, theme: &Theme) {
    let title = Line::from(Span::styled(
        "Clients",
        Style::default()
            .fg(theme.fg_emphasis)
            .add_modifier(Modifier::BOLD),
    ));
    let rule = Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(theme.border_muted),
    ));

    // Pre-collect data to avoid lifetime issues with borrowed temporaries.
    let rows: Vec<(ClientKind, bool, String)> = ALL_CLIENTS
        .iter()
        .map(|&kind| {
            let enabled = m.enabled.get(&kind).copied().unwrap_or(true);
            let path = m
                .custom_paths
                .get(&kind)
                .cloned()
                .unwrap_or_else(|| "(default)".into());
            (kind, enabled, path)
        })
        .collect();

    let mut lines: Vec<Line> = vec![title, rule, Line::from("")];
    for (kind, enabled, path) in &rows {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<14}", kind.as_str()),
                Style::default().fg(theme.fg_default),
            ),
            controls::checkbox(*enabled, theme),
            Span::raw("  "),
            Span::styled(format!("[ {path} ]"), Style::default().fg(theme.fg_default)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
