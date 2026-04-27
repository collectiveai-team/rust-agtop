mod snapshot_helpers;

use ratatui::layout::Rect;

use agtop_cli::tui::screens::aggregation::AggregationState;
use agtop_cli::tui::theme_v2::vscode_dark_plus;
use agtop_core::aggregate::{GroupBy, TimeRange};
use agtop_core::session::{
    ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
};

use snapshot_helpers::{buffer_to_text, render_to_buffer};

fn mk_session(client: ClientKind, hours_ago: i64, tokens: u64, cost: f64, duration: u64) -> SessionAnalysis {
    let now = chrono::Utc::now();
    let summary = SessionSummary::new(
        client,
        None,
        format!("session-{}", hours_ago),
        None,
        Some(now - chrono::Duration::hours(hours_ago)),
        None,
        None,
        std::path::PathBuf::from("/tmp/test"),
        None,
        None,
        None,
        None,
    );
    let mut tok = TokenTotals::default();
    tok.input = tokens;
    let mut c = CostBreakdown::default();
    c.total = cost;
    SessionAnalysis::new(summary, tok, c, None, 0, None, Some(duration), None, None, None)
}

fn fixture() -> Vec<SessionAnalysis> {
    vec![
        mk_session(ClientKind::Claude, 1, 12_400, 0.18, 180),
        mk_session(ClientKind::Claude, 3, 8_100, 0.09, 180),
        mk_session(ClientKind::Codex, 2, 4_200, 0.07, 180),
        mk_session(ClientKind::GeminiCli, 12, 2_100, 0.02, 180),
    ]
}

#[test]
fn aggregation_today_by_client_140x20() {
    let theme = vscode_dark_plus::theme();
    let mut state = AggregationState::default();
    state.sessions = fixture();
    state.controls.group_by = GroupBy::Client;
    state.controls.range = TimeRange::Today;
    state.recompute();
    let buf = render_to_buffer(140, 20, |f| state.render(f, Rect::new(0, 0, 140, 20), &theme));
    insta::assert_snapshot!("aggregation_today_by_client_140x20", buffer_to_text(&buf));
}
