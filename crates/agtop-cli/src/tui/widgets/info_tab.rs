//! "Info" tab: static metadata for the selected session.

use chrono::{DateTime, Local, Utc};
use ratatui::{
    layout::Alignment,
    prelude::*,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Info ");

    let body = match app.selected() {
        None => {
            let msg = "(no session selected — run an agent to populate this list)";
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().add_modifier(Modifier::DIM))
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
                kv_line("provider", s.provider.as_str().to_string()),
                kv_line("session_id", s.session_id.clone()),
                kv_line("started", fmt_dt(s.started_at)),
                kv_line("last_active", fmt_dt(s.last_active)),
                kv_line("model", s.model.clone().unwrap_or_else(|| "?".into())),
                kv_line(
                    "effective_model",
                    a.effective_model.clone().unwrap_or_else(|| "-".into()),
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
        Span::styled(
            format!("{key:>16}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(value),
    ])
}
