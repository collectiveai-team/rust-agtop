//! Usage Quota panel with three modes: Short / Long / Hidden.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use agtop_core::session::ClientKind;

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::theme_v2::{client_palette, Theme};
use crate::tui::widgets::gradient_bar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuotaMode {
    #[default]
    Short,
    Long,
    Hidden,
}

impl QuotaMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Short => Self::Long,
            Self::Long => Self::Hidden,
            Self::Hidden => Self::Short,
        }
    }

    pub fn rows_needed(self) -> u16 {
        match self {
            Self::Short => 4,
            Self::Long => 12,
            Self::Hidden => 0,
        }
    }
}



/// One client's closest-to-limit window (short mode) or full set of windows (long mode).
#[derive(Debug, Clone)]
pub struct QuotaCardModel {
    pub client_kind: ClientKind,
    pub client_label: String,
    pub closest: WindowModel,
    pub all_windows: Vec<WindowModel>,
}

#[derive(Debug, Clone)]
pub struct WindowModel {
    pub label: String,         // e.g. "5h", "weekly"
    pub used_pct: f32,          // 0..=1
    pub note: Option<String>,
    pub reset_in: Option<String>,
}

#[derive(Debug, Default)]
pub struct QuotaPanel {
    pub mode: QuotaMode,
    pub cards: Vec<QuotaCardModel>,
    /// Last render area; used to hit-test mouse clicks on the `[u]` title button.
    pub last_area: Option<Rect>,
    /// Scroll offset for Long mode (lines scrolled from top).
    pub scroll_offset: usize,
}

fn build_card_lines<'a>(card: &'a QuotaCardModel, label_width: usize, theme: &'a Theme) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        card.client_label.clone(),
        Style::default()
            .fg(client_palette::color_for(card.client_kind))
            .add_modifier(Modifier::BOLD),
    )));
    for w in &card.all_windows {
        let (filled, color, empty) = gradient_bar::render_bar(w.used_pct, 18, theme);
        let mut spans: Vec<Span<'static>> = vec![
            Span::raw("  "),
            Span::styled(
                format!("{:>width$}", w.label, width = label_width),
                Style::default().fg(theme.fg_muted),
            ),
            Span::raw("  "),
            Span::styled(filled, Style::default().fg(color)),
            Span::styled(empty, Style::default().fg(theme.border_muted)),
            Span::raw("  "),
            Span::styled(
                format!("{:>3.0}%", w.used_pct * 100.0),
                Style::default().fg(theme.fg_default),
            ),
        ];
        if let Some(note) = &w.note {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(note.clone(), Style::default().fg(theme.fg_muted)));
        }
        if let Some(reset) = &w.reset_in {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(reset.clone(), Style::default().fg(theme.fg_muted)));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines
}

