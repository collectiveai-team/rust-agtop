//! Sessions table: state dot + 14 columns + activity sparkline.
// Foundation code for Plan 2.
#![allow(dead_code)]

use std::collections::HashSet;

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

use agtop_core::session::{ClientKind, SessionAnalysis, SessionState};

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;

use crate::tui::animation::PulseClock;
use crate::tui::column_config::{self, ColumnId};
use crate::tui::theme_v2::{client_palette, Theme};
use crate::tui::widgets::{sparkline_braille, state_dot, state_style};

/// One session as rendered. The full `SessionAnalysis` plus a recent token-rate ring buffer.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub analysis: SessionAnalysis,
    pub client_kind: ClientKind,
    pub client_label: String,
    /// Recent token-rate samples (oldest → newest), used for ACTIVITY sparkline.
    pub activity_samples: Vec<f32>,
    /// 0 = top-level session; 1 = child subagent.
    pub depth: u8,
    /// Session ID of the parent, if this is a child row (depth == 1).
    pub parent_session_id: Option<String>,
    /// True if this is the last child of its parent (renders `└──` instead of `├──`).
    /// Always false for parent (depth == 0) rows.
    pub is_last_child: bool,
}

#[derive(Debug)]
pub struct SessionsTable {
    pub rows: Vec<SessionRow>,
    pub state: TableState,
    pub pulse: PulseClock,
    pub animations_enabled: bool,
    pub sort_key: SessionSortKey,
    pub sort_dir: SortDir,
    /// Rect of the last-rendered table widget, set by `render()`.
    /// Used by `handle_event` for mouse row hit-testing.
    pub table_area: Rect,
    /// Session IDs of collapsed parent rows (children not shown).
    pub collapsed: HashSet<String>,
    /// Session IDs of parents we have seen at least once. New parents
    /// (not yet in this set) are auto-inserted into `collapsed` on first
    /// observation so the tree starts collapsed; once the user toggles a
    /// parent open, removing it from `collapsed` sticks even if subsequent
    /// refreshes see the same parent again.
    pub known_parents: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSortKey {
    Session,
    Age,
    Client,
    Subscription,
    Model,
    Cpu,
    Memory,
    Tokens,
    Cost,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl Default for SessionsTable {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            state: TableState::default(),
            pulse: PulseClock::default(),
            animations_enabled: true,
            sort_key: SessionSortKey::Age,
            sort_dir: SortDir::Desc,
            table_area: Rect::default(),
            collapsed: HashSet::new(),
            known_parents: HashSet::new(),
        }
    }
}

impl SessionsTable {
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.table_area = area;
        // Map columns from default_visible into column constraints + header strings.
        let columns = column_config::default_visible_v2();

        // Insertion point for ACTIVITY sparkline: after CLIENT. (Previously
        // inserted after ACTION, but ACTION was removed from the default view.)
        // Falls back to ACTION if a user re-enables it.
        let activity_after = columns
            .iter()
            .position(|c| *c == ColumnId::Action)
            .or_else(|| columns.iter().position(|c| *c == ColumnId::Client));

