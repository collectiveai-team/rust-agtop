#![allow(dead_code, unused)]
//! Text-filter predicate for the session table.

use agtop_core::session::SessionAnalysis;

/// Returns `true` when `a` matches the text filter.
///
/// An empty filter matches everything. The filter is compared
/// case-insensitively against: session id, model, effective_model, cwd,
/// client name, and subscription — covering the fields users typically
/// search for.
pub(super) fn matches_filter(a: &SessionAnalysis, filter_lower: &str) -> bool {
    if filter_lower.is_empty() {
        return true;
    }
    let s = &a.summary;
    let candidates: [Option<&str>; 6] = [
        Some(s.session_id.as_str()),
        s.model.as_deref(),
        a.effective_model.as_deref(),
        s.cwd.as_deref(),
        Some(s.client.as_str()),
        s.subscription.as_deref(),
    ];
    candidates
        .iter()
        .flatten()
        .any(|hay| hay.to_ascii_lowercase().contains(filter_lower))
}
