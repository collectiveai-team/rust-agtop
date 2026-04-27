//! Sessions table: state dot + 14 columns + activity sparkline.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

use agtop_core::session::{ClientKind, SessionAnalysis, SessionState};

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
}

#[derive(Debug)]
pub struct SessionsTable {
    pub rows: Vec<SessionRow>,
    pub state: TableState,
    pub pulse: PulseClock,
    pub animations_enabled: bool,
    pub sort_key: SessionSortKey,
    pub sort_dir: SortDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSortKey {
    Session, Age, Client, Subscription, Model, Cpu, Memory, Tokens, Cost, Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir { Asc, Desc }

impl Default for SessionsTable {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            state: TableState::default(),
            pulse: PulseClock::default(),
            animations_enabled: true,
            sort_key: SessionSortKey::Age,
            sort_dir: SortDir::Asc,
        }
    }
}

impl SessionsTable {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        // Map columns from default_visible into column constraints + header strings.
        let columns = column_config::default_visible_v2();
        let mut header_cells: Vec<Cell> = Vec::with_capacity(columns.len() + 2);
        // State dot column has no header label.
        header_cells.push(Cell::from(""));
        for c in &columns {
            header_cells.push(Cell::from(c.label()).style(
                Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD),
            ));
        }

        let body_rows: Vec<Row> = self
            .rows
            .iter()
            .map(|sr| self.render_row(sr, &columns, theme))
            .collect();

        let mut constraints: Vec<Constraint> = Vec::with_capacity(columns.len() + 1);
        constraints.push(Constraint::Length(2)); // state dot + space
        for c in &columns {
            constraints.push(width_for(*c));
        }

        let table = Table::new(body_rows, &constraints)
            .header(Row::new(header_cells));
        frame.render_widget(table, area);
    }

    fn render_row<'a>(
        &'a self,
        row: &'a SessionRow,
        columns: &[ColumnId],
        theme: &Theme,
    ) -> Row<'a> {
        // Read state directly from core (post-normalization field).
        let state: SessionState = row
            .analysis
            .session_state
            .clone()
            .unwrap_or(SessionState::Closed);

        // Apply muted styling to closed rows.
        let row_style = if state_style::is_muted_row(&state) {
            Style::default().fg(theme.fg_muted).add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(theme.fg_default)
        };

        // Build cells.
        let mut cells: Vec<Cell> = Vec::with_capacity(columns.len() + 1);
        // State dot — reads SessionState directly via state_style.
        cells.push(Cell::from(Line::from(state_dot::render(
            &state,
            &self.pulse,
            self.animations_enabled,
            theme,
        ))));

        for c in columns {
            cells.push(self.render_cell(row, *c, &state, theme));
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
            ColumnId::Model => Cell::from(
                row.analysis.summary.model.clone().unwrap_or_default(),
            ),
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
            ColumnId::Tokens => Cell::from(
                format_tokens_compact(row.analysis.tokens.grand_total()),
            ),
            ColumnId::Cost => Cell::from(
                if row.analysis.cost.total > 0.0 {
                    format!("${:.2}", row.analysis.cost.total)
                } else {
                    "—".into()
                }
            ),
            ColumnId::Project => Cell::from(
                row.analysis
                    .summary
                    .cwd
                    .as_deref()
                    .map(project_basename)
                    .unwrap_or_default(),
            ),
            ColumnId::SessionName => Cell::from(
                row.analysis.summary.session_title.clone().unwrap_or_default(),
            ),
            // Defaults for anything else.
            _ => Cell::from(""),
        }
    }

    pub fn apply_sort(&mut self) {
        match self.sort_key {
            SessionSortKey::Age => {
                self.rows.sort_by_key(|r| {
                    r.analysis.summary.last_active.unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::MIN_UTC)
                });
                if self.sort_dir == SortDir::Desc { self.rows.reverse() }
            }
            SessionSortKey::Session => {
                self.rows.sort_by(|a, b| a.analysis.summary.session_id.cmp(&b.analysis.summary.session_id));
                if self.sort_dir == SortDir::Desc { self.rows.reverse() }
            }
            SessionSortKey::Client => {
                self.rows.sort_by(|a, b| a.client_label.cmp(&b.client_label));
                if self.sort_dir == SortDir::Desc { self.rows.reverse() }
            }
            SessionSortKey::Cost => {
                self.rows.sort_by(|a, b| {
                    a.analysis.cost.total.partial_cmp(&b.analysis.cost.total)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                if self.sort_dir == SortDir::Desc { self.rows.reverse() }
            }
            SessionSortKey::Tokens => {
                self.rows.sort_by_key(|r| r.analysis.tokens.grand_total());
                if self.sort_dir == SortDir::Desc { self.rows.reverse() }
            }
            SessionSortKey::Cpu => {
                self.rows.sort_by(|a, b| {
                    let ca = a.analysis.process_metrics.as_ref().map(|m| m.cpu_percent).unwrap_or(0.0);
                    let cb = b.analysis.process_metrics.as_ref().map(|m| m.cpu_percent).unwrap_or(0.0);
                    ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
                });
                if self.sort_dir == SortDir::Desc { self.rows.reverse() }
            }
            SessionSortKey::Memory => {
                self.rows.sort_by_key(|r| r.analysis.process_metrics.as_ref().map(|m| m.memory_bytes).unwrap_or(0));
                if self.sort_dir == SortDir::Desc { self.rows.reverse() }
            }
            _ => {}
        }
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
    let show_action = matches!(
        state,
        SessionState::Running | SessionState::Waiting(_)
    );
    if !show_action {
        return Cell::from("—");
    }
    let action = row.analysis.current_action.as_deref().unwrap_or("—");
    // Permission-pending styling: state_style decides; gives us a single source of truth.
    let style = if state_style::action_needs_warning_modifier(state) {
        Style::default().fg(theme.status_warning).add_modifier(Modifier::BOLD)
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
    if secs < 60 { format!("{secs}s") }
    else if secs < 3600 { format!("{}m", secs / 60) }
    else if secs < 86400 { format!("{}h", secs / 3600) }
    else { format!("{}d", secs / 86400) }
}

fn format_bytes_compact(b: u64) -> String {
    const K: f32 = 1024.0;
    let f = b as f32;
    if f >= K * K * K { format!("{:.1}G", f / (K * K * K)) }
    else if f >= K * K { format!("{:.0}M", f / (K * K)) }
    else if f >= K { format!("{:.0}K", f / K) }
    else { format!("{b}B") }
}

fn format_tokens_compact(t: u64) -> String {
    let f = t as f32;
    if f >= 1_000_000.0 { format!("{:.1}M", f / 1_000_000.0) }
    else if f >= 1_000.0 { format!("{:.1}k", f / 1_000.0) }
    else { format!("{t}") }
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
        assert_eq!(format_session_id("a3f2c1de-bbbb-cccc-dddd-eeeeffff0000"), "a3f2c1de");
    }

    #[test]
    fn project_basename_returns_last_segment() {
        assert_eq!(project_basename("/home/user/projects/rust-agtop"), "rust-agtop");
        assert_eq!(project_basename("rust-agtop"), "rust-agtop");
    }

    #[test]
    fn default_sort_is_age_asc() {
        let table = SessionsTable::default();
        assert_eq!(table.sort_key, SessionSortKey::Age);
        assert_eq!(table.sort_dir, SortDir::Asc);
    }
}
