//! Config sidebar — section list (Appearance / Columns / …).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::msg::ConfigSection;
use crate::tui::theme_v2::Theme;
use crate::tui::widgets::icon::Icon;

const SECTIONS: [(ConfigSection, &str, Icon); 7] = [
    (ConfigSection::Appearance, "Appearance", Icon::Palette),
    (ConfigSection::Columns, "Columns", Icon::TableColumn),
    (ConfigSection::Refresh, "Refresh", Icon::Refresh),
    (ConfigSection::Clients, "Clients", Icon::AccountMultiple),
    (ConfigSection::Keybinds, "Keybinds", Icon::KeyboardOutline),
    (
        ConfigSection::DataSources,
        "Data sources",
        Icon::DatabaseOutline,
    ),
    (ConfigSection::About, "About", Icon::InformationOutline),
];

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    current: ConfigSection,
    nerd_font: bool,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme.border_muted));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = SECTIONS
        .iter()
        .map(|(sec, label, icon)| {
            let icon_str = icon.render(nerd_font);
            let prefix = if !icon_str.is_empty() {
                format!("  {icon_str}  ")
            } else {
                "  ".to_string()
            };
            if *sec == current {
                Line::from(vec![
                    Span::styled(prefix, Style::default().fg(theme.accent_primary)),
                    Span::styled(
                        format!("‹ {label} ›"),
                        Style::default()
                            .fg(theme.accent_primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled(prefix, Style::default().fg(theme.fg_muted)),
                    Span::styled(label.to_string(), Style::default().fg(theme.fg_muted)),
                ])
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn renders_without_panic() {
        let backend = TestBackend::new(30, 20);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        term.draw(|f| {
            render(
                f,
                Rect::new(0, 0, 30, 20),
                ConfigSection::Appearance,
                false,
                &theme,
            )
        })
        .unwrap();
    }
}