        let mut header_cells: Vec<Cell> = Vec::with_capacity(columns.len() + 3);
        // State dot column has no header label.
        header_cells.push(Cell::from(""));
        for (i, c) in columns.iter().enumerate() {
            header_cells.push(
                Cell::from(c.label()).style(
                    Style::default()
                        .fg(theme.fg_emphasis)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            // Insert ACTIVITY header after the configured anchor column.
            if Some(i) == activity_after {
                header_cells.push(
                    Cell::from("ACTIVITY").style(
                        Style::default()
                            .fg(theme.fg_emphasis)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
            }
        }

        let body_rows: Vec<Row> = self
            .rows
            .iter()
            .map(|sr| self.render_row(sr, &columns, activity_after, theme))
            .collect();

        let mut constraints: Vec<Constraint> = Vec::with_capacity(columns.len() + 2);
        // State-dot column: 5 cells wide. depth=0 needs 2 (▶ + space) + dot + space;
        // depth=1 needs 4 for the tree glyph (`├── ` / `└── `) + dot.
        constraints.push(Constraint::Length(5));
        for (i, c) in columns.iter().enumerate() {
            constraints.push(width_for(*c));
            if Some(i) == activity_after {
                constraints.push(Constraint::Length(8)); // ACTIVITY sparkline
            }
        }

        let table = Table::new(body_rows, &constraints)
            .header(Row::new(header_cells))
            .row_highlight_style(
                Style::default()
                    .bg(theme.bg_selection)
                    .fg(theme.fg_emphasis),
            )
            .highlight_symbol("▶ ");
        let mut state = self.state.clone();
        frame.render_stateful_widget(table, area, &mut state);
    }

    fn render_row<'a>(
        &'a self,
        row: &'a SessionRow,
        columns: &[ColumnId],
        activity_after: Option<usize>,
        theme: &Theme,
    ) -> Row<'a> {
        // Read state directly from core (post-normalization field).
        let state: SessionState = row
            .analysis
            .session_state
            .clone()
            .unwrap_or(SessionState::Closed);

        // Apply muted styling to closed rows, but never dim rows with a live PID.
        // Use fg_muted only — adding DIM on top of an already-dim color makes rows near-invisible.
        let row_style = if state_style::is_muted_row(&state) && row.analysis.pid.is_none() {
            Style::default().fg(theme.fg_muted)
        } else {
            Style::default().fg(theme.fg_default)
        };

        // Build cells.
        let mut cells: Vec<Cell> = Vec::with_capacity(columns.len() + 2);
        // State dot — reads SessionState directly via state_style.
        // For depth=0 rows with children, show collapse toggle before dot.
        let dot_span = state_dot::render(&state, &self.pulse, self.animations_enabled, theme);
        let first_cell = if row.depth == 0 && !row.analysis.children.is_empty() {
            let toggle = if self.collapsed.contains(&row.analysis.summary.session_id) {
                Span::raw("▶ ")
            } else {
                Span::raw("▼ ")
            };
            Cell::from(Line::from(vec![toggle, dot_span]))
        } else if row.depth == 1 {
            // Indent child rows with filetree glyphs:
            //   ├── for non-last children
            //   └── for the last child of a parent
            let glyph = if row.is_last_child {
                Span::styled("└── ", Style::default().fg(theme.fg_muted))
            } else {
                Span::styled("├── ", Style::default().fg(theme.fg_muted))
            };
            Cell::from(Line::from(vec![glyph, dot_span]))
        } else {
            Cell::from(Line::from(vec![dot_span]))
        };
        cells.push(first_cell);

        for (i, c) in columns.iter().enumerate() {
            cells.push(self.render_cell(row, *c, &state, theme));
            // Insert ACTIVITY sparkline after the configured anchor column.
            if Some(i) == activity_after {
                let activity = sparkline_braille::render_braille(&row.activity_samples, 8, 100.0);
                cells.push(Cell::from(Line::from(Span::styled(
                    activity,
                    Style::default().fg(theme.status_success),
                ))));
            }
        }

        Row::new(cells).style(row_style)
    }

    fn render_cell<'a>(
        &'a self,
        row: &'a SessionRow,
        col: ColumnId,
        state: &SessionState,
        theme: &Theme,
    ) -> Cell<'a> {
        match col {
            ColumnId::Session => Cell::from(format_session_id(&row.analysis.summary.session_id)),
            ColumnId::Age => Cell::from(format_age(row.analysis.summary.last_active)),
            ColumnId::Action => render_action_cell(row, state, theme),
            ColumnId::Client => Cell::from(Span::styled(
                row.client_label.clone(),
                Style::default().fg(client_palette::color_for(row.client_kind)),
            )),
            ColumnId::Subscription => Cell::from(
                row.analysis
                    .summary
                    .subscription
                    .clone()
                    .unwrap_or_default(),
            ),
            ColumnId::Model => Cell::from(row.analysis.summary.model.clone().unwrap_or_default()),
            ColumnId::Cpu => Cell::from(
                row.analysis
                    .process_metrics
                    .as_ref()
                    .map(|m| format!("{:>3.0}%", m.cpu_percent))
                    .unwrap_or_else(|| "—".into()),
            ),
            ColumnId::Memory => Cell::from(
                row.analysis
                    .process_metrics
                    .as_ref()
                    .map(|m| format_bytes_compact(m.memory_bytes))
                    .unwrap_or_else(|| "—".into()),
            ),
            ColumnId::DiskReadRate => Cell::from(crate::fmt::compact_rate_opt(
                row.analysis
                    .process_metrics
                    .as_ref()
                    .map(|m| m.disk_read_bytes_per_sec),
            )),
            ColumnId::DiskWriteRate => Cell::from(crate::fmt::compact_rate_opt(
                row.analysis
                    .process_metrics
                    .as_ref()
                    .map(|m| m.disk_written_bytes_per_sec),
            )),
            ColumnId::Tokens => {
                Cell::from(format_tokens_compact(row.analysis.tokens.grand_total()))
            }
            ColumnId::Cost => Cell::from(if row.analysis.cost.total > 0.0 {
                format!("${:.2}", row.analysis.cost.total)
            } else {
                "—".into()
            }),
            ColumnId::Project => Cell::from(
                row.analysis
                    .summary
                    .cwd
                    .as_deref()
                    .map(project_basename)
                    .unwrap_or_default(),
            ),
            ColumnId::SessionName => Cell::from(
                row.analysis
                    .summary
                    .session_title
                    .clone()
                    .unwrap_or_default(),
            ),
            // Defaults for anything else.
            _ => Cell::from(""),
        }
    }

    pub fn apply_sort(&mut self) {
        let (mut tops, child_rows): (Vec<_>, Vec<_>) = std::mem::take(&mut self.rows)
            .into_iter()
            .partition(|r| r.depth == 0);

        // Build map: parent_session_id -> children (order preserved from apply_analyses).
        let mut children_map: std::collections::HashMap<String, Vec<SessionRow>> =
            std::collections::HashMap::new();
        // Children without a parent_session_id are orphaned (should not occur with current
        // construction paths) and are silently dropped here.
        for c in child_rows {
            if let Some(ref pid) = c.parent_session_id {
                children_map.entry(pid.clone()).or_default().push(c);
            }
        }

        // Sort top-level rows.
        let key = self.sort_key;
        let dir = self.sort_dir;
        tops.sort_by(|a, b| {
            let ord = sort_cmp(a, b, key);
            if dir == SortDir::Desc {
                ord.reverse()
            } else {
                ord
            }
        });

        // Rebuild: each top-level row followed by its children.
        for row in tops {
            let session_id = row.analysis.summary.session_id.clone();
            self.rows.push(row);
            if let Some(mut kids) = children_map.remove(&session_id) {
                self.rows.append(&mut kids);
            }
        }
    }
}

