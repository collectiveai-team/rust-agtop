#![allow(dead_code, unused)]
//! Top half of the TUI: the session list table.
//!
//! Pure rendering — all business logic (sort / filter / selection)
//! lives in [`crate::tui::app`]. This module just turns the app
//! snapshot into ratatui widgets.

use chrono::{DateTime, Utc};
use ratatui::{
    layout::Constraint,
    prelude::*,
    style::Style,
    widgets::{Block, Borders, Cell, Row, Table, TableState},
};

use crate::fmt;
use crate::tui::app::{App, SortColumn, SortDir};
use crate::tui::column_config::ColumnId;
use crate::tui::theme as th;
use agtop_core::session::SessionState;

/// Render the session table into `area`. Takes a `TableState` so scroll
/// offset survives redraws — ratatui doesn't maintain it internally.
///
/// `header_cols` is overwritten with the absolute terminal x-ranges of
/// every sortable header cell so the mouse handler can hit-test clicks.
///
/// `logo_rects` is overwritten with one entry per visible data row: the
/// `Rect` of the `SubscriptionLogo` cell for that row, paired with the
/// `ClientKind` of that session.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    state: &mut TableState,
    header_cols: &mut Vec<(u16, u16, SortColumn)>,
    logo_rects: &mut Vec<(Rect, agtop_core::ClientKind)>,
) {
    // Sync the widget's idea of selection with the app's.
    state.select(app.selected_idx());

    let header_style = th::HEADER;

    // Build a compact arrow so the header advertises the active sort
    // column without stealing a full column of width.
    let arrow = match app.sort_dir() {
        SortDir::Asc => "↑",
        SortDir::Desc => "↓",
    };

    let col_cfg = app.column_config();
    // Hide the SubscriptionLogo slot when no logos are loaded
    // (terminals without a graphics protocol). Otherwise the column
    // reserves dead space.
    let visible = col_cfg.visible_ext(app.has_logos());

    let header_cells: Vec<Cell<'static>> = visible
        .iter()
        .map(|&col_id| {
            let label = col_id.label();
            match col_id.sort_col() {
                Some(sc) => header_with_marker(label, sc, app, arrow),
                None => header_cell(label),
            }
        })
        .collect();
    let header = Row::new(header_cells).style(header_style).height(1);

    let last_idx = visible.len().saturating_sub(1);
    let widths: Vec<Constraint> = visible
        .iter()
        .enumerate()
        .map(|(i, &col_id)| {
            if col_id.is_flexible() || i == last_idx {
                Constraint::Fill(1)
            } else {
                Constraint::Length(col_id.fixed_width().unwrap_or(8))
            }
        })
        .collect();

    // ── Compute absolute x-ranges for sortable header cells ──────────────
    //
    // Mirror ratatui's Table::get_columns_widths + render_header exactly:
    //
    //   ratatui flow:
    //     table_area  = block.inner_if_some(outer_area)   // strips borders
    //     col_widths  = get_columns_widths(table_area.width, selection_width, ...)
    //                   → splits Rect::new(0, 0, table_area.width, 1) into
    //                     [selection_area, columns_area]; then columns_area
    //                     is split by widths/spacing.
    //     render_header(header_area, ...) renders each cell at
    //                     header_area.x + col_x  (where col_x from above)
    //     header_area.x == table_area.x  (same horizontal origin, no extra offset)
    //
    //   Therefore absolute screen x = table_area.x + col_x_from_get_columns_widths.
    //
    // This is the only way to get correct offsets when the terminal is
    // narrower than the sum of all Length constraints (ratatui compresses
    // them proportionally; our old manual cursor arithmetic did not).
    header_cols.clear();

    // `"▶ "` is 2 terminal columns wide (▶ = 1 col, space = 1 col).
    // HighlightSpacing::WhenSelected (ratatui default) reserves the symbol
    // width only when a row is selected.
    let selection_width: u16 = if app.selected_idx().is_some() { 2 } else { 0 };

    // table_area (inner) dimensions, matching ratatui's block.inner_if_some(area).
    let table_inner_x = area.x + 1;
    let inner_width = area.width.saturating_sub(2);

    // Mirror Table::get_columns_widths: split a 0-based rect of inner_width
    // into [selection_area, columns_area], then split columns_area by our widths.
    let columns_area = {
        let [_, cols] =
            Layout::horizontal([Constraint::Length(selection_width), Constraint::Fill(0)])
                .areas::<2>(Rect::new(0, 0, inner_width, 1));
        cols
    };

    // Split exactly as Table does: Layout::horizontal(widths).spacing(1).
    // clone: widths ownership is needed by Table::new below.
    let col_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(widths.clone())
        .spacing(1)
        .split(columns_area);

    for (i, &col_id) in visible.iter().enumerate() {
        if let Some(sc) = col_id.sort_col() {
            let r = col_rects[i];
            // col_rect.x is relative to the 0-based inner rect; add table_inner_x
            // to get the absolute terminal column, matching render_header's offset.
            let abs_start = table_inner_x + r.x;
            header_cols.push((abs_start, abs_start + r.width, sc));
        }
    }
    // ─────────────────────────────────────────────────────────────────────

    let title = format!(
        " Sessions ({visible}/{total})  sort:{col}{dir}  filter:\"{f}\" ",
        visible = app.view_len(),
        total = app.total_count(),
        col = app.sort_col().label(),
        dir = match app.sort_dir() {
            SortDir::Asc => "↑",
            SortDir::Desc => "↓",
        },
        f = app.filter(),
    );

    let now = Utc::now();
    let view = app.view_with_kinds();
    let rows: Vec<Row> = view
        .iter()
        .map(|(a, is_child)| {
            let kind = if *is_child {
                RowKind::Child
            } else if !a.children.is_empty() {
                if app.is_expanded(&a.summary.session_id) {
                    RowKind::ExpandedParent
                } else {
                    RowKind::CollapsedParent
                }
            } else {
                RowKind::Normal
            };
            row_for(a, now, &visible, kind)
        })
        .collect();

    let table = Table::new(rows, widths.clone())
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .row_highlight_style(th::SELECTED)
        .highlight_symbol("▶ ");

    frame.render_stateful_widget(table, area, state);

    // ── Compute SubscriptionLogo column rects for the post-table overlay ──
    //
    // Find the index of SubscriptionLogo in the visible column list, then
    // re-use the same layout split that was already computed for header_cols
    // to determine where that column lands on screen for each data row.
    let logo_col_idx = visible
        .iter()
        .position(|&c| c == ColumnId::SubscriptionLogo);

    logo_rects.clear();

    if let Some(logo_idx) = logo_col_idx {
        // Reuse col_rects (already computed above for header_cols).
        // Only x and width are used from col_rects (y is the header row;
        // we replace it with screen_row below).
        let logo_col_rect = col_rects[logo_idx];

        // Data rows start at area.y + 2 (border row + header row).
        let data_start_y = area.y + 2;

        // Skip rows that are scrolled off the top; screen_row is relative
        // to the first *visible* row, not the start of the view slice.
        let scroll_offset = state.offset();
        for (row_idx, (analysis, _is_child)) in view.iter().enumerate().skip(scroll_offset) {
            let screen_row = data_start_y + (row_idx - scroll_offset) as u16;
            if screen_row >= area.y + area.height.saturating_sub(1) {
                break; // outside the visible area (bottom border)
            }
            let rect = Rect::new(logo_col_rect.x, screen_row, logo_col_rect.width, 1);
            logo_rects.push((rect, analysis.summary.client));
        }
    }
    // ─────────────────────────────────────────────────────────────────────
}

