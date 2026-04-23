//! Shared bar-rendering helpers for the quota pane.

use agtop_core::quota::QuotaError;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::tui::theme as th;

/// Unicode "BLACK SQUARE" — used for filled bar cells.
pub const BAR_FILLED: char = '■';

/// Resolve a style for a bar based on `used_percent` (0..100).
/// `stale=true` forces the dim/gray variant regardless of threshold.
pub fn bar_style(used_percent: Option<f64>, stale: bool) -> Style {
    if stale {
        return th::QUOTA_BAR_STALE;
    }
    match used_percent {
        None => th::QUOTA_EMPTY,
        Some(p) if p < 75.0 => th::QUOTA_BAR_OK,
        Some(p) if p < 90.0 => th::QUOTA_BAR_WARN,
        Some(_) => th::QUOTA_BAR_CRIT,
    }
}

/// Build a pair of spans `[filled, empty]` of total width `width`.
pub fn bar_spans(used_percent: Option<f64>, width: usize, stale: bool) -> [Span<'static>; 2] {
    let width = width.max(1);
    let fill = used_percent
        .map(|p| {
            let clamped = p.clamp(0.0, 100.0);
            ((clamped / 100.0) * width as f64).round() as usize
        })
        .unwrap_or(0)
        .min(width);
    let empty = width - fill;
    let style = bar_style(used_percent, stale);
    [
        Span::styled(BAR_FILLED.to_string().repeat(fill), style),
        Span::raw(" ".repeat(empty)),
    ]
}

/// Short (≤ 5 char) identifier for error display in cards.
pub fn error_token(err: &QuotaError) -> String {
    use agtop_core::quota::ErrorKind;
    match &err.kind {
        ErrorKind::NotConfigured => "n/c".to_string(),
        ErrorKind::Http { status, .. } => status.to_string(),
        ErrorKind::Transport => "net".to_string(),
        ErrorKind::Parse => "parse".to_string(),
        ErrorKind::Provider { code } => code.clone().unwrap_or_else(|| "err".to_string()),
    }
}

/// Status glyph for the Dashboard list column.
#[allow(dead_code)]
pub fn status_glyph(current_ok: bool, last_good_some: bool, loading: bool) -> char {
    if loading {
        '○'
    } else if current_ok {
        '●'
    } else if last_good_some {
        '▲'
    } else {
        '✗'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_below_75_is_ok() {
        assert_eq!(bar_style(Some(74.9), false), th::QUOTA_BAR_OK);
    }

    #[test]
    fn style_75_to_90_is_warn() {
        assert_eq!(bar_style(Some(75.0), false), th::QUOTA_BAR_WARN);
        assert_eq!(bar_style(Some(89.9), false), th::QUOTA_BAR_WARN);
    }

    #[test]
    fn style_at_or_above_90_is_crit() {
        assert_eq!(bar_style(Some(90.0), false), th::QUOTA_BAR_CRIT);
        assert_eq!(bar_style(Some(100.0), false), th::QUOTA_BAR_CRIT);
    }

    #[test]
    fn style_stale_overrides_threshold() {
        assert_eq!(bar_style(Some(50.0), true), th::QUOTA_BAR_STALE);
    }

    #[test]
    fn spans_fill_calculation() {
        let [filled, empty] = bar_spans(Some(50.0), 10, false);
        assert_eq!(filled.content.chars().count(), 5);
        assert_eq!(empty.content.chars().count(), 5);
    }

    #[test]
    fn spans_fill_rounds_to_nearest() {
        let [filled, _] = bar_spans(Some(33.0), 10, false);
        assert_eq!(filled.content.chars().count(), 3);
        let [filled, _] = bar_spans(Some(35.0), 10, false);
        assert_eq!(filled.content.chars().count(), 4);
    }

    #[test]
    fn spans_none_is_all_empty() {
        let [filled, empty] = bar_spans(None, 6, false);
        assert_eq!(filled.content.chars().count(), 0);
        assert_eq!(empty.content.chars().count(), 6);
    }

    #[test]
    fn spans_total_width_always_matches() {
        for pct in [0.0, 10.0, 50.0, 99.0, 100.0] {
            let [f, e] = bar_spans(Some(pct), 20, false);
            assert_eq!(f.content.chars().count() + e.content.chars().count(), 20);
        }
    }

    #[test]
    fn error_token_401() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Http {
                status: 401,
                retry_after: None,
            },
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "401");
    }

    #[test]
    fn error_token_429() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Http {
                status: 429,
                retry_after: Some(30),
            },
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "429");
    }

    #[test]
    fn error_token_transport() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Transport,
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "net");
    }

    #[test]
    fn error_token_parse() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Parse,
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "parse");
    }

    #[test]
    fn error_token_provider_with_code() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Provider {
                code: Some("E001".into()),
            },
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "E001");
    }

    #[test]
    fn error_token_provider_no_code() {
        use agtop_core::quota::{ErrorKind, QuotaError};
        let e = QuotaError {
            kind: ErrorKind::Provider { code: None },
            detail: "".into(),
        };
        assert_eq!(error_token(&e), "err");
    }
}
