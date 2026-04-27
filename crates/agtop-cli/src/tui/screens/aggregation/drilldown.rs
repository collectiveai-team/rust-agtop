//! Drill-down overlay: full-screen `SessionsTable` filtered to one group.

use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear},
    Frame,
};

use agtop_core::aggregate::GroupBy;
use agtop_core::session::SessionAnalysis;

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::screens::dashboard::sessions::{SessionRow, SessionsTable};
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct DrillDown {
    open: bool,
    label: String,
    table: SessionsTable,
}

impl DrillDown {
    pub fn is_open(&self) -> bool { self.open }

    pub fn open(&mut self, label: String, sessions: &[SessionAnalysis], by: GroupBy) {
        self.label = label.clone();
        self.table.rows = sessions
            .iter()
            .filter(|s| Self::matches_group(s, &label, by))
            .map(|s| SessionRow {
                analysis: s.clone(),
                client_kind: s.summary.client,
                client_label: s.summary.client.as_str().to_string(),
                activity_samples: vec![],
            })
            .collect();
        self.table.apply_sort();
        self.open = true;
    }

    fn matches_group(s: &SessionAnalysis, label: &str, by: GroupBy) -> bool {
        match by {
            GroupBy::Client | GroupBy::Provider => s.summary.client.as_str() == label,
            GroupBy::Model => s.summary.model.as_deref() == Some(label),
            GroupBy::Project => {
                let basename = s.summary.cwd.as_deref()
                    .and_then(|p| std::path::Path::new(p).file_name())
                    .map(|n| n.to_string_lossy().into_owned());
                basename.as_deref() == Some(label)
            }
            GroupBy::Subscription => s.summary.subscription.as_deref() == Some(label),
        }
    }

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        if !self.open { return }
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" Drill-down: {}  [Esc] close ", self.label))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.bg_overlay));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.table.render(frame, inner, theme);
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{KeyCode, KeyEvent};
        if let AppEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) = event {
            self.open = false;
            return Some(Msg::Noop);
        }
        // Forward to inner table so j/k still work inside the drill-down.
        self.table.handle_event(event)
    }
}