fn header_cell(s: &'static str) -> Cell<'static> {
    Cell::from(s)
}

/// Format and colorize the context usage column.
///
/// Returns `(text, style)`. The color threshold gates are:
/// - < 30%  → green
/// - 30–80% → yellow
/// - ≥ 80%  → red
fn format_context(
    used_tokens: Option<u64>,
    window: Option<u64>,
    pct: Option<f64>,
) -> (String, Style) {
    let Some(pct_val) = pct else {
        return ("-".to_string(), Style::new());
    };

    let text = match (used_tokens, window) {
        (Some(u), Some(w)) => format!("{}/{} ({:.1}%)", fmt::compact(u), fmt::compact(w), pct_val),
        _ => format!("({:.1}%)", pct_val),
    };

    let style = if pct_val >= 80.0 {
        th::PLAN_BAR_RED
    } else if pct_val >= 30.0 {
        th::PLAN_BAR_YELLOW
    } else {
        th::PLAN_BAR_GREEN
    };

    (text, style)
}

/// Append a direction arrow to the column header when it matches the
/// app's active sort column. This is what htop does with F6.
fn header_with_marker(
    label: &'static str,
    col: SortColumn,
    app: &App,
    arrow: &'static str,
) -> Cell<'static> {
    if app.sort_col() == col {
        Cell::from(format!("{label}{arrow}"))
    } else {
        Cell::from(label)
    }
}

