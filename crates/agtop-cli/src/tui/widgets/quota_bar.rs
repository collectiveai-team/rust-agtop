//! Shared bar-rendering helpers for the quota pane.

use agtop_core::quota::QuotaError;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::tui::theme as th;

/// Unicode "BLACK SQUARE" — used for filled bar cells.
pub const BAR_FILLED: char = '■';
/// Unicode "WHITE SQUARE" — used for empty bar cells.
pub const BAR_EMPTY: char = '□';

/// Resolve a style for the filled portion of a bar based on `used_percent` (0..100).
/// `stale=true` forces the dim/gray variant regardless of threshold.
///
/// Thresholds: green < 30 %, yellow 30–80 %, red ≥ 80 %.
pub fn bar_style(used_percent: Option<f64>, stale: bool) -> Style {
    if stale {
        return th::QUOTA_BAR_STALE;
    }
    match used_percent {
        None => th::QUOTA_EMPTY,
        Some(p) if p < 30.0 => th::QUOTA_BAR_OK,
        Some(p) if p < 80.0 => th::QUOTA_BAR_WARN,
        Some(_) => th::QUOTA_BAR_CRIT,
    }
}

/// Build a triple of spans `[filled, empty, reset]` of total bar width `width`.
///
/// Both filled and empty cells use `■`/`□` so the bar always occupies
/// exactly `width` columns. The empty portion uses a dim style so it
/// reads as "unoccupied" without disappearing entirely.
pub fn bar_spans(used_percent: Option<f64>, width: usize, stale: bool) -> [Span<'static>; 2] {
    let width = width.max(1);
    let fill = used_percent
        .map(|p| {
            let clamped = p.clamp(0.0, 100.0);
            ((clamped / 100.0) * width as f64).round() as usize
        })
        .unwrap_or(0)
        .min(width);
    let empty_count = width - fill;
    let fill_style = bar_style(used_percent, stale);
    let empty_style = th::QUOTA_BAR_EMPTY_CELL;
    [
        Span::styled(BAR_FILLED.to_string().repeat(fill), fill_style),
        Span::styled(BAR_EMPTY.to_string().repeat(empty_count), empty_style),
    ]
}

/// Short, consistent provider name matching the rest of the TUI's labelling.
///
/// Avoids long marketing names like "GitHub Copilot" or "Codex / ChatGPT Plus"
/// in favour of compact identifiers that fit in list rows and card slots.
pub fn provider_short_name(id: agtop_core::quota::ProviderId) -> &'static str {
    use agtop_core::quota::ProviderId;
    match id {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Copilot => "Copilot",
        ProviderId::CopilotAddon => "Copilot+",
        ProviderId::Zai => "z.ai",
        ProviderId::Google => "Google",
    }
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

/// Returns the ratatui `Style` for the status glyph (● / ✗ / ▲ / ○).
pub fn status_style(ok: bool, has_last_good: bool, loading: bool) -> Style {
    if ok && !loading {
        th::QUOTA_BAR_OK
    } else if !ok && has_last_good {
        th::QUOTA_BAR_STALE
    } else if loading {
        th::QUOTA_BAR_STALE
    } else {
        th::QUOTA_BAR_CRIT
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
    fn style_below_30_is_ok() {
        assert_eq!(bar_style(Some(0.0), false), th::QUOTA_BAR_OK);
        assert_eq!(bar_style(Some(29.9), false), th::QUOTA_BAR_OK);
    }

    #[test]
    fn style_30_to_80_is_warn() {
        assert_eq!(bar_style(Some(30.0), false), th::QUOTA_BAR_WARN);
        assert_eq!(bar_style(Some(79.9), false), th::QUOTA_BAR_WARN);
    }

    #[test]
    fn style_at_or_above_80_is_crit() {
        assert_eq!(bar_style(Some(80.0), false), th::QUOTA_BAR_CRIT);
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
        // Empty cells are □ characters, not spaces.
        assert_eq!(empty.content.chars().count(), 6);
        assert!(empty.content.chars().all(|c| c == '□'));
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