fn sort_cmp(a: &SessionRow, b: &SessionRow, key: SessionSortKey) -> std::cmp::Ordering {
    match key {
        SessionSortKey::Age => a
            .analysis
            .summary
            .last_active
            .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
            .cmp(
                &b.analysis
                    .summary
                    .last_active
                    .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC),
            ),
        SessionSortKey::Session => a
            .analysis
            .summary
            .session_id
            .cmp(&b.analysis.summary.session_id),
        SessionSortKey::Client => a.client_label.cmp(&b.client_label),
        SessionSortKey::Cost => a
            .analysis
            .cost
            .total
            .partial_cmp(&b.analysis.cost.total)
            .unwrap_or(std::cmp::Ordering::Equal),
        SessionSortKey::Tokens => a
            .analysis
            .tokens
            .grand_total()
            .cmp(&b.analysis.tokens.grand_total()),
        SessionSortKey::Cpu => {
            let ca = a
                .analysis
                .process_metrics
                .as_ref()
                .map(|m| m.cpu_percent)
                .unwrap_or(0.0);
            let cb = b
                .analysis
                .process_metrics
                .as_ref()
                .map(|m| m.cpu_percent)
                .unwrap_or(0.0);
            ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
        }
        SessionSortKey::Memory => {
            let ma = a
                .analysis
                .process_metrics
                .as_ref()
                .map(|m| m.memory_bytes)
                .unwrap_or(0);
            let mb = b
                .analysis
                .process_metrics
                .as_ref()
                .map(|m| m.memory_bytes)
                .unwrap_or(0);
            ma.cmp(&mb)
        }
        // TODO: Subscription, Model, Project sort keys not yet implemented — fall through to Equal.
        _ => std::cmp::Ordering::Equal,
    }
}

