//! Google API calls used by the quota provider.
//!
//! Two endpoints are queried:
//! - `:retrieveUserQuota` — returns quota buckets. Gemini source only.
//! - `:fetchAvailableModels` — returns per-model quota info. Both sources.
//!
//! `:fetchAvailableModels` has a fallback URL chain (openchamber behaviour).
//! The primary host sometimes 404s for accounts routed to sandbox backends,
//! so we try the sandbox URLs before giving up.

use crate::quota::http::{classify_response, redact_auth_headers, HttpClient, HttpRequest};
use crate::quota::types::QuotaError;
use std::time::Duration;

pub const PRIMARY_HOST: &str = "https://cloudcode-pa.googleapis.com";
pub const DAILY_SANDBOX_HOST: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";
pub const AUTOPUSH_SANDBOX_HOST: &str = "https://autopush-cloudcode-pa.sandbox.googleapis.com";

pub const RETRIEVE_QUOTA_PATH: &str = "/v1internal:retrieveUserQuota";
pub const FETCH_MODELS_PATH: &str = "/v1internal:fetchAvailableModels";
pub const LOAD_CODE_ASSIST_PATH: &str = "/v1internal:loadCodeAssist";

pub const USER_AGENT: &str = "antigravity/1.11.5 windows/amd64";
pub const X_GOOG_API_CLIENT: &str = "google-cloud-sdk vscode_cloudshelleditor/0.1";
pub const CLIENT_METADATA: &str = "{\"ideType\":\"IDE_UNSPECIFIED\",\"platform\":\"PLATFORM_UNSPECIFIED\",\"pluginType\":\"GEMINI\"}";

/// User-Agent sent for `:retrieveUserQuota`. Gemini CLI 0.36.0 uses a
/// `code-assist/<version>` string without the `X-Goog-Api-Client` header.
/// Google's backend gates `retrieveUserQuota` on the caller identity encoded
/// in `X-Goog-Api-Client`; the VSCode/Antigravity value produces 403 for
/// personal free-tier accounts. Omitting the header (or using a neutral UA)
/// allows the call to succeed.
pub const RETRIEVE_QUOTA_USER_AGENT: &str = "code-assist/0.36.0";

pub const GOOGLE_TIMEOUT: Duration = Duration::from_secs(15);

/// Call `:loadCodeAssist` in `HEALTH_CHECK` mode. Used to discover the
/// account's tier and `cloudaicompanionProject`.
///
/// This is the same call Gemini CLI makes at startup (see the upstream
/// source: `refreshAvailableCredits` in `packages/core/src/code_assist`).
/// It works for both free-tier and paid-tier users without a caller-supplied
/// project id.
///
/// The HEALTH_CHECK mode tells the server we want current tier/quota info
/// without triggering onboarding. Returns raw bytes on 2xx.
pub fn load_code_assist(http: &dyn HttpClient, access_token: &str) -> Result<Vec<u8>, QuotaError> {
    let url = format!("{PRIMARY_HOST}{LOAD_CODE_ASSIST_PATH}");
    let body = serde_json::to_vec(&serde_json::json!({
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        },
        "mode": "HEALTH_CHECK",
    }))
    .unwrap();
    request_once(http, &url, access_token, body)
}

/// Fetch quota buckets. Only meaningful for the Gemini source.
/// Returns Ok(response bytes) or Err(QuotaError). 2xx only.
///
/// Note: this call intentionally omits `X-Goog-Api-Client`. The VSCode /
/// Antigravity value (`google-cloud-sdk vscode_cloudshelleditor/0.1`) causes
/// Google's backend to return 403 PERMISSION_DENIED for personal free-tier
/// accounts, even when `:loadCodeAssist` succeeds. Gemini CLI 0.36.0 sends
/// its own `code-assist/<version>` User-Agent without that header.
pub fn fetch_quota_buckets(
    http: &dyn HttpClient,
    access_token: &str,
    project_id: Option<&str>,
) -> Result<Vec<u8>, QuotaError> {
    let url = format!("{PRIMARY_HOST}{RETRIEVE_QUOTA_PATH}");
    let body = build_project_body(project_id);
    request_quota(http, &url, access_token, body)
}

/// Fetch models + per-model quota info. Tries the fallback URL chain;
/// first 2xx response wins. If all fail, returns the last error.
pub fn fetch_available_models(
    http: &dyn HttpClient,
    access_token: &str,
    project_id: Option<&str>,
) -> Result<Vec<u8>, QuotaError> {
    let hosts = [DAILY_SANDBOX_HOST, AUTOPUSH_SANDBOX_HOST, PRIMARY_HOST];
    let body = build_project_body(project_id);
    let mut last_err: Option<QuotaError> = None;

    for host in hosts {
        let url = format!("{host}{FETCH_MODELS_PATH}");
        match request_once(http, &url, access_token, body.clone()) {
            Ok(bytes) => return Ok(bytes),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| QuotaError {
        kind: crate::quota::types::ErrorKind::Transport,
        detail: "all fallback hosts exhausted".to_string(),
    }))
}

