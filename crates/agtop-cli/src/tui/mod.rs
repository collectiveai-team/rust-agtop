//! Interactive htop-style dashboard, built on ratatui.
//!
//! Entry point: [`run`]. Sets up the alternate screen + raw mode,
//! starts a background refresh worker, and pumps the event loop until
//! the user quits. All state mutation happens through [`app::App`]
//! methods; all rendering through [`widgets`].

pub mod app;
mod events;
mod refresh;
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
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Tabs},
    Terminal,
};

use agtop_core::pricing::Plan;
use agtop_core::Provider;

use app::{App, InputMode, Tab};
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
}

/// Run the interactive TUI. Blocks until the user quits or the terminal
/// raises an IO error. On exit, the terminal is returned to its
/// previous state regardless of success/failure.
pub fn run(
    providers: Vec<Arc<dyn Provider>>,
    plan: Plan,
    refresh_interval: Duration,
) -> Result<()> {
    let mut terminal = setup_terminal().context("set up terminal for TUI")?;
    // Ensure the terminal is always restored, even on panic. We install
    // a panic hook that tears down the screen before the backtrace runs,
    // so stack traces don't land inside the alternate screen where the
    // user can't read them.
    install_panic_hook();

    let mut handle = refresh::spawn(providers, plan, refresh_interval)
        .context("spawn background refresh worker")?;
    let mut app = App::new();

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
                RefreshMsg::Snapshot { analyses, .. } => app.set_sessions(analyses),
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
        // ── Scroll wheel anywhere ─────────────────────────────────────────
        MouseEventKind::ScrollDown => app.move_selection(3),
        MouseEventKind::ScrollUp => app.move_selection(-3),

        // ── Left-click ────────────────────────────────────────────────────
        MouseEventKind::Down(MouseButton::Left) => {
            let (col, row) = (event.column, event.row);

            // Click on the tab bar → cycle to the clicked tab.
            if rect_contains(layout.tab_bar_area, col, row) {
                let x_offset = col.saturating_sub(layout.tab_bar_area.x) as usize;
                // Tab titles are rendered with a "│" divider between them.
                // "Info" occupies ~4 chars, "│" = 1, "Cost" occupies ~4.
                // Any click in the first ~4 columns = Info; anything after = Cost.
                // We use a simple threshold: if x < 5 → Info, else Cost.
                if x_offset < 5 {
                    app.set_tab(Tab::Info);
                } else {
                    app.set_tab(Tab::Cost);
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
    col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
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

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sel = app
        .selected()
        .map(|(i, a)| {
            format!(
                "[{}/{}] {}:{}",
                i + 1,
                app.view_len(),
                a.summary.provider.as_str(),
                short_id(&a.summary.session_id)
            )
        })
        .unwrap_or_else(|| "[--]".into());

    let status = format!(
        " agtop  refresh#{}  {}  {}",
        app.refresh_count(),
        sel,
        app.last_error().unwrap_or(""),
    );
    let p = Paragraph::new(status).style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
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
                Line::from(Span::styled(
                    t.title(),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(t.title())
            }
        })
        .collect();

    let tab_bar = Tabs::new(titles)
        .select(match app.tab() {
            Tab::Info => 0,
            Tab::Cost => 1,
        })
        .block(Block::default().borders(Borders::NONE))
        .divider("│");

    frame.render_widget(tab_bar, rows[0]);

    match app.tab() {
        Tab::Info => widgets::info_tab::render(frame, rows[1], app),
        Tab::Cost => widgets::cost_tab::render(frame, rows[1], app),
    }
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let (text, style) = match app.mode() {
        InputMode::Filter => (
            format!("/{}_  (Enter=confirm, Esc=clear)", app.filter()),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        InputMode::Normal => (
            concat!(
                " q:quit  j/k:↕  click:select  scroll:↕  Tab:tab  /:filter  >:sort  i:dir  r:refresh  ",
                "g/G:top/bot  PgUp/PgDn:10"
            )
            .to_string(),
            Style::default().fg(Color::Gray),
        ),
    };
    let p = Paragraph::new(text).style(style);
    frame.render_widget(p, area);
}

fn short_id(id: &str) -> String {
    if id.starts_with("ses_") {
        id[..id.len().min(10)].to_string()
    } else {
        id.chars().take(8).collect()
    }
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
        let _ = disable_raw_mode();
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

        let s1 = SessionAnalysis {
            summary: SessionSummary {
                provider: ProviderKind::Claude,
                session_id: "deadbeef-aaaa-bbbb-cccc-1234".into(),
                started_at: Some(ts_started),
                last_active: Some(ts_last),
                model: Some("claude-opus-4-6".into()),
                cwd: Some("/tmp/proj".into()),
                data_path: PathBuf::from("/tmp/deadbeef.jsonl"),
            },
            tokens: TokenTotals {
                input: 1_000,
                output: 500,
                cache_read: 20_000,
                ..Default::default()
            },
            cost: CostBreakdown {
                input: 0.003,
                output: 0.0075,
                cache_read: 0.010,
                total: 0.0205,
                included: false,
                ..Default::default()
            },
            effective_model: Some("claude-opus-4-6".into()),
            subagent_file_count: 2,
        };

        let s2 = SessionAnalysis {
            summary: SessionSummary {
                provider: ProviderKind::Codex,
                session_id: "ses_gpt5".into(),
                started_at: Some(ts_started),
                last_active: Some(ts_last),
                model: Some("gpt-5".into()),
                cwd: Some("/tmp/other".into()),
                data_path: PathBuf::from("/tmp/codex.jsonl"),
            },
            tokens: TokenTotals {
                input: 2_000,
                output: 1_000,
                ..Default::default()
            },
            cost: CostBreakdown {
                input: 0.0025,
                output: 0.01,
                total: 0.0125,
                included: false,
                ..Default::default()
            },
            effective_model: Some("gpt-5".into()),
            subagent_file_count: 0,
        };

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
        assert!(contents.contains("bucket"), "bucket header missing:\n{contents}");
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
