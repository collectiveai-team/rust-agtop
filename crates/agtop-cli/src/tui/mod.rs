//! Interactive htop-style dashboard, built on ratatui.
//!
//! Entry point: [`run`]. Sets up the alternate screen + raw mode,
//! starts a background refresh worker, and pumps the event loop until
//! the user quits. All state mutation happens through [`app::App`]
//! methods; all rendering through [`widgets`].

pub mod app;
pub mod column_config;
mod events;
mod refresh;
pub mod theme;
pub mod widgets;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags,
        MouseButton, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::*,
    widgets::{Block, Borders, Paragraph, Tabs},
    Terminal,
};

use theme as th;

use crate::fmt;
use agtop_core::pricing::Plan;
use agtop_core::Provider;

use app::{App, InputMode, Tab, UiMode};
use events::{apply_key, Action};
use refresh::{RefreshHandle, RefreshMsg};

/// Geometry of the last-rendered frame. Written by [`render`], read by
/// the event loop to map mouse coordinates to UI regions.
#[derive(Debug, Clone, Default)]
struct UiLayout {
    /// The full table widget area (including its border).
    table_area: Rect,
    /// The one-row tab bar area at the top of the bottom panel.
    tab_bar_area: Rect,
    /// Scroll offset of the table, so we can convert a clicked row-pixel
    /// to the correct view index even when the list is scrolled.
    table_scroll_offset: usize,
    /// Absolute terminal x-ranges for each sortable header column.
    /// Each entry: `(x_start, x_end_exclusive, SortColumn)`.
    /// Populated by `widgets::session_table::render`.
    header_cols: Vec<(u16, u16, app::SortColumn)>,
    /// Absolute terminal x-ranges for each tab-bar tab button.
    /// Each entry: `(x_start, x_end_exclusive, Tab)`.
    /// Populated by `render_bottom_panel` after measuring actual title widths.
    tab_cells: Vec<(u16, u16, Tab)>,
    /// The one-row tab bar area for the Cost Summary sub-tab in the dashboard.
    cost_tab_bar_area: Rect,
    /// Absolute terminal x-ranges for each Cost Summary sub-tab button.
    /// Each entry: `(x_start, x_end_exclusive, CostTab)`.
    /// Populated by `dashboard_cost::render`.
    cost_tab_cells: Vec<(u16, u16, app::CostTab)>,
    /// The single-row area that holds the period toggle ("total" / "month").
    /// Used for both row-range and x-range hit-testing.
    cost_period_row_area: Rect,
    /// Click ranges for the "total" / "month" period toggle labels.
    /// Each entry: `(x_start, x_end_exclusive, CostPeriod)`.
    cost_period_cells: Vec<(u16, u16, app::CostPeriod)>,
    /// Full area of the Cost Summary panel (for scroll-wheel hit-testing).
    cost_panel_area: Rect,
    /// Number of data rows in the current breakdown (for scroll clamping).
    cost_row_count: usize,
    /// Number of visible data rows in the current breakdown (for scroll clamping).
    cost_visible_rows: usize,
}

/// Run the interactive TUI. Blocks until the user quits or the terminal
/// raises an IO error. On exit, the terminal is returned to its
/// previous state regardless of success/failure.
pub fn run(
    providers: Vec<Arc<dyn Provider>>,
    enabled_initial: std::collections::HashSet<agtop_core::ProviderKind>,
    plan: Plan,
    refresh_interval: Duration,
    start_dashboard: bool,
) -> Result<()> {
    let mut terminal = setup_terminal().context("set up terminal for TUI")?;
    // Ensure the terminal is always restored, even on panic. We install
    // a panic hook that tears down the screen before the backtrace runs,
    // so stack traces don't land inside the alternate screen where the
    // user can't read them.
    install_panic_hook();

    let _enabled_arc = std::sync::Arc::new(std::sync::RwLock::new(enabled_initial));

    let mut handle = refresh::spawn(providers, plan, refresh_interval)
        .context("spawn background refresh worker")?;
    let mut app = App::new();
    if start_dashboard {
        app.set_ui_mode(UiMode::Dashboard);
    }

    let result = event_loop(&mut terminal, &mut app, &mut handle);

    restore_terminal(&mut terminal).ok();
    result
}

