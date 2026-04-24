//! z.ai Coding Plan quota provider.
//!
//! Endpoint: `GET https://api.z.ai/api/monitor/usage/quota/limit`
//! Auth:     `Bearer <api key>`
//!
//! Response has `data.limits[]` containing one or more entries with
//! `type: "TOKENS_LIMIT"` (token-window quotas) and optionally one with
//! `type: "TIME_LIMIT"` (web-tool usage counters).
//!
//! Deltas vs openchamber:
//! - Iterate ALL TOKENS_LIMIT entries (openchamber takes only the first).
//! - Emit TIME_LIMIT to `extras["web-tools"]` (openchamber drops it).
//! - Expose `data.level` in `meta`.
//! - Safety net: when the hardcoded unit→seconds table and the observed
//!   `nextResetTime - now` delta disagree by more than 2×, log a
//!   `kind=inference_mismatch` WARN and trust the observed value.
//!
//! See spec section 3.4 for the full parsing contract.

use crate::quota::auth::OpencodeAuth;
use crate::quota::http::{
    classify_response, redact_auth_headers, truncate_body, HttpClient, HttpRequest,
};
use crate::quota::providers::Provider;
use crate::quota::time::{clamp_percent, normalize_numeric_ts};
use crate::quota::types::{
    ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageExtra, UsageWindow,
};
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

const PROVIDER_ID: ProviderId = ProviderId::Zai;
const PROVIDER_NAME: &str = "z.ai";
const ALIASES: &[&str] = &["zai-coding-plan", "zai", "z.ai"];
const URL: &str = "https://api.z.ai/api/monitor/usage/quota/limit";

pub struct Zai;

impl Provider for Zai {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }
    fn display_name(&self) -> &'static str {
        PROVIDER_NAME
    }
    fn is_configured(&self, auth: &OpencodeAuth) -> bool {
        auth.lookup(ALIASES)
            .map(|e| e.key.is_some() || e.token.is_some())
            .unwrap_or(false)
    }
    fn fetch(&self, auth: &OpencodeAuth, http: &dyn HttpClient) -> ProviderResult {
        fetch_impl(auth, http)
    }
}

