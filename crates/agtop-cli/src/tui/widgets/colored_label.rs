//! Colored label helper: render a string in a per-client (or arbitrary) color.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]

use ratatui::{
    style::{Modifier, Style},
    text::Span,
};

use agtop_core::session::ClientKind;

use crate::tui::theme_v2::client_palette;

/// Build a styled `Span` for a client name in its theme color, no modifiers.
#[must_use]
pub fn client_span<'a>(name: &'a str, kind: ClientKind) -> Span<'a> {
    Span::styled(
        name,
        Style::default().fg(client_palette::color_for(kind)),
    )
}

/// Same as `client_span` but with a bold modifier (used for selected row).
#[must_use]
pub fn client_span_bold<'a>(name: &'a str, kind: ClientKind) -> Span<'a> {
    Span::styled(
        name,
        Style::default()
            .fg(client_palette::color_for(kind))
            .add_modifier(Modifier::BOLD),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn client_span_uses_palette_color() {
        let s = client_span("claude", ClientKind::Claude);
        assert_eq!(s.style.fg, Some(Color::Rgb(0xD9, 0x77, 0x57)));
    }

    #[test]
    fn bold_variant_adds_bold_modifier() {
        let s = client_span_bold("codex", ClientKind::Codex);
        assert!(s.style.add_modifier.contains(Modifier::BOLD));
    }
}
