//! Merged Details tab for the dashboard info drawer.

use ratatui::{layout::Rect, style::{Modifier, Style}, text::{Line, Span}, widgets::Paragraph, Frame};
use agtop_core::session::SessionAnalysis;

use crate::tui::theme_v2::Theme;
use super::info_format::{dash_if_empty, human_bytes, human_duration_secs, human_tokens, kv_line, money_details, truncate_to};

pub struct DetailsModel<'a> {
    pub analysis: &'a SessionAnalysis,
    pub parent_session_id: Option<&'a str>,
    pub subagent_count: usize,
    pub scroll_offset: usize,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, model: &DetailsModel<'_>, theme: &Theme) {
    let mut lines = build_lines(model, theme, area.width as usize);
    let visible = area.height as usize;
    let start = model.scroll_offset.min(lines.len().saturating_sub(visible));
    lines = lines.into_iter().skip(start).take(visible).collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn build_lines(model: &DetailsModel<'_>, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let a = model.analysis;
    let max_value = width.saturating_sub(18).max(8);
    let mut lines = Vec::new();
    section(&mut lines, "Identity", theme);
    lines.push(kv_line("Client", a.summary.client.as_str().to_string(), theme));
    lines.push(kv_line("Subscription", dash_if_empty(a.summary.subscription.as_deref()), theme));
    lines.push(kv_line("Model", dash_if_empty(a.summary.model.as_deref()), theme));
    lines.push(kv_line("Effort", dash_if_empty(a.summary.model_effort.as_deref()), theme));
    lines.push(kv_line("Project", truncate_to(a.summary.cwd.as_deref().unwrap_or("-"), max_value), theme));
    lines.push(kv_line("Session id", truncate_to(&a.summary.session_id, max_value), theme));
    lines.push(kv_line("Parent", model.parent_session_id.map(|s| truncate_to(s, max_value)).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Subagents", model.subagent_count.to_string(), theme));
    lines.push(Line::from(""));

    section(&mut lines, "Usage", theme);
    let cache_write = a.tokens.cache_write_5m + a.tokens.cache_write_1h;
    lines.push(kv_line("Total cost", money_details(a.cost.total), theme));
    lines.push(kv_line("Tokens", format!("{} total", human_tokens(a.tokens.grand_total())), theme));
    lines.push(kv_line("Input", human_tokens(a.tokens.input), theme));
    lines.push(kv_line("Output", human_tokens(a.tokens.output), theme));
    lines.push(kv_line("Reasoning", human_tokens(a.tokens.reasoning_output), theme));
    lines.push(kv_line("Cache read", human_tokens(a.tokens.cache_read), theme));
    lines.push(kv_line("Cache write", human_tokens(cache_write), theme));
    lines.push(kv_line("Tool calls", a.tool_call_count.map(|n| n.to_string()).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Turns", turns_value(a), theme));
    lines.push(kv_line("Context", context_value(a), theme));
    lines.push(Line::from(""));

    section(&mut lines, "Process", theme);
    let metrics = a.process_metrics.as_ref();
    lines.push(kv_line("PID", a.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Liveness", a.liveness.as_ref().map(|l| format!("{l:?}")).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Confidence", a.match_confidence.as_ref().map(|c| format!("{c:?}")).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("CPU", metrics.map(|m| format!("{:.1}%", m.cpu_percent)).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Memory", metrics.map(|m| format!("{} RSS", human_bytes(m.memory_bytes))).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Disk", metrics.map(|m| format!("{} read / {} written", human_bytes(m.disk_read_bytes), human_bytes(m.disk_written_bytes))).unwrap_or_else(|| "-".into()), theme));
    lines.push(Line::from(""));

    section(&mut lines, "Timing", theme);
    lines.push(kv_line("Started", a.summary.started_at.map(|t| t.to_rfc3339()).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Last active", a.summary.last_active.map(|t| t.to_rfc3339()).unwrap_or_else(|| "-".into()), theme));
    lines.push(kv_line("Duration", human_duration_secs(a.duration_secs), theme));
    lines
}

fn section(lines: &mut Vec<Line<'static>>, title: &'static str, theme: &Theme) {
    lines.push(Line::from(Span::styled(title, Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD))));
}

fn turns_value(a: &SessionAnalysis) -> String {
    match (a.agent_turns, a.user_turns) {
        (Some(agent), Some(user)) => format!("{agent} agent / {user} user"),
        (Some(agent), None) => format!("{agent} agent"),
        (None, Some(user)) => format!("{user} user"),
        (None, None) => "-".into(),
    }
}

fn context_value(a: &SessionAnalysis) -> String {
    match (a.context_used_pct, a.context_used_tokens, a.context_window) {
        (Some(pct), Some(used), Some(window)) => format!("{pct:.0}%  {} / {}", human_tokens(used), human_tokens(window)),
        _ => "-".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals};
    use ratatui::{backend::TestBackend, Terminal};

    fn text(buf: &ratatui::buffer::Buffer) -> String {
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
            "ses_22e0b6925ffevYv6CspUqdm73V".into(),
            None,
            None,
            Some("claude-sonnet-4-6".into()),
            Some("/home/rbarriga/collective.ai/projects/rust-agtop".into()),
            std::path::PathBuf::new(),
            None,
            None,
            None,
        );
        let mut tokens = TokenTotals::default();
        tokens.input = 1_200_000;
        tokens.output = 340_000;
        tokens.reasoning_output = 18_000;
        tokens.cache_read = 2_700_000;
        tokens.cache_write_5m = 910_000;
        let mut cost = CostBreakdown::default();
        cost.total = 1.928;
        let mut analysis = SessionAnalysis::new(
            summary,
            tokens,
            cost,
            None,
            0,
            Some(41),
            Some(317),
            Some(71.0),
            Some(142_000),
            Some(200_000),
        );
        analysis.agent_turns = Some(23);
        analysis.user_turns = Some(18);
        analysis
    }

    #[test]
    fn details_renders_identity_usage_process_and_timing() {
        let theme = crate::tui::theme_v2::vscode_dark_plus::theme();
        let mut term = Terminal::new(TestBackend::new(100, 50)).unwrap();
        let model = DetailsModel { analysis: &analysis(), parent_session_id: None, subagent_count: 2, scroll_offset: 0 };
        term.draw(|f| render(f, f.area(), &model, &theme)).unwrap();
        let out = text(term.backend().buffer());
        assert!(out.contains("Identity"));
        assert!(out.contains("Usage"));
        assert!(out.contains("Process"));
        assert!(out.contains("Timing"));
        assert!(out.contains("5.15M total"));
        assert!(out.contains("$1.9280"));
        assert!(out.contains("23 agent / 18 user"));
    }
}
