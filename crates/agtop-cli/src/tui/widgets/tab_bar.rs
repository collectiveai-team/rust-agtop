//! Top-of-screen view-switcher tab bar.
//!
//! Renders: ` agtop ─── [d]ashboard [a]ggregation [c]onfig  q=quit ?=help  vX.Y.Z `
//! Active view is wrapped in `accent.primary` color + bold; inactive views in `fg.muted`.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::msg::ScreenId;
use crate::tui::theme_v2::Theme;

/// Stateful tab bar that records each tab's rendered area for mouse hit-testing.
#[derive(Debug, Default)]
pub struct TabBar {
    /// (screen_id, rect) for each tab, populated by `render()`.
    tab_rects: Vec<(ScreenId, Rect)>,
}

impl TabBar {
    /// Render the tab bar and record tab rects for mouse hit-testing.
    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        current: ScreenId,
        version: &str,
        theme: &Theme,
    ) {
        self.tab_rects.clear();
        let mut spans: Vec<Span> = Vec::with_capacity(16);

        let logo = " agtop ";
        let sep = "│ ";
        spans.push(Span::styled(
            logo,
            Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(sep, Style::default().fg(theme.border_muted)));

        let mut x_offset: u16 = (logo.chars().count() + sep.chars().count()) as u16 + area.x;

        for (id, label) in [
            (ScreenId::Dashboard, "[d]ashboard"),
            (ScreenId::Aggregation, "[a]ggregation"),
            (ScreenId::Config, "[c]onfig"),
        ] {
            let w = label.chars().count() as u16;
            self.tab_rects.push((id, Rect::new(x_offset, area.y, w, 1)));
            x_offset += w;

            let style = if id == current {
                Style::default().fg(theme.accent_primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_muted)
            };
            spans.push(Span::styled(label, style));

            // Two trailing spaces after each tab.
            spans.push(Span::raw("  "));
            x_offset += 2;
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

    /// Return the `ScreenId` whose tab was clicked, or `None`.
    pub fn hit_test(&self, column: u16, row: u16) -> Option<ScreenId> {
        for &(id, rect) in &self.tab_rects {
            if row == rect.y && column >= rect.x && column < rect.x + rect.width {
                return Some(id);
            }
        }
        None
    }
}

// Keep the old free function as a thin wrapper for any callers not yet migrated.
pub fn render(frame: &mut Frame<'_>, area: Rect, current: ScreenId, version: &str, theme: &Theme) {
    let mut bar = TabBar::default();
    bar.render(frame, area, current, version, theme);
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
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 40, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_on_wide_terminal() {
        let backend = TestBackend::new(200, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 200, 1), ScreenId::Aggregation, "0.4.0", &theme))
            .unwrap();
    }

    #[test]
    fn hit_test_returns_correct_screen_for_each_tab() {
        let backend = TestBackend::new(120, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 120, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
        // " agtop " (7) + "│ " (2) = 9 chars before first tab.
        // "[d]ashboard" is 11 chars wide, x = 9..=19.
        assert_eq!(bar.hit_test(14, 0), Some(ScreenId::Dashboard));
        // "[a]ggregation" starts at 9+11+2=22.
        assert_eq!(bar.hit_test(22, 0), Some(ScreenId::Aggregation));
        // "[c]onfig" starts at 22+13+2=37.
        assert_eq!(bar.hit_test(37, 0), Some(ScreenId::Config));
        // Inter-tab gap (column 20-21 are trailing spaces after dashboard tab).
        assert_eq!(bar.hit_test(20, 0), None);
        // Before any tab.
        assert_eq!(bar.hit_test(2, 0), None);
    }

    #[test]
    fn hit_test_wrong_row_returns_none() {
        let backend = TestBackend::new(120, 1);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut bar = TabBar::default();
        term.draw(|f| bar.render(f, Rect::new(0, 0, 120, 1), ScreenId::Dashboard, "0.4.0", &theme))
            .unwrap();
        assert_eq!(bar.hit_test(14, 1), None);
    }
}