fn event_loop<B: ratatui::backend::Backend + std::io::Write>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    handle: &mut RefreshHandle,
) -> Result<()> {
    use ratatui::widgets::TableState;

    // Selection state for the main table. ratatui keeps scroll offset
    // on the widget-side state, but we drive the *highlighted index*
    // from `app` so tests stay pure.
    let mut table_state = TableState::default();
    // Tight poll interval so the UI feels responsive. The actual redraw
    // only happens on an event or a new snapshot; idle CPU is bounded
    // by this timeout.
    let poll_interval = Duration::from_millis(100);
    // Geometry written by the last render call; used for mouse hit-testing.
    let mut layout = UiLayout::default();

    loop {
        // 1. Drain any fresh snapshots from the background worker.
        while let Some(msg) = handle.try_recv() {
            match msg {
                RefreshMsg::Snapshot {
                    analyses,
                    plan_usage,
                    ..
                } => app.set_snapshot(analyses, plan_usage),
                RefreshMsg::Error { message, .. } => app.set_refresh_error(message),
            }
        }

        // 2. Render and capture geometry for mouse hit-testing.
        terminal.draw(|f| render(f, app, &mut table_state, &mut layout))?;
        // Keep scroll offset in sync after every draw so clicks are mapped
        // correctly even when the user has scrolled far down the list.
        layout.table_scroll_offset = table_state.offset();

        if app.should_quit() {
            break;
        }

        // 3. Wait for the next input event OR the poll timeout so the
        //    snapshot drain loop keeps running.
        if event::poll(poll_interval)? {
            match event::read()? {
                Event::Key(k) => match apply_key(app, k) {
                    Action::None => {}
                    Action::ManualRefresh => handle.trigger_manual(),
                },
                Event::Mouse(m) => apply_mouse(app, m, &layout),
                Event::Resize(_, _) => {
                    // Ratatui redraws the whole screen on every `draw()`
                    // call, so all we need to do is loop back. Keeping
                    // the explicit arm documents that resize events are
                    // expected, not a pitfall.
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Translate a crossterm mouse event into an App mutation. All geometry
/// is sourced from the `UiLayout` captured during the previous render.
fn apply_mouse(app: &mut App, event: MouseEvent, layout: &UiLayout) {
    match event.kind {
        // ── Scroll wheel ──────────────────────────────────────────────────
        MouseEventKind::ScrollDown => {
            if rect_contains(layout.cost_panel_area, event.column, event.row) {
                // cost panel takes priority; table area may overlap in classic mode
                app.scroll_cost_down(2, layout.cost_row_count, layout.cost_visible_rows);
            } else if rect_contains(layout.table_area, event.column, event.row) {
                app.move_selection(3);
            }
        }
        MouseEventKind::ScrollUp => {
            if rect_contains(layout.cost_panel_area, event.column, event.row) {
                app.scroll_cost_up(2);
            } else if rect_contains(layout.table_area, event.column, event.row) {
                app.move_selection(-3);
            }
        }

        // ── Left-click ────────────────────────────────────────────────────
        MouseEventKind::Down(MouseButton::Left) => {
            let (col, row) = (event.column, event.row);

            // Click on the Cost Summary period toggle row ("total" / "month").
            // Use the precise row Rect so clicks on the tab bar below don't
            // accidentally trigger the period check.
            if rect_contains(layout.cost_period_row_area, col, row) {
                for &(x_start, x_end, period) in &layout.cost_period_cells {
                    if col >= x_start && col < x_end {
                        app.set_cost_period(period);
                        return;
                    }
                }
                // Click landed on the period row but not on a label — consume
                // the event so it doesn't fall through to the session table.
                return;
            }

            // Click on the Cost Summary sub-tab bar (dashboard mode).
            if rect_contains(layout.cost_tab_bar_area, col, row) {
                for &(x_start, x_end, tab) in &layout.cost_tab_cells {
                    if col >= x_start && col < x_end {
                        app.set_cost_tab(tab);
                        return;
                    }
                }
                // Click on the tab bar area but not on a tab title — consume.
                return;
            }

            // Click on the tab bar → activate the tab under the cursor.
            // We use the pixel-accurate ranges recorded during render so
            // the hit-test is correct regardless of tab title length.
            if rect_contains(layout.tab_bar_area, col, row) {
                for &(x_start, x_end, tab) in &layout.tab_cells {
                    if col >= x_start && col < x_end {
                        app.set_tab(tab);
                        break;
                    }
                }
                return;
            }

            // Click inside the table widget area.
            if rect_contains(layout.table_area, col, row) {
                let rel_row = row.saturating_sub(layout.table_area.y) as usize;
                // rel_row 0 = top border, rel_row 1 = header row.
                // Data rows start at rel_row 2.
                if rel_row == 1 {
                    // Header click → sort by the column under the cursor,
                    // or toggle direction if it is already the active column.
                    for &(x_start, x_end, sc) in &layout.header_cols {
                        if col >= x_start && col < x_end {
                            app.set_sort_column(sc);
                            break;
                        }
                    }
                } else if rel_row >= 2 {
                    let data_row = rel_row - 2;
                    let view_idx = layout.table_scroll_offset + data_row;
                    app.select_at(view_idx);
                }
            }
        }

        _ => {}
    }
}

/// Return true when `(col, row)` falls inside `rect` (all inclusive of
/// the border).
#[inline]
fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// Compose the full UI: header + session table + bottom tabs + footer.
/// Also writes the geometry of key areas into `layout` so the event
/// loop can do mouse hit-testing without re-computing the split.
fn render(
    frame: &mut Frame<'_>,
    app: &App,
    table_state: &mut ratatui::widgets::TableState,
    layout: &mut UiLayout,
) {
    if app.ui_mode() == UiMode::Dashboard {
        render_dashboard(frame, app, table_state, layout);
        return;
    }

    // Layout: 3 cells total — status header (1), split area (flex),
    // footer (1). The split area is 60/40 table-vs-bottompanel by
    // default; tests hit this layout via the TestBackend snapshots.
    // Bottom panel needs at least 12 rows to fit the Cost tab (tab bar
    // + 2 border rows + header row + 6 bucket rows + total row + slack).
    // The table gets everything else. On narrow terminals the
    // `Percentage(60)` cap keeps the bottom panel from shrinking below
    // usefulness.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // status bar
            Constraint::Min(8),     // table (flexes)
            Constraint::Length(14), // bottom panel (Info/Cost)
            Constraint::Length(1),  // footer / filter input
        ])
        .split(frame.area());

    // Capture geometry for mouse hit-testing.
    layout.table_area = outer[1];

    render_status(frame, outer[0], app);
    widgets::session_table::render(frame, outer[1], app, table_state, &mut layout.header_cols);
    render_bottom_panel(frame, outer[2], app, layout);
    render_footer(frame, outer[3], app);
}

fn render_dashboard(
    frame: &mut Frame<'_>,
    app: &App,
    table_state: &mut ratatui::widgets::TableState,
    layout: &mut UiLayout,
) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // status
            Constraint::Length(9),  // usage chart
            Constraint::Length(12), // plan + cost panes
            Constraint::Min(8),     // sessions table
            Constraint::Length(1),  // footer
        ])
        .split(frame.area());

    render_status(frame, outer[0], app);
    widgets::dashboard_usage::render(frame, outer[1], app);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(outer[2]);
    widgets::dashboard_plan::render(frame, mid[0], app);
    widgets::dashboard_cost::render(
        frame,
        mid[1],
        app,
        widgets::dashboard_cost::CostRenderOut {
            tab_bar_area: &mut layout.cost_tab_bar_area,
            tab_cells: &mut layout.cost_tab_cells,
            period_row_area: &mut layout.cost_period_row_area,
            period_cells: &mut layout.cost_period_cells,
            cost_panel_area: &mut layout.cost_panel_area,
            cost_row_count: &mut layout.cost_row_count,
            cost_visible_rows: &mut layout.cost_visible_rows,
        },
    );

    layout.table_area = outer[3];
    layout.tab_bar_area = Rect::default();
    widgets::session_table::render(frame, outer[3], app, table_state, &mut layout.header_cols);
    render_footer(frame, outer[4], app);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sel = app
        .selected()
        .map(|(i, a)| {
            format!(
                "[{}/{}] {}:{}",
                i + 1,
                app.view_len(),
                a.summary.provider.as_str(),
                fmt::short_id(&a.summary.session_id)
            )
        })
        .unwrap_or_else(|| "[--]".into());

    let mode = match app.ui_mode() {
        UiMode::Classic => "classic",
        UiMode::Dashboard => "dashboard",
    };
    let status = format!(
        " agtop [{mode}]  refresh#{}  {}  {}",
        app.refresh_count(),
        sel,
        app.last_error().unwrap_or(""),
    );
    let p = Paragraph::new(status).style(th::STATUS_BAR);
    frame.render_widget(p, area);
}

