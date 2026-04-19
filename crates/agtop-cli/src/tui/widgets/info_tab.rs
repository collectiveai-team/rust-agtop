//! "Info" tab: static metadata for the selected session.

use chrono::{DateTime, Local, Utc};
use ratatui::{
    layout::Alignment,
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::fmt;
use crate::tui::app::App;
use crate::tui::column_config::ColumnId;
use crate::tui::theme as th;
use crate::tui::widgets::state_display::display_state;
use agtop_core::session::SessionAnalysis;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Info ");

    let body = match app.selected() {
        None => {
            let msg = "(no session selected — run an agent to populate this list)";
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(th::EMPTY_HINT)
                .block(block)
        }
        Some((_, a)) => {
            let now = Utc::now();
            let mut lines: Vec<Line<'static>> = ColumnId::all()
                .iter()
                .map(|&col| column_line(col, a, now))
                .collect();

            lines.push(kv_line(
                "effective_model",
                a.effective_model.clone().unwrap_or_else(|| "-".into()),
            ));
            lines.push(kv_line(
                "data_path",
                a.summary.data_path.display().to_string(),
            ));
            lines.push(kv_line(
                "subagents",
                if a.subagent_file_count == 0 {
                    "-".to_string()
                } else {
                    format!("{} file(s)", a.subagent_file_count)
                },
            ));
            lines.push(kv_line(
                "billing",
                if a.cost.included {
                    "included (plan covers this)".to_string()
                } else {
                    "retail".to_string()
                },
            ));

            if let Some(detail) = &a.summary.state_detail {
                lines.push(kv_line("state_detail", detail.clone()));
            }
            if let Some(detail) = &a.summary.model_effort_detail {
                lines.push(kv_line("effort_detail", detail.clone()));
            }

            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block)
        }
    };

    frame.render_widget(body, area);
}

fn column_line(col: ColumnId, a: &SessionAnalysis, now: DateTime<Utc>) -> Line<'static> {
    let s = &a.summary;
    let t = &a.tokens;
    let c = &a.cost;
    let cache_total = t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input;

    match col {
        ColumnId::Provider => kv_line("agentic_provider", s.provider.as_str().to_string()),
        ColumnId::Subscription => kv_line(
            "subscription",
            s.subscription.clone().unwrap_or_else(|| "-".into()),
        ),
        ColumnId::Session => kv_line("session_id", fmt::short_id(&s.session_id)),
        ColumnId::Started => kv_line("started", fmt_dt(s.started_at)),
        ColumnId::Age => kv_line(
            "age",
            s.last_active
                .map(|ts| fmt::relative_age(ts, now))
                .unwrap_or_else(|| "-".into()),
        ),
        ColumnId::Model => kv_line("model", s.model.clone().unwrap_or_else(|| "?".into())),
        ColumnId::Duration => {
            let val = a
                .duration_secs
                .map(format_duration_secs)
                .or_else(|| {
                    s.started_at.zip(s.last_active).and_then(|(start, end)| {
                        if end >= start {
                            Some(format_duration_secs((end - start).num_seconds() as u64))
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| "-".to_string());
            kv_line("duration", val)
        }
        ColumnId::LastActive => kv_line("last_active", fmt_dt(s.last_active)),
        ColumnId::State => {
            let (label, style) = display_state(a, now);
            kv_line_styled("state", label.to_string(), style)
        }
        ColumnId::Effort => kv_line(
            "effort",
            s.model_effort.clone().unwrap_or_else(|| "-".into()),
        ),
        ColumnId::Tokens => kv_line(
            "total_tokens",
            compact_tokens(t.input + t.output + cache_total),
        ),
        ColumnId::OutputTokens => kv_line("out_tokens", compact_tokens(t.output)),
        ColumnId::CacheTokens => kv_line("cache_tokens", compact_tokens(cache_total)),
        ColumnId::Cost => kv_line(
            "cost",
            if c.included {
                "included".to_string()
            } else {
                format!("${:.4}", c.total)
            },
        ),
        ColumnId::ToolCalls => kv_line(
            "tool_calls",
            a.tool_call_count
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        ColumnId::Context => kv_line(
            "context_used",
            match (a.context_used_tokens, a.context_window, a.context_used_pct) {
                (Some(n), Some(max), Some(pct)) => {
                    format!(
                        "{}/{} ({:.1}%)",
                        compact_tokens(n),
                        compact_tokens(max),
                        pct
                    )
                }
                (_, _, Some(pct)) => format!("{pct:.1}%"),
                _ => "-".to_string(),
            },
        ),
        ColumnId::AgentTurns => kv_line(
            "agent_turns",
            a.agent_turns
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        ColumnId::UserTurns => kv_line(
            "user_turns",
            a.user_turns
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        ColumnId::Project => kv_line(
            "project",
            a.project_name.clone().unwrap_or_else(|| "-".into()),
        ),
        ColumnId::Cwd => kv_line("cwd", s.cwd.clone().unwrap_or_else(|| "-".into())),
    }
}

fn kv_line(key: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:>16}"), th::INFO_KEY),
        Span::raw("  "),
        Span::raw(value),
    ])
}

fn kv_line_styled(key: &'static str, value: String, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:>16}"), th::INFO_KEY),
        Span::raw("  "),
        Span::styled(value, style),
    ])
}

fn fmt_dt(dt: Option<DateTime<Utc>>) -> String {
    match dt {
        Some(t) => t
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S %Z")
            .to_string(),
        None => "-".into(),
    }
}

fn compact_tokens(n: u64) -> String {
    fmt::compact(n)
}

fn format_duration_secs(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}
