//! "Process" tab: live OS resource metrics for the selected session.

use ratatui::{
    layout::Alignment,
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::fmt;
use crate::tui::app::App;
use crate::tui::theme as th;
use agtop_core::process::{Confidence, Liveness};

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Process ");

    let body = match app.selected() {
        None => Paragraph::new("(no session selected)")
            .alignment(Alignment::Center)
            .style(th::EMPTY_HINT)
            .block(block),

        Some((_, a)) => match a.pid {
            None => Paragraph::new("(no live process for this session)")
                .alignment(Alignment::Center)
                .style(th::EMPTY_HINT)
                .block(block),

            Some(pid) => {
                let liveness = match a.liveness {
                    Some(Liveness::Live) => "live",
                    Some(Liveness::Stopped) => "stopped",
                    None => "-",
                };
                let pid_val = format!("{pid} ({liveness})");

                let match_label = match a.match_confidence {
                    Some(Confidence::High) => "fd",
                    Some(Confidence::Medium) => "cwd+argv",
                    None => "-",
                };

                let (cpu, mem, vsz, disk_r, disk_w) = if let Some(ref m) = a.process_metrics {
                    (
                        fmt::format_percent(Some(m.cpu_percent)),
                        fmt::compact_opt(Some(m.memory_bytes)),
                        fmt::compact_opt(Some(m.virtual_memory_bytes)),
                        fmt::compact_opt(Some(m.disk_read_bytes)),
                        fmt::compact_opt(Some(m.disk_written_bytes)),
                    )
                } else {
                    (
                        "-".into(),
                        "-".into(),
                        "-".into(),
                        "-".into(),
                        "-".into(),
                    )
                };

                let lines = vec![
                    kv_line("pid", pid_val),
                    kv_line("match", match_label.to_string()),
                    kv_line("cpu", cpu),
                    kv_line("memory", mem),
                    kv_line("virtual_memory", vsz),
                    kv_line("disk_read", disk_r),
                    kv_line("disk_written", disk_w),
                ];

                let outer_block = block;
                let inner = outer_block.inner(area);
                frame.render_widget(outer_block, area);
                frame.render_widget(
                    Paragraph::new(lines).wrap(Wrap { trim: false }),
                    inner,
                );
                return;
            }
        },
    };

    frame.render_widget(body, area);
}

fn kv_line(key: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:>16}"), th::INFO_KEY),
        Span::raw("  "),
        Span::raw(value),
    ])
}
