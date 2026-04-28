//! Summary tab content for the info drawer.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use agtop_core::session::{ClientKind, SessionAnalysis, SessionState};

use crate::tui::theme_v2::{client_palette, Theme};
use crate::tui::widgets::{icon::Icon, state_style};

/// Latest message turn for the recent-messages block.
#[derive(Debug, Clone)]
pub struct MessageTurn {
    pub role: Role,
    pub preview: String,       // ~one-liner; truncated by caller
    pub tools: Vec<String>,    // inline tool calls within this turn
    pub current_tool: bool,    // is one of these tools currently in flight?
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { User, Agent, Tool }

/// Input model for the Summary tab.
#[derive(Debug, Clone)]
pub struct SummaryModel<'a> {
    pub analysis: &'a SessionAnalysis,
    pub client_label: &'a str,
    pub client_kind: ClientKind,
    pub state: &'a SessionState,
    pub recent_turns: Vec<MessageTurn>,
    pub nerd_font: bool,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &SummaryModel<'_>, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // hero block
            Constraint::Length(5), // status block
            Constraint::Min(0),    // messages
        ])
        .split(area);
    render_hero(frame, layout[0], m, theme);
    render_status(frame, layout[1], m, theme);
    render_messages(frame, layout[2], &m.recent_turns, theme);
}

fn render_hero(frame: &mut Frame<'_>, area: Rect, m: &SummaryModel<'_>, theme: &Theme) {
    let title_color = client_palette::color_for(m.client_kind);
    let project_basename = m.analysis.summary.cwd
        .as_deref()
        .map(|p| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string()))
        .unwrap_or_default();

    let folder = Icon::Folder.render(m.nerd_font);
    let clock = Icon::Clock.render(m.nerd_font);
    let cwd = m.analysis.summary.cwd.as_deref().unwrap_or("");

    let started_str = m.analysis.summary.started_at.map(|t| {
        let d = chrono::Utc::now() - t;
        if d.num_seconds() < 60 { "just now".to_string() }
        else if d.num_minutes() < 60 { format!("{}m ago", d.num_minutes()) }
        else if d.num_hours() < 24 { format!("{}h ago", d.num_hours()) }
        else { format!("{}d ago", d.num_days()) }
    }).unwrap_or_else(|| "—".into());

    let state_label = state_style::label_for(m.state);
    let state_color_style = state_color_style(m.state, theme);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(project_basename, Style::default().fg(title_color).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(state_label, state_color_style),
        ]),
        Line::from(vec![
            Span::styled(format!("{folder} "), Style::default().fg(theme.fg_muted)),
            Span::styled(cwd.to_string(), Style::default().fg(theme.syntax_string)),
        ]),
        Line::from(vec![
            Span::styled(format!("{clock} "), Style::default().fg(theme.fg_muted)),
            Span::styled(started_str, Style::default().fg(theme.fg_muted)),
        ]),
        Line::from(vec![
            pill(m.client_label, title_color, theme),
            Span::raw(" "),
            pill(m.analysis.summary.subscription.as_deref().unwrap_or(""), theme.accent_secondary, theme),
            Span::raw(" "),
            pill(m.analysis.summary.model.as_deref().unwrap_or(""), theme.syntax_keyword, theme),
        ]),
    ];
    // Pad to 5 lines.
    while lines.len() < 5 { lines.push(Line::from("")); }
    frame.render_widget(Paragraph::new(lines), area);
}

fn pill(text: &str, fg: ratatui::style::Color, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default().fg(fg).bg(theme.bg_overlay).add_modifier(Modifier::BOLD),
    )
}

fn render_status(frame: &mut Frame<'_>, area: Rect, m: &SummaryModel<'_>, theme: &Theme) {
    let state_label = state_style::label_for(m.state);
    let lines = vec![
        Line::from(vec![
            Span::styled(" Status ", Style::default().fg(theme.fg_muted)),
            Span::styled("─".repeat(area.width.saturating_sub(8) as usize), Style::default().fg(theme.border_muted)),
        ]),
        Line::from(vec![
            Span::styled("State    ", Style::default().fg(theme.fg_muted)),
            Span::styled("● ", state_color_style(m.state, theme)),
            Span::styled(state_label.to_string(), Style::default().fg(theme.fg_default)),
            Span::raw("  "),
            Span::styled(
                m.analysis.current_action.as_deref().unwrap_or(""),
                Style::default().fg(theme.fg_muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("Session  ", Style::default().fg(theme.fg_muted)),
            Span::styled(m.analysis.summary.session_id.as_str().to_string(), Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled("Tokens   ", Style::default().fg(theme.fg_muted)),
            Span::styled(
                format!("{}", m.analysis.tokens.grand_total()),
                Style::default().fg(theme.fg_default),
            ),
            Span::raw("    "),
            Span::styled("Cost ", Style::default().fg(theme.fg_muted)),
            Span::styled(
                if m.analysis.cost.total > 0.0 {
                    format!("${:.2}", m.analysis.cost.total)
                } else {
                    "—".into()
                },
                Style::default().fg(theme.fg_default),
            ),
        ]),
        Line::from(""),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_messages(frame: &mut Frame<'_>, area: Rect, turns: &[MessageTurn], theme: &Theme) {
    if turns.is_empty() {
        let line = Line::from(Span::styled(
            "Recent messages not available for this client.",
            Style::default().fg(theme.fg_muted),
        ));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(turns.len() * 2);
    lines.push(Line::from(vec![
        Span::styled(" Recent messages ", Style::default().fg(theme.fg_muted)),
        Span::styled("─".repeat(area.width.saturating_sub(18) as usize), Style::default().fg(theme.border_muted)),
    ]));
    for t in turns {
        let role_label = match t.role {
            Role::User  => Span::styled("  user │ ", Style::default().fg(theme.accent_primary)),
            Role::Agent => Span::styled(" agent │ ", Style::default().fg(theme.status_success)),
            Role::Tool  => Span::styled("  tool │ ", Style::default().fg(theme.fg_muted)),
        };
        lines.push(Line::from(vec![role_label, Span::styled(t.preview.clone(), Style::default().fg(theme.fg_default))]));
        for tool in &t.tools {
            let marker = if t.current_tool { "▸ " } else { "  " };
            lines.push(Line::from(vec![
                Span::raw("        "),
                Span::styled(format!("{marker}[tool] "), Style::default().fg(theme.fg_muted)),
                Span::styled(tool.clone(), Style::default().fg(theme.syntax_string)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn state_color_style(state: &SessionState, theme: &Theme) -> Style {
    use crate::tui::widgets::state_style::dot_color;
    let color = dot_color(state, theme).unwrap_or(theme.fg_muted);
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;

    #[test]
    fn running_and_idle_have_different_state_styles() {
        let t = vscode_dark_plus::theme();
        assert_ne!(
            state_color_style(&SessionState::Running, &t),
            state_color_style(&SessionState::Idle, &t),
        );
    }
}
