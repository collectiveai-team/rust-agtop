mod snapshot_helpers;

use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use ratatui::text::Line;

use agtop_cli::tui::animation::PulseClock;
use agtop_cli::tui::theme_v2::vscode_dark_plus;
use agtop_cli::tui::widgets::state_dot;
use agtop_core::session::{SessionState, WaitReason, WarningReason};

use snapshot_helpers::{render_to_buffer, buffer_to_text};

fn render_dots_row(animations: bool) -> String {
    let theme = vscode_dark_plus::theme();
    let pulse = PulseClock::default();
    let states: Vec<SessionState> = vec![
        SessionState::Running,
        SessionState::Waiting(WaitReason::Input),
        SessionState::Warning(WarningReason::Stalled { since: chrono::Utc::now() }),
        SessionState::Error(agtop_core::session::ErrorReason::Crash),
        SessionState::Idle,
        SessionState::Closed,
    ];
    let buf = render_to_buffer(20, 1, |f| {
        let line = Line::from(
            states
                .iter()
                .flat_map(|s| {
                    [
                        state_dot::render(s, &pulse, animations, &theme),
                        ratatui::text::Span::raw(" "),
                    ]
                })
                .collect::<Vec<_>>(),
        );
        f.render_widget(Paragraph::new(line), Rect::new(0, 0, 20, 1));
    });
    buffer_to_text(&buf)
}

#[test]
fn state_dots_animations_off_snapshot() {
    let text = render_dots_row(false);
    insta::assert_snapshot!("state_dots_anim_off", text);
}

#[test]
fn state_dots_animations_on_snapshot() {
    // Animation factor is time-dependent; we only verify the textual content
    // (glyph positions). Color is in the buffer's Style which we strip.
    let text = render_dots_row(true);
    insta::assert_snapshot!("state_dots_anim_on", text);
}
