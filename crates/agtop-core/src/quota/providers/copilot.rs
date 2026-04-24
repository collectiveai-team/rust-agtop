//! GitHub Copilot quota provider.
//!
//! Endpoint: `GET https://api.github.com/copilot_internal/user`
//! Auth:     `token <oauth_token>`  (NOT Bearer — the `token ` scheme is required)
//! Headers:  Accept, Editor-Version, X-Github-Api-Version
//!
//! Honors `unlimited: true` snapshots (openchamber does not). Prefers
//! server-computed `percent_remaining` over local math. Surfaces
//! `overage_count` in the value label and exposes plan/sku metadata.
//!
//! See spec section 3.3 for the full parsing contract.

use crate::quota::auth::OpencodeAuth;
use crate::quota::http::{
    classify_response, redact_auth_headers, truncate_body, HttpClient, HttpRequest,
};
use crate::quota::providers::Provider;
use crate::quota::time::{clamp_percent, iso_to_epoch_ms};
use crate::quota::types::{ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageWindow};
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

pub(crate) const PROVIDER_ID: ProviderId = ProviderId::Copilot;
pub(crate) const PROVIDER_NAME: &str = "GitHub Copilot";
pub(crate) const ALIASES: &[&str] = &["github-copilot", "copilot"];
pub(crate) const URL: &str = "https://api.github.com/copilot_internal/user";
pub(crate) const EDITOR_VERSION: &str = "vscode/1.96.2";
pub(crate) const GH_API_VERSION: &str = "2025-04-01";

pub struct Copilot;

impl Provider for Copilot {
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
        fetch_impl(auth, http, WindowFilter::All, PROVIDER_ID, PROVIDER_NAME)
    }
}

/// Controls whether Copilot Add-on reuse filters the windows down to
/// `premium` only. The main Copilot provider passes `All`.
pub(crate) enum WindowFilter {
    All,
    PremiumOnly,
}

pub(crate) fn fetch_impl(
    auth: &OpencodeAuth,
    http: &dyn HttpClient,
    filter: WindowFilter,
    provider_id: ProviderId,
    provider_name: &'static str,
) -> ProviderResult {
    let entry = match auth.lookup(ALIASES) {
        Some(e) => e,
        None => return ProviderResult::not_configured(provider_id, provider_name),
    };
    let token = match entry.access.as_deref().or(entry.token.as_deref()) {
        Some(t) => t,
        None => return ProviderResult::not_configured(provider_id, provider_name),
    };

    let req = HttpRequest::get(URL)
        .header("Authorization", format!("token {token}"))
        .header("Accept", "application/json")
        .header("Editor-Version", EDITOR_VERSION)
        .header("X-Github-Api-Version", GH_API_VERSION)
        .with_timeout(Duration::from_secs(10));

    let mut logged = req.clone();
    redact_auth_headers(&mut logged.headers);
    tracing::debug!(provider = "copilot", url = %logged.url, "quota.fetch started");

    let resp = match http.request(req) {
        Ok(r) => r,
        Err(e) => {
            return ProviderResult::err(
                provider_id,
                provider_name,
                QuotaError {
                    kind: ErrorKind::Transport,
                    detail: e.to_string(),
                },
            );
        }
    };

    if let Some(err) = classify_response(&resp) {
        return ProviderResult::err(provider_id, provider_name, err);
    }

    parse(&resp.body, filter, provider_id, provider_name)
}

pub(crate) fn parse(
    body: &[u8],
    filter: WindowFilter,
    provider_id: ProviderId,
    provider_name: &'static str,
) -> ProviderResult {
    let raw: RawResponse = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => {
            return ProviderResult::err(
                provider_id,
                provider_name,
                QuotaError {
                    kind: ErrorKind::Parse,
                    detail: format!("{e}: {}", truncate_body(body, 200)),
                },
            );
        }
    };

    let reset_at = raw.quota_reset_date.as_deref().and_then(|s| {
        // quota_reset_date is an ISO date like "2026-05-01".
        // Parse as midnight UTC.
        iso_to_epoch_ms(&format!("{s}T00:00:00Z"))
    });

    let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
    for (raw_key, snapshot) in raw.quota_snapshots.iter() {
        let label = match raw_key.as_str() {
            "premium_interactions" => "premium",
            other => other,
        };
        if matches!(filter, WindowFilter::PremiumOnly) && label != "premium" {
            continue;
        }
        windows.insert(label.to_string(), snapshot_to_window(snapshot, reset_at));
    }

    let plan_label = if provider_id == crate::quota::types::ProviderId::CopilotAddon {
        crate::quota::subscription::copilot_addon_plan(raw.copilot_plan.as_deref())
    } else {
        crate::quota::subscription::copilot_plan(raw.copilot_plan.as_deref())
    };

    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    meta.insert("plan".to_string(), plan_label);
    if let Some(v) = raw.access_type_sku.clone() {
        meta.insert("sku".to_string(), v);
    }
    if let Some(v) = raw.login.clone() {
        meta.insert("login".to_string(), v);
    }
    if let Some(v) = raw.quota_reset_date.clone() {
        meta.insert("reset_date".to_string(), v);
    }

    let usage = Usage {
        windows,
        models: IndexMap::new(),
        extras: IndexMap::new(),
    };
    ProviderResult::ok(provider_id, provider_name, usage, meta)
}

