mod snapshot_helpers;

use ratatui::layout::Rect;

use agtop_cli::tui::screens::aggregation::{controls::ControlsModel, AggregationState};
use agtop_cli::tui::theme_v2::vscode_dark_plus;
use agtop_core::aggregate::{GroupBy, TimeRange};
use agtop_core::session::{
    ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals,
};

use snapshot_helpers::{buffer_to_text, render_to_buffer};

/// Fixed "now" for deterministic aggregation: 2026-04-26T12:00:00Z
fn fixed_now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-04-26T12:00:00Z")
        .unwrap()
        .to_utc()
}

fn mk_session(client: ClientKind, hours_ago: i64, tokens: u64, cost: f64, duration: u64) -> SessionAnalysis {
    let now = fixed_now();
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
    );
    #[allow(clippy::field_reassign_with_default)]
    let tok = {
        let mut t = TokenTotals::default();
        t.input = tokens;
        t
    };
    #[allow(clippy::field_reassign_with_default)]
    let c = {
        let mut cb = CostBreakdown::default();
        cb.total = cost;
        cb
    };
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

/// Replace time-relative strings like "1h ago", "42m ago", "3d ago", "just now"
/// with a stable placeholder so snapshots don't break every hour.
fn redact_relative_time(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        // Replace patterns: <N>h ago / <N>m ago / <N>d ago / just now
        let mut result = line.to_string();
        // Simple two-pass: find suffixes "h ago", "m ago", "d ago", "just now"
        for suffix in &["h ago", "m ago", "d ago"] {
            while let Some(pos) = result.find(suffix) {
                // Walk backward to find start of number.
                let before = &result[..pos];
                let start = before
                    .rfind(|c: char| !c.is_ascii_digit())
                    .map(|p| p + 1)
                    .unwrap_or(0);
                let after = &result[pos + suffix.len()..].to_string();
                result = format!("{}<TIME>{}", &result[..start], after);
            }
        }
        result = result.replace("just now", "<TIME>   ");
        out.push_str(&result);
        out.push('\n');
    }
    // Remove trailing newline added by the loop.
    if out.ends_with('\n') { out.pop(); }
    out
}

#[test]
fn aggregation_today_by_client_140x20() {
    let theme = vscode_dark_plus::theme();
    let fixed_now = fixed_now();
    let mut state = AggregationState {
        sessions: fixture(),
        controls: ControlsModel {
            group_by: GroupBy::Client,
            range: TimeRange::Today,
            ..ControlsModel::default()
        },
        ..AggregationState::default()
    };
    // Recompute with fixed now so filtering is deterministic.
    {
        let groups = agtop_core::aggregate::aggregate(
            &state.sessions,
            state.controls.group_by,
            state.controls.range,
            fixed_now,
            12,
        );
        state.table.groups = groups;
    }
    let buf = render_to_buffer(140, 20, |f| state.render(f, Rect::new(0, 0, 140, 20), &theme));
    let text = buffer_to_text(&buf);
    let stable_text = redact_relative_time(&text);
    insta::assert_snapshot!("aggregation_today_by_client_140x20", stable_text);
}