fn render_bottom_panel(frame: &mut Frame<'_>, area: Rect, app: &App, layout: &mut UiLayout) {
    // Two-row layout inside the panel: tab bar + the active tab body.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Capture the tab bar rect for mouse hit-testing.
    layout.tab_bar_area = rows[0];

    let titles: Vec<Line<'_>> = Tab::all()
        .iter()
        .map(|t| {
            if *t == app.tab() {
                Line::from(Span::styled(t.title(), th::TAB_ACTIVE))
            } else {
                Line::from(t.title())
            }
        })
        .collect();

    let tab_bar = Tabs::new(titles)
        .select(match app.tab() {
            Tab::Info => 0,
            Tab::Cost => 1,
            Tab::Config => 2,
        })
        .block(Block::default().borders(Borders::NONE))
        .divider("│");

    // Compute pixel-accurate hit-test ranges for each tab button.
    //
    // ratatui's `Tabs` widget renders each title with a 1-column left
    // pad and 1-column right pad (verified against ratatui 0.29
    // src/widgets/tabs.rs). A 1-column divider is placed *between*
    // tabs; no trailing divider is emitted after the last tab. For
    // titles t0..tN-1 with char-widths w0..wN-1 the full bar is
    //   " t0 │ t1 │ ... │ tN-1 "
    // and cell i (the clickable region for tab i) covers
    //   [x_i, x_i + 2 + w_i)
    // NOTE: `chars().count()` is only correct because current tab
    // titles are pure ASCII. If non-ASCII titles are introduced,
    // switch to `unicode-width::UnicodeWidthStr::width` and add the
    // crate as a dependency.
    layout.tab_cells.clear();
    let mut x = rows[0].x;
    let tabs = Tab::all();
    for (i, &tab) in tabs.iter().enumerate() {
        let w = tab.title().chars().count() as u16;
        let cell_width = w + 2; // 1 pad left + title + 1 pad right
        layout.tab_cells.push((x, x + cell_width, tab));
        x += cell_width;
        if i + 1 < tabs.len() {
            x += 1; // divider column between tabs only
        }
    }

    frame.render_widget(tab_bar, rows[0]);

    match app.tab() {
        Tab::Info => widgets::info_tab::render(frame, rows[1], app),
        Tab::Cost => widgets::cost_tab::render(frame, rows[1], app),
        Tab::Config => widgets::config_tab::render(frame, rows[1], app),
    }
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let (text, style) = match app.mode() {
        InputMode::Filter => (
            format!("/{}_  (Enter=confirm, Esc=clear)", app.filter()),
            th::FOOTER_FILTER,
        ),
        InputMode::Normal => (
            concat!(
                " q:quit  d:dashboard  j/k:↕  click:select  scroll:↕  Tab:tab  /:filter  >:sort  i:dir  r:refresh  ",
                "g/G:top/bot  PgUp/PgDn:10"
            )
            .to_string(),
            th::FOOTER_NORMAL,
        ),
    };
    let p = Paragraph::new(text).style(style);
    frame.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Terminal setup / teardown
// ---------------------------------------------------------------------------

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    // Keyboard enhancement flags let us distinguish Shift-Tab etc. on
    // terminals that support kitty's CSI-u protocol. We install them
    // best-effort and ignore failure on legacy terminals — the base
    // crossterm event decoder already covers the standard bindings.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        )
    );
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal<B: ratatui::backend::Backend + io::Write>(
    terminal: &mut Terminal<B>,
) -> io::Result<()> {
    disable_raw_mode()?;
    let backend = terminal.backend_mut();
    let _ = execute!(backend, PopKeyboardEnhancementFlags);
    execute!(backend, LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()
}

fn install_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort terminal restore so the backtrace is readable.
        // Pop keyboard-enhancement flags first: kitty-protocol terminals
        // keep them active until explicitly popped, corrupting subsequent
        // programs if we leave them pushed after a panic.
        let _ = disable_raw_mode();
        let mut stderr = io::stderr();
        let _ = execute!(stderr, PopKeyboardEnhancementFlags);
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, DisableMouseCapture);
        hook(info);
    }));
}

