//! New theme system (v2) — semantic tokens with true-color support.
//! Coexists with the legacy `theme` module during migration.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]

pub mod client_palette;
pub mod color;
pub mod vscode_dark_plus;

use ratatui::style::Color;

/// Semantic color slots. All widgets read colors via these slots, never raw `Color::*`.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    // Backgrounds
    pub bg_base: Color,
    pub bg_surface: Color,
    pub bg_overlay: Color,
    pub bg_selection: Color,
    // Foregrounds
    pub fg_default: Color,
    pub fg_muted: Color,
    pub fg_emphasis: Color,
    // Borders
    pub border_muted: Color,
    pub border_focused: Color,
    // Accents
    pub accent_primary: Color,
    pub accent_secondary: Color,
    // Status
    pub status_error: Color,
    pub status_warning: Color,
    pub status_attention: Color,
    pub status_success: Color,
    pub status_info: Color,
    // Syntax-style accents (for project paths, model names, etc.)
    pub syntax_string: Color,
    pub syntax_keyword: Color,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vscode_dark_plus_theme_loads() {
        let t = vscode_dark_plus::theme();
        // Spot-check a few well-known colors from the spec.
        assert_eq!(t.bg_base, Color::Rgb(0x1E, 0x1E, 0x1E));
        assert_eq!(t.accent_primary, Color::Rgb(0x00, 0x7A, 0xCC));
        assert_eq!(t.status_success, Color::Rgb(0x89, 0xD1, 0x85));
        assert_eq!(t.status_attention, Color::Rgb(0xD8, 0x96, 0x14));
    }
}
