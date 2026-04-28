//! Info drawer: bottom-right floating panel with Summary/General/Costs/Process tabs.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{info_details, info_summary};
use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::screens::dashboard::sessions::SessionRow;
use crate::tui::theme_v2::Theme;
use crate::tui::widgets::drawer::{self, Anchor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawerVis {
    #[default]
    Closed,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InfoTab {
    #[default]
    Summary,
    Details,
}

impl InfoTab {
    pub const ALL: [InfoTab; 2] = [Self::Summary, Self::Details];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Details => "Details",
        }
    }
}

#[derive(Debug, Default)]
pub struct InfoDrawer {
    pub vis: DrawerVis,
    pub tab: InfoTab,
    /// Selected session row from the table; drives all tab bodies.
    pub selected_row: Option<SessionRow>,
    /// Last area occupied by the drawer (set during render). Used to block
    /// click-through to the sessions table behind the drawer.
    pub last_area: Option<Rect>,
}

impl InfoDrawer {
    /// Sync the selected row from the sessions table. Call after every
    /// selection change.
    pub fn set_row(&mut self, row: Option<SessionRow>) {
        self.selected_row = row;
    }

    /// Convenience accessor for the selected session id (for the drawer title).
    fn selected_session_id(&self) -> Option<&str> {
        self.selected_row
            .as_ref()
            .map(|r| r.analysis.summary.session_id.as_str())
    }

    /// Build the drawer title string for a given drawer area. Returns the
    /// composed title and the column-offset (chars, not bytes) within `title`
    /// where the tabs segment begins (used for hit-testing tab markers).
    fn build_title(&self, area: Rect) -> (String, usize) {
        let id_str = self.selected_session_id().unwrap_or("—");
        let tabs_full = Self::TABS_FULL;
        let tabs_short = Self::TABS_SHORT;
        let id_part = format!(" Session: {id_str} ");
        // Use char count (≈ display columns) for the width check and for the
        // returned offset. A literal byte-length comparison would mis-count
        // multi-byte glyphs like the em-dash placeholder.
        let id_cols = id_part.chars().count();
        let tabs_full_cols = tabs_full.chars().count();
        let tabs_part = if (area.width as usize) >= id_cols + tabs_full_cols {
            tabs_full
        } else {
            tabs_short
        };
        let title = format!("{id_part}{tabs_part}");
        (title, id_cols)
    }

    const TABS_FULL: &'static str = " [1] Summary  [2] Details ";
    const TABS_SHORT: &'static str = " [1]Sum [2]Details ";