fn row_for<'a>(
    a: &'a agtop_core::session::SessionAnalysis,
    now: DateTime<Utc>,
    visible: &[ColumnId],
    kind: RowKind,
) -> Row<'a> {
    let s = &a.summary;

    // For collapsed parents show merged (parent + all children) totals.
    let (t_owned, c_owned);
    let (t, c) = match kind {
        RowKind::CollapsedParent => {
            t_owned = merged_tokens(a);
            c_owned = merged_cost(a);
            (&t_owned, &c_owned)
        }
        _ => (&a.tokens, &a.cost),
    };

    let started = s
        .started_at
        .map(fmt::format_local_datetime)
        .unwrap_or_else(|| "-".into());
    let age = s
        .last_active
        .map(|ts| fmt::relative_age(ts, now))
        .unwrap_or_else(|| "-".into());
    let last_active_abs = s
        .last_active
        .map(fmt::format_local_datetime)
        .unwrap_or_else(|| "-".into());
    let closed = SessionState::Closed;
    let session_state = a.session_state.as_ref().unwrap_or(&closed);
    let state = session_state.as_str();
    let state_style = match session_state {
        SessionState::Running => th::STATE_WORKING,
        SessionState::Waiting(_) => th::STATE_WAITING,
        _ => th::STATE_STALE,
    };
    let effort = s.model_effort.clone().unwrap_or_else(|| "-".into());
    let model = s.model.clone().unwrap_or_else(|| "?".into());
    let subscription = s.subscription.clone().unwrap_or_else(|| "-".into());
    let cwd = fmt::shorten_path(s.cwd.as_deref().unwrap_or("-"));
    let cost_str = if c.included {
        "incl".to_string()
    } else {
        format!("{:.4}", c.total)
    };
    let short = {
        let id = fmt::short_id(&s.session_id);
        match kind {
            RowKind::CollapsedParent => format!("▶ {}+{}", id, a.subagent_file_count),
            RowKind::ExpandedParent => format!("▼ {}", id),
            RowKind::Child => format!("  {}", id),
            RowKind::Normal => {
                if a.subagent_file_count > 0 {
                    format!("  {}+{}", id, a.subagent_file_count)
                } else {
                    format!("  {}", id)
                }
            }
        }
    };
    let cache_total = t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input;

    // Color the cost cell for quick at-a-glance reading:
    let cost_style = if c.included {
        th::COST_INCLUDED
    } else if c.total >= 5.0 {
        th::COST_HIGH
    } else {
        Style::new()
    };

    // Client color: cheap "tag" for eye-tracking.
    let client_style = match s.client {
        agtop_core::session::ClientKind::Claude => th::CLIENT_CLAUDE,
        agtop_core::session::ClientKind::Codex => th::CLIENT_CODEX,
        agtop_core::session::ClientKind::OpenCode => th::CLIENT_OPENCODE,
        agtop_core::session::ClientKind::Copilot => th::CLIENT_COPILOT,
        agtop_core::session::ClientKind::GeminiCli => th::CLIENT_GEMINI_CLI,
        _ => Style::new(),
    };

    // Context column: pre-compute text and style.
    let (context_text, context_style) =
        format_context(a.context_used_tokens, a.context_window, a.context_used_pct);

    let cells: Vec<Cell<'a>> = visible
        .iter()
        .map(|&col_id| match col_id {
            ColumnId::Client => Cell::from(s.client.as_str()).style(client_style),
            ColumnId::Subscription => Cell::from(subscription.clone()),
            ColumnId::Session => Cell::from(short.clone()),
            ColumnId::Started => Cell::from(started.clone()),
            ColumnId::Age => Cell::from(age.clone()),
            ColumnId::Model => Cell::from(model.clone()),
            ColumnId::Cwd => Cell::from(cwd.clone()),
            ColumnId::Tokens => Cell::from(fmt::compact(t.input + t.output + cache_total)),
            ColumnId::OutputTokens => Cell::from(fmt::compact(t.output)),
            ColumnId::CacheTokens => Cell::from(fmt::compact(cache_total)),
            ColumnId::Cost => Cell::from(cost_str.clone()).style(cost_style),
            ColumnId::ToolCalls => Cell::from(
                a.tool_call_count
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
            ),
            ColumnId::Duration => Cell::from(
                a.duration_secs
                    .map(fmt::format_duration_compact)
                    .unwrap_or_else(|| "-".into()),
            ),
            ColumnId::LastActive => Cell::from(last_active_abs.clone()),
            ColumnId::State => Cell::from(state).style(state_style),
            ColumnId::Effort => Cell::from(effort.clone()),
            ColumnId::AgentTurns => Cell::from(
                a.agent_turns
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
            ),
            ColumnId::UserTurns => Cell::from(
                a.user_turns
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
            ),
            ColumnId::Context => Cell::from(context_text.clone()).style(context_style),
            ColumnId::Project => Cell::from(a.project_name.clone().unwrap_or_else(|| "-".into())),
            ColumnId::SessionName => {
                Cell::from(s.session_title.clone().unwrap_or_else(|| "-".into()))
            }
            ColumnId::Pid => Cell::from(match (a.pid, a.liveness) {
                (Some(pid), Some(agtop_core::Liveness::Live)) => pid.to_string(),
                (Some(pid), Some(agtop_core::Liveness::Stopped)) => {
                    format!("{pid}\u{2020}") // † dagger
                }
                _ => "-".into(),
            }),
            ColumnId::Cpu => Cell::from(crate::fmt::format_percent(
                a.process_metrics.as_ref().map(|m| m.cpu_percent),
            )),
            ColumnId::Memory => Cell::from(crate::fmt::compact_opt(
                a.process_metrics.as_ref().map(|m| m.memory_bytes),
            )),
            ColumnId::VirtualMemory => Cell::from(crate::fmt::compact_opt(
                a.process_metrics.as_ref().map(|m| m.virtual_memory_bytes),
            )),
            ColumnId::DiskRead => Cell::from(crate::fmt::compact_opt(
                a.process_metrics.as_ref().map(|m| m.disk_read_bytes),
            )),
            ColumnId::DiskWritten => Cell::from(crate::fmt::compact_opt(
                a.process_metrics.as_ref().map(|m| m.disk_written_bytes),
            )),
            ColumnId::DiskReadRate => Cell::from(crate::fmt::compact_rate_opt(
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_read_bytes_per_sec),
            )),
            ColumnId::DiskWriteRate => Cell::from(crate::fmt::compact_rate_opt(
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_written_bytes_per_sec),
            )),
            // SubscriptionLogo is injected by visible() — rendered as empty for now.
            ColumnId::SubscriptionLogo => Cell::from(""),
            // Action is a new v2 column; fallback to current_action in old table.
            ColumnId::Action => Cell::from(a.current_action.as_deref().unwrap_or("").to_string()),
        })
        .collect();

    let row = Row::new(cells);
    match kind {
        RowKind::Child => row.style(th::SUBAGENT_CHILD),
        _ => row,
    }
}