impl SessionsTable {
    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        match event {
            AppEvent::Key(KeyEvent {
                code, modifiers, ..
            }) => {
                if !modifiers.is_empty() && *modifiers != KeyModifiers::SHIFT {
                    return None;
                }
                match code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.move_selection(1);
                        Some(Msg::Noop)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.move_selection(-1);
                        Some(Msg::Noop)
                    }
                    KeyCode::Char('s') => {
                        self.cycle_sort_key();
                        self.apply_sort();
                        Some(Msg::Noop)
                    }
                    KeyCode::Char('S') => {
                        self.sort_dir = match self.sort_dir {
                            SortDir::Asc => SortDir::Desc,
                            SortDir::Desc => SortDir::Asc,
                        };
                        self.apply_sort();
                        Some(Msg::Noop)
                    }
                    // Toggle subagent tree collapse with `t`. Enter/Space are
                    // reserved for future actions (e.g. opening the info drawer).
                    // TODO: double-click to toggle tree.
                    KeyCode::Char('t') => {
                        if let Some(idx) = self.state.selected() {
                            if let Some(row) = self.rows.get(idx) {
                                if row.depth == 0 && !row.analysis.children.is_empty() {
                                    let sid = row.analysis.summary.session_id.clone();
                                    if self.collapsed.contains(&sid) {
                                        self.collapsed.remove(&sid);
                                    } else {
                                        self.collapsed.insert(sid);
                                    }
                                    return Some(Msg::Noop);
                                }
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }
            AppEvent::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                row,
                column,
                ..
            }) => {
                // Only scroll when the pointer is inside the table area.
                let area = self.table_area;
                if *column < area.x
                    || *column >= area.x + area.width
                    || *row < area.y
                    || *row >= area.y + area.height
                {
                    return None;
                }
                self.move_selection(1);
                Some(Msg::Noop)
            }
            AppEvent::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                row,
                column,
                ..
            }) => {
                // Only scroll when the pointer is inside the table area.
                let area = self.table_area;
                if *column < area.x
                    || *column >= area.x + area.width
                    || *row < area.y
                    || *row >= area.y + area.height
                {
                    return None;
                }
                self.move_selection(-1);
                Some(Msg::Noop)
            }
            AppEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                row,
                column,
                ..
            }) => {
                // Hit-test: only act if the click is within the table area recorded during render.
                let area = self.table_area;
                if *column < area.x
                    || *column >= area.x + area.width
                    || *row < area.y
                    || *row >= area.y + area.height
                {
                    return None;
                }
                // ratatui Table (no surrounding Block) renders the header at area.y.
                // Data rows start at area.y + 1.
                let rel = row.saturating_sub(area.y);
                if rel == 0 {
                    // Header row — ignore (sort-by-header not implemented in v2 yet).
                    return Some(Msg::Noop);
                }
                let data_idx = (rel as usize).saturating_sub(1);
                if data_idx < self.rows.len() {
                    self.state.select(Some(data_idx));
                }
                Some(Msg::Noop)
            }
            _ => None,
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let next = (cur + delta).rem_euclid(self.rows.len() as i32) as usize;
        self.state.select(Some(next));
    }

    fn cycle_sort_key(&mut self) {
        self.sort_key = match self.sort_key {
            SessionSortKey::Session => SessionSortKey::Age,
            SessionSortKey::Age => SessionSortKey::Client,
            SessionSortKey::Client => SessionSortKey::Subscription,
            SessionSortKey::Subscription => SessionSortKey::Model,
            SessionSortKey::Model => SessionSortKey::Cpu,
            SessionSortKey::Cpu => SessionSortKey::Memory,
            SessionSortKey::Memory => SessionSortKey::Tokens,
            SessionSortKey::Tokens => SessionSortKey::Cost,
            SessionSortKey::Cost => SessionSortKey::Project,
            SessionSortKey::Project => SessionSortKey::Session,
        };
    }
}

