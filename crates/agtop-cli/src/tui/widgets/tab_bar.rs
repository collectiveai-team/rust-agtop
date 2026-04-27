//! Top-of-screen view-switcher tab bar.
//!
//! Renders: ` agtop ─── [d]ashboard [a]ggregation [c]onfig  q=quit ?=help  vX.Y.Z `
//! Active view is wrapped in `accent.primary` color + bold; inactive views in `fg.muted`.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::msg::ScreenId;
use crate::tui::theme_v2::Theme;

/// Render the tab bar inside `area` (typically the very top row of the terminal).
pub fn render(frame: &mut Frame<'_>, area: Rect, current: ScreenId, version: &str, theme: &Theme) {
    let mut spans: Vec<Span> = Vec::with_capacity(16);
    spans.push(Span::styled(
        " agtop ",
        Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled("│ ", Style::default().fg(theme.border_muted)));

    for (id, label) in [
        (ScreenId::Dashboard, "[d]ashboard"),
        (ScreenId::Aggregation, "[a]ggregation"),
        (ScreenId::Config, "[c]onfig"),
    ] {
        let style = if id == current {
            Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_muted)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("  "));
    }

    spans.push(Span::styled("│  ", Style::default().fg(theme.border_muted)));
    spans.push(Span::styled("q=quit  ?=help", Style::default().fg(theme.fg_muted)));

    // Spacer pushing version to the right edge.
    let prefix_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let version_str = format!(" v{version} ");
    let pad = (area.width as usize)
        .saturating_sub(prefix_len)
        .saturating_sub(version_str.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(
        version_str,
        Style::default().fg(theme.fg_muted),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn render_does_not_panic_on_narrow_terminal() {
        let backend = TestBackend::new(40, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        term.draw(|f| render(f, Rect::new(0, 0, 40, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_on_wide_terminal() {
        let backend = TestBackend::new(200, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        term.draw(|f| render(f, Rect::new(0, 0, 200, 1), ScreenId::Aggregation, "0.4.0", &theme))
            .unwrap();
    }
}
