#![allow(dead_code, unused)]
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

/// Render the Process tab into `area`, showing live OS metrics for the selected session.
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

                let cpu = fmt::format_percent(a.process_metrics.as_ref().map(|m| m.cpu_percent));
                let mem = fmt::compact_opt(a.process_metrics.as_ref().map(|m| m.memory_bytes));
                let vsz =
                    fmt::compact_opt(a.process_metrics.as_ref().map(|m| m.virtual_memory_bytes));
                let disk_r =
                    fmt::compact_opt(a.process_metrics.as_ref().map(|m| m.disk_read_bytes));
                let disk_w =
                    fmt::compact_opt(a.process_metrics.as_ref().map(|m| m.disk_written_bytes));
                let disk_r_rate = fmt::compact_rate_opt(
                    a.process_metrics
                        .as_ref()
                        .map(|m| m.disk_read_bytes_per_sec),
                );
                let disk_w_rate = fmt::compact_rate_opt(
                    a.process_metrics
                        .as_ref()
                        .map(|m| m.disk_written_bytes_per_sec),
                );

                let lines = vec![
                    super::kv_line("pid", pid_val),
                    super::kv_line("match", match_label.to_string()),
                    super::kv_line("cpu", cpu),
                    super::kv_line("memory", mem),
                    super::kv_line("virtual_memory", vsz),
                    super::kv_line("disk_read", disk_r),
                    super::kv_line("disk_written", disk_w),
                    super::kv_line("disk_read/s", disk_r_rate),
                    super::kv_line("disk_written/s", disk_w_rate),
                ];

                let inner = block.inner(area);
                frame.render_widget(block, area);
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
                return;
            }
        },
    };

    frame.render_widget(body, area);
}
