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
    last_area: Option<Rect>,
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
                depth: 0,
                parent_session_id: None,
                is_last_child: false,
            })
            .collect();
        self.table.apply_sort();
        self.open = true;
        self.last_area = None;
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

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        if !self.open { return }
        self.last_area = Some(area);
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
        use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
        // Keyboard Esc closes.
        if let AppEvent::Key(KeyEvent { code: KeyCode::Esc, .. }) = event {
            self.open = false;
            return Some(Msg::Noop);
        }
        // Mouse click on "[Esc] close" in the title bar closes.
        if let AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column, row, ..
        }) = event {
            if let Some(area) = self.last_area {
                if *row == area.y {
                    let title = format!(" Drill-down: {}  [Esc] close ", self.label);
                    let title_start = area.x + 1; // +1 for left border
                    if *column >= title_start {
                        let rel = (*column - title_start) as usize;
                        if let Some(esc_pos) = title.find("[Esc]") {
                            let esc_end = esc_pos + "[Esc] close".len();
                            if rel >= esc_pos && rel < esc_end {
                                self.open = false;
                                return Some(Msg::Noop);
                            }
                        }
                    }
                }
            }
        }
        // Forward to inner table so j/k still work inside the drill-down.
        self.table.handle_event(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn click_on_esc_close_button_closes_drilldown() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;

        let backend = TestBackend::new(120, 30);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();

        let mut d = DrillDown::default();
        d.open("Claude".to_string(), &[], GroupBy::Client);
        assert!(d.is_open());

        // Render to populate last_area.
        term.draw(|f| d.render(f, ratatui::layout::Rect::new(0, 0, 120, 30), &theme)).unwrap();

        // The title bar row is y=0. Title is " Drill-down: Claude  [Esc] close ".
        // "[Esc]" starts at offset 22 in the title (after " Drill-down: Claude  ").
        // With a left border at x=0, title chars start at x=1.
        let area = d.last_area.unwrap();
        let title_str = format!(" Drill-down: {}  [Esc] close ", "Claude");
        let esc_offset = title_str.find("[Esc]").unwrap() as u16;
        let click_col = area.x + 1 + esc_offset + 1; // +1 for border, +1 for inside "[Esc]"

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: click_col,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        });
        d.handle_event(&click);
        assert!(!d.is_open(), "clicking [Esc] close must close the drill-down");
    }
}
