//! Render helpers for setting controls.

use ratatui::{
    style::{Modifier, Style},
    text::Span,
};

use crate::tui::theme_v2::Theme;

pub fn checkbox<'a>(checked: bool, theme: &Theme) -> Span<'a> {
    let s = if checked { "[x]" } else { "[ ]" };
    let color = if checked {
        theme.accent_primary
    } else {
        theme.fg_muted
    };
    Span::styled(
        s.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

pub fn radio<'a>(selected: bool, theme: &Theme) -> Span<'a> {
    let s = if selected { "(•)" } else { "( )" };
    let color = if selected {
        theme.accent_primary
    } else {
        theme.fg_muted
    };
    Span::styled(s.to_string(), Style::default().fg(color))
}

pub fn dropdown<'a>(value: &str, theme: &Theme) -> Span<'a> {
    Span::styled(
        format!("[ {value} ▾ ]"),
        Style::default()
            .fg(theme.fg_default)
            .add_modifier(Modifier::BOLD),
    )
}

pub fn text_input<'a>(value: &'a str, theme: &Theme) -> Span<'a> {
    Span::styled(
        format!("[ {value} ]"),
        Style::default().fg(theme.fg_default),
    )
}

pub fn swatch<'a>(rgb: (u8, u8, u8), theme: &Theme) -> Vec<Span<'a>> {
    let _ = theme;
    let block = Span::styled(
        "████".to_string(),
        Style::default().fg(ratatui::style::Color::Rgb(rgb.0, rgb.1, rgb.2)),
    );
    let hex = Span::styled(
        format!("  #{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2),
        Style::default(),
    );
    vec![block, hex]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;

    #[test]
    fn checkbox_uses_x_when_checked() {
        let s = checkbox(true, &vscode_dark_plus::theme());
        assert_eq!(s.content, "[x]");
    }

    #[test]
    fn radio_uses_dot_when_selected() {
        let s = radio(true, &vscode_dark_plus::theme());
        assert_eq!(s.content, "(•)");
    }
}
