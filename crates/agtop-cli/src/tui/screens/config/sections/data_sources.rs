//! Data sources section: paths agtop reads from per client.

use ratatui::{layout::Rect, style::{Modifier, Style}, text::{Line, Span}, widgets::Paragraph, Frame};

use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone, Default)]
pub struct DataSourcesModel {
    pub entries: Vec<(String, String, String)>, // (client, path, status)
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &DataSourcesModel, theme: &Theme) {
    let title = Line::from(Span::styled("Data sources", Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD)));
    let rule = Line::from(Span::styled("─".repeat(40), Style::default().fg(theme.border_muted)));

    let mut lines = vec![title, rule, Line::from("")];
    for (client, path, status) in &m.entries {
        lines.push(Line::from(vec![
            Span::styled(format!("  {client:<14}"), Style::default().fg(theme.fg_default)),
            Span::styled(path.clone(), Style::default().fg(theme.syntax_string)),
            Span::raw("   "),
            Span::styled(format!("[{status}]"), Style::default().fg(theme.fg_muted)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
