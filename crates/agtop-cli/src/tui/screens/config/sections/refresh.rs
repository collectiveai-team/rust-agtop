//! Refresh section: refresh interval, stalled threshold, lazy-load on startup.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::screens::config::controls;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone, Default)]
pub struct RefreshModel {
    pub interval_secs: u64,
    pub stalled_threshold_secs: u64,
    pub pause_on_idle: bool,
    pub lazy_load_on_startup: bool,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &RefreshModel, theme: &Theme) {
    let title = Line::from(Span::styled(
        "Refresh",
        Style::default()
            .fg(theme.fg_emphasis)
            .add_modifier(Modifier::BOLD),
    ));
    let rule = Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(theme.border_muted),
    ));

    let interval_str = m.interval_secs.to_string();
    let stalled_str = m.stalled_threshold_secs.to_string();
    let stalled_min = format!("  ({} min)", m.stalled_threshold_secs / 60);

    let lines: Vec<Line> = vec![
        title,
        rule,
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("  {:<22}", "Refresh interval (s)"),
                Style::default().fg(theme.fg_default),
            ),
            controls::text_input(&interval_str, theme),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {:<22}", "Stalled threshold (s)"),
                Style::default().fg(theme.fg_default),
            ),
            controls::text_input(&stalled_str, theme),
            Span::styled(stalled_min, Style::default().fg(theme.fg_muted)),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {:<22}", "Pause on idle"),
                Style::default().fg(theme.fg_default),
            ),
            controls::checkbox(m.pause_on_idle, theme),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {:<22}", "Lazy-load on startup"),
                Style::default().fg(theme.fg_default),
            ),
            controls::checkbox(m.lazy_load_on_startup, theme),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}
