//! Block-gradient progress bar. Used for memory, quota, etc.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]
//! Color shifts from green → yellow → orange → red as fill increases.

use ratatui::style::Color;

use crate::tui::theme_v2::Theme;

/// Render a horizontal gradient bar of length `width` with `pct` ∈ [0, 1] filled.
/// Returns (filled_string, color_for_filled, empty_string).
#[must_use]
pub fn render_bar(pct: f32, width: usize, theme: &Theme) -> (String, Color, String) {
    let pct = pct.clamp(0.0, 1.0);
    // Determine color zone.
    let color = if pct >= 0.95 {
        theme.status_error
    } else if pct >= 0.80 {
        theme.status_attention
    } else if pct >= 0.60 {
        theme.status_warning
    } else {
        theme.status_success
    };

    if width == 0 {
        return (String::new(), color, String::new());
    }

    // Use the 8-step block character family for sub-cell precision.
    let total_eighths = ((pct * width as f32) * 8.0).round() as usize;
    let full_cells = total_eighths / 8;
    let leftover = total_eighths % 8;
    let mut filled = "█".repeat(full_cells);
    if leftover > 0 && full_cells < width {
        const PARTIAL: [char; 8] = ['█', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
        filled.push(PARTIAL[leftover.min(7)]);
    }
    let empty_count = width.saturating_sub(filled.chars().count());
    let empty: String = "░".repeat(empty_count);
    (filled, color, empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tui::theme_v2::vscode_dark_plus;

    #[test]
    fn zero_pct_produces_empty_filled_full_empty() {
        let (filled, _, empty) = render_bar(0.0, 10, &vscode_dark_plus::theme());
        assert_eq!(filled.chars().count(), 0);
        assert_eq!(empty.chars().count(), 10);
    }

    #[test]
    fn full_pct_produces_full_filled_no_empty() {
        let (filled, _, empty) = render_bar(1.0, 10, &vscode_dark_plus::theme());
        assert_eq!(filled.chars().count(), 10);
        assert_eq!(empty.chars().count(), 0);
    }

    #[test]
    fn color_transitions_at_thresholds() {
        let theme = vscode_dark_plus::theme();
        let (_, c, _) = render_bar(0.30, 10, &theme);
        assert_eq!(c, theme.status_success);
        let (_, c, _) = render_bar(0.65, 10, &theme);
        assert_eq!(c, theme.status_warning);
        let (_, c, _) = render_bar(0.85, 10, &theme);
        assert_eq!(c, theme.status_attention);
        let (_, c, _) = render_bar(0.97, 10, &theme);
        assert_eq!(c, theme.status_error);
    }
}
