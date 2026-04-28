//! Shared text-formatting helpers for the CLI and TUI.
//!
//! Centralises the small presentation utilities that would otherwise be
//! duplicated between `main.rs` (the `--list` table) and the various
//! TUI widget modules.

use chrono::{DateTime, Local, Utc};

// ---------------------------------------------------------------------------
// Token / number formatting
// ---------------------------------------------------------------------------

/// Format a token or byte count in a compact htop-style representation:
/// `1234` → `"1.2K"`, `1_234_567` → `"1.2M"`, etc.
pub fn compact(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}G", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

// ---------------------------------------------------------------------------
// Session ID formatting
// ---------------------------------------------------------------------------

/// Shorten a session ID for display:
/// - OpenCode `ses_*` IDs are truncated to 10 chars.
/// - Everything else (UUID-style) is truncated to 8 chars.
pub fn short_id(id: &str) -> String {
    if id.starts_with("ses_") {
        return id[..id.len().min(10)].to_string();
    }
    id.chars().take(8).collect()
}

// ---------------------------------------------------------------------------
// Path formatting
// ---------------------------------------------------------------------------

/// Replace the home directory prefix with `~` so long paths fit on screen.
pub fn shorten_path(p: &str) -> String {
    if let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) {
        if let Some(rest) = p.strip_prefix(&home) {
            return format!("~{}", rest);
        }
    }
    p.to_string()
}

// ---------------------------------------------------------------------------
// Time / duration formatting
// ---------------------------------------------------------------------------

/// Format a UTC timestamp as a short local datetime: `"YYYY-MM-DD HH:MM"`.
/// Uses the system's local timezone so the output is meaningful to the user.
pub fn format_local_datetime(ts: DateTime<Utc>) -> String {
    ts.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// htop/agtop-style relative age.  Returns `"now"` for the sub-minute range,
/// then `"<n>m"`, `"<n>h"`, `"<n>d"`, `"<n>w"`, `"<n>mo"`, `"<n>y"`.
pub fn relative_age(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    if secs < 60 {
        return "now".into();
    }
    if secs < 3_600 {
        return format!("{}m", secs / 60);
    }
    if secs < 86_400 {
        return format!("{}h", secs / 3_600);
    }
    if secs < 604_800 {
        return format!("{}d", secs / 86_400);
    }
    if secs < 2_592_000 {
        return format!("{}w", secs / 604_800);
    }
    if secs < 31_536_000 {
        return format!("{}mo", secs / 2_592_000);
    }
    format!("{}y", secs / 31_536_000)
}

/// Format a duration in seconds as a compact human string:
/// `"<n>h<m>m"` / `"<n>m<s>s"` / `"<n>s"`.
pub fn format_duration_compact(secs: u64) -> String {
    if secs >= 3_600 {
        format!("{}h{}m", secs / 3_600, (secs % 3_600) / 60)
    } else if secs >= 60 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

// ---------------------------------------------------------------------------
// Table cell formatting (CLI --list table)
// ---------------------------------------------------------------------------

/// Format an optional CPU percentage with one decimal place.
/// Returns `"-"` when the value is absent.
pub fn format_percent(value: Option<f32>) -> String {
    value
        .map(|v| format!("{v:.1}%"))
        .unwrap_or_else(|| "-".to_string())
}

/// Format an optional byte count using [`compact`].
/// Returns `"-"` when the value is absent.
pub fn compact_opt(value: Option<u64>) -> String {
    value.map(compact).unwrap_or_else(|| "-".to_string())
}

/// Format an optional byte-per-second rate using [`compact`] plus `/s`.
/// Returns `"-"` when the value is absent.
pub fn compact_rate_opt(value: Option<f64>) -> String {
    match value {
        Some(v) if v.is_finite() && v > 0.0 => format!("{}/s", compact(v.round() as u64)),
        Some(_) => "0/s".to_string(),
        None => "-".to_string(),
    }
}

/// Fit `s` into a field of exactly `w` display columns: pad with spaces if
/// shorter, truncate with an ellipsis (`…`) if longer.
pub fn fit(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        format!("{:<w$}", s, w = w)
    } else {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_rate_formats_optional_bytes_per_second() {
        assert_eq!(compact_rate_opt(None), "-");
        assert_eq!(compact_rate_opt(Some(0.0)), "0/s");
        assert_eq!(compact_rate_opt(Some(512.0)), "512/s");
        assert_eq!(compact_rate_opt(Some(1_280.0)), "1.3K/s");
        assert_eq!(compact_rate_opt(Some(1_250_000.0)), "1.2M/s");
    }
}