    /// Compute clickable column ranges for each tab marker, expressed in
    /// absolute terminal coordinates. Returns a Vec of (start_col, end_col, tab)
    /// where `end_col` is exclusive.
    ///
    /// Pattern: tab marker `[N]` starts at the position of `[`. The clickable
    /// region for tab N runs from that position up to (but not including) the
    /// position of `[N+1]`. The last tab's region runs to the end of `tabs_part`.
    ///
    /// All returned columns are clamped to the drawer area to avoid spurious
    /// out-of-band hits when the title is narrower than the drawer width.
    fn tab_marker_ranges(&self, area: Rect) -> Vec<(u16, u16, InfoTab)> {
        let (title, _tabs_offset) = self.build_title(area);
        // ratatui's Block::title is rendered starting at area.x + 1 (after
        // the top-left border corner glyph).
        let title_start_col = area.x.saturating_add(1);
        let max_col = area.x.saturating_add(area.width);

        // Walk the title char-by-char (terminal column = 1 per char for the
        // characters we use here — ASCII tabs + em-dash placeholder which
        // we treat as 1 column for simplicity; ratatui itself renders it as
        // one cell). Find each `[N]` marker's column offset within `title`.
        let chars: Vec<char> = title.chars().collect();
        let mut markers: Vec<(usize, InfoTab)> = Vec::with_capacity(4);
        for (tab, n) in [
            (InfoTab::Summary, '1'),
            (InfoTab::Details, '2'),
        ] {
            for i in 0..chars.len().saturating_sub(2) {
                if chars[i] == '[' && chars[i + 1] == n && chars[i + 2] == ']' {
                    markers.push((i, tab));
                    break;
                }
            }
        }

        let title_end = title_start_col.saturating_add(chars.len() as u16);
        let mut out: Vec<(u16, u16, InfoTab)> = Vec::with_capacity(markers.len());
        for i in 0..markers.len() {
            let (start_off, tab) = markers[i];
            let start = title_start_col.saturating_add(start_off as u16);
            let end = if i + 1 < markers.len() {
                title_start_col.saturating_add(markers[i + 1].0 as u16)
            } else {
                title_end
            };
            let start = start.min(max_col);
            let end = end.min(max_col);
            if start < end {
                out.push((start, end, tab));
            }
        }
        out
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, parent: Rect, theme: &Theme) {
        if self.vis == DrawerVis::Closed {
            self.last_area = None;
            return;
        }
        let area = drawer::rect_for(parent, Anchor::BottomRight, 0.5, 0.6);
        self.last_area = Some(area);

        let (title, _) = self.build_title(area);
        let inner = drawer::render_chrome(frame, area, &title, theme);

        // Tab body — dispatch to real content modules when a row is selected;
        // show a friendly placeholder otherwise.
        match (self.tab, self.selected_row.as_ref()) {
            (InfoTab::Summary, Some(row)) => {
                use agtop_core::session::SessionState;
                let state = row
                    .analysis
                    .session_state
                    .clone()
                    .unwrap_or(SessionState::Closed);
                let model = info_summary::SummaryModel {
                    analysis: &row.analysis,
                    client_label: &row.client_label,
                    client_kind: row.client_kind,
                    state: &state,
                    recent_turns: vec![],
                    nerd_font: false,
                };
                info_summary::render(frame, inner, &model, theme);
            }
            (InfoTab::Details, Some(row)) => {
                let model = info_details::DetailsModel {
                    analysis: &row.analysis,
                    parent_session_id: row.parent_session_id.as_deref(),
                    subagent_count: row.analysis.children.len(),
                    scroll_offset: 0,
                };
                info_details::render(frame, inner, &model, theme);
            }
            (_, None) => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "No session selected — press j/k or click a row",
                        Style::default().fg(theme.fg_muted),
                    ))),
                    inner,
                );
            }
        }
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };

        // Mouse: when drawer is open, clicking a tab marker in the title row
        // switches tabs.
        if let AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row,
            column,
            ..
        }) = event
        {
            if self.vis == DrawerVis::Open {
                if let Some(area) = self.last_area {
                    if *row == area.y && *column >= area.x && *column < area.x + area.width {
                        for (start, end, tab) in self.tab_marker_ranges(area) {
                            if *column >= start && *column < end {
                                self.tab = tab;
                                return Some(Msg::Noop);
                            }
                        }
                        // Click was on the title row but not on a tab marker;
                        // swallow it so it doesn't fall through to widgets behind.
                        return Some(Msg::Noop);
                    }
                }
            }
            return None;
        }

        let AppEvent::Key(KeyEvent {
            code, modifiers, ..
        }) = event
        else {
            return None;
        };
        if !modifiers.is_empty() && *modifiers != KeyModifiers::SHIFT {
            return None;
        }
        match code {
            KeyCode::Char('i') if self.vis == DrawerVis::Open => {
                self.vis = DrawerVis::Closed;
                Some(Msg::Noop)
            }
            KeyCode::Esc if self.vis == DrawerVis::Open => {
                self.vis = DrawerVis::Closed;
                Some(Msg::Noop)
            }
            KeyCode::Char('i') => {
                self.vis = DrawerVis::Open;
                self.tab = InfoTab::Summary;
                Some(Msg::Noop)
            }
            KeyCode::Char('1') if self.vis == DrawerVis::Open => {
                self.tab = InfoTab::Summary;
                Some(Msg::Noop)
            }
            KeyCode::Char('2') if self.vis == DrawerVis::Open => {
                self.tab = InfoTab::Details;
                Some(Msg::Noop)
            }
            KeyCode::Char('3') | KeyCode::Char('4') if self.vis == DrawerVis::Open => {
                Some(Msg::Noop)
            }
            KeyCode::Tab if self.vis == DrawerVis::Open => {
                self.tab = match self.tab {
                    InfoTab::Summary => InfoTab::Details,
                    InfoTab::Details => InfoTab::Summary,
                };
                Some(Msg::Noop)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn k(c: char) -> AppEvent {
        AppEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn i_toggles_open_close() {
        let mut d = InfoDrawer::default();
        assert_eq!(d.vis, DrawerVis::Closed);
        d.handle_event(&k('i'));
        assert_eq!(d.vis, DrawerVis::Open);
        d.handle_event(&k('i'));
        assert_eq!(d.vis, DrawerVis::Closed);
    }

    #[test]
    fn open_drawer_defaults_to_summary_tab() {
        let mut d = InfoDrawer::default();
        d.handle_event(&k('i'));
        assert_eq!(d.tab, InfoTab::Summary);
    }

    #[test]
    fn tab_keys_switch_tabs_when_open() {
        let mut d = InfoDrawer::default();
        d.handle_event(&k('i'));
        d.handle_event(&k('2'));
        assert_eq!(d.tab, InfoTab::Details);
    }

    #[test]
    fn mouse_click_on_tab_marker_switches_tab() {
        // Render the drawer to a TestBackend, scan the top border row for a
        // [2] marker, simulate a left-click at that column, and assert the
        // tab switches to General.
        use crate::tui::theme_v2::vscode_dark_plus;
        use crossterm::event::{KeyModifiers as KM, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let theme = vscode_dark_plus::theme();
        let mut d = InfoDrawer {
            vis: DrawerVis::Open,
            ..InfoDrawer::default()
        };

        // Use a parent area large enough to fit the full title string.
        let parent = Rect::new(0, 0, 200, 40);
        let mut term = Terminal::new(TestBackend::new(parent.width, parent.height)).unwrap();
        term.draw(|f| d.render(f, parent, &theme)).unwrap();

        let drawer_area = d.last_area.expect("drawer must record its area on render");
        let buf = term.backend().buffer();

        // Scan the top border row of the drawer for the `[2]` marker.
        let title_row = drawer_area.y;
        let mut click_col: Option<u16> = None;
        for x in drawer_area.x..(drawer_area.x + drawer_area.width).saturating_sub(2) {
            if buf[(x, title_row)].symbol() == "["
                && buf[(x + 1, title_row)].symbol() == "2"
                && buf[(x + 2, title_row)].symbol() == "]"
            {
                // Click on the `2` itself.
                click_col = Some(x + 1);
                break;
            }
        }
        let click_col = click_col.expect("[2] marker must be present in drawer title");

        // Sanity: starting tab is Summary.
        assert_eq!(d.tab, InfoTab::Summary);

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: click_col,
            row: title_row,
            modifiers: KM::NONE,
        });
        d.handle_event(&click);

        assert_eq!(
            d.tab,
            InfoTab::Details,
            "clicking [2] in the drawer title must switch tab to Details"
        );
    }

    #[test]
    fn mouse_click_outside_title_row_does_not_switch_tab() {
        use crate::tui::theme_v2::vscode_dark_plus;
        use crossterm::event::{KeyModifiers as KM, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let theme = vscode_dark_plus::theme();
        let mut d = InfoDrawer {
            vis: DrawerVis::Open,
            ..InfoDrawer::default()
        };
        let parent = Rect::new(0, 0, 200, 40);
        let mut term = Terminal::new(TestBackend::new(parent.width, parent.height)).unwrap();
        term.draw(|f| d.render(f, parent, &theme)).unwrap();
        let drawer_area = d.last_area.unwrap();

        // Click in the body of the drawer, NOT the title row.
        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: drawer_area.x + 5,
            row: drawer_area.y + 3, // body, not the border
            modifiers: KM::NONE,
        });
        d.handle_event(&click);

        // Tab unchanged.
        assert_eq!(d.tab, InfoTab::Summary);
    }

    #[test]
    fn tab_keys_inert_when_closed() {
        let mut d = InfoDrawer::default();
        d.handle_event(&k('2'));
        assert_eq!(d.vis, DrawerVis::Closed);
        assert_eq!(d.tab, InfoTab::Summary); // unchanged from default
    }
}
