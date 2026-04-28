mod snapshot_helpers;

use ratatui::layout::Rect;

use agtop_cli::tui::screens::dashboard::header::{render, HeaderModel};
use agtop_cli::tui::theme_v2::vscode_dark_plus;

use snapshot_helpers::{buffer_to_text, render_to_buffer};

fn realistic_model() -> HeaderModel {
    HeaderModel {
        procs: 12,
        cpu_history: vec![10.0, 20.0, 35.0, 45.0, 55.0, 40.0, 30.0, 25.0, 30.0, 50.0],
        cpu_max: 100.0,
        mem_used_bytes: 12 * 1_073_741_824,
        mem_total_bytes: 16 * 1_073_741_824,
        sessions_active: 8,
        sessions_idle: 3,
        sessions_today: 47,
        refresh_secs: 2,
        clock: "14:25:49".to_string(),
    }
}

#[test]
fn header_140_cols_snapshot() {
    let theme = vscode_dark_plus::theme();
    let model = realistic_model();
    let buf = render_to_buffer(140, 3, |f| {
        render(f, Rect::new(0, 0, 140, 3), &model, &theme)
    });
    insta::assert_snapshot!("header_140", buffer_to_text(&buf));
}

#[test]
fn header_80_cols_snapshot() {
    let theme = vscode_dark_plus::theme();
    let model = realistic_model();
    let buf = render_to_buffer(80, 3, |f| render(f, Rect::new(0, 0, 80, 3), &model, &theme));
    insta::assert_snapshot!("header_80", buffer_to_text(&buf));
}
