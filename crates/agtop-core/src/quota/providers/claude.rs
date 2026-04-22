//! Claude (Anthropic Pro/Max) quota provider.
//!
//! Endpoint: GET https://api.anthropic.com/api/oauth/usage
//! Header:  anthropic-beta: oauth-2025-04-20
//! Auth:    Bearer <oauth access token>
//!
//! See spec section 3.1 for the full parsing contract.

use crate::quota::auth::OpencodeAuth;
use crate::quota::http::{
    classify_response, redact_auth_headers, truncate_body, HttpClient, HttpRequest,
};
use crate::quota::providers::Provider;
use crate::quota::time::{clamp_percent, iso_to_epoch_ms};
use crate::quota::types::{
    ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageExtra, UsageWindow,
};
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

const PROVIDER_ID: ProviderId = ProviderId::Claude;
const PROVIDER_NAME: &str = "Claude";
const ALIASES: &[&str] = &["anthropic", "claude"];
const URL: &str = "https://api.anthropic.com/api/oauth/usage";
const ANTHROPIC_BETA: &str = "oauth-2025-04-20";

pub struct Claude;

impl Provider for Claude {
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

    let req = HttpRequest::get(URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", ANTHROPIC_BETA)
        .with_timeout(Duration::from_secs(10));

    let mut logged = req.clone();
    redact_auth_headers(&mut logged.headers);
    tracing::debug!(provider = "claude", url = %logged.url, "quota.fetch started");

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

    parse(&resp.body)
}

/// Parse a Claude `/api/oauth/usage` response body into a `ProviderResult`.
/// Public within the crate so tests can exercise it with fixture bytes.
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
    for (label, bucket) in [
        ("5h", &raw.five_hour),
        ("7d", &raw.seven_day),
        ("7d-sonnet", &raw.seven_day_sonnet),
        ("7d-opus", &raw.seven_day_opus),
        ("7d-oauth-apps", &raw.seven_day_oauth_apps),
        ("7d-cowork", &raw.seven_day_cowork),
        ("7d-omelette", &raw.seven_day_omelette),
    ] {
        if let Some(b) = bucket {
            windows.insert(
                label.to_string(),
                UsageWindow {
                    used_percent: clamp_percent(b.utilization),
                    window_seconds: None,
                    reset_at: b.resets_at.as_deref().and_then(iso_to_epoch_ms),
                    value_label: None,
                },
            );
        }
    }

    let mut extras: IndexMap<String, UsageExtra> = IndexMap::new();
    if let Some(eu) = raw.extra_usage {
        extras.insert(
            "extra_usage".to_string(),
            UsageExtra::OverageBudget {
                monthly_limit: eu.monthly_limit,
                used: eu.used_credits,
                utilization: eu.utilization,
                currency: eu.currency,
                enabled: eu.is_enabled.unwrap_or(false),
            },
        );
    }

    let usage = Usage {
        windows,
        models: IndexMap::new(),
        extras,
    };
    ProviderResult::ok(PROVIDER_ID, PROVIDER_NAME, usage, BTreeMap::new())
}

// ---------- Raw response shape ----------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawResponse {
    five_hour: Option<Bucket>,
    seven_day: Option<Bucket>,
    seven_day_sonnet: Option<Bucket>,
    seven_day_opus: Option<Bucket>,
    seven_day_oauth_apps: Option<Bucket>,
    seven_day_cowork: Option<Bucket>,
    seven_day_omelette: Option<Bucket>,
    extra_usage: Option<ExtraUsage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Bucket {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ExtraUsage {
    is_enabled: Option<bool>,
    monthly_limit: Option<f64>,
    used_credits: Option<f64>,
    utilization: Option<f64>,
    currency: Option<String>,
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/claude/200_active_subscription.json");
    const MINIMAL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/fixtures/claude/200_minimal.json");

    #[test]
    fn parse_active_subscription_surfaces_every_populated_window() {
        let r = parse(ACTIVE_FIXTURE);
        assert!(r.ok, "fetch result should be ok, got {:?}", r.error);
        let u = r.usage.as_ref().expect("usage present");

        // Populated windows: 5h, 7d, 7d-sonnet, 7d-omelette. Null ones omitted.
        assert!(u.windows.contains_key("5h"));
        assert!(u.windows.contains_key("7d"));
        assert!(u.windows.contains_key("7d-sonnet"));
        assert!(u.windows.contains_key("7d-omelette"));
        assert!(!u.windows.contains_key("7d-opus"));
        assert!(!u.windows.contains_key("7d-oauth-apps"));
        assert!(!u.windows.contains_key("7d-cowork"));

        let five_h = &u.windows["5h"];
        assert_eq!(five_h.used_percent, Some(89.0));
        assert!(five_h.reset_at.is_some());

        let omelette = &u.windows["7d-omelette"];
        assert_eq!(omelette.used_percent, Some(0.0));
        assert!(omelette.reset_at.is_none()); // resets_at was null
    }

    #[test]
    fn parse_active_subscription_surfaces_extra_usage_even_when_disabled() {
        let r = parse(ACTIVE_FIXTURE);
        let u = r.usage.as_ref().unwrap();
        let extra = u.extras.get("extra_usage").expect("extra_usage present");
        match extra {
            UsageExtra::OverageBudget {
                enabled,
                monthly_limit,
                used,
                currency,
                ..
            } => {
                assert!(!enabled);
                assert!(monthly_limit.is_none());
                assert!(used.is_none());
                assert!(currency.is_none());
            }
            other => panic!("wrong extra variant: {:?}", other),
        }
    }

    #[test]
    fn parse_minimal_fixture_has_only_five_hour_window() {
        let r = parse(MINIMAL_FIXTURE);
        let u = r.usage.as_ref().unwrap();
        assert_eq!(u.windows.len(), 1);
        let w = &u.windows["5h"];
        assert_eq!(w.used_percent, Some(12.5));
    }

    #[test]
    fn parse_garbage_returns_parse_error() {
        let r = parse(b"not json");
        assert!(!r.ok);
        assert!(matches!(r.error.as_ref().unwrap().kind, ErrorKind::Parse));
    }
}
