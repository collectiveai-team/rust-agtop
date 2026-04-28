//! Per-client accent colors. Used to render client names in their brand color
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]
//! across the Sessions table, Aggregation view, Usage Quota labels, and Info drawer.

use agtop_core::session::ClientKind;
use ratatui::style::Color;

/// Returns the brand color for a client.
///
/// Returns a muted gray for unknown/future variants. The palette is
/// color-blind-safe at distinguishability and was chosen to render well
/// on the VS Code Dark+ background.
#[must_use]
pub const fn color_for(kind: ClientKind) -> Color {
    match kind {
        ClientKind::Claude => Color::Rgb(0xD9, 0x77, 0x57), // Claude Code orange
        ClientKind::Codex => Color::Rgb(0x00, 0xA6, 0x7E),  // Codex green
        ClientKind::GeminiCli => Color::Rgb(0x42, 0x85, 0xF4), // Google blue
        ClientKind::Copilot => Color::Rgb(0xFF, 0xD4, 0x3B), // Copilot yellow
        ClientKind::Cursor => Color::Rgb(0xA7, 0x8B, 0xFA), // Cursor purple
        ClientKind::Antigravity => Color::Rgb(0x22, 0xD3, 0xEE), // Antigravity cyan
        ClientKind::OpenCode => Color::Rgb(0xF4, 0x72, 0xB6), // OpenCode pink
        // Catch-all for any future variants added to the non_exhaustive enum.
        _ => Color::Rgb(0x6B, 0x72, 0x80),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_clients_have_color() {
        // These are the spec'd colors; failure means the spec drifted from impl.
        assert_eq!(color_for(ClientKind::Claude), Color::Rgb(0xD9, 0x77, 0x57));
        assert_eq!(color_for(ClientKind::Codex), Color::Rgb(0x00, 0xA6, 0x7E));
        assert_eq!(
            color_for(ClientKind::GeminiCli),
            Color::Rgb(0x42, 0x85, 0xF4)
        );
        assert_eq!(color_for(ClientKind::Copilot), Color::Rgb(0xFF, 0xD4, 0x3B));
        assert_eq!(color_for(ClientKind::Cursor), Color::Rgb(0xA7, 0x8B, 0xFA));
        assert_eq!(
            color_for(ClientKind::Antigravity),
            Color::Rgb(0x22, 0xD3, 0xEE)
        );
        assert_eq!(
            color_for(ClientKind::OpenCode),
            Color::Rgb(0xF4, 0x72, 0xB6)
        );
    }
}
