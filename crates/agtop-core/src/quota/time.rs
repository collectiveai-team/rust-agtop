//! Shared timestamp helpers for parsing provider responses.

use chrono::DateTime;

/// Convert ISO-8601 (RFC 3339) string to epoch milliseconds.
/// Returns None on parse failure or empty input.
pub fn iso_to_epoch_ms(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Normalize a numeric timestamp: if value < 1e12, treat as seconds and
/// multiply by 1000; else treat as milliseconds already.
pub fn normalize_numeric_ts(value: i64) -> i64 {
    if value < 1_000_000_000_000 {
        value.saturating_mul(1000)
    } else {
        value
    }
}

/// Clamp a percentage value to [0.0, 100.0]. Returns None if input is None or NaN.
pub fn clamp_percent(value: Option<f64>) -> Option<f64> {
    value.filter(|v| !v.is_nan()).map(|v| v.clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_parses_utc_rfc3339() {
        let ms = iso_to_epoch_ms("2026-04-21T20:59:59Z").unwrap();
        // Sanity: this is a valid ms timestamp near 2026.
        assert!(ms > 1_700_000_000_000);
    }

    #[test]
    fn iso_parses_offset_rfc3339() {
        let a = iso_to_epoch_ms("2026-04-21T20:59:59+00:00").unwrap();
        let b = iso_to_epoch_ms("2026-04-21T20:59:59Z").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn iso_rejects_garbage() {
        assert_eq!(iso_to_epoch_ms(""), None);
        assert_eq!(iso_to_epoch_ms("not a timestamp"), None);
    }

    #[test]
    fn numeric_seconds_become_ms() {
        // 2021-01-01 in seconds.
        let secs = 1_609_459_200_i64;
        assert_eq!(normalize_numeric_ts(secs), secs * 1000);
    }

    #[test]
    fn numeric_ms_pass_through() {
        let ms = 1_700_000_000_000_i64;
        assert_eq!(normalize_numeric_ts(ms), ms);
    }

    #[test]
    fn clamp_basic() {
        assert_eq!(clamp_percent(Some(-5.0)), Some(0.0));
        assert_eq!(clamp_percent(Some(105.0)), Some(100.0));
        assert_eq!(clamp_percent(Some(42.0)), Some(42.0));
        assert_eq!(clamp_percent(None), None);
        assert_eq!(clamp_percent(Some(f64::NAN)), None);
    }
}
