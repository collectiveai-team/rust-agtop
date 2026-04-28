//! Aggregation screen: pickers + table + drill-down overlay.
#![allow(dead_code)]

pub mod controls;
pub mod table;
pub mod drilldown;

use ratatui::{layout::{Constraint, Direction, Layout, Rect}, Frame};

use agtop_core::aggregate::{aggregate, GroupBy, TimeRange};
use agtop_core::session::SessionAnalysis;

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct AggregationState {
    pub controls: controls::ControlsModel,
    pub table: table::AggregationTable,
    pub drill: drilldown::DrillDown,
    /// Session input (filled by refresh adapter).
    pub sessions: Vec<SessionAnalysis>,
}

impl AggregationState {
    /// Recompute aggregates from current sessions + controls.
    pub fn recompute(&mut self) {
        let now = chrono::Utc::now();
        self.table.groups = aggregate(
            &self.sessions,
            self.controls.group_by,
            self.controls.range,
            now,
            12,
        );
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),    // controls
                Constraint::Min(0),       // table
                Constraint::Length(1),    // footer hint
            ])
            .split(area);

        self.controls.render(frame, layout[0], theme);
        self.table.render(frame, layout[1], theme);
        // Footer hint
        use ratatui::{text::{Line, Span}, style::Style, widgets::Paragraph};
        let footer = Line::from(vec![Span::styled(
            " [g] group   [r] range   [s] sort   [/] filter   [Enter] drill into group   [?] help ",
            Style::default().fg(theme.fg_muted),
        )]);
        frame.render_widget(Paragraph::new(footer), layout[2]);

        // Drill-down overlay
        self.drill.render(frame, area, theme);
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        // Drill-down captures events when open.
        if self.drill.is_open() {
            return self.drill.handle_event(event);
        }
        // Route mouse clicks to controls (group-by / range chips).
        if matches!(event, AppEvent::Mouse(_)) {
            if let Some(msg) = self.controls.handle_event(event) {
                self.recompute();
                return Some(msg);
            }
        }

        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let AppEvent::Key(KeyEvent { code, modifiers, .. }) = event else { return None };
        if !modifiers.is_empty() && *modifiers != KeyModifiers::SHIFT { return None }
        match code {
            KeyCode::Char('g') => {
                self.controls.group_by = match self.controls.group_by {
                    GroupBy::Client => GroupBy::Provider,
                    GroupBy::Provider => GroupBy::Model,
                    GroupBy::Model => GroupBy::Project,
                    GroupBy::Project => GroupBy::Subscription,
                    GroupBy::Subscription => GroupBy::Client,
                };
                self.recompute();
                Some(Msg::Noop)
            }
            KeyCode::Char('r') => {
                self.controls.range = match self.controls.range {
                    TimeRange::Today => TimeRange::Week,
                    TimeRange::Week => TimeRange::Month,
                    TimeRange::Month => TimeRange::All,
                    TimeRange::All => TimeRange::Today,
                };
                self.recompute();
                Some(Msg::Noop)
            }
            KeyCode::Enter => {
                if let Some(idx) = self.table.state.selected() {
                    if let Some(g) = self.table.groups.get(idx) {
                        self.drill.open(g.label.clone(), &self.sessions, self.controls.group_by);
                        return Some(Msg::Noop);
                    }
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.table.state.selected().unwrap_or(0);
                if !self.table.groups.is_empty() {
                    self.table.state.select(Some((cur + 1) % self.table.groups.len()));
                }
                Some(Msg::Noop)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let cur = self.table.state.selected().unwrap_or(0);
                if !self.table.groups.is_empty() {
                    let next = if cur == 0 { self.table.groups.len() - 1 } else { cur - 1 };
                    self.table.state.select(Some(next));
                }
                Some(Msg::Noop)
            }
            _ => None,
        }
    }
}
