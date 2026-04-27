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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaMode {
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

impl Default for QuotaMode {
    fn default() -> Self { Self::Short }
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
}

#[derive(Debug, Default)]
pub struct QuotaPanel {
    pub mode: QuotaMode,
    pub cards: Vec<QuotaCardModel>,
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

        // Stack cards vertically. Each card = 1 header line + N window lines.
        let mut lines: Vec<Line> = Vec::new();
        for card in &self.cards {
            lines.push(Line::from(Span::styled(
                card.client_label.clone(),
                Style::default()
                    .fg(client_palette::color_for(card.client_kind))
                    .add_modifier(Modifier::BOLD),
            )));
            for w in &card.all_windows {
                let (filled, color, empty) = gradient_bar::render_bar(w.used_pct, 18, theme);
                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(format!("{:>10}", w.label), Style::default().fg(theme.fg_muted)),
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
                lines.push(Line::from(spans));
            }
            lines.push(Line::from(""));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let AppEvent::Key(KeyEvent { code, modifiers, .. }) = event else { return None };
        if !modifiers.is_empty() && *modifiers != KeyModifiers::SHIFT { return None; }
        if matches!(code, KeyCode::Char('u')) {
            self.mode = self.mode.cycle();
            return Some(Msg::Noop);
        }
        None
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
}