fn fetch_impl(auth: &OpencodeAuth, http: &dyn HttpClient) -> ProviderResult {
    let entry = match auth.lookup(ALIASES) {
        Some(e) => e,
        None => return ProviderResult::not_configured(PROVIDER_ID, PROVIDER_NAME),
    };
    let token = match entry.key.as_deref().or(entry.token.as_deref()) {
        Some(t) => t,
        None => return ProviderResult::not_configured(PROVIDER_ID, PROVIDER_NAME),
    };

    let req = HttpRequest::get(URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .with_timeout(Duration::from_secs(10));

    let mut logged = req.clone();
    redact_auth_headers(&mut logged.headers);
    tracing::debug!(provider = "zai", url = %logged.url, "quota.fetch started");

    let resp = match http.request(req) {
        Ok(r) => r,
        Err(e) => {
            return ProviderResult::err(
                PROVIDER_ID,
                PROVIDER_NAME,
                QuotaError {
                    kind: ErrorKind::Transport,
                    detail: e.to_string(),
                },
            );
        }
    };

    if let Some(err) = classify_response(&resp) {
        return ProviderResult::err(PROVIDER_ID, PROVIDER_NAME, err);
    }

    parse(&resp.body, now_epoch_ms())
}

fn now_epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Parse a z.ai quota/limit response body. `now_ms` is the reference time for
/// the inference-mismatch safety net — tests inject a fixed value so the
/// assertions are deterministic.
pub(crate) fn parse(body: &[u8], now_ms: i64) -> ProviderResult {
    let raw: RawResponse = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => {
            return ProviderResult::err(
                PROVIDER_ID,
                PROVIDER_NAME,
                QuotaError {
                    kind: ErrorKind::Parse,
                    detail: format!("{e}: {}", truncate_body(body, 200)),
                },
            );
        }
    };

    let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
    let mut extras: IndexMap<String, UsageExtra> = IndexMap::new();

    let limits = raw.data.as_ref().map(|d| &d.limits[..]).unwrap_or(&[]);

    for limit in limits {
        match limit.type_.as_deref() {
            Some("TOKENS_LIMIT") => {
                let (label, window_seconds) = resolve_window(limit, now_ms);
                let reset_at = limit.next_reset_time.map(normalize_numeric_ts);
                windows.insert(
                    label,
                    UsageWindow {
                        used_percent: clamp_percent(limit.percentage),
                        window_seconds,
                        reset_at,
                        value_label: None,
                    },
                );
            }
            Some("TIME_LIMIT") => {
                let items: Vec<(String, u64)> = limit
                    .usage_details
                    .iter()
                    .filter_map(|d| {
                        let code = d.model_code.clone()?;
                        let usage = d.usage.unwrap_or(0).max(0) as u64;
                        Some((code, usage))
                    })
                    .collect();
                let total_cap = limit.usage.map(|v| v.max(0) as u64);
                let reset_at = limit.next_reset_time.map(normalize_numeric_ts);
                extras.insert(
                    "web-tools".to_string(),
                    UsageExtra::PerToolCounts {
                        items,
                        total_cap,
                        reset_at,
                    },
                );
            }
            _ => {
                // Unknown limit type — skip but don't fail. Log at DEBUG for
                // future investigation.
                tracing::debug!(
                    provider = "zai",
                    limit_type = ?limit.type_,
                    "quota.fetch unknown limit type"
                );
            }
        }
    }

    let raw_level = raw.data.as_ref().and_then(|d| d.level.as_deref());
    let plan_label = crate::quota::subscription::zai_plan(raw_level);

    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    meta.insert("plan".to_string(), plan_label);
    // Keep the raw level for any tooling that reads meta directly.
    if let Some(level) = raw.data.as_ref().and_then(|d| d.level.clone()) {
        meta.insert("level".to_string(), level);
    }

    let usage = Usage {
        windows,
        models: IndexMap::new(),
        extras,
    };
    ProviderResult::ok(PROVIDER_ID, PROVIDER_NAME, usage, meta)
}

/// Resolve a TOKENS_LIMIT entry into `(window_label, window_seconds)`.
///
/// Strategy:
/// 1. Compute `hardcoded_seconds` from the (unit, number) table.
/// 2. Compute `observed_seconds = (nextResetTime - now) / 1000` when available.
/// 3. If both are present and disagree by >2× in either direction, log a
///    WARN and trust the observed value.
/// 4. If only one is present, use it.
/// 5. If neither is present, window_seconds = None and label falls back to
///    "tokens".
fn resolve_window(limit: &Limit, now_ms: i64) -> (String, Option<u64>) {
    let hardcoded = hardcoded_seconds(limit.unit, limit.number);
    let observed: Option<u64> = match limit.next_reset_time {
        Some(reset_ms) => {
            let reset_ms = normalize_numeric_ts(reset_ms);
            let delta_ms = (reset_ms - now_ms).max(0);
            Some((delta_ms / 1000) as u64)
        }
        None => None,
    };

    let chosen = match (hardcoded, observed) {
        (Some(h), Some(o)) => {
            let max = h.max(o) as f64;
            let min = h.min(o) as f64;
            if min > 0.0 && max / min > 2.0 {
                tracing::warn!(
                    provider = "zai",
                    kind = "inference_mismatch",
                    unit = limit.unit,
                    number = limit.number,
                    hardcoded_seconds = h,
                    observed_seconds = o,
                    "quota.fetch hardcoded window mismatch"
                );
                Some(o)
            } else {
                Some(h)
            }
        }
        (Some(h), None) => Some(h),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    };

    // Use the hardcoded window size for the label when available, so that a
    // "weekly" window is always labelled "weekly" regardless of how much time
    // remains until the next reset (which the API reports as nextResetTime).
    let label_secs = hardcoded.or(chosen);
    let label = match label_secs {
        Some(s) => seconds_to_label(s),
        None => "tokens".to_string(),
    };
    (label, chosen)
}

