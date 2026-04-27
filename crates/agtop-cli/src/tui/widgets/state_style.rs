//! Style choices for `SessionState`. The single place where state → color/animation
//! decisions live in the TUI. State semantics are owned by `agtop-core`.

use ratatui::style::Color;

use agtop_core::session::{SessionState, WaitReason, WarningReason};

use crate::tui::theme_v2::Theme;

/// Returns the dot color for a state, or `None` for `Closed` (no dot).
#[must_use]
pub fn dot_color(state: &SessionState, theme: &Theme) -> Option<Color> {
    match state {
        SessionState::Running       => Some(theme.status_warning),
        SessionState::Waiting(_)    => Some(theme.accent_secondary),
        SessionState::Warning(_)    => Some(theme.status_attention),
        SessionState::Error(_)      => Some(theme.status_error),
        SessionState::Idle          => Some(theme.status_success),
        SessionState::Closed        => None,
        _                           => None, // future variants are non-rendering
    }
}

/// Whether the state's dot should pulsate (animation on).
#[must_use]
pub fn should_pulse(state: &SessionState) -> bool {
    matches!(state, SessionState::Waiting(_))
}

/// Whether row text should render dim/muted (closed sessions).
#[must_use]
pub fn is_muted_row(state: &SessionState) -> bool {
    matches!(state, SessionState::Closed)
}

/// Whether the ACTION cell for this state should use `status.warning` modifier
/// (permission-pending requires extra visual emphasis).
#[must_use]
pub fn action_needs_warning_modifier(state: &SessionState) -> bool {
    matches!(state, SessionState::Waiting(WaitReason::Permission))
}

/// Coarse text label suitable for the (hidden by default) STATE column.
/// More expressive than `as_str()` because it surfaces the Reason for ambiguity.
#[must_use]
pub fn label_for(state: &SessionState) -> &'static str {
    match state {
        SessionState::Running                                       => "running",
        SessionState::Waiting(WaitReason::Input)                    => "waiting:input",
        SessionState::Waiting(WaitReason::Permission)               => "waiting:perm",
        SessionState::Waiting(_)                                    => "waiting",
        SessionState::Warning(WarningReason::Stalled { .. })        => "stalled",
        SessionState::Warning(_)                                    => "warning",
        SessionState::Error(_)                                      => "error",
        SessionState::Idle                                          => "idle",
        SessionState::Closed                                        => "closed",
        _                                                            => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;

    fn theme() -> Theme { vscode_dark_plus::theme() }

    #[test]
    fn running_dot_is_status_warning() {
        let t = theme();
        assert_eq!(dot_color(&SessionState::Running, &t), Some(t.status_warning));
    }

    #[test]
    fn waiting_dot_is_accent_secondary_regardless_of_reason() {
        let t = theme();
        assert_eq!(
            dot_color(&SessionState::Waiting(WaitReason::Input), &t),
            Some(t.accent_secondary)
        );
        assert_eq!(
            dot_color(&SessionState::Waiting(WaitReason::Permission), &t),
            Some(t.accent_secondary)
        );
    }

    #[test]
    fn warning_dot_is_status_attention() {
        let t = theme();
        let s = SessionState::Warning(WarningReason::Stalled {
            since: chrono::Utc::now(),
        });
        assert_eq!(dot_color(&s, &t), Some(t.status_attention));
    }

    #[test]
    fn closed_has_no_dot() {
        let t = theme();
        assert_eq!(dot_color(&SessionState::Closed, &t), None);
    }

    #[test]
    fn waiting_should_pulse() {
        assert!(should_pulse(&SessionState::Waiting(WaitReason::Input)));
        assert!(!should_pulse(&SessionState::Running));
        assert!(!should_pulse(&SessionState::Idle));
    }

    #[test]
    fn closed_row_is_muted() {
        assert!(is_muted_row(&SessionState::Closed));
        assert!(!is_muted_row(&SessionState::Running));
    }

    #[test]
    fn permission_waiting_triggers_action_warning() {
        assert!(action_needs_warning_modifier(
            &SessionState::Waiting(WaitReason::Permission)
        ));
        assert!(!action_needs_warning_modifier(
            &SessionState::Waiting(WaitReason::Input)
        ));
        assert!(!action_needs_warning_modifier(&SessionState::Running));
    }

    #[test]
    fn label_distinguishes_wait_reasons() {
        assert_eq!(
            label_for(&SessionState::Waiting(WaitReason::Input)),
            "waiting:input"
        );
        assert_eq!(
            label_for(&SessionState::Waiting(WaitReason::Permission)),
            "waiting:perm"
        );
    }
}
