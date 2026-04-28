//! Columns section: visibility toggles + reorder.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::column_config::{self, ColumnId};
use crate::tui::screens::config::controls;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone)]
pub struct ColumnsModel {
    /// Ordered list of all columns; visibility flag per column.
    pub items: Vec<(ColumnId, bool)>,
    /// Index of currently focused row (drives Up/Down).
    pub cursor: usize,
}

impl Default for ColumnsModel {
    fn default() -> Self {
        let visible = column_config::default_visible_v2();
        let mut items: Vec<(ColumnId, bool)> = visible.iter().map(|c| (*c, true)).collect();
        for c in ColumnId::all() {
            if !items.iter().any(|(x, _)| x == c) {
                items.push((*c, false));
            }
        }
        Self { items, cursor: 0 }
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &ColumnsModel, theme: &Theme) {
    let title = Line::from(Span::styled(
        "Columns",
        Style::default()
            .fg(theme.fg_emphasis)
            .add_modifier(Modifier::BOLD),
    ));
    let rule = Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(theme.border_muted),
    ));
    let hint = Line::from(Span::styled(
        "  Space toggles · J/K reorders · Enter saves",
        Style::default().fg(theme.fg_muted),
    ));

    let mut lines: Vec<Line> = vec![title, rule, hint, Line::from("")];
    for (i, (col, visible)) in m.items.iter().enumerate() {
        let marker = if i == m.cursor {
            Span::styled("▌ ", Style::default().fg(theme.accent_primary))
        } else {
            Span::raw("  ")
        };
        let cb = controls::checkbox(*visible, theme);
        let label = Span::styled(
            format!("  {}", col.label()),
            Style::default().fg(if *visible {
                theme.fg_default
            } else {
                theme.fg_muted
            }),
        );
        lines.push(Line::from(vec![marker, cb, label]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