/// Hardcoded unit→seconds table. Returns None for unknown unit codes.
/// Multiplied by `number` to get the full window duration.
///
/// Units 1/2/3 are minute-/hour-based; 4/5 are day-based; 6 is month-based
/// (inferred from live data at 2026-04-21 showing a ~22-day remaining delta
/// on unit 6 / number 1). Units 1, 2, 4 are guesses — if live data ever
/// contradicts them, the safety net in `resolve_window` will log a warning.
fn hardcoded_seconds(unit: Option<i64>, number: Option<i64>) -> Option<u64> {
    let unit = unit?;
    let number = number.filter(|&n| n > 0)?;
    let base = match unit {
        1 => 60,          // minute (guess)
        2 => 3_600,       // hour (guess)
        3 => 3_600,       // hour (confirmed via openchamber)
        4 => 86_400,      // day (guess)
        5 => 86_400,      // day (seen on TIME_LIMIT)
        6 => 86_400 * 30, // month (inferred from live data)
        _ => return None,
    };
    Some((base as u64) * (number as u64))
}

/// Given a window duration in seconds, pick a display label.
fn seconds_to_label(seconds: u64) -> String {
    if seconds == 0 {
        return "tokens".to_string();
    }
    if seconds % 86_400 == 0 {
        let days = seconds / 86_400;
        if days == 7 {
            return "weekly".to_string();
        }
        if (28..=31).contains(&days) {
            return "monthly".to_string();
        }
        return format!("{days}d");
    }
    if seconds % 3_600 == 0 {
        return format!("{}h", seconds / 3_600);
    }
    if seconds % 60 == 0 {
        return format!("{}m", seconds / 60);
    }
    format!("{seconds}s")
}