impl QuotaPanel {
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        match self.mode {
            QuotaMode::Hidden => {}
            QuotaMode::Short => self.render_short(frame, area, theme),
            QuotaMode::Long => self.render_long(frame, area, theme),
        }
    }

    fn render_short(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Usage Quota (short)  [u]sage ",
                Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(theme.border_muted));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Build a single-line set of segments separated by " · ".
        let mut spans: Vec<Span> = Vec::with_capacity(self.cards.len() * 8);
        for (i, card) in self.cards.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ·  ", Style::default().fg(theme.border_muted)));
            }
            spans.push(Span::styled(
                card.client_label.clone(),
                Style::default().fg(client_palette::color_for(card.client_kind)),
            ));
            spans.push(Span::raw("  "));
            let (filled, color, empty) = gradient_bar::render_bar(card.closest.used_pct, 12, theme);
            spans.push(Span::styled(filled, Style::default().fg(color)));
            spans.push(Span::styled(empty, Style::default().fg(theme.border_muted)));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("{:>3.0}%", card.closest.used_pct * 100.0),
                Style::default().fg(theme.fg_default),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                card.closest.label.clone(),
                Style::default().fg(theme.fg_muted),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), inner);
    }

    fn render_long(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Usage Quota (long)  [u]sage ",
                Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(theme.border_muted));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let label_width = self.cards.iter()
            .flat_map(|c| c.all_windows.iter().map(|w| w.label.len()))
            .max()
            .unwrap_or(4)
            .max(4);

        let inner_h = inner.height as usize;
        // Reserve 1 row for overflow indicators if needed.
        let view_h = inner_h.saturating_sub(1).max(1);

        if area.width > 80 && self.cards.len() >= 2 {
            // 2-column layout.
            use ratatui::layout::{Direction, Layout, Constraint};
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let mid = (self.cards.len() + 1) / 2;
            let left_lines: Vec<Line<'static>> = self.cards[..mid].iter()
                .flat_map(|c| build_card_lines(c, label_width, theme))
                .collect();
            let right_lines: Vec<Line<'static>> = self.cards[mid..].iter()
                .flat_map(|c| build_card_lines(c, label_width, theme))
                .collect();

            let total = left_lines.len().max(right_lines.len());
            let offset = self.scroll_offset.min(total.saturating_sub(view_h));

            let overflow_below = total.saturating_sub(offset + view_h);
            if overflow_below > 0 {
                let hint = Line::from(Span::styled(
                    format!("  ↓ {overflow_below} more lines — press [u] for short view"),
                    Style::default().fg(theme.fg_muted),
                ));
                let hint_rect = Rect::new(inner.x, inner.y + inner_h as u16 - 1, inner.width, 1);
                frame.render_widget(Paragraph::new(hint), hint_rect);
            }

            let mut render_col = |lines: Vec<Line<'static>>, col_rect: Rect| {
                let mut visible: Vec<Line<'static>> = lines.into_iter().skip(offset).take(view_h).collect();
                if offset > 0 {
                    if visible.len() == view_h {
                        visible.insert(0, Line::from(Span::styled(
                            format!("  ↑ {offset} lines above"),
                            Style::default().fg(theme.fg_muted),
                        )));
                        visible.truncate(view_h);
                    }
                }
                frame.render_widget(Paragraph::new(visible), col_rect);
            };

            render_col(left_lines, cols[0]);
            render_col(right_lines, cols[1]);
        } else {
            // Single-column layout.
            let all_lines: Vec<Line<'static>> = self.cards.iter()
                .flat_map(|c| build_card_lines(c, label_width, theme))
                .collect();
            let total = all_lines.len();
            let offset = self.scroll_offset.min(total.saturating_sub(view_h));
            let mut visible: Vec<Line<'static>> = all_lines.into_iter().skip(offset).take(view_h).collect();

            let overflow_below = total.saturating_sub(offset + view_h);
            if overflow_below > 0 {
                visible.push(Line::from(Span::styled(
                    format!("  ↓ {overflow_below} more lines — press [u] for short view"),
                    Style::default().fg(theme.fg_muted),
                )));
            }
            frame.render_widget(Paragraph::new(visible), inner);
        }
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

        match event {
            AppEvent::Key(KeyEvent { code: KeyCode::Char('u'), modifiers, .. })
                if modifiers.is_empty() || *modifiers == KeyModifiers::SHIFT =>
            {
                self.mode = self.mode.cycle();
                self.scroll_offset = 0;
                Some(Msg::Noop)
            }
            AppEvent::Key(KeyEvent { code: KeyCode::Char('j'), modifiers, .. })
                if modifiers.is_empty() && self.mode == QuotaMode::Long =>
            {
                self.scroll_offset += 1;
                Some(Msg::Noop)
            }
            AppEvent::Key(KeyEvent { code: KeyCode::Char('k'), modifiers, .. })
                if modifiers.is_empty() && self.mode == QuotaMode::Long =>
            {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                Some(Msg::Noop)
            }
            AppEvent::Mouse(MouseEvent {
                kind: kind @ (MouseEventKind::ScrollDown | MouseEventKind::ScrollUp),
                column,
                row,
                ..
            }) => {
                if let Some(area) = self.last_area {
                    if self.mode == QuotaMode::Long
                        && *row >= area.y
                        && *row < area.y + area.height
                        && *column >= area.x
                        && *column < area.x + area.width
                    {
                        if *kind == MouseEventKind::ScrollDown {
                            self.scroll_offset += 1;
                        } else {
                            self.scroll_offset = self.scroll_offset.saturating_sub(1);
                        }
                        return Some(Msg::Noop);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_short_long_hidden_short() {
        assert_eq!(QuotaMode::Short.cycle(), QuotaMode::Long);
        assert_eq!(QuotaMode::Long.cycle(), QuotaMode::Hidden);
        assert_eq!(QuotaMode::Hidden.cycle(), QuotaMode::Short);
    }

    #[test]
    fn u_key_cycles_mode() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut panel = QuotaPanel::default();
        assert_eq!(panel.mode, QuotaMode::Short);
        let ev = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        panel.handle_event(&ev);
        assert_eq!(panel.mode, QuotaMode::Long);
    }

    #[test]
    fn scroll_offset_advances_on_scroll_down_within_area() {
        use crossterm::event::{MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;
        use ratatui::layout::Rect;

        let mut panel = QuotaPanel::default();
        panel.mode = QuotaMode::Long;
        panel.last_area = Some(Rect::new(0, 10, 80, 10));
        // Add enough cards to overflow.
        panel.cards = (0..4).map(|i| QuotaCardModel {
            client_kind: agtop_core::session::ClientKind::Claude,
            client_label: format!("Provider {i}"),
            closest: WindowModel { label: "5h".into(), used_pct: 0.5, note: None, reset_in: None },
            all_windows: vec![
                WindowModel { label: "5h".into(), used_pct: 0.5, note: None, reset_in: None },
                WindowModel { label: "1d".into(), used_pct: 0.3, note: None, reset_in: None },
            ],
        }).collect();

        let scroll_down = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 12, // within last_area (y=10, height=10)
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(panel.scroll_offset, 0);
        panel.handle_event(&scroll_down);
        assert_eq!(panel.scroll_offset, 1);
    }

    #[test]
    fn scroll_up_does_not_go_below_zero() {
        use crossterm::event::{MouseEvent, MouseEventKind, KeyModifiers};
        use crate::tui::input::AppEvent;
        use ratatui::layout::Rect;

        let mut panel = QuotaPanel::default();
        panel.mode = QuotaMode::Long;
        panel.last_area = Some(Rect::new(0, 0, 80, 10));
        assert_eq!(panel.scroll_offset, 0);

        let scroll_up = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        panel.handle_event(&scroll_up);
        assert_eq!(panel.scroll_offset, 0, "scroll_offset must not go below 0");
    }

    #[test]
    fn mode_cycle_resets_scroll_offset() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use crate::tui::input::AppEvent;

        let mut panel = QuotaPanel::default();
        panel.mode = QuotaMode::Long;
        panel.scroll_offset = 5;

        let u_key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        panel.handle_event(&u_key);
        assert_eq!(panel.scroll_offset, 0, "cycling mode must reset scroll offset");
    }
}