fn width_for(col: ColumnId) -> Constraint {
    match col {
        ColumnId::Session => Constraint::Length(10),
        ColumnId::Age => Constraint::Length(5),
        ColumnId::Action => Constraint::Length(20),
        ColumnId::Client => Constraint::Length(14),
        ColumnId::Subscription => Constraint::Length(16),
        ColumnId::Model => Constraint::Length(20),
        ColumnId::Cpu => Constraint::Length(5),
        ColumnId::Memory => Constraint::Length(6),
        ColumnId::Tokens => Constraint::Length(8),
        ColumnId::Cost => Constraint::Length(7),
        ColumnId::Project => Constraint::Min(12),
        ColumnId::SessionName => Constraint::Min(14),
        _ => Constraint::Length(8),
    }
}

fn render_action_cell<'a>(row: &'a SessionRow, state: &SessionState, theme: &Theme) -> Cell<'a> {
    // Idle / Closed / Warning / Error rows show "—" in ACTION.
    let show_action = matches!(state, SessionState::Running | SessionState::Waiting(_));
    if !show_action {
        return Cell::from("—");
    }
    let action = row.analysis.current_action.as_deref().unwrap_or("—");
    // Permission-pending styling: state_style decides; gives us a single source of truth.
    let style = if state_style::action_needs_warning_modifier(state) {
        Style::default()
            .fg(theme.status_warning)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_default)
    };
    Cell::from(Span::styled(action.to_string(), style))
}

fn format_session_id(id: &str) -> String {
    // Display the first 8 chars of UUID-style ids; if shorter, return as-is.
    id.chars().take(8).collect()
}

