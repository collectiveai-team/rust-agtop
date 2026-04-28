//! Costs tab: total cost, token split, per-model breakdown.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::theme_v2::Theme;
use agtop_core::session::SessionAnalysis;

pub fn render(frame: &mut Frame<'_>, area: Rect, a: &SessionAnalysis, theme: &Theme) {
    let total = if a.cost.total > 0.0 {
        format!("${:.4}", a.cost.total)
    } else {
        "—".into()
    };
    let total_tokens = a.tokens.grand_total();

    let lines = vec![
        Line::from(Span::styled(
            "Costs",
            Style::default()
                .fg(theme.fg_emphasis)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Total cost   ", Style::default().fg(theme.fg_muted)),
            Span::styled(total, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Tokens (in+out+cache)  ",
                Style::default().fg(theme.fg_muted),
            ),
            Span::styled(
                format!("{total_tokens}"),
                Style::default().fg(theme.fg_default),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}
