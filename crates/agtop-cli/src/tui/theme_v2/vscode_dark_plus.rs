//! VS Code Dark+ color palette as a `Theme`.

use ratatui::style::Color;

use super::Theme;

/// Returns the VS Code Dark+ theme.
#[must_use]
pub const fn theme() -> Theme {
    Theme {
        bg_base: Color::Rgb(0x1E, 0x1E, 0x1E),
        bg_surface: Color::Rgb(0x25, 0x25, 0x26),
        bg_overlay: Color::Rgb(0x2D, 0x2D, 0x30),
        bg_selection: Color::Rgb(0x26, 0x4F, 0x78),

        fg_default: Color::Rgb(0xD4, 0xD4, 0xD4),
        fg_muted: Color::Rgb(0x85, 0x85, 0x85),
        fg_emphasis: Color::Rgb(0xFF, 0xFF, 0xFF),

        border_muted: Color::Rgb(0x3C, 0x3C, 0x3C),
        border_focused: Color::Rgb(0x00, 0x7A, 0xCC),

        accent_primary: Color::Rgb(0x00, 0x7A, 0xCC),
        accent_secondary: Color::Rgb(0xC5, 0x86, 0xC0),

        status_error: Color::Rgb(0xF4, 0x87, 0x71),
        status_warning: Color::Rgb(0xCC, 0xA7, 0x00),
        status_attention: Color::Rgb(0xD8, 0x96, 0x14),
        status_success: Color::Rgb(0x89, 0xD1, 0x85),
        status_info: Color::Rgb(0x4F, 0xC1, 0xFF),

        syntax_string: Color::Rgb(0xCE, 0x91, 0x78),
        syntax_keyword: Color::Rgb(0x56, 0x9C, 0xD6),
    }
}
