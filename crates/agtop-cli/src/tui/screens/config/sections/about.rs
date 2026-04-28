//! About section: version, build, links, config path.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::theme_v2::Theme;

#[derive(Debug, Clone)]
pub struct AboutModel {
    pub version: &'static str,
    pub git_sha: &'static str,
    pub config_path: String,
}

pub fn render(frame: &mut Frame<'_>, area: Rect, m: &AboutModel, theme: &Theme) {
    let title = Line::from(Span::styled(
        "About",
        Style::default()
            .fg(theme.fg_emphasis)
            .add_modifier(Modifier::BOLD),
    ));
    let rule = Line::from(Span::styled(
        "─".repeat(40),
        Style::default().fg(theme.border_muted),
    ));
    let kv = |k: &'static str, v: String| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {k:<14}"), Style::default().fg(theme.fg_muted)),
            Span::styled(v, Style::default().fg(theme.fg_default)),
        ])
    };
    let lines = vec![
        title,
        rule,
        Line::from(""),
        kv("Version", m.version.into()),
        kv("Git SHA", m.git_sha.into()),
        kv("Config file", m.config_path.clone()),
        kv(
            "Repository",
            "https://github.com/collectiveai-team/rust-agtop".into(),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

impl Default for AboutModel {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            git_sha: option_env!("AGTOP_GIT_SHA").unwrap_or("dev"),
            config_path: dirs::config_dir()
                .map(|p| p.join("agtop/config.toml").display().to_string())
                .unwrap_or_default(),
        }
    }
}
