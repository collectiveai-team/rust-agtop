//! 3-row dashboard header: row 1 = procs/CPU sparkline, row 2 = mem bar +
//! aggregate session counts, row 3 = Sessions section divider.
// Foundation code for Plan 2.
#![allow(dead_code)]

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::theme_v2::Theme;
use crate::tui::widgets::{gradient_bar, sparkline_braille};

/// Aggregated input the header needs each frame.
#[derive(Debug, Default, Clone)]
pub struct HeaderModel {
    pub procs: usize,
    pub cpu_history: Vec<f32>, // recent CPU% samples; last is current
    pub cpu_max: f32,           // typically 100.0 or n_cores * 100
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub sessions_active: usize,
    pub sessions_idle: usize,
    pub sessions_today: usize,
    pub refresh_secs: u64,
    pub clock: String, // pre-formatted "HH:MM:SS"
}

pub fn render(frame: &mut Frame<'_>, area: Rect, model: &HeaderModel, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_row1(frame, layout[0], model, theme);
    render_row2(frame, layout[1], model, theme);
    render_row3(frame, layout[2], theme);
}

fn render_row1(frame: &mut Frame<'_>, area: Rect, m: &HeaderModel, theme: &Theme) {
    // Layout: "Procs N   CPU <sparkline> NN%       <pad>   ⟳ Ns · HH:MM:SS"
    let cpu_now = m.cpu_history.last().copied().unwrap_or(0.0);
    let spark = sparkline_braille::render_braille(&m.cpu_history, 20, m.cpu_max.max(1.0));
    let left = format!(" Procs {procs}   CPU  ", procs = m.procs);
    let cpu_pct = format!("  {pct:>3.0}%", pct = cpu_now);
    let right = format!("⟳ {s}s · {clk} ", s = m.refresh_secs, clk = m.clock);

    let total_chars = left.chars().count() + spark.chars().count() + cpu_pct.chars().count() + right.chars().count();
    let pad = (area.width as usize).saturating_sub(total_chars);

    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(theme.fg_default)),
        Span::styled(spark, Style::default().fg(theme.accent_primary)),
        Span::styled(cpu_pct, Style::default().fg(theme.fg_default)),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, Style::default().fg(theme.fg_muted)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_row2(frame: &mut Frame<'_>, area: Rect, m: &HeaderModel, theme: &Theme) {
    let pct = if m.mem_total_bytes > 0 {
        m.mem_used_bytes as f32 / m.mem_total_bytes as f32
    } else {
        0.0
    };
    let (filled, color, empty) = gradient_bar::render_bar(pct, 20, theme);
    let mem_label = format!(
        " {used:.1}G/{total:.0}G ",
        used = m.mem_used_bytes as f32 / 1_073_741_824.0,
        total = m.mem_total_bytes as f32 / 1_073_741_824.0,
    );
    let sessions = format!(
        " Sessions: {a} active · {i} idle · {t} today ",
        a = m.sessions_active,
        i = m.sessions_idle,
        t = m.sessions_today,
    );

    let prefix = " Mem ";
    let used_chars = prefix.chars().count() + filled.chars().count() + empty.chars().count() + mem_label.chars().count() + sessions.chars().count();
    let pad = (area.width as usize).saturating_sub(used_chars);

    let line = Line::from(vec![
        Span::styled(prefix, Style::default().fg(theme.fg_default)),
        Span::styled(filled, Style::default().fg(color)),
        Span::styled(empty, Style::default().fg(theme.border_muted)),
        Span::styled(mem_label, Style::default().fg(theme.fg_muted)),
        Span::raw(" ".repeat(pad)),
        Span::styled(sessions, Style::default().fg(theme.fg_default)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_row3(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    // Section divider with embedded "Sessions" label.
    let mut s = String::with_capacity(area.width as usize);
    s.push(' ');
    s.push_str("Sessions ");
    let dashes = (area.width as usize).saturating_sub(s.chars().count());
    s.push_str(&"─".repeat(dashes));
    let line = Line::from(Span::styled(
        s,
        Style::default()
            .fg(theme.border_muted)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn renders_without_panic_with_empty_model() {
        let backend = TestBackend::new(120, 3);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let model = HeaderModel::default();
        term.draw(|f| render(f, Rect::new(0, 0, 120, 3), &model, &theme))
            .unwrap();
    }

    #[test]
    fn renders_without_panic_with_realistic_model() {
        let backend = TestBackend::new(140, 3);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let model = HeaderModel {
            procs: 12,
            cpu_history: vec![10.0, 20.0, 35.0, 45.0, 55.0, 40.0, 30.0, 25.0],
            cpu_max: 100.0,
            mem_used_bytes: 12 * 1_073_741_824,
            mem_total_bytes: 16 * 1_073_741_824,
            sessions_active: 8,
            sessions_idle: 3,
            sessions_today: 47,
            refresh_secs: 2,
            clock: "14:25:49".to_string(),
        };
        term.draw(|f| render(f, Rect::new(0, 0, 140, 3), &model, &theme))
            .unwrap();
    }
}
