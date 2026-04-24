//! Codex / ChatGPT Plus quota provider.
//!
//! Endpoint: `GET https://chatgpt.com/backend-api/wham/usage`
//! Auth:     `Bearer <ChatGPT session access token>`
//! Header:   `ChatGPT-Account-Id: <uuid>`  (optional, added if entry has accountId)
//!
//! Note: 401 means the session access token expired. We do NOT refresh it —
//! that would mutate state in opencode's auth.json. User must re-auth via
//! their upstream tool (e.g. `opencode auth login openai`).
//!
//! See spec section 3.2 for the full parsing contract.

use crate::quota::auth::OpencodeAuth;
use crate::quota::http::{
    classify_response, redact_auth_headers, truncate_body, HttpClient, HttpRequest,
};
use crate::quota::providers::Provider;
use crate::quota::time::{clamp_percent, normalize_numeric_ts};
use crate::quota::types::{ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageWindow};
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

const PROVIDER_ID: ProviderId = ProviderId::Codex;
const PROVIDER_NAME: &str = "Codex / ChatGPT Plus";
const ALIASES: &[&str] = &["openai", "codex", "chatgpt"];
const URL: &str = "https://chatgpt.com/backend-api/wham/usage";

pub struct Codex;

impl Provider for Codex {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }
    fn display_name(&self) -> &'static str {
        PROVIDER_NAME
    }
    fn is_configured(&self, auth: &OpencodeAuth) -> bool {
        auth.lookup(ALIASES)
            .map(|e| e.access.is_some() || e.token.is_some())
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
    let token = match entry.access.as_deref().or(entry.token.as_deref()) {
        Some(t) => t,
        None => return ProviderResult::not_configured(PROVIDER_ID, PROVIDER_NAME),
    };

    let mut req = HttpRequest::get(URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .with_timeout(Duration::from_secs(10));
    if let Some(acct) = entry.account_id.as_deref() {
        req = req.header("ChatGPT-Account-Id", acct);
    }

    let mut logged = req.clone();
    redact_auth_headers(&mut logged.headers);
    tracing::debug!(provider = "codex", url = %logged.url, "quota.fetch started");

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

    let mut result = parse(&resp.body);
    if let Some(plan) = crate::quota::subscription::codex_plan(auth) {
        result.meta.insert("plan".to_string(), plan);
    }
    result
}

pub(crate) fn parse(body: &[u8]) -> ProviderResult {
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

    if let Some(rl) = raw.rate_limit.as_ref() {
        if let Some(primary) = rl.get("primary_window") {
            windows.insert(
                "5h".to_string(),
                UsageWindow {
                    used_percent: clamp_percent(extract_f64(primary, "used_percent")),
                    window_seconds: extract_u64(primary, "limit_window_seconds"),
                    reset_at: extract_i64(primary, "reset_at").map(normalize_numeric_ts),
                    value_label: None,
                },
            );
        }
        if let Some(secondary) = rl.get("secondary_window") {
            windows.insert(
                "weekly".to_string(),
                UsageWindow {
                    used_percent: clamp_percent(extract_f64(secondary, "used_percent")),
                    window_seconds: extract_u64(secondary, "limit_window_seconds"),
                    reset_at: extract_i64(secondary, "reset_at").map(normalize_numeric_ts),
                    value_label: None,
                },
            );
        }
    }

    if let Some(credits) = raw.credits.as_ref() {
        let unlimited = credits
            .get("unlimited")
            .and_then(serde_json::Value::as_bool);
        let balance = extract_f64(credits, "balance");
        let label = if unlimited == Some(true) {
            Some("Unlimited".to_string())
        } else {
            balance.map(|b| format!("${:.2} remaining", b))
        };
        // Only emit the credits window if we have something to say.
        if label.is_some() {
            windows.insert(
                "credits".to_string(),
                UsageWindow {
                    used_percent: None,
                    window_seconds: None,
                    reset_at: None,
                    value_label: label,
                },
            );
        }
    }

    let usage = Usage {
        windows,
        models: IndexMap::new(),
        extras: IndexMap::new(),
    };
    // meta["plan"] is populated by fetch_impl via subscription::codex_plan.
    ProviderResult::ok(PROVIDER_ID, PROVIDER_NAME, usage, BTreeMap::new())
}

// ---------- Raw response shape ----------
//
// The real API response shape has drifted from the original spec. We use
// serde_json::Value for sub-objects that contain fields with inconsistent
// types (e.g. numeric fields that arrive as strings in some plan tiers).
// Extraction helpers below treat type errors as "field absent".

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawResponse {
    // Nested rate-limit windows (spec shape: primary_window / secondary_window).
    // Kept as Value to tolerate real-API shape divergence.
    rate_limit: Option<serde_json::Value>,
    // Also kept as Value: the real API returns `balance` as a JSON string
    // (e.g. "0") on some plan tiers, which breaks a strict Option<f64>
    // deserialize. Extraction helpers below tolerate both number and string.
    credits: Option<serde_json::Value>,
}

/// Extract an `f64` from a JSON value that may be a number or a numeric string.
fn extract_f64(v: &serde_json::Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Extract a `u64` from a JSON value that may be a number or a numeric string.
fn extract_u64(v: &serde_json::Value, key: &str) -> Option<u64> {
    match v.get(key)? {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Extract an `i64` from a JSON value that may be a number or a numeric string.
fn extract_i64(v: &serde_json::Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/codex/200_sample.json");
    const UNLIMITED: &[u8] =
        include_bytes!("../../../tests/fixtures/codex/200_unlimited_credits.json");
    const BALANCE_AS_STRING: &[u8] =
        include_bytes!("../../../tests/fixtures/codex/200_balance_as_string.json");

    #[test]
    fn parse_sample_has_all_three_windows() {
        let r = parse(SAMPLE);
        assert!(r.ok, "{:?}", r.error);
        let u = r.usage.as_ref().unwrap();
        assert!(u.windows.contains_key("5h"));
        assert!(u.windows.contains_key("weekly"));
        assert!(u.windows.contains_key("credits"));

        let five_h = &u.windows["5h"];
        assert_eq!(five_h.used_percent, Some(34.0));
        assert_eq!(five_h.window_seconds, Some(18000));
        // reset_at was 1779175500 (seconds), must be converted to ms.
        assert_eq!(five_h.reset_at, Some(1_779_175_500_000));

        let credits = &u.windows["credits"];
        assert_eq!(credits.used_percent, None);
        assert_eq!(credits.value_label.as_deref(), Some("$12.34 remaining"));
    }

    #[test]
    fn parse_unlimited_surfaces_unlimited_label() {
        let r = parse(UNLIMITED);
        let u = r.usage.as_ref().unwrap();
        let credits = &u.windows["credits"];
        assert_eq!(credits.value_label.as_deref(), Some("Unlimited"));
    }

    #[test]
    fn parse_tolerates_balance_as_string() {
        // Real-world regression: chatgpt.com/backend-api/wham/usage returns
        // credits.balance as a JSON string ("0") on the Plus plan. A strict
        // Option<f64> deserialize fails the whole payload. The parser must
        // tolerate both numeric and string balances.
        let r = parse(BALANCE_AS_STRING);
        assert!(r.ok, "{:?}", r.error);
        let u = r.usage.as_ref().unwrap();
        let credits = &u.windows["credits"];
        assert_eq!(credits.value_label.as_deref(), Some("$0.00 remaining"));
    }

    #[test]
    fn parse_garbage_returns_parse_error() {
        let r = parse(b"not json");
        assert!(!r.ok);
        assert!(matches!(r.error.as_ref().unwrap().kind, ErrorKind::Parse));
    }
}
