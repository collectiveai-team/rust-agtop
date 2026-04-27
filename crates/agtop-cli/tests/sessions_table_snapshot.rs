mod snapshot_helpers;

use ratatui::layout::Rect;

use agtop_cli::tui::screens::dashboard::sessions::{SessionRow, SessionsTable};
use agtop_cli::tui::theme_v2::vscode_dark_plus;
use agtop_core::session::{
    ClientKind, CostBreakdown, SessionAnalysis, SessionState, SessionSummary, TokenTotals,
    WaitReason, WarningReason,
};

use snapshot_helpers::{buffer_to_text, render_to_buffer};

fn mk(
    id: &str,
    client: ClientKind,
    label: &str,
    state: SessionState,
    action: Option<&str>,
    age_min: i64,
    samples: Vec<f32>,
) -> SessionRow {
    let summary = SessionSummary::new(
        client,
        None,
        id.to_string(),
        None,
        Some(chrono::Utc::now() - chrono::Duration::minutes(age_min)),
        None,
        None,
        std::path::PathBuf::new(),
        None,
        None,
        None,
        None,
    );
    let mut analysis = SessionAnalysis::new(
        summary,
        TokenTotals::default(),
        CostBreakdown::default(),
        None,
        0,
        None,
        None,
        None,
        None,
        None,
    );
    analysis.session_state = Some(state);
    analysis.current_action = action.map(String::from);
    SessionRow {
        analysis,
        client_kind: client,
        client_label: label.into(),
        activity_samples: samples,
        depth: 0,
        parent_session_id: None,
    }
}

fn fixture() -> Vec<SessionRow> {
    vec![
        mk(
            "a3f2c1de",
            ClientKind::Claude,
            "claude-code",
            SessionState::Running,
            Some("Bash: cargo test"),
            3,
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        ),
        mk(
            "b81e9402",
            ClientKind::Claude,
            "claude-code",
            SessionState::Waiting(WaitReason::Permission),
            Some("permission: bash"),
            12,
            vec![1.0, 1.0, 2.0, 3.0, 2.0, 1.0],
        ),
        mk(
            "c2d4f6a1",
            ClientKind::Codex,
            "codex",
            SessionState::Running,
            Some("Edit: src/lib.rs"),
            1,
            vec![3.0, 4.0, 5.0, 6.0, 5.0, 4.0],
        ),
        mk(
            "d9e8a7b3",
            ClientKind::GeminiCli,
            "gemini-cli",
            SessionState::Idle,
            None,
            18,
            vec![1.0; 6],
        ),
        mk(
            "e5f4a3b2",
            ClientKind::Copilot,
            "copilot",
            SessionState::Closed,
            None,
            120,
            vec![],
        ),
        mk(
            "f1e2d3c4",
            ClientKind::Codex,
            "codex",
            SessionState::Warning(WarningReason::Stalled {
                since: chrono::Utc::now() - chrono::Duration::minutes(8),
            }),
            None,
            8,
            vec![1.0, 0.5, 0.0],
        ),
    ]
}

#[test]
fn sessions_table_140x12_snapshot() {
    let theme = vscode_dark_plus::theme();
    let mut table = SessionsTable {
        rows: fixture(),
        animations_enabled: false, // deterministic for snapshot
        ..SessionsTable::default()
    };
    table.apply_sort();
    let buf = render_to_buffer(140, 12, |f| {
        table.render(f, Rect::new(0, 0, 140, 12), &theme)
    });
    insta::assert_snapshot!("sessions_table_140x12", buffer_to_text(&buf));
}

#[test]
fn sessions_table_renders_selected_row_highlight() {
    let theme = vscode_dark_plus::theme();
    let mut table = SessionsTable {
        rows: fixture(),
        animations_enabled: false,
        ..SessionsTable::default()
    };
    table.state.select(Some(1));

    let buf = render_to_buffer(140, 12, |f| {
        table.render(f, Rect::new(0, 0, 140, 12), &theme)
    });

    // The highlight is now bg_selection + fg_emphasis (no REVERSED).
    // Check that at least one cell on the selected row has bg == theme.bg_selection.
    let selected_row_y = 2;
    let highlighted = (0..buf.area.width).any(|x| {
        buf[(x, selected_row_y)].style().bg == Some(theme.bg_selection)
    });
    assert!(
        highlighted,
        "selected row should render with bg_selection background as visible highlight"
    );
}

#[test]
fn sessions_table_default_sort_shows_recent_sessions_first() {
    let mut table = SessionsTable {
        rows: fixture(),
        ..SessionsTable::default()
    };

    table.apply_sort();

    assert_eq!(
        table
            .rows
            .first()
            .map(|row| row.analysis.summary.session_id.as_str()),
        Some("c2d4f6a1"),
        "default dashboard sort should put the most recently active session first"
    );
}
