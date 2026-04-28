mod snapshot_helpers;

use ratatui::layout::Rect;

use agtop_cli::tui::msg::ScreenId;
use agtop_cli::tui::theme_v2::vscode_dark_plus;
use agtop_cli::tui::widgets::tab_bar::TabBar;

use snapshot_helpers::{buffer_to_text, render_to_buffer};

#[test]
fn tab_bar_dashboard_active_120_cols() {
    let theme = vscode_dark_plus::theme();
    let buf = render_to_buffer(120, 1, |f| {
        let mut bar = TabBar::default();
        bar.render(
            f,
            Rect::new(0, 0, 120, 1),
            ScreenId::Dashboard,
            "0.4.0",
            &theme,
        );
    });
    insta::assert_snapshot!("tab_bar_dashboard_120", buffer_to_text(&buf));
}

#[test]
fn tab_bar_config_active_140_cols() {
    let theme = vscode_dark_plus::theme();
    let buf = render_to_buffer(140, 1, |f| {
        let mut bar = TabBar::default();
        bar.render(
            f,
            Rect::new(0, 0, 140, 1),
            ScreenId::Config,
            "0.4.0",
            &theme,
        );
    });
    insta::assert_snapshot!("tab_bar_config_140", buffer_to_text(&buf));
}
