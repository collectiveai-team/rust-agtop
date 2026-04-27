mod snapshot_helpers;

use ratatui::layout::Rect;

use agtop_cli::tui::screens::dashboard::quota::{
    QuotaCardModel, QuotaMode, QuotaPanel, WindowModel,
};
use agtop_cli::tui::theme_v2::vscode_dark_plus;
use agtop_core::session::ClientKind;

use snapshot_helpers::{buffer_to_text, render_to_buffer};

fn cards() -> Vec<QuotaCardModel> {
    vec![
        QuotaCardModel {
            client_kind: ClientKind::Claude,
            client_label: "claude-code".into(),
            closest: WindowModel { label: "5h".into(), used_pct: 0.78, note: None, reset_in: None },
            all_windows: vec![
                WindowModel { label: "5h".into(), used_pct: 0.78, note: None, reset_in: None },
                WindowModel { label: "weekly".into(), used_pct: 0.42, note: Some("142h to reset".into()), reset_in: None },
            ],
        },
        QuotaCardModel {
            client_kind: ClientKind::Codex,
            client_label: "codex".into(),
            closest: WindowModel { label: "weekly".into(), used_pct: 0.31, note: None, reset_in: None },
            all_windows: vec![
                WindowModel { label: "weekly".into(), used_pct: 0.31, note: None, reset_in: None },
            ],
        },
    ]
}

#[test]
fn quota_short_140x4_snapshot() {
    let theme = vscode_dark_plus::theme();
    let p = QuotaPanel {
        mode: QuotaMode::Short,
        cards: cards(),
        last_area: None,
        scroll_offset: 0,
    };
    let buf = render_to_buffer(140, 4, |f| p.render(f, Rect::new(0, 0, 140, 4), &theme));
    insta::assert_snapshot!("quota_short_140x4", buffer_to_text(&buf));
}

#[test]
fn quota_long_140x12_snapshot() {
    let theme = vscode_dark_plus::theme();
    let p = QuotaPanel {
        mode: QuotaMode::Long,
        cards: cards(),
        last_area: None,
        scroll_offset: 0,
    };
    let buf = render_to_buffer(140, 12, |f| p.render(f, Rect::new(0, 0, 140, 12), &theme));
    insta::assert_snapshot!("quota_long_140x12", buffer_to_text(&buf));
}