// ── Row kind ──────────────────────────────────────────────────────────────

/// Controls prefix/indent and which token/cost totals are displayed.
enum RowKind {
    /// Standalone session with no children.
    Normal,
    /// Collapsed parent: show `▶ id+N` and merged totals.
    CollapsedParent,
    /// Expanded parent: show `▼ id` and own (direct) totals.
    ExpandedParent,
    /// Child of an expanded parent: show `  id` (2-space indent), own totals, dimmed.
    Child,
}

// ── Merged-total helpers ───────────────────────────────────────────────────

/// Sum a parent's direct tokens + all children tokens for collapsed display.
fn merged_tokens(a: &agtop_core::session::SessionAnalysis) -> agtop_core::session::TokenTotals {
    let mut t = a.tokens.clone();
    for child in &a.children {
        t.input += child.tokens.input;
        t.cached_input += child.tokens.cached_input;
        t.output += child.tokens.output;
        t.reasoning_output += child.tokens.reasoning_output;
        t.cache_write_5m += child.tokens.cache_write_5m;
        t.cache_write_1h += child.tokens.cache_write_1h;
        t.cache_read += child.tokens.cache_read;
    }
    t
}

/// Sum a parent's direct cost + all children cost for collapsed display.
fn merged_cost(a: &agtop_core::session::SessionAnalysis) -> agtop_core::session::CostBreakdown {
    let mut c = a.cost.clone();
    for child in &a.children {
        c.input += child.cost.input;
        c.cached_input += child.cost.cached_input;
        c.output += child.cost.output;
        c.cache_write_5m += child.cost.cache_write_5m;
        c.cache_write_1h += child.cost.cache_write_1h;
        c.cache_read += child.cost.cache_read;
        c.total += child.cost.total;
    }
    c
}