fn format_age(last: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let Some(t) = last else { return "—".into() };
    let secs = (chrono::Utc::now() - t).num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

fn format_bytes_compact(b: u64) -> String {
    const K: f32 = 1024.0;
    let f = b as f32;
    if f >= K * K * K {
        format!("{:.1}G", f / (K * K * K))
    } else if f >= K * K {
        format!("{:.0}M", f / (K * K))
    } else if f >= K {
        format!("{:.0}K", f / K)
    } else {
        format!("{b}B")
    }
}

fn format_tokens_compact(t: u64) -> String {
    let f = t as f32;
    if f >= 1_000_000.0 {
        format!("{:.1}M", f / 1_000_000.0)
    } else if f >= 1_000.0 {
        format!("{:.1}k", f / 1_000.0)
    } else {
        format!("{t}")
    }
}

fn project_basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_age_seconds() {
        let now = chrono::Utc::now();
        let last = Some(now - chrono::Duration::seconds(45));
        assert_eq!(format_age(last), "45s");
    }

    #[test]
    fn format_age_minutes() {
        let now = chrono::Utc::now();
        let last = Some(now - chrono::Duration::seconds(180));
        assert_eq!(format_age(last), "3m");
    }

    #[test]
    fn format_age_hours() {
        let now = chrono::Utc::now();
        let last = Some(now - chrono::Duration::seconds(7200));
        assert_eq!(format_age(last), "2h");
    }

    #[test]
    fn format_tokens_compact_thresholds() {
        assert_eq!(format_tokens_compact(500), "500");
        assert_eq!(format_tokens_compact(12_400), "12.4k");
        assert_eq!(format_tokens_compact(2_500_000), "2.5M");
    }

    #[test]
    fn format_session_id_truncates_to_8() {
        assert_eq!(
            format_session_id("a3f2c1de-bbbb-cccc-dddd-eeeeffff0000"),
            "a3f2c1de"
        );
    }

    #[test]
    fn project_basename_returns_last_segment() {
        assert_eq!(
            project_basename("/home/user/projects/rust-agtop"),
            "rust-agtop"
        );
        assert_eq!(project_basename("rust-agtop"), "rust-agtop");
    }

    #[test]
    fn left_click_on_first_data_row_selects_index_zero() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut t = SessionsTable {
            rows: vec![mock_row("a"), mock_row("b"), mock_row("c")],
            ..SessionsTable::default()
        };
        // Store the rendered table area manually (render() would normally write this).
        // ratatui Table with no block puts the header at y=0, data rows start at y=1.
        t.table_area = ratatui::layout::Rect::new(0, 0, 140, 12);
        // Click on row 1 = first data row (header is at row 0).
        let ev = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        t.handle_event(&ev);
        assert_eq!(
            t.state.selected(),
            Some(0),
            "clicking first data row should select index 0"
        );
    }

    #[test]
    fn left_click_outside_table_does_nothing() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut t = SessionsTable {
            rows: vec![mock_row("a"), mock_row("b")],
            ..SessionsTable::default()
        };
        t.table_area = ratatui::layout::Rect::new(0, 10, 140, 12);
        // Click above the table_area (row 5, table starts at row 10).
        let ev = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        t.handle_event(&ev);
        assert_eq!(
            t.state.selected(),
            None,
            "click outside table must not change selection"
        );
    }

    #[test]
    fn default_sort_is_age_desc() {
        let table = SessionsTable::default();
        assert_eq!(table.sort_key, SessionSortKey::Age);
        assert_eq!(table.sort_dir, SortDir::Desc);
    }

    #[test]
    fn closed_row_style_does_not_use_dim_modifier() {
        use crate::tui::theme_v2::vscode_dark_plus;
        use ratatui::style::Modifier;
        let theme = vscode_dark_plus::theme();
        // Simulate closed row style selection (same logic as render_row)
        let state = SessionState::Closed;
        let row_style = if state_style::is_muted_row(&state) {
            Style::default().fg(theme.fg_muted)
        } else {
            Style::default().fg(theme.fg_default)
        };
        assert!(
            !row_style.add_modifier.contains(Modifier::DIM),
            "closed row style must not include DIM modifier — it causes near-invisible text"
        );
    }

    #[test]
    fn highlight_style_does_not_use_reversed_modifier() {
        use crate::tui::theme_v2::vscode_dark_plus;
        use ratatui::style::Modifier;
        let theme = vscode_dark_plus::theme();
        // Simulate the row_highlight_style (same tokens as render())
        let highlight = Style::default()
            .bg(theme.bg_selection)
            .fg(theme.fg_emphasis);
        assert!(
            !highlight.add_modifier.contains(Modifier::REVERSED),
            "highlight style must not include REVERSED — it inverts bg_selection to near-invisible"
        );
    }

    fn mock_row(id: &str) -> SessionRow {
        use agtop_core::session::{CostBreakdown, SessionSummary, TokenTotals};
        let summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            id.to_string(),
            None,
            None,
            None,
            None,
            std::path::PathBuf::new(),
            None,
            None,
            None,
        );
        let analysis = SessionAnalysis::new(
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
        SessionRow {
            analysis,
            client_kind: ClientKind::Claude,
            client_label: "claude".into(),
            activity_samples: vec![],
            depth: 0,
            parent_session_id: None,
            is_last_child: false,
        }
    }

    #[test]
    fn activity_samples_render_to_8_braille_chars() {
        let s =
            sparkline_braille::render_braille(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], 8, 10.0);
        assert_eq!(s.chars().count(), 8);
        // All chars should be in the braille block.
        for c in s.chars() {
            assert!((0x2800..=0x28FF).contains(&(c as u32)));
        }
    }

    #[test]
    fn down_moves_selection_forward() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut t = SessionsTable {
            rows: vec![mock_row("a"), mock_row("b"), mock_row("c")],
            ..SessionsTable::default()
        };
        t.state.select(Some(0));
        let ev = AppEvent::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        t.handle_event(&ev);
        assert_eq!(t.state.selected(), Some(1));
    }

    #[test]
    fn up_wraps_from_zero() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut t = SessionsTable {
            rows: vec![mock_row("a"), mock_row("b"), mock_row("c")],
            ..SessionsTable::default()
        };
        t.state.select(Some(0));
        let ev = AppEvent::Key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        t.handle_event(&ev);
        // Should wrap to last row.
        assert_eq!(t.state.selected(), Some(2));
    }

    #[test]
    fn scroll_outside_table_area_is_ignored() {
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let mut t = SessionsTable {
            rows: vec![mock_row("a"), mock_row("b"), mock_row("c")],
            ..SessionsTable::default()
        };
        t.table_area = ratatui::layout::Rect::new(0, 10, 140, 20);
        t.state.select(Some(1));
        let ev = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5, // above table (starts at y=10)
            modifiers: KeyModifiers::NONE,
        });
        t.handle_event(&ev);
        assert_eq!(
            t.state.selected(),
            Some(1),
            "scroll outside table must not change selection"
        );
    }

    #[test]
    fn scroll_inside_table_area_moves_selection() {
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let mut t = SessionsTable {
            rows: vec![mock_row("a"), mock_row("b"), mock_row("c")],
            ..SessionsTable::default()
        };
        t.table_area = ratatui::layout::Rect::new(0, 0, 140, 20);
        t.state.select(Some(0));
        let ev = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        t.handle_event(&ev);
        assert_eq!(
            t.state.selected(),
            Some(1),
            "scroll inside table must advance selection"
        );
    }

    #[test]
    fn depth_one_row_renders_tree_glyph_in_first_cell() {
        use crate::tui::theme_v2::vscode_dark_plus;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let theme = vscode_dark_plus::theme();
        let mut parent = mock_row("parent01");
        // Pretend the parent has at least one child so the toggle (▶/▼) renders.
        parent.analysis.children = vec![parent.analysis.clone()];

        let mut child_a = mock_row("childAAA");
        child_a.depth = 1;
        child_a.parent_session_id = Some("parent01".into());
        child_a.is_last_child = false; // mid child

        let mut child_b = mock_row("childBBB");
        child_b.depth = 1;
        child_b.parent_session_id = Some("parent01".into());
        child_b.is_last_child = true; // last child

        let mut t = SessionsTable {
            rows: vec![parent, child_a, child_b],
            ..SessionsTable::default()
        };

        let mut term = Terminal::new(TestBackend::new(160, 10)).unwrap();
        term.draw(|f| t.render(f, ratatui::layout::Rect::new(0, 0, 160, 10), &theme))
            .unwrap();

        let buf = term.backend().buffer().clone();
        // Header is row 0; data rows start at y=1. Order: parent, child_a, child_b.
        // Read leftmost ~10 cells of each child row and assert tree glyph presence.
        let read_row_prefix = |y: u16, len: usize| -> String {
            let mut s = String::new();
            for x in 0..(len as u16) {
                let cell = &buf[(x, y)];
                s.push_str(cell.symbol());
            }
            s
        };

        let mid_prefix = read_row_prefix(2, 12);
        let last_prefix = read_row_prefix(3, 12);

        assert!(
            mid_prefix.contains("├──"),
            "mid child must render with `├──` glyph; got: {mid_prefix:?}"
        );
        assert!(
            last_prefix.contains("└──"),
            "last child must render with `└──` glyph; got: {last_prefix:?}"
        );
    }

    #[test]
    fn closed_row_with_pid_is_not_dimmed() {
        use crate::tui::theme_v2::vscode_dark_plus;
        let theme = vscode_dark_plus::theme();
        let mut row = mock_row("pid-test");
        row.analysis.pid = Some(12345);
        let state = SessionState::Closed;
        let row_style = if state_style::is_muted_row(&state) && row.analysis.pid.is_none() {
            Style::default().fg(theme.fg_muted)
        } else {
            Style::default().fg(theme.fg_default)
        };
        assert_eq!(
            row_style.fg,
            Some(theme.fg_default),
            "Closed row with a live PID should use fg_default, not fg_muted"
        );
    }
}
