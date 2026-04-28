//! Inline color picker: 16 ANSI presets + hex input.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::theme_v2::Theme;

const PRESETS: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

#[derive(Debug, Default)]
pub struct ColorPicker {
    pub open: bool,
    pub hex_input: String,
    pub preview: Option<(u8, u8, u8)>,
}

impl ColorPicker {
    pub fn open_at(&mut self, current: (u8, u8, u8)) {
        self.open = true;
        self.preview = Some(current);
        self.hex_input = format!("{:02X}{:02X}{:02X}", current.0, current.1, current.2);
    }

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(" Pick a color  [Enter] confirm  [Esc] cancel ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.bg_overlay));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines = vec![
            Line::from(vec![Span::styled(
                " Presets:",
                Style::default().fg(theme.fg_muted),
            )]),
            Line::from(
                PRESETS
                    .iter()
                    .map(|(r, g, b)| {
                        Span::styled(" ████ ", Style::default().fg(Color::Rgb(*r, *g, *b)))
                    })
                    .collect::<Vec<_>>(),
            ),
            Line::from(""),
            Line::from(vec![
                Span::styled(" Hex: ", Style::default().fg(theme.fg_muted)),
                Span::styled(
                    format!("#{}", self.hex_input),
                    Style::default()
                        .fg(theme.fg_default)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        if let Some((r, g, b)) = self.preview {
            lines.push(Line::from(vec![
                Span::styled(" Preview: ", Style::default().fg(theme.fg_muted)),
                Span::styled("████", Style::default().fg(Color::Rgb(r, g, b))),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    pub fn confirm(&self) -> Option<(u8, u8, u8)> {
        parse_hex(&self.hex_input)
    }
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_accepts_6chars() {
        assert_eq!(parse_hex("ABCDEF"), Some((0xAB, 0xCD, 0xEF)));
        assert_eq!(parse_hex("123"), None);
        assert_eq!(parse_hex("ZZZZZZ"), None);
    }
}
