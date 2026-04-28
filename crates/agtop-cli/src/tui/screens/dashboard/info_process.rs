//! Process tab: PID tree, CPU/MEM history, parent process info.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use agtop_core::session::SessionAnalysis;
use crate::tui::theme_v2::Theme;
use crate::tui::widgets::sparkline_braille;

pub fn render(frame: &mut Frame<'_>, area: Rect, a: &SessionAnalysis, cpu_hist: &[f32], theme: &Theme) {
    let pid = a.pid.map(|p| p.to_string()).unwrap_or_else(|| "—".into());
    let cpu = a.process_metrics
        .as_ref()
        .map(|m| format!("{:.1}%", m.cpu_percent))
        .unwrap_or_else(|| "—".into());
    let resident = a.process_metrics
        .as_ref()
        .map(|m| format!("{:.1}M", m.memory_bytes as f32 / 1_048_576.0))
        .unwrap_or_else(|| "—".into());
    let disk_read_rate = crate::fmt::compact_rate_opt(
        a.process_metrics.as_ref().map(|m| m.disk_read_bytes_per_sec),
    );
    let disk_write_rate = crate::fmt::compact_rate_opt(
        a.process_metrics.as_ref().map(|m| m.disk_written_bytes_per_sec),
    );

    let spark = sparkline_braille::render_braille(cpu_hist, 16, 100.0);

    let lines = vec![
        Line::from(vec![
            Span::styled("  PID          ", Style::default().fg(theme.fg_muted)),
            Span::styled(pid, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled("  CPU %        ", Style::default().fg(theme.fg_muted)),
            Span::styled(cpu, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled("  Resident mem ", Style::default().fg(theme.fg_muted)),
            Span::styled(resident, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled("  Disk read/s  ", Style::default().fg(theme.fg_muted)),
            Span::styled(disk_read_rate, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled("  Disk write/s ", Style::default().fg(theme.fg_muted)),
            Span::styled(disk_write_rate, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  CPU history  ", Style::default().fg(theme.fg_muted)),
            Span::styled(spark, Style::default().fg(theme.accent_primary)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}
