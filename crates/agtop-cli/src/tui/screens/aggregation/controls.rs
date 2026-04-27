//! Top-of-screen pickers: Group by + Range + Sort/Reverse.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use agtop_core::aggregate::{GroupBy, TimeRange};

use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone, Copy)]
pub struct ControlsModel {
    pub group_by: GroupBy,
    pub range: TimeRange,
    pub sort_label: &'static str,
    pub reverse: bool,
}

impl Default for ControlsModel {
    fn default() -> Self {
        Self {
            group_by: GroupBy::Client,
            range: TimeRange::Today,
            sort_label: "COST",
            reverse: false,
        }
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &ControlsModel, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    render_row1(frame, layout[0], m, theme);
    render_row2(frame, layout[1], m, theme);
}

fn render_row1(frame: &mut Frame<'_>, area: Rect, m: &ControlsModel, theme: &Theme) {
    let mut spans = vec![
        Span::styled(" Group by:  ", Style::default().fg(theme.fg_muted)),
    ];
    for g in [GroupBy::Client, GroupBy::Provider, GroupBy::Model, GroupBy::Project, GroupBy::Subscription] {
        let label = match g {
            GroupBy::Client => "Client",
            GroupBy::Provider => "Provider",
            GroupBy::Model => "Model",
            GroupBy::Project => "Project",
            GroupBy::Subscription => "Subscription",
        };
        if g == m.group_by {
            spans.push(Span::styled(format!("‹ {label} › "), Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)));
        } else {
            spans.push(Span::styled(format!("  {label}   "), Style::default().fg(theme.fg_muted)));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_row2(frame: &mut Frame<'_>, area: Rect, m: &ControlsModel, theme: &Theme) {
    let mut spans = vec![Span::styled(" Range:     ", Style::default().fg(theme.fg_muted))];
    for r in [TimeRange::Today, TimeRange::Week, TimeRange::Month, TimeRange::All] {
        let label = match r {
            TimeRange::Today => "Today",
            TimeRange::Week => "Week",
            TimeRange::Month => "Month",
            TimeRange::All => "All",
        };
        if r == m.range {
            spans.push(Span::styled(format!("‹ {label} › "), Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)));
        } else {
            spans.push(Span::styled(format!("  {label}   "), Style::default().fg(theme.fg_muted)));
        }
    }
    spans.push(Span::styled("  |  Sort: ", Style::default().fg(theme.fg_muted)));
    spans.push(Span::styled(
        format!("‹{}›", m.sort_label),
        Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled("  Reverse: ", Style::default().fg(theme.fg_muted)));
    spans.push(Span::styled(
        if m.reverse { "on" } else { "off" },
        Style::default().fg(if m.reverse { theme.accent_primary } else { theme.fg_muted }),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn renders_without_panic() {
        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let m = ControlsModel::default();
        term.draw(|f| render(f, Rect::new(0, 0, 140, 2), &m, &theme)).unwrap();
    }
}
