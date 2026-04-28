//! Top-of-screen pickers: Group by + Range + Sort/Reverse.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use agtop_core::aggregate::{GroupBy, TimeRange};

use crate::tui::input::AppEvent;
use crate::tui::msg::Msg;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone)]
pub struct ControlsModel {
    pub group_by: GroupBy,
    pub range: TimeRange,
    pub sort_label: &'static str,
    pub reverse: bool,
    /// (label, rect) for each rendered chip.  Populated by `render()`.
    pub chip_rects: Vec<(String, Rect)>,
}

impl Default for ControlsModel {
    fn default() -> Self {
        Self {
            group_by: GroupBy::Client,
            range: TimeRange::Today,
            sort_label: "COST",
            reverse: false,
            chip_rects: Vec::new(),
        }
    }
}

impl ControlsModel {
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        self.chip_rects.clear();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);
        self.render_row1(frame, layout[0], theme);
        self.render_row2(frame, layout[1], theme);
    }

    fn render_row1(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let prefix = " Group by:  ";
        let mut spans = vec![Span::styled(prefix, Style::default().fg(theme.fg_muted))];
        let mut x = area.x + prefix.chars().count() as u16;

        for g in [
            GroupBy::Client,
            GroupBy::Provider,
            GroupBy::Model,
            GroupBy::Project,
            GroupBy::Subscription,
        ] {
            let label = match g {
                GroupBy::Client => "Client",
                GroupBy::Provider => "Provider",
                GroupBy::Model => "Model",
                GroupBy::Project => "Project",
                GroupBy::Subscription => "Subscription",
            };
            let chip_str = if g == self.group_by {
                format!("‹ {label} › ")
            } else {
                format!("  {label}   ")
            };
            let chip_w = chip_str.chars().count() as u16;
            self.chip_rects
                .push((label.to_string(), Rect::new(x, area.y, chip_w, 1)));
            x += chip_w;

            let style = if g == self.group_by {
                Style::default()
                    .fg(theme.accent_primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_muted)
            };
            spans.push(Span::styled(chip_str, style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_row2(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let prefix = " Range:     ";
        let mut spans = vec![Span::styled(prefix, Style::default().fg(theme.fg_muted))];
        let mut x = area.x + prefix.chars().count() as u16;

        for r in [
            TimeRange::Today,
            TimeRange::Week,
            TimeRange::Month,
            TimeRange::All,
        ] {
            let label = match r {
                TimeRange::Today => "Today",
                TimeRange::Week => "Week",
                TimeRange::Month => "Month",
                TimeRange::All => "All",
            };
            let chip_str = if r == self.range {
                format!("‹ {label} › ")
            } else {
                format!("  {label}   ")
            };
            let chip_w = chip_str.chars().count() as u16;
            self.chip_rects
                .push((label.to_string(), Rect::new(x, area.y, chip_w, 1)));
            x += chip_w;

            let style = if r == self.range {
                Style::default()
                    .fg(theme.accent_primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_muted)
            };
            spans.push(Span::styled(chip_str, style));
        }

        // Sort chip
        let sort_sep = "  |  Sort: ";
        spans.push(Span::styled(sort_sep, Style::default().fg(theme.fg_muted)));
        x += sort_sep.chars().count() as u16;
        let sort_str = format!("‹{}›", self.sort_label);
        let sort_w = sort_str.chars().count() as u16;
        self.chip_rects
            .push(("__sort__".to_string(), Rect::new(x, area.y, sort_w, 1)));
        x += sort_w;
        spans.push(Span::styled(
            sort_str,
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ));

        // Reverse toggle
        let rev_sep = "  Reverse: ";
        spans.push(Span::styled(rev_sep, Style::default().fg(theme.fg_muted)));
        x += rev_sep.chars().count() as u16;
        let rev_str = if self.reverse { "on" } else { "off" };
        let rev_w = rev_str.chars().count() as u16;
        self.chip_rects
            .push(("__reverse__".to_string(), Rect::new(x, area.y, rev_w, 1)));
        spans.push(Span::styled(
            rev_str,
            Style::default().fg(if self.reverse {
                theme.accent_primary
            } else {
                theme.fg_muted
            }),
        ));

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Handle an event. Returns `Some(Msg::Noop)` if state changed, `None` otherwise.
    /// **Does not call `recompute()`** — callers (e.g. `AggregationState`) must do that.
    pub fn handle_event(&mut self, event: &AppEvent) -> Option<Msg> {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        if let AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            ..
        }) = event
        {
            for (label, rect) in &self.chip_rects {
                if *row == rect.y && *column >= rect.x && *column < rect.x + rect.width {
                    match label.as_str() {
                        "Client" => self.group_by = GroupBy::Client,
                        "Provider" => self.group_by = GroupBy::Provider,
                        "Model" => self.group_by = GroupBy::Model,
                        "Project" => self.group_by = GroupBy::Project,
                        "Subscription" => self.group_by = GroupBy::Subscription,
                        "Today" => self.range = TimeRange::Today,
                        "Week" => self.range = TimeRange::Week,
                        "Month" => self.range = TimeRange::Month,
                        "All" => self.range = TimeRange::All,
                        "__sort__" => { /* cycle sort — handled by caller */ }
                        "__reverse__" => self.reverse = !self.reverse,
                        _ => {}
                    }
                    return Some(Msg::Noop);
                }
            }
        }
        None
    }
}

// Keep free function for compile compatibility during migration.
pub fn render(frame: &mut Frame<'_>, area: Rect, m: &ControlsModel, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Static render (no rect recording) — used only by old callers.
    let mut spans = vec![Span::styled(
        " Group by:  ",
        Style::default().fg(theme.fg_muted),
    )];
    for g in [
        GroupBy::Client,
        GroupBy::Provider,
        GroupBy::Model,
        GroupBy::Project,
        GroupBy::Subscription,
    ] {
        let label = match g {
            GroupBy::Client => "Client",
            GroupBy::Provider => "Provider",
            GroupBy::Model => "Model",
            GroupBy::Project => "Project",
            GroupBy::Subscription => "Subscription",
        };
        let style = if g == m.group_by {
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_muted)
        };
        let s = if g == m.group_by {
            format!("‹ {label} › ")
        } else {
            format!("  {label}   ")
        };
        spans.push(Span::styled(s, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), layout[0]);

    let mut spans2 = vec![Span::styled(
        " Range:     ",
        Style::default().fg(theme.fg_muted),
    )];
    for r in [
        TimeRange::Today,
        TimeRange::Week,
        TimeRange::Month,
        TimeRange::All,
    ] {
        let label = match r {
            TimeRange::Today => "Today",
            TimeRange::Week => "Week",
            TimeRange::Month => "Month",
            TimeRange::All => "All",
        };
        let style = if r == m.range {
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_muted)
        };
        let s = if r == m.range {
            format!("‹ {label} › ")
        } else {
            format!("  {label}   ")
        };
        spans2.push(Span::styled(s, style));
    }
    spans2.push(Span::styled(
        "  |  Sort: ",
        Style::default().fg(theme.fg_muted),
    ));
    spans2.push(Span::styled(
        format!("‹{}›", m.sort_label),
        Style::default()
            .fg(theme.accent_primary)
            .add_modifier(Modifier::BOLD),
    ));
    spans2.push(Span::styled(
        "  Reverse: ",
        Style::default().fg(theme.fg_muted),
    ));
    spans2.push(Span::styled(
        if m.reverse { "on" } else { "off" },
        Style::default().fg(if m.reverse {
            theme.accent_primary
        } else {
            theme.fg_muted
        }),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans2)), layout[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn renders_without_panic() {
        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut m = ControlsModel::default();
        term.draw(|f| m.render(f, Rect::new(0, 0, 140, 2), &theme))
            .unwrap();
    }

    #[test]
    fn click_on_provider_chip_sets_group_by_provider() {
        use crate::tui::input::AppEvent;
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut m = ControlsModel::default();
        term.draw(|f| m.render(f, Rect::new(0, 0, 140, 2), &theme))
            .unwrap();

        let provider_rect = m
            .chip_rects
            .iter()
            .find(|(label, _)| label == "Provider")
            .map(|(_, r)| *r)
            .expect("Provider chip rect must exist after render");

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: provider_rect.x + provider_rect.width / 2,
            row: provider_rect.y,
            modifiers: KeyModifiers::NONE,
        });
        m.handle_event(&click);
        assert!(matches!(
            m.group_by,
            agtop_core::aggregate::GroupBy::Provider
        ));
    }

    #[test]
    fn click_on_reverse_toggle_flips_it() {
        use crate::tui::input::AppEvent;
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let backend = TestBackend::new(140, 2);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let mut m = ControlsModel::default();
        assert!(!m.reverse);
        term.draw(|f| m.render(f, Rect::new(0, 0, 140, 2), &theme))
            .unwrap();

        let rev_rect = m
            .chip_rects
            .iter()
            .find(|(label, _)| label == "__reverse__")
            .map(|(_, r)| *r)
            .expect("__reverse__ chip rect must exist after render");

        let click = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rev_rect.x,
            row: rev_rect.y,
            modifiers: KeyModifiers::NONE,
        });
        m.handle_event(&click);
        assert!(m.reverse, "reverse should flip to true");
        m.handle_event(&click);
        assert!(!m.reverse, "reverse should flip back to false");
    }
}