// ---------- Raw response shape ----------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawResponse {
    data: Option<Data>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Data {
    level: Option<String>,
    limits: Vec<Limit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Limit {
    #[serde(rename = "type")]
    type_: Option<String>,
    unit: Option<i64>,
    number: Option<i64>,
    percentage: Option<f64>,
    usage: Option<i64>,
    #[serde(rename = "nextResetTime")]
    next_reset_time: Option<i64>,
    #[serde(rename = "usageDetails")]
    usage_details: Vec<UsageDetail>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UsageDetail {
    #[serde(rename = "modelCode")]
    model_code: Option<String>,
    usage: Option<i64>,
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    const LITE: &[u8] = include_bytes!("../../../tests/fixtures/zai/200_lite_both_windows.json");
    const TIME_ONLY: &[u8] = include_bytes!("../../../tests/fixtures/zai/200_time_limit_only.json");
    const UNKNOWN_UNIT: &[u8] = include_bytes!("../../../tests/fixtures/zai/200_unknown_unit.json");

    /// Fixed reference time close to the fixture timestamps so the
    /// inference-mismatch comparison yields predictable results.
    /// 2026-04-21T12:00:00Z = 1777075200000 ms.
    const NOW_MS: i64 = 1_777_075_200_000;

    #[test]
    fn parse_lite_emits_both_tokens_windows_and_web_tools() {
        let r = parse(LITE, NOW_MS);
        assert!(r.ok, "{:?}", r.error);
        let u = r.usage.as_ref().unwrap();

        // Both TOKENS_LIMIT entries surfaced — this is the primary openchamber
        // bug we're fixing.
        assert!(
            u.windows.contains_key("5h"),
            "expected 5h window, got {:?}",
            u.windows.keys().collect::<Vec<_>>()
        );
        assert!(
            u.windows.contains_key("monthly"),
            "expected monthly window, got {:?}",
            u.windows.keys().collect::<Vec<_>>()
        );

        let five_h = &u.windows["5h"];
        assert_eq!(five_h.window_seconds, Some(3600 * 5));
        assert_eq!(five_h.used_percent, Some(0.0));

        let monthly = &u.windows["monthly"];
        assert_eq!(monthly.used_percent, Some(48.0));
        assert!(monthly.reset_at.is_some());

        // TIME_LIMIT surfaced as extras.
        let web = u.extras.get("web-tools").expect("web-tools extra");
        match web {
            UsageExtra::PerToolCounts {
                items, total_cap, ..
            } => {
                assert_eq!(items.len(), 3);
                assert!(items.iter().any(|(k, _)| k == "search-prime"));
                assert_eq!(*total_cap, Some(100));
            }
            _ => panic!("wrong extras variant"),
        }

        // Meta contains level.
        assert_eq!(r.meta.get("level").map(String::as_str), Some("lite"));
    }

    #[test]
    fn parse_time_only_has_no_windows_but_has_extras() {
        let r = parse(TIME_ONLY, NOW_MS);
        assert!(r.ok);
        let u = r.usage.as_ref().unwrap();
        assert!(u.windows.is_empty(), "expected no windows");
        assert!(u.extras.contains_key("web-tools"));
        assert_eq!(r.meta.get("level").map(String::as_str), Some("pro"));
    }

    #[test]
    fn parse_unknown_unit_falls_back_to_observed_seconds() {
        let r = parse(UNKNOWN_UNIT, NOW_MS);
        assert!(r.ok);
        let u = r.usage.as_ref().unwrap();
        // The fixture's nextResetTime is 1777000000000 (before NOW_MS),
        // so delta = max(0, 1777000000000 - 1777075200000) = 0. That's an
        // expired window — we still emit it, but with windowSeconds=Some(0)
        // and label falling back through seconds_to_label (0s → "tokens").
        assert_eq!(u.windows.len(), 1);
        assert!(u.windows.contains_key("tokens"));
    }

    #[test]
    fn resolve_window_prefers_hardcoded_when_observed_agrees() {
        // unit=3, number=5 → hardcoded 18000s. Observed 18060s (1 minute
        // later than expected, within 2× tolerance) → pick hardcoded.
        let limit = Limit {
            type_: Some("TOKENS_LIMIT".into()),
            unit: Some(3),
            number: Some(5),
            percentage: Some(0.0),
            next_reset_time: Some(NOW_MS + 18_060 * 1000),
            ..Limit::default()
        };
        let (label, secs) = resolve_window(&limit, NOW_MS);
        assert_eq!(label, "5h");
        assert_eq!(secs, Some(18_000));
    }

    #[test]
    fn resolve_window_trusts_observed_on_large_disagreement() {
        // unit=3, number=5 → hardcoded 18000s. Observed 100_000s → ratio > 5×.
        // Safety net: trust observed.
        let limit = Limit {
            type_: Some("TOKENS_LIMIT".into()),
            unit: Some(3),
            number: Some(5),
            percentage: Some(0.0),
            next_reset_time: Some(NOW_MS + 100_000 * 1000),
            ..Limit::default()
        };
        let (_label, secs) = resolve_window(&limit, NOW_MS);
        assert_eq!(secs, Some(100_000));
    }

    #[test]
    fn seconds_to_label_handles_canonical_durations() {
        assert_eq!(seconds_to_label(60), "1m");
        assert_eq!(seconds_to_label(3_600), "1h");
        assert_eq!(seconds_to_label(18_000), "5h");
        assert_eq!(seconds_to_label(86_400), "1d");
        assert_eq!(seconds_to_label(7 * 86_400), "weekly");
        assert_eq!(seconds_to_label(30 * 86_400), "monthly");
        assert_eq!(seconds_to_label(31 * 86_400), "monthly");
        assert_eq!(seconds_to_label(500), "500s");
    }

    #[test]
    fn parse_garbage_returns_parse_error() {
        let r = parse(b"not json", NOW_MS);
        assert!(!r.ok);
        assert!(matches!(r.error.unwrap().kind, ErrorKind::Parse));
    }
}
