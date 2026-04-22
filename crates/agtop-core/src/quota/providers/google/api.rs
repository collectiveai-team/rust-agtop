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

pub const USER_AGENT: &str = "antigravity/1.11.5 windows/amd64";
pub const X_GOOG_API_CLIENT: &str = "google-cloud-sdk vscode_cloudshelleditor/0.1";
pub const CLIENT_METADATA: &str = "{\"ideType\":\"IDE_UNSPECIFIED\",\"platform\":\"PLATFORM_UNSPECIFIED\",\"pluginType\":\"GEMINI\"}";

pub const GOOGLE_TIMEOUT: Duration = Duration::from_secs(15);

/// Fetch quota buckets. Only meaningful for the Gemini source.
/// Returns Ok(response bytes) or Err(QuotaError). 2xx only.
pub fn fetch_quota_buckets(
    http: &dyn HttpClient,
    access_token: &str,
    project_id: Option<&str>,
) -> Result<Vec<u8>, QuotaError> {
    let url = format!("{PRIMARY_HOST}{RETRIEVE_QUOTA_PATH}");
    let body = build_project_body(project_id);
    request_once(http, &url, access_token, body)
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
        assert_eq!(ua.1, USER_AGENT);
        let goog = req
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("x-goog-api-client"))
            .unwrap();
        assert_eq!(goog.1, X_GOOG_API_CLIENT);
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
