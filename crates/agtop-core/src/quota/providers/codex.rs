//! Codex / ChatGPT Plus quota provider.
//!
//! Endpoint: GET https://chatgpt.com/backend-api/wham/usage
//! Auth:     Bearer <ChatGPT session access token>
//! Header:   ChatGPT-Account-Id: <uuid>  (optional, added if entry has accountId)
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

    if let Some(primary) = raw
        .rate_limit
        .as_ref()
        .and_then(|r| r.primary_window.as_ref())
    {
        windows.insert(
            "5h".to_string(),
            UsageWindow {
                used_percent: clamp_percent(primary.used_percent),
                window_seconds: primary.limit_window_seconds,
                reset_at: primary.reset_at.map(normalize_numeric_ts),
                value_label: None,
            },
        );
    }

    if let Some(secondary) = raw
        .rate_limit
        .as_ref()
        .and_then(|r| r.secondary_window.as_ref())
    {
        windows.insert(
            "weekly".to_string(),
            UsageWindow {
                used_percent: clamp_percent(secondary.used_percent),
                window_seconds: secondary.limit_window_seconds,
                reset_at: secondary.reset_at.map(normalize_numeric_ts),
                value_label: None,
            },
        );
    }

    if let Some(credits) = raw.credits.as_ref() {
        let label = if credits.unlimited == Some(true) {
            Some("Unlimited".to_string())
        } else {
            credits
                .balance
                .map(|balance| format!("${:.2} remaining", balance))
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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawResponse {
    rate_limit: Option<RateLimit>,
    credits: Option<Credits>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RateLimit {
    primary_window: Option<Window>,
    secondary_window: Option<Window>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Window {
    used_percent: Option<f64>,
    limit_window_seconds: Option<u64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Credits {
    balance: Option<f64>,
    unlimited: Option<bool>,
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/codex/200_sample.json");
    const UNLIMITED: &[u8] =
        include_bytes!("../../../tests/fixtures/codex/200_unlimited_credits.json");

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
    fn parse_garbage_returns_parse_error() {
        let r = parse(b"not json");
        assert!(!r.ok);
        assert!(matches!(r.error.as_ref().unwrap().kind, ErrorKind::Parse));
    }
}
