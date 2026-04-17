//! Cost-tab helpers: per-row formatting used by both the widget and tests.

use agtop_core::session::{CostBreakdown, TokenTotals};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a single cost-row triple `(label, tokens_str, dollars_str)`.
/// Lives here (rather than in the widget) so it is unit-testable without
/// a terminal backend.
pub fn cost_row(label: &'static str, tokens: u64, dollars: f64) -> (&'static str, String, String) {
    (label, format_tokens(tokens), format_dollars(dollars))
}

/// Return the ordered set of cost rows for the cost tab.  Predictable
/// ordering keeps the widget trivial and snapshot-testable.
pub fn cost_rows(
    tokens: &TokenTotals,
    cost: &CostBreakdown,
) -> Vec<(&'static str, String, String)> {
    vec![
        cost_row("input", tokens.input, cost.input),
        cost_row("cached_input", tokens.cached_input, cost.cached_input),
        cost_row("output", tokens.output, cost.output),
        cost_row("cache_write_5m", tokens.cache_write_5m, cost.cache_write_5m),
        cost_row("cache_write_1h", tokens.cache_write_1h, cost.cache_write_1h),
        cost_row("cache_read", tokens.cache_read, cost.cache_read),
    ]
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}G", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.2}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn format_dollars(d: f64) -> String {
    if d == 0.0 {
        "-".into()
    } else {
        // Four decimal places everywhere — session costs are typically
        // in the $0.001–$10 range, so a uniform width keeps columns
        // aligned without hiding sub-cent figures.
        format!("${:.4}", d)
    }
}
