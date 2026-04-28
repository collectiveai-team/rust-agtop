//! Usage Quota panel with three modes: Short / Long / Hidden.
// Foundation code for Plan 2.
#![allow(dead_code)]

use std::collections::HashMap;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use agtop_core::quota::{ProviderId, ProviderResult};
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
            // Long mode: 15 rows fits a typical 4-window provider card
            // (header + 4 windows + spacer = 6 lines) plus chrome and
            // overflow hint, with room for at least two providers on a
            // 2-column 80+ wide terminal.
            Self::Long => 15,
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
    /// Lifecycle status used to drive the "stale"/"error" decorations in
    /// the rendered card. Defaults to `Ok` for fresh successful fetches.
    pub status: CardStatus,
}

/// Per-card lifecycle status. Drives the long-mode banner that distinguishes
/// fresh / stale-with-cached-data / outright-failed providers.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum CardStatus {
    /// Fresh, successful fetch. No banner.
    #[default]
    Ok,
    /// The most recent fetch failed but a previous successful fetch is
    /// being shown instead. `last_ok_ms` is the epoch-ms of that
    /// successful fetch; `current_error` is the new error to surface.
    Stale {
        last_ok_ms: i64,
        current_error: String,
    },
    /// No cached data is available — the provider has only ever errored
    /// since the panel was created. The string is the error to render.
    Error(String),
}

#[derive(Debug, Clone)]
pub struct WindowModel {
    pub label: String, // e.g. "5h", "weekly"
    pub used_pct: f32, // 0..=1
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
    /// Most-recent successful card per provider, keyed by `ProviderId`.
    /// Stored as `(fetched_at_ms, card)` so we can re-render a stale-but-
    /// useful view when a subsequent fetch fails. Survives across calls
    /// to `apply_results` so transient errors don't blow away the panel.
    pub last_good: HashMap<ProviderId, (i64, QuotaCardModel)>,
}

fn client_kind_for(id: ProviderId) -> ClientKind {
    use agtop_core::logo::provider_id_to_client_kind;
    provider_id_to_client_kind(id).unwrap_or(ClientKind::Claude)
}

/// Build a fresh QuotaCardModel from a successful ProviderResult.
/// Returns None only if the result has no usage payload (which should
/// never happen for `ok` results but we don't want to panic).
fn build_card_from_ok(r: &ProviderResult, now_ms: i64) -> Option<QuotaCardModel> {
    let usage = r.usage.as_ref()?;
    let all_windows: Vec<WindowModel> = usage
        .windows
        .iter()
        .map(|(label, w)| WindowModel {
            label: label.clone(),
            used_pct: w.used_percent.map(|p| (p / 100.0) as f32).unwrap_or(0.0),
            note: w.value_label.clone(),
            reset_in: w.reset_at.and_then(|ms| {
                let diff_secs = (ms - now_ms) / 1000;
                if diff_secs <= 0 {
                    return None;
                }
                let h = diff_secs / 3600;
                let m = (diff_secs % 3600) / 60;
                Some(if h > 24 {
                    format!("resets in {}d {}h", h / 24, h % 24)
                } else if h > 0 {
                    format!("resets in {h}h {m}m")
                } else {
                    format!("resets in {m}m")
                })
            }),
        })
        .collect();
    let closest = all_windows
        .iter()
        .max_by(|a, b| {
            a.used_pct
                .partial_cmp(&b.used_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
        .unwrap_or(WindowModel {
            label: "—".into(),
            used_pct: 0.0,
            note: None,
            reset_in: None,
        });
    Some(QuotaCardModel {
        client_kind: client_kind_for(r.provider_id),
        client_label: r.provider_name.to_string(),
        closest,
        all_windows,
        status: CardStatus::Ok,
    })
}

/// Format a millisecond-epoch timestamp as a relative "Nm ago" / "Nh ago"
/// string. Used for stale-card banners.
fn format_age_ms(ms: i64, now_ms: i64) -> String {
    let secs = ((now_ms - ms) / 1000).max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn build_card_lines<'a>(
    card: &'a QuotaCardModel,
    label_width: usize,
    theme: &'a Theme,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        card.client_label.clone(),
        Style::default()
            .fg(client_palette::color_for(card.client_kind))
            .add_modifier(Modifier::BOLD),
    )));

    // Status banner for non-Ok cards: shown immediately after the header
    // so it sits above the (possibly stale) usage bars.
    match &card.status {
        CardStatus::Ok => {}
        CardStatus::Stale {
            last_ok_ms,
            current_error,
        } => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let ago = format_age_ms(*last_ok_ms, now_ms);
            lines.push(Line::from(Span::styled(
                format!("  ⚠ stale: last fetched {ago} · {current_error}"),
                Style::default().fg(theme.status_warning),
            )));
        }
        CardStatus::Error(msg) => {
            lines.push(Line::from(Span::styled(
                format!("  ⚠ {msg}"),
                Style::default().fg(theme.status_error),
            )));
        }
    }

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
            spans.push(Span::styled(
                note.clone(),
                Style::default().fg(theme.fg_muted),
            ));
        }
        if let Some(reset) = &w.reset_in {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                reset.clone(),
                Style::default().fg(theme.fg_muted),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines
}

