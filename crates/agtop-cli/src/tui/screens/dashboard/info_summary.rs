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

use super::info_format::{human_tokens, money_summary, truncate_to};
use crate::tui::theme_v2::{client_palette, Theme};
use crate::tui::widgets::{icon::Icon, state_style};

#[derive(Debug, Clone)]
pub struct MessageTurn {
    pub role: Role,
    pub preview: String,
    pub tools: Vec<String>,
    pub current_tool: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    Tool,
}

impl From<&agtop_core::session::SessionMessageTurn> for MessageTurn {
    fn from(t: &agtop_core::session::SessionMessageTurn) -> Self {
        MessageTurn {
            role: match t.role {
                agtop_core::session::SessionMessageRole::User => Role::User,
                agtop_core::session::SessionMessageRole::Agent => Role::Agent,
                agtop_core::session::SessionMessageRole::Tool => Role::Tool,
            },
            preview: t.preview.clone(),
            tools: t.tools.clone(),
            current_tool: t.current_tool,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SummaryModel<'a> {
    pub analysis: &'a SessionAnalysis,
    pub client_label: &'a str,
    pub client_kind: ClientKind,
    pub state: &'a SessionState,
    pub recent_turns: Vec<MessageTurn>,
    pub message_scroll_from_bottom: usize,
    pub activity_samples: Vec<f32>,
    pub parent_session_id: Option<&'a str>,
    pub subagent_count: usize,
    pub nerd_font: bool,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &SummaryModel<'_>, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(2),
            Constraint::Min(6),
        ])
        .split(area);
    render_hero(frame, layout[0], m, theme);
    render_status(frame, layout[1], m, theme);
    render_activity(frame, layout[2], m, theme);
    render_messages(
        frame,
        layout[3],
        &m.recent_turns,
        m.message_scroll_from_bottom,
        theme,
    );
}