fn snapshot_to_window(snap: &Snapshot, reset_at: Option<i64>) -> UsageWindow {
    // The three shape branches, matching spec section 3.3:
    //   1) unlimited:true           -> used_percent=None, label="Unlimited"
    //   2) entitlement>0, remaining -> used_percent=100-percent_remaining
    //                                  (fall back to 100-remaining/entitlement*100)
    //                                  label="R / E left"
    //   3) otherwise                -> both None
    let (used_percent, mut value_label) = if snap.unlimited == Some(true) {
        (None, Some("Unlimited".to_string()))
    } else if let (Some(entitlement), Some(remaining)) = (snap.entitlement, snap.remaining) {
        if entitlement > 0.0 {
            let used = match snap.percent_remaining {
                Some(pr) => (100.0 - pr).max(0.0),
                None => (100.0 - (remaining / entitlement) * 100.0).max(0.0),
            };
            (
                clamp_percent(Some(used)),
                Some(format!("{:.0} / {:.0} left", remaining, entitlement)),
            )
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    if snap.overage_count.unwrap_or(0.0) > 0.0 {
        let overage = snap.overage_count.unwrap_or(0.0) as u64;
        value_label = Some(match value_label {
            Some(existing) => format!("{existing} (overage: {overage})"),
            None => format!("(overage: {overage})"),
        });
    }

    UsageWindow {
        used_percent,
        window_seconds: None,
        reset_at,
        value_label,
    }
}

// ---------- Raw response shape ----------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawResponse {
    login: Option<String>,
    copilot_plan: Option<String>,
    access_type_sku: Option<String>,
    quota_reset_date: Option<String>,
    quota_snapshots: IndexMap<String, Snapshot>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Snapshot {
    unlimited: Option<bool>,
    entitlement: Option<f64>,
    remaining: Option<f64>,
    percent_remaining: Option<f64>,
    overage_count: Option<f64>,
    overage_permitted: Option<bool>,
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    const INDIVIDUAL: &[u8] =
        include_bytes!("../../../tests/fixtures/copilot/200_individual_unlimited.json");
    const BUSINESS: &[u8] =
        include_bytes!("../../../tests/fixtures/copilot/200_business_metered.json");

    fn do_parse(body: &[u8], filter: WindowFilter) -> ProviderResult {
        parse(body, filter, PROVIDER_ID, PROVIDER_NAME)
    }

    #[test]
    fn unlimited_snapshots_render_as_unlimited_label() {
        let r = do_parse(INDIVIDUAL, WindowFilter::All);
        assert!(r.ok);
        let u = r.usage.as_ref().unwrap();
        let chat = &u.windows["chat"];
        assert_eq!(chat.used_percent, None);
        assert_eq!(chat.value_label.as_deref(), Some("Unlimited"));
        let completions = &u.windows["completions"];
        assert_eq!(completions.value_label.as_deref(), Some("Unlimited"));
    }

    #[test]
    fn premium_metered_uses_server_percent_remaining() {
        let r = do_parse(INDIVIDUAL, WindowFilter::All);
        let u = r.usage.as_ref().unwrap();
        let premium = &u.windows["premium"];
        // server reports percent_remaining = 90.3 → used = 9.7
        assert_eq!(premium.used_percent, Some(100.0 - 90.3));
        assert_eq!(premium.value_label.as_deref(), Some("271 / 300 left"));
    }

    #[test]
    fn overage_count_appears_in_value_label() {
        let r = do_parse(BUSINESS, WindowFilter::All);
        let u = r.usage.as_ref().unwrap();
        let chat = &u.windows["chat"];
        // entitlement=500, remaining=125, percent_remaining=25 → used=75
        assert_eq!(chat.used_percent, Some(75.0));
        let label = chat.value_label.as_deref().unwrap();
        assert!(label.contains("125 / 500 left"));
        assert!(label.contains("(overage: 17)"));
    }

    #[test]
    fn meta_contains_plan_sku_login() {
        let r = do_parse(INDIVIDUAL, WindowFilter::All);
        assert_eq!(
            r.meta.get("plan").map(String::as_str),
            Some("GitHub Copilot · Individual")
        );
        assert_eq!(
            r.meta.get("sku").map(String::as_str),
            Some("monthly_subscriber_quota")
        );
        assert_eq!(r.meta.get("login").map(String::as_str), Some("jedzill4"));
        assert_eq!(
            r.meta.get("reset_date").map(String::as_str),
            Some("2026-05-01")
        );
    }

    #[test]
    fn premium_only_filter_drops_everything_else() {
        let r = do_parse(INDIVIDUAL, WindowFilter::PremiumOnly);
        let u = r.usage.as_ref().unwrap();
        assert_eq!(u.windows.len(), 1);
        assert!(u.windows.contains_key("premium"));
        assert!(!u.windows.contains_key("chat"));
    }

    #[test]
    fn label_remap_premium_interactions_to_premium() {
        let r = do_parse(INDIVIDUAL, WindowFilter::All);
        let u = r.usage.as_ref().unwrap();
        assert!(u.windows.contains_key("premium"));
        assert!(!u.windows.contains_key("premium_interactions"));
    }

    #[test]
    fn reset_at_parsed_from_iso_date() {
        let r = do_parse(INDIVIDUAL, WindowFilter::All);
        let u = r.usage.as_ref().unwrap();
        let chat = &u.windows["chat"];
        assert!(chat.reset_at.is_some());
    }

    #[test]
    fn parse_garbage_returns_parse_error() {
        let r = do_parse(b"not json", WindowFilter::All);
        assert!(!r.ok);
        assert!(matches!(r.error.unwrap().kind, ErrorKind::Parse));
    }
}
