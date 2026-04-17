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
            let lines = vec![
                kv_line("agentic_provider", s.provider.as_str().to_string()),
                kv_line(
                    "subscription",
                    s.subscription.clone().unwrap_or_else(|| "-".into()),
                ),
                kv_line("session_id", s.session_id.clone()),
                kv_line("started", fmt_dt(s.started_at)),
                kv_line("last_active", fmt_dt(s.last_active)),
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
                    "subagents",
                    if a.subagent_file_count == 0 {
                        "-".to_string()
                    } else {
                        format!("{} file(s)", a.subagent_file_count)
                    },
                ),
                kv_line(
                    "billing",
                    if a.cost.included {
                        "included (plan covers this)".to_string()
                    } else {
                        "retail".to_string()
                    },
                ),
            ];
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
