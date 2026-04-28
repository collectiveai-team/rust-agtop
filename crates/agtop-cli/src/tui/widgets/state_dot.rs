//! State dot widget: a single cell rendering `●` (or empty) for a `SessionState`.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.
#![allow(dead_code)]
//! Color, pulse, and modifier choices are delegated to `state_style`.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

use agtop_core::session::SessionState;

use crate::tui::animation::{dim_rgb, PulseClock};
use crate::tui::theme_v2::Theme;
use crate::tui::widgets::state_style;

/// Render the state dot for one row.
///
/// Reads color from `state_style::dot_color`. Applies pulsation when
/// `state_style::should_pulse(state)` is true and `animations` is enabled.
#[must_use]
pub fn render<'a>(
    state: &SessionState,
    pulse: &PulseClock,
    animations: bool,
    theme: &Theme,
) -> Span<'a> {
    let Some(color) = state_style::dot_color(state, theme) else {
        return Span::raw(" "); // Closed: no dot
    };

    let bold = matches!(
        state,
        SessionState::Running | SessionState::Error(_) | SessionState::Waiting(_)
    );

    let final_color = if animations && state_style::should_pulse(state) {
        match color {
            Color::Rgb(r, g, b) => {
                let (r, g, b) = dim_rgb(r, g, b, pulse.brightness());
                Color::Rgb(r, g, b)
            }
            other => other,
        }
    } else {
        color
    };

    let mut style = Style::default().fg(final_color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Span::styled("●", style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use agtop_core::session::{WaitReason, WarningReason};

    #[test]
    fn closed_renders_a_space() {
        let pulse = PulseClock::default();
        let s = render(
            &SessionState::Closed,
            &pulse,
            true,
            &vscode_dark_plus::theme(),
        );
        assert_eq!(s.content, " ");
    }

    #[test]
    fn running_renders_yellow_dot_bold() {
        let pulse = PulseClock::default();
        let theme = vscode_dark_plus::theme();
        let s = render(&SessionState::Running, &pulse, true, &theme);
        assert_eq!(s.content, "●");
        assert_eq!(s.style.fg, Some(theme.status_warning));
        assert!(s.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn idle_renders_green_dot_no_bold() {
        let pulse = PulseClock::default();
        let theme = vscode_dark_plus::theme();
        let s = render(&SessionState::Idle, &pulse, true, &theme);
        assert_eq!(s.style.fg, Some(theme.status_success));
        assert!(!s.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn waiting_animations_off_uses_static_accent_secondary() {
        let pulse = PulseClock::default();
        let theme = vscode_dark_plus::theme();
        let s = render(
            &SessionState::Waiting(WaitReason::Permission),
            &pulse,
            false,
            &theme,
        );
        assert_eq!(s.style.fg, Some(theme.accent_secondary));
    }

    #[test]
    fn waiting_animations_on_dims_accent_secondary() {
        let pulse = PulseClock::default();
        let theme = vscode_dark_plus::theme();
        let s = render(
            &SessionState::Waiting(WaitReason::Input),
            &pulse,
            true,
            &theme,
        );
        // Color must be Rgb (dimmed channels).
        assert!(matches!(s.style.fg, Some(Color::Rgb(_, _, _))));
    }

    #[test]
    fn warning_stalled_renders_orange_dot() {
        let pulse = PulseClock::default();
        let theme = vscode_dark_plus::theme();
        let s = render(
            &SessionState::Warning(WarningReason::Stalled {
                since: chrono::Utc::now(),
            }),
            &pulse,
            true,
            &theme,
        );
        assert_eq!(s.style.fg, Some(theme.status_attention));
    }
}
