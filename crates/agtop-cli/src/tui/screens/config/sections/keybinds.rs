//! Keybinds section: read-only reference table.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::theme_v2::Theme;

const BINDINGS: &[(&str, &str)] = &[
    ("d / a / c", "Switch view (Dashboard/Aggregation/Config)"),
    ("q", "Quit"),
    ("?", "Help overlay"),
    ("i", "Toggle Info drawer (Dashboard)"),
    ("u", "Cycle Usage Quota mode (Dashboard)"),
    ("/", "Filter / search"),
    ("s / S", "Cycle sort key / reverse direction"),
    ("g / r", "Cycle group-by / time range (Aggregation)"),
    ("Enter", "Open / drill into selection"),
    ("Esc", "Close overlay / cancel"),
    ("Tab / Shift+Tab", "Switch focus / cycle tabs"),
    ("1 / 2 / 3 / 4", "Switch Info drawer tab"),
    ("j / k", "Move down / up"),
    (
        "Shift+click",
        "Bypass mouse capture for native text selection",
    ),
];

pub fn render(frame: &mut Frame<'_>, area: Rect, _: &(), theme: &Theme) {
    let title = Line::from(Span::styled(
        "Keybinds (read-only)",
        Style::default()
            .fg(theme.fg_emphasis)
            .add_modifier(Modifier::BOLD),
    ));
    let rule = Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(theme.border_muted),
    ));
    let mut lines = vec![title, rule, Line::from("")];
    for (key, desc) in BINDINGS {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {key:<18}"),
                Style::default()
                    .fg(theme.accent_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(*desc, Style::default().fg(theme.fg_default)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
