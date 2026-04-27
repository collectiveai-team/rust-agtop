//! Appearance section: theme, true color, mouse capture, animations,
//! version badge, header density, status colors, client colors.

use ratatui::{layout::Rect, style::{Modifier, Style}, text::{Line, Span}, widgets::Paragraph, Frame};

use agtop_core::session::ClientKind;

use crate::tui::msg::HeaderDensity;
use crate::tui::screens::config::controls;
use crate::tui::theme_v2::{client_palette, Theme};

#[derive(Debug, Clone)]
pub struct AppearanceModel {
    pub theme_name: String,
    pub true_color_label: String,        // "auto" / "on" / "off"
    pub mouse_capture: bool,
    pub version_badge: bool,
    pub animations: bool,
    pub nerd_font: bool,
    pub header_density: HeaderDensity,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &AppearanceModel, theme: &Theme) {
    let setting = |label: &'static str, control: Vec<Span<'static>>| -> Line<'static> {
        let mut spans = vec![Span::styled(format!("  {label:<22}"), Style::default().fg(theme.fg_default))];
        spans.extend(control);
        Line::from(spans)
    };
    let title = |t: &'static str| -> Line<'static> {
        Line::from(Span::styled(t, Style::default().fg(theme.fg_emphasis).add_modifier(Modifier::BOLD)))
    };

    let lines: Vec<Line> = vec![
        title("Appearance"),
        Line::from(Span::styled("─".repeat(40), Style::default().fg(theme.border_muted))),
        Line::from(""),
        setting("Theme", vec![controls::dropdown(&m.theme_name, theme)]),
        setting("True color", vec![controls::dropdown(&m.true_color_label, theme), Span::styled("   auto / on / off", Style::default().fg(theme.fg_muted))]),
        setting("Mouse capture", vec![controls::checkbox(m.mouse_capture, theme), Span::styled("   (Shift+click for text selection)", Style::default().fg(theme.fg_muted))]),
        setting("Show version badge", vec![controls::checkbox(m.version_badge, theme)]),
        setting("Animations (waiting pulse)", vec![controls::checkbox(m.animations, theme)]),
        setting("Nerd Font icons", vec![controls::checkbox(m.nerd_font, theme), Span::styled("   (requires Nerd Font in terminal)", Style::default().fg(theme.fg_muted))]),
        setting("Header density", vec![
            controls::radio(m.header_density == HeaderDensity::Compact, theme),
            Span::styled(" compact  ", Style::default().fg(theme.fg_default)),
            controls::radio(m.header_density == HeaderDensity::Normal, theme),
            Span::styled(" normal  ", Style::default().fg(theme.fg_default)),
            controls::radio(m.header_density == HeaderDensity::Detailed, theme),
            Span::styled(" detailed", Style::default().fg(theme.fg_default)),
        ]),
        Line::from(""),
        title("Client colors"),
        Line::from(Span::styled("─".repeat(40), Style::default().fg(theme.border_muted))),
    ];

    let mut all = lines;
    for (kind, label) in [
        (ClientKind::Claude, "claude-code"),
        (ClientKind::Codex, "codex"),
        (ClientKind::GeminiCli, "gemini-cli"),
        (ClientKind::Copilot, "copilot"),
        (ClientKind::Cursor, "cursor"),
        (ClientKind::Antigravity, "antigravity"),
        (ClientKind::OpenCode, "opencode"),
    ] {
        let color = client_palette::color_for(kind);
        let rgb = match color {
            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
            _ => (0x88, 0x88, 0x88),
        };
        let mut spans = vec![Span::styled(format!("  {label:<22}"), Style::default().fg(theme.fg_default))];
        spans.extend(controls::swatch(rgb, theme));
        all.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(all), area);
}

impl Default for AppearanceModel {
    fn default() -> Self {
        Self {
            theme_name: "vscode-dark+".into(),
            true_color_label: "auto".into(),
            mouse_capture: true,
            version_badge: true,
            animations: true,
            nerd_font: false,
            header_density: HeaderDensity::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme_v2::vscode_dark_plus;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn renders_without_panic() {
        let backend = TestBackend::new(80, 30);
        let mut term = Terminal::new(backend).unwrap();
        let theme = vscode_dark_plus::theme();
        let m = AppearanceModel::default();
        term.draw(|f| render(f, Rect::new(0, 0, 80, 30), &m, &theme)).unwrap();
    }
}