impl QuotaPanel {
    /// Title strings rendered in each mode's top border.  Centralized so
    /// click-hit-testing in `handle_event` and rendering stay in sync.
    const TITLE_SHORT: &'static str = " Usage Quota (short)  [u]sage ";
    const TITLE_LONG: &'static str = " Usage Quota (long)  [u]sage  [r]efresh ";

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.last_area = Some(area);
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
                Self::TITLE_SHORT,
                Style::default()
                    .fg(theme.fg_emphasis)
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(theme.border_muted));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Build a single-line set of segments separated by " · ".
        let mut spans: Vec<Span> = Vec::with_capacity(self.cards.len() * 8);
        for (i, card) in self.cards.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(
                    "  ·  ",
                    Style::default().fg(theme.border_muted),
                ));
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
                Self::TITLE_LONG,
                Style::default()
                    .fg(theme.fg_emphasis)
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(theme.border_muted));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let label_width = self
            .cards
            .iter()
            .flat_map(|c| c.all_windows.iter().map(|w| w.label.len()))
            .max()
            .unwrap_or(4)
            .max(4);

        let inner_h = inner.height as usize;
        // Reserve 1 row for overflow indicators if needed.
        let view_h = inner_h.saturating_sub(1).max(1);

        if area.width > 80 && self.cards.len() >= 2 {
            // 2-column layout.
            use ratatui::layout::{Constraint, Direction, Layout};
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            let mid = self.cards.len().div_ceil(2);
            let left_lines: Vec<Line<'static>> = self.cards[..mid]
                .iter()
                .flat_map(|c| build_card_lines(c, label_width, theme))
                .collect();
            let right_lines: Vec<Line<'static>> = self.cards[mid..]
                .iter()
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
                let mut visible: Vec<Line<'static>> =
                    lines.into_iter().skip(offset).take(view_h).collect();
                if offset > 0 && visible.len() == view_h {
                    visible.insert(
                        0,
                        Line::from(Span::styled(
                            format!("  ↑ {offset} lines above"),
                            Style::default().fg(theme.fg_muted),
                        )),
                    );
                    visible.truncate(view_h);
                }
                frame.render_widget(Paragraph::new(visible), col_rect);
            };

            render_col(left_lines, cols[0]);
            render_col(right_lines, cols[1]);
        } else {
            // Single-column layout.
            let all_lines: Vec<Line<'static>> = self
                .cards
                .iter()
                .flat_map(|c| build_card_lines(c, label_width, theme))
                .collect();
            let total = all_lines.len();
            let offset = self.scroll_offset.min(total.saturating_sub(view_h));
            let mut visible: Vec<Line<'static>> =
                all_lines.into_iter().skip(offset).take(view_h).collect();

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

    /// Convert fresh provider quota results into card models for rendering.
    ///
    /// Behavior per `ProviderResult`:
    ///   * Not configured        → omit (no card).
    ///   * Configured + ok       → fresh card; cached as last_good.
    ///   * Configured + errored  → if cached last_good present, render
    ///     that card with `CardStatus::Stale`; otherwise render an error
    ///     placeholder with `CardStatus::Error`.
    ///
    /// Cached last_good entries persist across calls so transient fetch
    /// failures don't blow the user-visible panel away.
    pub fn apply_results(&mut self, results: &[ProviderResult]) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut new_cards: Vec<QuotaCardModel> = Vec::with_capacity(results.len());

        for r in results {
            if !r.configured {
                continue;
            }
            if r.ok {
                if let Some(card) = build_card_from_ok(r, now_ms) {
                    // Update the last_good cache with this fresh card.
                    self.last_good
                        .insert(r.provider_id, (r.fetched_at, card.clone()));
                    new_cards.push(card);
                }
            } else {
                let err_text = r
                    .error
                    .as_ref()
                    .map(|e| e.detail.clone())
                    .unwrap_or_else(|| "unknown error".into());
                if let Some((last_ok_ms, cached)) = self.last_good.get(&r.provider_id) {
                    let mut stale = cached.clone();
                    stale.status = CardStatus::Stale {
                        last_ok_ms: *last_ok_ms,
                        current_error: err_text,
                    };
                    new_cards.push(stale);
                } else {
                    let client_kind = client_kind_for(r.provider_id);
                    new_cards.push(QuotaCardModel {
                        client_kind,
                        client_label: r.provider_name.to_string(),
                        closest: WindowModel {
                            label: "—".into(),
                            used_pct: 0.0,
                            note: None,
                            reset_in: None,
                        },
                        all_windows: Vec::new(),
                        status: CardStatus::Error(err_text),
                    });
                }
            }
        }

        self.cards = new_cards;
    }

    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };

        match event {
            AppEvent::Key(KeyEvent {
                code: KeyCode::Char('u'),
                modifiers,
                ..
            }) if modifiers.is_empty() || *modifiers == KeyModifiers::SHIFT => {
                self.mode = self.mode.cycle();
                self.scroll_offset = 0;
                Some(Msg::Noop)
            }
            AppEvent::Key(KeyEvent {
                code: KeyCode::Char('r'),
                modifiers,
                ..
            }) if modifiers.is_empty() && self.mode == QuotaMode::Long => Some(Msg::RefreshQuota),
            AppEvent::Key(KeyEvent {
                code: KeyCode::Char('j'),
                modifiers,
                ..
            }) if modifiers.is_empty() && self.mode == QuotaMode::Long => {
                self.scroll_offset += 1;
                Some(Msg::Noop)
            }
            AppEvent::Key(KeyEvent {
                code: KeyCode::Char('k'),
                modifiers,
                ..
            }) if modifiers.is_empty() && self.mode == QuotaMode::Long => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                Some(Msg::Noop)
            }
            AppEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                if let Some(area) = self.last_area {
                    if *row == area.y {
                        let title = match self.mode {
                            QuotaMode::Short => Self::TITLE_SHORT,
                            QuotaMode::Long => Self::TITLE_LONG,
                            QuotaMode::Hidden => return None,
                        };
                        let title_start = area.x + 1; // +1 for left border
                        if *column >= title_start {
                            let rel = (*column - title_start) as usize;
                            // [u]sage button: cycle mode.
                            if let Some(u_pos) = title.find("[u]") {
                                let u_end = u_pos + "[u]sage".len();
                                if rel >= u_pos && rel < u_end {
                                    self.mode = self.mode.cycle();
                                    self.scroll_offset = 0;
                                    return Some(Msg::Noop);
                                }
                            }
                            // [r]efresh button: only present in Long mode.
                            if self.mode == QuotaMode::Long {
                                if let Some(r_pos) = title.find("[r]") {
                                    let r_end = r_pos + "[r]efresh".len();
                                    if rel >= r_pos && rel < r_end {
                                        return Some(Msg::RefreshQuota);
                                    }
                                }
                            }
                        }
                    }
                }
                None
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

    fn ok_result_with_window(
        id: agtop_core::quota::ProviderId,
        name: &'static str,
        used: f64,
    ) -> agtop_core::quota::ProviderResult {
        use agtop_core::quota::{Usage, UsageWindow};
        use indexmap::IndexMap;

        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        windows.insert(
            "5h".to_string(),
            UsageWindow {
                used_percent: Some(used),
                window_seconds: None,
                reset_at: None,
                value_label: None,
            },
        );
        let usage = Usage {
            windows,
            ..Usage::default()
        };
        agtop_core::quota::ProviderResult::ok(id, name, usage, std::collections::BTreeMap::new())
    }

    #[test]
    fn reset_in_uses_days_and_hours_only_after_24_hours() {
        use agtop_core::quota::{ProviderId, ProviderResult, Usage, UsageWindow};
        use indexmap::IndexMap;

        let now_ms = 1_700_000_000_000;
        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        windows.insert(
            "daily".to_string(),
            UsageWindow {
                used_percent: Some(10.0),
                window_seconds: None,
                reset_at: Some(now_ms + 24 * 60 * 60 * 1000),
                value_label: None,
            },
        );
        windows.insert(
            "weekly".to_string(),
            UsageWindow {
                used_percent: Some(20.0),
                window_seconds: None,
                reset_at: Some(now_ms + 49 * 60 * 60 * 1000),
                value_label: None,
            },
        );
        let result = ProviderResult::ok(
            ProviderId::Claude,
            "Claude",
            Usage {
                windows,
                ..Usage::default()
            },
            std::collections::BTreeMap::new(),
        );

        let card = build_card_from_ok(&result, now_ms).expect("card");

        assert_eq!(
            card.all_windows[0].reset_in.as_deref(),
            Some("resets in 24h 0m")
        );
        assert_eq!(
            card.all_windows[1].reset_in.as_deref(),
            Some("resets in 2d 1h")
        );
    }

    #[test]
    fn errored_provider_with_cache_renders_stale_marker() {
        // Sequence: a successful fetch followed by an errored fetch for the
        // same provider must keep the card visible and mark it as stale.
        use agtop_core::quota::{ErrorKind, ProviderId, ProviderResult, QuotaError};

        let mut panel = QuotaPanel::default();

        // 1) ok result populates the card and the last_good cache.
        panel.apply_results(&[ok_result_with_window(ProviderId::Claude, "Claude", 42.0)]);
        assert_eq!(panel.cards.len(), 1, "ok result must produce one card");
        assert!(matches!(panel.cards[0].status, CardStatus::Ok));

        // 2) Subsequent err result for the same provider — card stays
        // visible, status flips to Stale with a snapshot of when the
        // last-good fetch happened and the current error message.
        let err_result = ProviderResult::err(
            ProviderId::Claude,
            "Claude",
            QuotaError {
                kind: ErrorKind::Transport,
                detail: "connection refused".into(),
            },
        );
        panel.apply_results(&[err_result]);
        assert_eq!(
            panel.cards.len(),
            1,
            "errored provider with cached last-good must remain visible"
        );
        match &panel.cards[0].status {
            CardStatus::Stale { current_error, .. } => {
                assert!(
                    current_error.contains("connection refused"),
                    "stale status must surface the current error message; got: {current_error:?}"
                );
            }
            other => panic!("expected Stale status, got {other:?}"),
        }
    }

    #[test]
    fn errored_provider_without_cache_renders_error_placeholder() {
        use agtop_core::quota::{ErrorKind, ProviderId, ProviderResult, QuotaError};
        let mut panel = QuotaPanel::default();
        let err = ProviderResult::err(
            ProviderId::Codex,
            "Codex",
            QuotaError {
                kind: ErrorKind::Http {
                    status: 401,
                    retry_after: None,
                },
                detail: "401 Unauthorized".into(),
            },
        );
        panel.apply_results(&[err]);
        assert_eq!(
            panel.cards.len(),
            1,
            "errored provider with no cached last-good must still render a placeholder card"
        );
        match &panel.cards[0].status {
            CardStatus::Error(msg) => {
                assert!(
                    msg.contains("401 Unauthorized"),
                    "Error status must surface error detail; got: {msg:?}"
                );
            }
            other => panic!("expected Error status, got {other:?}"),
        }
        assert_eq!(panel.cards[0].client_label, "Codex");
    }

    #[test]
    fn unconfigured_provider_is_omitted() {
        use agtop_core::quota::{ProviderId, ProviderResult};
        let mut panel = QuotaPanel::default();
        let r = ProviderResult::not_configured(ProviderId::Google, "Google");
        panel.apply_results(&[r]);
        assert!(
            panel.cards.is_empty(),
            "not-configured providers must not render any card"
        );
    }

    #[test]
    fn r_key_in_long_mode_emits_refresh_quota_msg() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut panel = QuotaPanel {
            mode: QuotaMode::Long,
            last_area: Some(Rect::new(0, 0, 80, 15)),
            ..Default::default()
        };
        let ev = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let result = panel.handle_event(&ev);
        assert_eq!(
            result,
            Some(Msg::RefreshQuota),
            "[r] in Long mode must emit Msg::RefreshQuota"
        );
    }

    #[test]
    fn r_key_inert_in_short_mode() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut panel = QuotaPanel::default();
        // Short mode (default): [r]efresh button is not exposed.
        let ev = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let result = panel.handle_event(&ev);
        assert_eq!(result, None, "[r] must be inert outside Long mode");
    }

    #[test]
    fn click_on_r_button_in_long_mode_emits_refresh() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut panel = QuotaPanel {
            mode: QuotaMode::Long,
            last_area: Some(Rect::new(0, 0, 80, 15)),
            ..Default::default()
        };
        // Title in Long mode: " Usage Quota (long)  [u]sage  [r]efresh "
        let title_long = " Usage Quota (long)  [u]sage  [r]efresh ";
        let r_pos = title_long.find("[r]").expect("title must contain [r]");
        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1 + r_pos as u16, // border (+1) + offset
            row: 0,                   // == area.y
            modifiers: KeyModifiers::NONE,
        });
        let result = panel.handle_event(&click);
        assert_eq!(result, Some(Msg::RefreshQuota));
    }

    #[test]
    fn long_mode_min_height_is_at_least_15_rows() {
        // Increase the Long-mode panel height so multi-window providers
        // (Codex / Claude with weekly + 5h + opus etc.) have enough space
        // before scrolling kicks in. Bumped from 12 → 15 (+3 rows).
        assert!(
            QuotaMode::Long.rows_needed() >= 15,
            "Long mode must allocate at least 15 rows; got {}",
            QuotaMode::Long.rows_needed()
        );
    }

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
        use crate::tui::input::AppEvent;
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        use ratatui::layout::Rect;

        let mut panel = QuotaPanel {
            mode: QuotaMode::Long,
            last_area: Some(Rect::new(0, 10, 80, 10)),
            // Add enough cards to overflow.
            cards: (0..4)
                .map(|i| QuotaCardModel {
                    client_kind: agtop_core::session::ClientKind::Claude,
                    client_label: format!("Provider {i}"),
                    closest: WindowModel {
                        label: "5h".into(),
                        used_pct: 0.5,
                        note: None,
                        reset_in: None,
                    },
                    all_windows: vec![
                        WindowModel {
                            label: "5h".into(),
                            used_pct: 0.5,
                            note: None,
                            reset_in: None,
                        },
                        WindowModel {
                            label: "1d".into(),
                            used_pct: 0.3,
                            note: None,
                            reset_in: None,
                        },
                    ],
                    status: CardStatus::Ok,
                })
                .collect(),
            ..Default::default()
        };

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
        use crate::tui::input::AppEvent;
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        use ratatui::layout::Rect;

        let mut panel = QuotaPanel {
            mode: QuotaMode::Long,
            last_area: Some(Rect::new(0, 0, 80, 10)),
            ..Default::default()
        };
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
        use crate::tui::input::AppEvent;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        let mut panel = QuotaPanel {
            mode: QuotaMode::Long,
            scroll_offset: 5,
            ..Default::default()
        };

        let u_key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('u'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        panel.handle_event(&u_key);
        assert_eq!(
            panel.scroll_offset, 0,
            "cycling mode must reset scroll offset"
        );
    }

    #[test]
    fn mouse_click_on_u_title_button_cycles_mode() {
        use crate::tui::input::AppEvent;
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Rect;

        // Title for Short: " Usage Quota (short)  [u]sage "
        // area.x = 0, title_start = 0 + 1 = 1 (left border)
        // "[u]" starts at index 22 in the title string
        // So column for "[u]" = title_start + u_pos = 1 + 22 = 23
        let title_short = " Usage Quota (short)  [u]sage ";
        let u_pos = title_short.find("[u]").unwrap();

        let mut panel = QuotaPanel::default();
        assert_eq!(panel.mode, QuotaMode::Short);
        panel.last_area = Some(Rect::new(0, 5, 80, 10)); // area.y = 5

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1 + u_pos as u16, // title_start + u_pos
            row: 5,                   // == area.y
            modifiers: KeyModifiers::NONE,
        });
        let result = panel.handle_event(&click);
        assert_eq!(
            result,
            Some(Msg::Noop),
            "click on [u] should return Msg::Noop"
        );
        assert_eq!(
            panel.mode,
            QuotaMode::Long,
            "mode should cycle from Short to Long"
        );
        assert_eq!(
            panel.scroll_offset, 0,
            "scroll offset should reset on cycle"
        );
    }

    #[test]
    fn mouse_click_outside_u_title_button_does_nothing() {
        use crate::tui::input::AppEvent;
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Rect;

        let mut panel = QuotaPanel {
            last_area: Some(Rect::new(0, 5, 80, 10)),
            ..Default::default()
        };

        // Click on column 0 (before title_start=1) on the title row — should not cycle
        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        let result = panel.handle_event(&click);
        assert_eq!(result, None, "click outside [u] should return None");
        assert_eq!(panel.mode, QuotaMode::Short, "mode should not change");
    }
}
