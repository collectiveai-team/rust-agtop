//! Semantic icon registry. The single source of truth for which characters
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]
//! we render where.
//!
//! Tier 0 (T0) is universal Unicode (always renders). Tier 2 (T2) is
//! Nerd Font Material Design Icons (opt-in via `appearance.nerd_font`).

/// All icon sites in the app. Adding a new icon = add a variant + its codepoint + fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    // Header
    Refresh,
    Pause,
    Clock,
    Procs,
    // Info drawer hero
    Folder,
    // Config sections
    Palette,
    TableColumn,
    AccountMultiple,
    KeyboardOutline,
    DatabaseOutline,
    InformationOutline,
    // Config controls
    Search,
    // Empty states
    Tray,
    DatabaseOff,
    AlertCircle,
}

impl Icon {
    /// Render the icon as a string slice. When `nerd_font` is true, returns
    /// the Material Design Icon codepoint. Otherwise returns the T0/text fallback,
    /// which may be empty for icons that have no compatible fallback.
    #[must_use]
    pub fn render(self, nerd_font: bool) -> &'static str {
        match (self, nerd_font) {
            // T2 with T0 fallback
            (Self::Refresh, true) => "\u{F01DA}", // nf-md-refresh
            (Self::Refresh, false) => "\u{27F3}", // ⟳
            (Self::Pause, true) => "\u{F03E4}",   // nf-md-pause
            (Self::Pause, false) => "\u{2016}",   // ‖
            (Self::Search, true) => "\u{F0349}",  // nf-md-magnify
            (Self::Search, false) => "/",
            // T2 with no fallback (omitted when NF off)
            (Self::Clock, true) => "\u{F0954}", // nf-md-clock_outline
            (Self::Clock, false) => "",
            (Self::Procs, true) => "\u{F08BB}", // nf-md-cog_outline
            (Self::Procs, false) => "",
            (Self::Folder, true) => "\u{F024B}", // nf-md-folder
            (Self::Folder, false) => "",
            (Self::Palette, true) => "\u{F03D8}", // nf-md-palette
            (Self::Palette, false) => "",
            (Self::TableColumn, true) => "\u{F1377}", // nf-md-table_column
            (Self::TableColumn, false) => "",
            (Self::AccountMultiple, true) => "\u{F08C9}", // nf-md-account_multiple_outline
            (Self::AccountMultiple, false) => "",
            (Self::KeyboardOutline, true) => "\u{F097B}", // nf-md-keyboard_outline
            (Self::KeyboardOutline, false) => "",
            (Self::DatabaseOutline, true) => "\u{F01BC}", // nf-md-database_outline
            (Self::DatabaseOutline, false) => "",
            (Self::InformationOutline, true) => "\u{F02FD}", // nf-md-information_outline
            (Self::InformationOutline, false) => "",
            (Self::Tray, true) => "\u{F0DF9}", // nf-md-tray
            (Self::Tray, false) => "",
            (Self::DatabaseOff, true) => "\u{F0A8E}", // nf-md-database_off_outline (approximate)
            (Self::DatabaseOff, false) => "",
            (Self::AlertCircle, true) => "\u{F015A}", // nf-md-alert_circle_outline
            (Self::AlertCircle, false) => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_has_t0_fallback() {
        assert_eq!(Icon::Refresh.render(false), "⟳");
    }

    #[test]
    fn refresh_renders_nerd_font_when_enabled() {
        let s = Icon::Refresh.render(true);
        assert!(!s.is_empty());
        assert_ne!(s, "⟳");
    }

    #[test]
    fn folder_has_no_fallback_when_nerd_font_off() {
        assert_eq!(Icon::Folder.render(false), "");
    }

    #[test]
    fn search_falls_back_to_slash() {
        assert_eq!(Icon::Search.render(false), "/");
    }
}