// ---------------------------------------------------------------------------
// Snapshot tests using ratatui's TestBackend.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{
        CostBreakdown, ProviderKind, SessionAnalysis, SessionSummary, TokenTotals,
    };
    use chrono::{TimeZone, Utc};
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    /// Tiny fixture: two sessions, deterministic timestamps, Claude +
    /// Codex. We pin `last_active` in the past so the AGE column is
    /// stable ("2d" etc.) regardless of when the test runs — within a
    /// reasonable range.
    fn fixture_app() -> App {
        let ts_started = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let ts_last = Utc.with_ymd_and_hms(2026, 1, 1, 10, 30, 0).unwrap();

        let s1_summary = SessionSummary::new(
            ProviderKind::Claude,
            Some("Max 5x".into()),
            "deadbeef-aaaa-bbbb-cccc-1234".into(),
            Some(ts_started),
            Some(ts_last),
            Some("claude-opus-4-6".into()),
            Some("/tmp/proj".into()),
            PathBuf::from("/tmp/deadbeef.jsonl"),
            Some("waiting".into()),
            Some("tool approval pending".into()),
            Some("high".into()),
            Some("reasoning.effort=high".into()),
        );
        let mut s1_tokens = TokenTotals::default();
        s1_tokens.input = 1_000;
        s1_tokens.output = 500;
        s1_tokens.cache_read = 20_000;
        let mut s1_cost = CostBreakdown::default();
        s1_cost.input = 0.003;
        s1_cost.output = 0.0075;
        s1_cost.cache_read = 0.010;
        s1_cost.total = 0.0205;
        let s1 = SessionAnalysis::new(
            s1_summary,
            s1_tokens,
            s1_cost,
            Some("claude-opus-4-6".into()),
            2,
            None,
            Some((ts_last - ts_started).num_seconds() as u64),
            None,
            None,
            None,
        );

        let s2_summary = SessionSummary::new(
            ProviderKind::Codex,
            Some("ChatGPT Plus".into()),
            "ses_gpt5".into(),
            Some(ts_started),
            Some(ts_last),
            Some("gpt-5".into()),
            Some("/tmp/other".into()),
            PathBuf::from("/tmp/codex.jsonl"),
            None,
            None,
            None,
            None,
        );
        let mut s2_tokens = TokenTotals::default();
        s2_tokens.input = 2_000;
        s2_tokens.output = 1_000;
        let mut s2_cost = CostBreakdown::default();
        s2_cost.input = 0.0025;
        s2_cost.output = 0.01;
        s2_cost.total = 0.0125;
        let s2 = SessionAnalysis::new(
            s2_summary,
            s2_tokens,
            s2_cost,
            Some("gpt-5".into()),
            0,
            Some(12),
            Some((ts_last - ts_started).num_seconds() as u64),
            Some(38.2),
            Some(98_380),
            Some(258_400),
        );

        let mut app = App::new();
        app.set_sessions(vec![s1, s2]);
        app
    }

    /// Draw the main layout to a 140×20 TestBackend and assert basic
    /// structural invariants (no panic, selected-row marker present,
    /// headers rendered). We don't do full buffer diffing because the
    /// AGE column is clock-dependent; checking for substrings keeps the
    /// test robust against time-zone drift.
    #[test]
    fn renders_main_layout_without_panicking() {
        let backend = TestBackend::new(140, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = fixture_app();
        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");

        let buffer = terminal.backend().buffer().clone();
        let contents = buffer_to_string(&buffer);
        assert!(contents.contains("Sessions"), "header title missing");
        assert!(contents.contains("claude-opus"), "claude model missing");
        assert!(contents.contains("gpt-5"), "gpt-5 model missing");
        assert!(contents.contains("▶"), "selected-row marker missing");
        assert!(contents.contains("Info"), "Info tab title missing");
        assert!(contents.contains("STATE"), "state header missing");
        assert!(contents.contains("EFFORT"), "effort header missing");
        assert!(contents.contains("waiting"), "state value missing");
        assert!(contents.contains("high"), "effort value missing");
    }

    #[test]
    fn renders_cost_tab() {
        // Use a taller backend so the Cost tab has room to render all 6
        // bucket rows plus the total. The default 10-line bottom panel
        // only fits 7 data rows after borders/headers, which on shorter
        // test backends drops the total row off the bottom.
        let backend = TestBackend::new(140, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = fixture_app();
        app.set_tab(Tab::Cost);
        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");
        let contents = buffer_to_string(&terminal.backend().buffer().clone());
        assert!(
            contents.contains("bucket"),
            "bucket header missing:\n{contents}"
        );
        assert!(contents.contains("tokens"), "tokens header missing");
        assert!(
            contents.contains("total"),
            "total row missing — buffer was:\n{contents}"
        );
    }

    #[test]
    fn renders_filter_mode_footer() {
        let backend = TestBackend::new(140, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = fixture_app();
        app.enter_filter_mode();
        app.push_filter_char('o');
        app.push_filter_char('p');
        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");
        let contents = buffer_to_string(&terminal.backend().buffer().clone());
        assert!(contents.contains("/op"), "filter prompt missing");
    }

    #[test]
    fn renders_empty_state() {
        let backend = TestBackend::new(120, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.set_sessions(vec![]);
        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");
        let contents = buffer_to_string(&terminal.backend().buffer().clone());
        assert!(contents.contains("no session selected"));
    }

    #[test]
    fn renders_dashboard_with_plan_usage() {
        use agtop_core::session::{PlanUsage, PlanWindow};

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.toggle_ui_mode(); // switch to Dashboard

        let reset_at = Utc.with_ymd_and_hms(2026, 4, 18, 13, 0, 0).unwrap();
        let windows = vec![
            PlanWindow::new("5h".into(), Some(0.71), Some(reset_at), None, true),
            PlanWindow::new("7d".into(), Some(0.18), Some(reset_at), None, false),
        ];
        let plan_usage = vec![PlanUsage::new(
            ProviderKind::Claude,
            "Max 5x via Claude Code".into(),
            Some("Max 5x".into()),
            windows,
            None,
            None,
        )];
        app.set_snapshot(vec![], plan_usage);

        let mut state = ratatui::widgets::TableState::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut UiLayout::default()))
            .expect("draw");

        let contents = buffer_to_string(&terminal.backend().buffer().clone());
        assert!(
            contents.contains("Subscription Details"),
            "panel title missing:\n{contents}"
        );
        assert!(
            contents.contains("Max 5x"),
            "subscription name missing:\n{contents}"
        );
    }

    /// Click hit-testing for the bottom-panel tabs must match the
    /// *rendered* x-range of each tab title, not just the title text
    /// width. ratatui's `Tabs` widget pads every title with a 1-column
    /// space on each side and places a 1-column divider between tabs
    /// (no trailing divider). This test scans the rendered buffer for
    /// each tab's title in *column* space (careful: dividers like '│'
    /// are 1 column but multiple UTF-8 bytes, so naive `String::find`
    /// would mix bytes with columns) and asserts that every
    /// `layout.tab_cells` entry covers the entire visible title range.
    #[test]
    fn tab_cells_cover_rendered_titles() {
        let backend = TestBackend::new(140, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = fixture_app();
        let mut state = ratatui::widgets::TableState::default();
        let mut layout = UiLayout::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut layout))
            .expect("draw");

        let buffer = terminal.backend().buffer().clone();
        let tab_row_y = layout.tab_bar_area.y;
        let row_width = buffer.area.width;

        // Build a diagnostic rendering of the tab row for panic messages.
        // This may contain multi-byte chars; it's for humans, not indexing.
        let mut row_display = String::new();
        for x in 0..row_width {
            let cell = buffer.cell((x, tab_row_y)).expect("cell in bounds");
            row_display.push_str(cell.symbol());
        }

        // Find `title` in *column* space: walk each starting column, and
        // for each byte of the (ASCII) title compare it to the single
        // byte emitted by the corresponding cell's symbol. Tab titles
        // are pure ASCII, so each title char occupies exactly one
        // column whose symbol is that char's single byte.
        for &tab in Tab::all() {
            let title = tab.title();
            let title_bytes = title.as_bytes();
            let title_cols = title_bytes.len() as u16;
            assert!(
                title.is_ascii(),
                "tab title '{title}' must be ASCII for this test"
            );
            let mut found_col: Option<u16> = None;
            if row_width >= title_cols {
                'outer: for x in 0..=(row_width - title_cols) {
                    for (i, &b) in title_bytes.iter().enumerate() {
                        let cell = buffer
                            .cell((x + i as u16, tab_row_y))
                            .expect("cell in bounds");
                        let sym = cell.symbol().as_bytes();
                        if sym != [b] {
                            continue 'outer;
                        }
                    }
                    found_col = Some(x);
                    break;
                }
            }
            let start = found_col
                .unwrap_or_else(|| panic!("tab title '{title}' not found in row: {row_display:?}"));
            let end = start + title_cols;

            let (cell_start, cell_end, recorded_tab) = layout
                .tab_cells
                .iter()
                .copied()
                .find(|&(_, _, t)| t == tab)
                .unwrap_or_else(|| panic!("no tab_cells entry for {tab:?}"));
            assert_eq!(recorded_tab, tab);
            assert!(
                cell_start <= start && cell_end >= end,
                "tab_cells range for {tab:?} [{cell_start},{cell_end}) does not \
                 cover rendered title '{title}' at columns [{start},{end}) in row: {row_display:?}"
            );
        }
    }

    /// A click anywhere inside the Config tab's rendered title must
    /// route through `apply_mouse` and activate the Config tab. We
    /// click the LAST letter rather than the first: a subtly-wrong
    /// tab_cells range can still cover the opening column by
    /// coincidence (e.g. the buggy range [10,16) happens to include
    /// column 15, the 'C' of Config on a 140-col terminal) but
    /// virtually never extends to the final column of a
    /// wider-than-expected title. Regression for the tab_cells math
    /// off-by-one that prevented Config clicks from registering.
    #[test]
    fn clicking_config_tab_activates_it() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let backend = TestBackend::new(140, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = fixture_app();
        // Start on a tab that is NOT Config so the assertion is meaningful.
        app.set_tab(Tab::Info);
        let mut state = ratatui::widgets::TableState::default();
        let mut layout = UiLayout::default();
        terminal
            .draw(|f| render(f, &app, &mut state, &mut layout))
            .expect("draw");

        // Find the rendered x of "Config" in the tab bar row.
        let buffer = terminal.backend().buffer().clone();
        let tab_row_y = layout.tab_bar_area.y;
        // Find the column of the 'C' in "Config" using the same ASCII
        // column-scan approach as `tab_cells_cover_rendered_titles` (see
        // comment there about byte vs column pitfalls).
        let config_title = "Config".as_bytes();
        let row_width = buffer.area.width;
        let mut config_x: Option<u16> = None;
        if row_width >= config_title.len() as u16 {
            'outer: for x in 0..=(row_width - config_title.len() as u16) {
                for (i, &b) in config_title.iter().enumerate() {
                    let cell = buffer
                        .cell((x + i as u16, tab_row_y))
                        .expect("cell in bounds");
                    if cell.symbol().as_bytes() != [b] {
                        continue 'outer;
                    }
                }
                config_x = Some(x);
                break;
            }
        }
        let config_x = config_x.expect("Config title in row");

        // Synthesise a click on the LAST letter of "Config". This is
        // more rigorous than clicking the first letter, because a
        // subtly-wrong tab_cells range may still cover the first
        // column by coincidence but is far less likely to extend all
        // the way to the final column of the title. The last letter
        // is at column (config_x + title.len() - 1).
        let last_col = config_x + ("Config".len() as u16) - 1;
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: last_col,
            row: tab_row_y,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        apply_mouse(&mut app, event, &layout);
        assert_eq!(app.tab(), Tab::Config);
    }

    /// Clicking each sortable column header must activate that column's sort.
    /// We scan the rendered buffer to find the actual rendered x of each label
    /// and simulate a click at that position, verifying the correct SortColumn fires.
    #[test]
    fn clicking_sortable_header_sorts_by_correct_column() {
        use crate::tui::app::SortColumn;
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        // Test at multiple widths to catch off-by-one in right-anchored columns.
        for &width in &[140u16, 160, 180, 200] {
            let backend = TestBackend::new(width, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            let app = fixture_app();
            let mut state = ratatui::widgets::TableState::default();
            let mut layout = UiLayout::default();
            terminal
                .draw(|f| render(f, &app, &mut state, &mut layout))
                .expect("draw");

            let buffer = terminal.backend().buffer().clone();
            let header_y = layout.table_area.y + 1;
            let row_width = buffer.area.width;

            // Map of rendered label text → expected SortColumn
            let cases: &[(&[u8], SortColumn)] = &[
                (b"PROVIDER", SortColumn::Provider),
                (b"STARTED", SortColumn::Started),
                (b"AGE", SortColumn::LastActive),
                (b"MODEL", SortColumn::Model),
                (b"TOK", SortColumn::Tokens),
                (b"OUT", SortColumn::OutputTokens),
                (b"CACHE", SortColumn::CacheTokens),
                (b"COST$", SortColumn::Cost),
            ];

            for &(label, expected_col) in cases {
                // Find rendered x of this label
                let mut found_x: Option<u16> = None;
                'search: for x in 0..=(row_width.saturating_sub(label.len() as u16)) {
                    for (i, &b) in label.iter().enumerate() {
                        if buffer
                            .cell((x + i as u16, header_y))
                            .unwrap()
                            .symbol()
                            .as_bytes()
                            != [b]
                        {
                            continue 'search;
                        }
                    }
                    found_x = Some(x);
                    break;
                }
                let label_x = match found_x {
                    Some(x) => x,
                    None => continue, // column hidden at this width
                };

                let mut app2 = fixture_app();
                let event = MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: label_x,
                    row: header_y,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                };
                apply_mouse(&mut app2, event, &layout);
                assert_eq!(
                    app2.sort_col(),
                    expected_col,
                    "width={width}: clicking '{}' header at x={label_x} should sort by {expected_col:?}, got {:?}",
                    std::str::from_utf8(label).unwrap(),
                    app2.sort_col()
                );
            }
        }
    }

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        let area = buf.area;
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = buf.cell((x, y)).expect("cell in bounds");
                out.push_str(cell.symbol());
            }
            out.push('\n');
        }
        out
    }
}