fn build_project_body(project_id: Option<&str>) -> Vec<u8> {
    match project_id {
        Some(p) => serde_json::to_vec(&serde_json::json!({ "project": p })).unwrap(),
        None => b"{}".to_vec(),
    }
}

/// Build and send a request for `:retrieveUserQuota`. Omits the
/// `X-Goog-Api-Client` header that causes 403 for personal free-tier accounts.
fn request_quota(
    http: &dyn HttpClient,
    url: &str,
    access_token: &str,
    body: Vec<u8>,
) -> Result<Vec<u8>, QuotaError> {
    let req = HttpRequest::post(url, body)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", RETRIEVE_QUOTA_USER_AGENT)
        .with_timeout(GOOGLE_TIMEOUT);

    let mut logged = req.clone();
    redact_auth_headers(&mut logged.headers);
    tracing::debug!(provider = "google", url = %logged.url, "quota.fetch started");

    let resp = http.request(req).map_err(|e| QuotaError {
        kind: crate::quota::types::ErrorKind::Transport,
        detail: e.to_string(),
    })?;
    if let Some(err) = classify_response(&resp) {
        return Err(err);
    }
    Ok(resp.body)
}

fn request_once(
    http: &dyn HttpClient,
    url: &str,
    access_token: &str,
    body: Vec<u8>,
) -> Result<Vec<u8>, QuotaError> {
    let req = HttpRequest::post(url, body)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", USER_AGENT)
        .header("X-Goog-Api-Client", X_GOOG_API_CLIENT)
        .header("Client-Metadata", CLIENT_METADATA)
        .with_timeout(GOOGLE_TIMEOUT);

    let mut logged = req.clone();
    redact_auth_headers(&mut logged.headers);
    tracing::debug!(provider = "google", url = %logged.url, "quota.fetch started");

    let resp = http.request(req).map_err(|e| QuotaError {
        kind: crate::quota::types::ErrorKind::Transport,
        detail: e.to_string(),
    })?;
    if let Some(err) = classify_response(&resp) {
        return Err(err);
    }
    Ok(resp.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::http::FakeHttp;

    #[test]
    fn fetch_quota_buckets_sends_bearer_and_headers() {
        let http = FakeHttp::new();
        http.push_ok(200, b"{\"buckets\":[]}");
        let _ = fetch_quota_buckets(&http, "ya29.test", Some("proj-x")).unwrap();
        let req = http.last_request().unwrap();
        assert_eq!(req.url, format!("{PRIMARY_HOST}{RETRIEVE_QUOTA_PATH}"));
        let auth = req
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .unwrap();
        assert_eq!(auth.1, "Bearer ya29.test");
        let ua = req
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
            .unwrap();
        assert_eq!(ua.1, RETRIEVE_QUOTA_USER_AGENT);
        // X-Goog-Api-Client must NOT be present: the VSCode/Antigravity value
        // causes 403 PERMISSION_DENIED for personal free-tier accounts.
        assert!(
            req.headers
                .iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("x-goog-api-client")),
            "X-Goog-Api-Client must be absent from retrieveUserQuota requests"
        );
    }

    #[test]
    fn fetch_quota_buckets_project_body() {
        let http = FakeHttp::new();
        http.push_ok(200, b"{}");
        fetch_quota_buckets(&http, "tok", Some("my-proj")).unwrap();
        let req = http.last_request().unwrap();
        let body = String::from_utf8(req.body.unwrap()).unwrap();
        assert!(body.contains("\"project\""));
        assert!(body.contains("my-proj"));
    }

    #[test]
    fn fetch_quota_buckets_no_project_sends_empty_body() {
        let http = FakeHttp::new();
        http.push_ok(200, b"{}");
        fetch_quota_buckets(&http, "tok", None).unwrap();
        let req = http.last_request().unwrap();
        assert_eq!(req.body.as_deref(), Some(&b"{}"[..]));
    }

    #[test]
    fn fetch_available_models_tries_fallbacks_until_success() {
        let http = FakeHttp::new();
        // First two hosts 404; third (primary) succeeds.
        http.push_ok(404, b"{}");
        http.push_ok(404, b"{}");
        http.push_ok(200, b"{\"models\":{}}");
        let body = fetch_available_models(&http, "tok", Some("proj")).unwrap();
        assert_eq!(&body[..], b"{\"models\":{}}");
    }

    #[test]
    fn fetch_available_models_returns_last_error_when_all_fail() {
        let http = FakeHttp::new();
        http.push_ok(500, b"err1");
        http.push_ok(502, b"err2");
        http.push_ok(503, b"err3");
        let err = fetch_available_models(&http, "tok", None).unwrap_err();
        match err.kind {
            crate::quota::types::ErrorKind::Http { status, .. } => assert_eq!(status, 503),
            other => panic!("wrong kind: {:?}", other),
        }
    }

    #[test]
    fn fetch_quota_buckets_401_returns_http_error() {
        let http = FakeHttp::new();
        http.push_ok(401, b"{\"error\":\"unauthorized\"}");
        let err = fetch_quota_buckets(&http, "tok", None).unwrap_err();
        assert!(matches!(
            err.kind,
            crate::quota::types::ErrorKind::Http { status: 401, .. }
        ));
    }
}
