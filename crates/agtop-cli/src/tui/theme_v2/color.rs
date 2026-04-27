//! Terminal color-tier detection and true-color RGB → 256/16 downsampling.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]

/// What level of color the terminal can render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTier {
    /// 16 ANSI colors only.
    Ansi16,
    /// 256-color palette.
    Indexed256,
    /// 24-bit true color.
    TrueColor,
}

/// Detect color tier from current environment variables.
#[must_use]
pub fn detect() -> ColorTier {
    detect_with_env(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
        std::env::var("NO_COLOR").ok().as_deref(),
    )
}

#[must_use]
pub fn detect_with_env(
    colorterm: Option<&str>,
    term: Option<&str>,
    no_color: Option<&str>,
) -> ColorTier {
    if no_color.is_some() {
        return ColorTier::Ansi16;
    }
    if matches!(colorterm, Some("truecolor") | Some("24bit")) {
        return ColorTier::TrueColor;
    }
    if let Some(t) = term {
        if t.contains("256color") {
            return ColorTier::Indexed256;
        }
    }
    ColorTier::Ansi16
}

/// User-facing config setting that overrides detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrueColorMode {
    /// Detect via env vars.
    #[default]
    Auto,
    /// Force true color.
    On,
    /// Force 256-color downsampling.
    Off,
}

impl TrueColorMode {
    pub fn resolve(self) -> ColorTier {
        match self {
            Self::Auto => detect(),
            Self::On => ColorTier::TrueColor,
            Self::Off => ColorTier::Indexed256,
        }
    }
}

/// Downsample a 24-bit RGB color to the closest xterm 256-color index.
/// Uses the standard 6×6×6 cube + 24-step grayscale ramp.
#[must_use]
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    // Grayscale ramp: indices 232–255 (24 steps from #080808 to #eeeeee).
    if r == g && g == b {
        if r < 8 { return 16; }
        if r > 248 { return 231; }
        return 232 + ((r as u16 - 8) * 24 / 247) as u8;
    }
    // 6×6×6 color cube: indices 16–231.
    fn step(c: u8) -> u8 {
        // 0, 95, 135, 175, 215, 255 are the cube steps.
        match c {
            0..=47 => 0,
            48..=114 => 1,
            115..=154 => 2,
            155..=194 => 3,
            195..=234 => 4,
            _ => 5,
        }
    }
    16 + 36 * step(r) + 6 * step(g) + step(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_truecolor_when_colorterm_set() {
        let tier = detect_with_env(Some("truecolor"), Some("xterm-256color"), None);
        assert_eq!(tier, ColorTier::TrueColor);

        let tier = detect_with_env(Some("24bit"), Some("xterm"), None);
        assert_eq!(tier, ColorTier::TrueColor);
    }

    #[test]
    fn detect_returns_256_when_term_is_256color() {
        let tier = detect_with_env(None, Some("xterm-256color"), None);
        assert_eq!(tier, ColorTier::Indexed256);
    }

    #[test]
    fn detect_returns_ansi16_otherwise() {
        let tier = detect_with_env(None, Some("xterm"), None);
        assert_eq!(tier, ColorTier::Ansi16);
    }

    #[test]
    fn detect_respects_no_color() {
        let tier = detect_with_env(Some("truecolor"), Some("xterm-256color"), Some(""));
        assert_eq!(tier, ColorTier::Ansi16); // NO_COLOR forces minimum
    }
}

#[cfg(test)]
mod downsample_tests {
    use super::*;

    #[test]
    fn pure_black_maps_to_16() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
    }

    #[test]
    fn pure_white_maps_to_231() {
        assert_eq!(rgb_to_256(255, 255, 255), 231);
    }

    #[test]
    fn vscode_dark_plus_blue_maps_to_cube_index() {
        // #007ACC → step(0)=0, step(0x7A=122)=2, step(0xCC=204)=4
        // → 16 + 36*0 + 6*2 + 4 = 32
        assert_eq!(rgb_to_256(0x00, 0x7A, 0xCC), 32);
    }

    #[test]
    fn mid_gray_lands_in_grayscale_ramp() {
        let idx = rgb_to_256(128, 128, 128);
        assert!((232..=255).contains(&idx), "expected grayscale, got {idx}");
    }
}
