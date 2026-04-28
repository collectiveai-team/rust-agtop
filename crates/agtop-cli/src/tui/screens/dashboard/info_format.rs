//! Shared formatting helpers for dashboard info drawer tabs.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::tui::theme_v2::Theme;

#[must_use]
pub fn human_tokens(value: u64) -> String {
    if value >= 1_000_000 {
        trim_decimal(value as f64 / 1_000_000.0, 2, "M")
    } else if value >= 1_000 {
        trim_decimal(value as f64 / 1_000.0, 1, "k")
    } else {
        value.to_string()
    }
}

#[must_use]
pub fn human_bytes(value: u64) -> String {
    const K: f64 = 1024.0;
    let f = value as f64;
    if f >= K * K * K {
        trim_decimal(f / (K * K * K), 1, "G")
    } else if f >= K * K {
        format!("{:.0}M", f / (K * K))
    } else if f >= K {
        trim_decimal(f / K, 1, "K")
    } else {
        format!("{value}B")
    }
}

#[must_use]
pub fn human_duration_secs(value: Option<u64>) -> String {
    let Some(secs) = value else { return "-".to_string() };
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[must_use]
pub fn money_summary(value: f64) -> String {
    if value > 0.0 { format!("${value:.2}") } else { "-".to_string() }
}

#[must_use]
pub fn money_details(value: f64) -> String {
    if value > 0.0 { format!("${value:.4}") } else { "-".to_string() }
}

#[must_use]
pub fn dash_if_empty(value: Option<&str>) -> String {
    value.filter(|s| !s.is_empty()).unwrap_or("-").to_string()
}

#[must_use]
pub fn truncate_to(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let prefix: String = value.chars().take(max_chars - 1).collect();
    format!("{prefix}…")
}

pub fn kv_line(key: &'static str, value: String, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key:<13} "), Style::default().fg(theme.fg_muted)),
        Span::styled(value, Style::default().fg(theme.fg_default)),
    ])
}

fn trim_decimal(value: f64, decimals: usize, suffix: &str) -> String {
    let mut s = format!("{value:.decimals$}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    format!("{s}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_tokens_uses_decimal_suffixes() {
        assert_eq!(human_tokens(999), "999");
        assert_eq!(human_tokens(1_200), "1.2k");
        assert_eq!(human_tokens(12_800), "12.8k");
        assert_eq!(human_tokens(5_167_742), "5.17M");
    }

    #[test]
    fn human_bytes_uses_binary_suffixes() {
        assert_eq!(human_bytes(999), "999B");
        assert_eq!(human_bytes(1_229), "1.2K");
        assert_eq!(human_bytes(441_450_496), "421M");
        assert_eq!(human_bytes(1_610_612_736), "1.5G");
    }

    #[test]
    fn dash_if_empty_replaces_missing_values() {
        assert_eq!(dash_if_empty(None), "-");
        assert_eq!(dash_if_empty(Some("")), "-");
        assert_eq!(dash_if_empty(Some("sonnet")), "sonnet");
    }
}