fn render_hero(frame: &mut Frame<'_>, area: Rect, m: &SummaryModel<'_>, theme: &Theme) {
    let title_color = client_palette::color_for(m.client_kind);
    let project_basename = m
        .analysis
        .summary
        .cwd
        .as_deref()
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string())
        })
        .unwrap_or_default();

    let folder = Icon::Folder.render(m.nerd_font);
    let clock = Icon::Clock.render(m.nerd_font);
    let cwd = m.analysis.summary.cwd.as_deref().unwrap_or("");

    let started_str = m
        .analysis
        .summary
        .started_at
        .map(|t| {
            let d = chrono::Utc::now() - t;
            if d.num_seconds() < 60 {
                "just now".to_string()
            } else if d.num_minutes() < 60 {
                format!("{}m ago", d.num_minutes())
            } else if d.num_hours() < 24 {
                format!("{}h ago", d.num_hours())
            } else {
                format!("{}d ago", d.num_days())
            }
        })
        .unwrap_or_else(|| "—".into());

    let state_label = state_style::label_for(m.state);
    let state_color_style = state_color_style(m.state, theme);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                project_basename,
                Style::default()
                    .fg(title_color)
                    .add_modifier(Modifier::BOLD),
            ),
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
            pill(
                m.analysis.summary.subscription.as_deref().unwrap_or(""),
                theme.accent_secondary,
                theme,
            ),
            Span::raw(" "),
            pill(
                m.analysis.summary.model.as_deref().unwrap_or(""),
                theme.syntax_keyword,
                theme,
            ),
        ]),
    ];
    while lines.len() < 4 {
        lines.push(Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn pill(text: &str, fg: ratatui::style::Color, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(fg)
            .bg(theme.bg_overlay)
            .add_modifier(Modifier::BOLD),
    )
}

fn render_status(frame: &mut Frame<'_>, area: Rect, m: &SummaryModel<'_>, theme: &Theme) {
    let state_label = state_style::label_for(m.state);
    let total = m.analysis.tokens.grand_total();
    let cost = money_summary(m.analysis.cost.total);
    let tool_count = m
        .analysis
        .tool_call_count
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".into());
    let agent_turns = m
        .analysis
        .agent_turns
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".into());
    let session_id = truncate_to(&m.analysis.summary.session_id, 20);
    let relation = if m.subagent_count > 0 {
        format!("Subagents {}", m.subagent_count)
    } else if m.parent_session_id.is_some() {
        "Parent → child".to_string()
    } else {
        "Standalone".to_string()
    };
    let action = m.analysis.current_action.as_deref().unwrap_or("");
    let lines = vec![
        Line::from(vec![
            Span::styled(" Status ", Style::default().fg(theme.fg_muted)),
            Span::styled(
                "─".repeat(area.width.saturating_sub(8) as usize),
                Style::default().fg(theme.border_muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("State    ", Style::default().fg(theme.fg_muted)),
            Span::styled("● ", state_color_style(m.state, theme)),
            Span::styled(
                state_label.to_string(),
                Style::default().fg(theme.fg_default),
            ),
            Span::raw("    "),
            Span::styled("Tool ", Style::default().fg(theme.fg_muted)),
            Span::styled(
                action.to_string(),
                Style::default().fg(theme.fg_default),
            ),
        ]),
        Line::from(vec![
            Span::styled("Session  ", Style::default().fg(theme.fg_muted)),
            Span::styled(session_id, Style::default().fg(theme.fg_default)),
            Span::raw("    "),
            Span::styled("Relation ", Style::default().fg(theme.fg_muted)),
            Span::styled(relation, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled("Usage    ", Style::default().fg(theme.fg_muted)),
            Span::styled(
                format!("{} tokens", human_tokens(total)),
                Style::default().fg(theme.fg_default),
            ),
            Span::raw("  "),
            Span::styled(cost, Style::default().fg(theme.fg_default)),
            Span::raw("  "),
            Span::styled(
                format!("{} tools", tool_count),
                Style::default().fg(theme.fg_default),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} agent turns", agent_turns),
                Style::default().fg(theme.fg_default),
            ),
        ]),
        Line::from(""),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_activity(frame: &mut Frame<'_>, area: Rect, m: &SummaryModel<'_>, theme: &Theme) {
    let width = area.width.saturating_sub(12).max(1) as usize;
    let plot = if m.activity_samples.is_empty() {
        "no activity samples".to_string()
    } else {
        crate::tui::widgets::sparkline_braille::render_braille(
            &m.activity_samples,
            width,
            100.0,
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Activity   ", Style::default().fg(theme.fg_muted)),
            Span::styled(plot, Style::default().fg(theme.accent_primary)),
        ])),
        area,
    );
}

fn render_messages(
    frame: &mut Frame<'_>,
    area: Rect,
    turns: &[MessageTurn],
    scroll_from_bottom: usize,
    theme: &Theme,
) {
    if turns.is_empty() {
        let line = Line::from(Span::styled(
            "Recent messages not available for this client.",
            Style::default().fg(theme.fg_muted),
        ));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let mut message_lines: Vec<Line<'static>> = Vec::new();
    for t in turns {
        let role_label = match t.role {
            Role::User => Span::styled("  user │ ", Style::default().fg(theme.accent_primary)),
            Role::Agent => Span::styled(" agent │ ", Style::default().fg(theme.status_success)),
            Role::Tool => Span::styled("  tool │ ", Style::default().fg(theme.fg_muted)),
        };
        message_lines.push(Line::from(vec![
            role_label,
            Span::styled(t.preview.clone(), Style::default().fg(theme.fg_default)),
        ]));
        for tool in &t.tools {
            let marker = if t.current_tool { "▸ " } else { "  " };
            message_lines.push(Line::from(vec![
                Span::raw("        "),
                Span::styled(
                    format!("{marker}[tool] "),
                    Style::default().fg(theme.fg_muted),
                ),
                Span::styled(tool.clone(), Style::default().fg(theme.syntax_string)),
            ]));
        }
    }

    let body_height = area.height.saturating_sub(1) as usize;
    let total = message_lines.len();
    let max_start = total.saturating_sub(body_height);
    let start = max_start.saturating_sub(scroll_from_bottom).min(max_start);
    let mut lines = vec![header_line(area.width, theme)];
    lines.extend(message_lines.into_iter().skip(start).take(body_height));
    if start > 0 && !lines.is_empty() {
        lines[0] = Line::from(vec![
            Span::styled(" Recent messages ", Style::default().fg(theme.fg_muted)),
            Span::styled("↑ older messages", Style::default().fg(theme.fg_muted)),
        ]);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn header_line(width: u16, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(" Recent messages ", Style::default().fg(theme.fg_muted)),
        Span::styled(
            "─".repeat(width.saturating_sub(18) as usize),
            Style::default().fg(theme.border_muted),
        ),
    ])
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

#[cfg(test)]
mod summary_tests {
    use super::*;
    use agtop_core::session::{
        ClientKind, CostBreakdown, SessionAnalysis, SessionState, SessionSummary, TokenTotals,
    };
    use ratatui::{backend::TestBackend, Terminal};

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut lines = Vec::new();
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }

    fn analysis() -> SessionAnalysis {
        let summary = SessionSummary::new(
            ClientKind::OpenCode,
            Some("Claude Max 5x".into()),
            "ses_summary".into(),
            None,
            None,
            Some("claude-sonnet-4-6".into()),
            Some("/tmp/rust-agtop".into()),
            std::path::PathBuf::new(),
            None,
            None,
            None,
        );
        let mut tokens = TokenTotals::default();
        tokens.input = 2_000_000;
        tokens.output = 300_000;
        tokens.cache_read = 2_867_742;
        let mut cost = CostBreakdown::default();
        cost.total = 1.93;
        let mut a = SessionAnalysis::new(
            summary,
            tokens,
            cost,
            None,
            0,
            Some(41),
            None,
            None,
            None,
            None,
        );
        a.current_action = Some("bash: cargo test".into());
        a.agent_turns = Some(23);
        a
    }

    #[test]
    fn summary_renders_live_usage_activity_and_latest_message_at_bottom() {
        let theme = crate::tui::theme_v2::vscode_dark_plus::theme();
        let turns = vec![
            MessageTurn {
                role: Role::User,
                preview: "first".into(),
                tools: vec![],
                current_tool: false,
            },
            MessageTurn {
                role: Role::Agent,
                preview: "middle".into(),
                tools: vec![],
                current_tool: false,
            },
            MessageTurn {
                role: Role::Tool,
                preview: "latest".into(),
                tools: vec![],
                current_tool: false,
            },
        ];
        let analysis = analysis();
        let state = SessionState::Running;
        let model = SummaryModel {
            analysis: &analysis,
            client_label: "opencode",
            client_kind: ClientKind::OpenCode,
            state: &state,
            recent_turns: turns,
            message_scroll_from_bottom: 0,
            activity_samples: vec![1.0, 2.0, 3.0, 4.0],
            parent_session_id: None,
            subagent_count: 2,
            nerd_font: false,
        };
        let mut term = Terminal::new(TestBackend::new(100, 22)).unwrap();
        term.draw(|f| render(f, f.area(), &model, &theme)).unwrap();
        let out = buffer_text(term.backend().buffer());
        assert!(out.contains("Tool bash: cargo test"));
        assert!(out.contains("Subagents 2"));
        assert!(out.contains("5.17M tokens"));
        assert!(out.contains("Activity"));
        assert!(out.contains("latest"));
    }
}
