//! "Info" tab: static metadata for the selected session.

use chrono::{DateTime, Local, Utc};
use ratatui::{
    layout::Alignment,
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::App;
use crate::tui::theme as th;

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
            let s = &a.summary;

            // Check if the selected session is a child of any parent.
            let parent_id: Option<String> = app.sessions().iter().find_map(|parent| {
                if parent
                    .children
                    .iter()
                    .any(|c| c.summary.session_id == s.session_id)
                {
                    Some(parent.summary.session_id.clone())
                } else {
                    None
                }
            });

            let fmt_dt = |dt: Option<DateTime<Utc>>| match dt {
                Some(t) => t
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S %Z")
                    .to_string(),
                None => "-".into(),
            };
            // `kv_line` owns the value string so temporaries can't go
            // out of scope mid-render. Build everything as `String`
            // first, then hand it over.
            let mut lines: Vec<Line<'static>> = Vec::new();
            // If this session is a subagent, show the parent backlink first.
            if let Some(ref pid) = parent_id {
                lines.push(kv_line("parent", pid.clone()));
            }
            lines.extend([
                kv_line("agentic_provider", s.provider.as_str().to_string()),
                kv_line(
                    "subscription",
                    s.subscription.clone().unwrap_or_else(|| "-".into()),
                ),
                kv_line("session_id", s.session_id.clone()),
                kv_line("started", fmt_dt(s.started_at)),
                kv_line("last_active", fmt_dt(s.last_active)),
                kv_line("state", s.state.clone().unwrap_or_else(|| "-".into())),
                kv_line(
                    "effort",
                    s.model_effort.clone().unwrap_or_else(|| "-".into()),
                ),
                kv_line(
                    "duration",
                    a.duration_secs
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
                        .unwrap_or_else(|| "-".to_string()),
                ),
                kv_line("model", s.model.clone().unwrap_or_else(|| "?".into())),
                kv_line(
                    "effective_model",
                    a.effective_model.clone().unwrap_or_else(|| "-".into()),
                ),
                kv_line(
                    "tool_calls",
                    a.tool_call_count
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
                kv_line(
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
                kv_line("cwd", s.cwd.clone().unwrap_or_else(|| "-".into())),
                kv_line("data_path", s.data_path.display().to_string()),
                kv_line(
                    "billing",
                    if a.cost.included {
                        "included (plan covers this)".to_string()
                    } else {
                        "retail".to_string()
                    },
                ),
                kv_line("total_tokens", {
                    let cache_total = a.tokens.cache_read
                        + a.tokens.cache_write_5m
                        + a.tokens.cache_write_1h
                        + a.tokens.cached_input;
                    compact_tokens(a.tokens.input + a.tokens.output + cache_total)
                }),
                kv_line("out_tokens", compact_tokens(a.tokens.output)),
                kv_line(
                    "cache_tokens",
                    compact_tokens(
                        a.tokens.cache_read
                            + a.tokens.cache_write_5m
                            + a.tokens.cache_write_1h
                            + a.tokens.cached_input,
                    ),
                ),
                kv_line(
                    "cost",
                    if a.cost.included {
                        "included".to_string()
                    } else {
                        format!("${:.4}", a.cost.total)
                    },
                ),
            ]);
            if let Some(detail) = &s.state_detail {
                lines.push(kv_line("state_detail", detail.clone()));
            }
            if let Some(detail) = &s.model_effort_detail {
                lines.push(kv_line("effort_detail", detail.clone()));
            }
            // Subagents section: shown only when the selected session has children.
            if !a.children.is_empty() {
                let total_tokens: u64 = a
                    .children
                    .iter()
                    .map(|c| {
                        c.tokens.input
                            + c.tokens.output
                            + c.tokens.cache_read
                            + c.tokens.cache_write_5m
                            + c.tokens.cache_write_1h
                            + c.tokens.cached_input
                    })
                    .sum();
                let total_cost: f64 = a.children.iter().map(|c| c.cost.total).sum();
                lines.push(Line::from(vec![
                    Span::styled(format!("{:>16}", "subagents"), th::INFO_KEY),
                    Span::raw(format!(
                        "  {} session(s) | {} tokens | ${:.4}",
                        a.children.len(),
                        compact_tokens(total_tokens),
                        total_cost,
                    )),
                ]));
                for child in &a.children {
                    let cs = &child.summary;
                    let child_tokens = child.tokens.input
                        + child.tokens.output
                        + child.tokens.cache_read
                        + child.tokens.cache_write_5m
                        + child.tokens.cache_write_1h
                        + child.tokens.cached_input;
                    let short_id = crate::fmt::short_id(&cs.session_id);
                    lines.push(Line::from(vec![
                        Span::raw(format!("{:>18}", "")),
                        Span::styled(format!("{:<12}", short_id), th::INFO_KEY),
                        Span::raw(format!(
                            "  {} tok  ${:.4}  {}",
                            compact_tokens(child_tokens),
                            child.cost.total,
                            cs.state.as_deref().unwrap_or("-"),
                        )),
                    ]));
                }
            }
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(block)
        }
    };

    frame.render_widget(body, area);
}

/// Build a "key: value" line with a bold key. Takes owned strings so
/// the caller can synthesize values from `format!` or `Option::unwrap_or_else`
/// without lifetime issues.
fn kv_line(key: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:>16}"), th::INFO_KEY),
        Span::raw("  "),
        Span::raw(value),
    ])
}

fn compact_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
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
