//! Integration tests for the Claude provider. Uses FakeHttp, never hits the network.

use agtop_core::quota::http::FakeHttp;
use agtop_core::quota::providers::{claude::Claude, Provider};
use agtop_core::quota::types::{ErrorKind, ProviderId};
use agtop_core::quota::OpencodeAuth;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures");
    p.push(name);
    p
}

fn auth_full() -> OpencodeAuth {
    OpencodeAuth::load_from(&fixture("auth/opencode_full.json")).unwrap()
}

fn auth_empty() -> OpencodeAuth {
    OpencodeAuth::empty()
}

fn claude_body_active() -> Vec<u8> {
    std::fs::read(fixture("claude/200_active_subscription.json")).unwrap()
}

#[test]
fn is_configured_true_when_access_present() {
    let auth = auth_full();
    assert!(Claude.is_configured(&auth));
}

#[test]
fn is_configured_false_when_missing() {
    let auth = auth_empty();
    assert!(!Claude.is_configured(&auth));
}

#[test]
fn fetch_constructs_expected_request() {
    let auth = auth_full();
    let http = FakeHttp::new();
    http.push_ok(200, &claude_body_active());

    let _ = Claude.fetch(&auth, &http);
    let req = http.last_request().expect("request captured");
    assert_eq!(req.url, "https://api.anthropic.com/api/oauth/usage");

    let auth_header = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .expect("authorization header present");
    assert_eq!(auth_header.1, "Bearer ANTHROPIC_ACCESS_PLACEHOLDER");

    let beta_header = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("anthropic-beta"))
        .expect("anthropic-beta header present");
    assert_eq!(beta_header.1, "oauth-2025-04-20");
}

#[test]
fn fetch_success_returns_usage() {
    let auth = auth_full();
    let http = FakeHttp::new();
    http.push_ok(200, &claude_body_active());
    let r = Claude.fetch(&auth, &http);
    assert!(r.ok);
    assert_eq!(r.provider_id, ProviderId::Claude);
    let u = r.usage.unwrap();
    assert!(u.windows.len() >= 4);
}

#[test]
fn fetch_401_returns_http_error() {
    let auth = auth_full();
    let http = FakeHttp::new();
    http.push_ok(401, b"{\"error\":\"invalid token\"}");
    let r = Claude.fetch(&auth, &http);
    assert!(!r.ok);
    let err = r.error.unwrap();
    assert!(matches!(err.kind, ErrorKind::Http { status: 401, .. }));
    assert!(err.detail.contains("invalid token"));
}

#[test]
fn fetch_transport_error_returns_transport_kind() {
    use agtop_core::quota::http::TransportError;
    let auth = auth_full();
    let http = FakeHttp::new();
    http.push_err(TransportError::Timeout);
    let r = Claude.fetch(&auth, &http);
    assert!(!r.ok);
    assert!(matches!(r.error.unwrap().kind, ErrorKind::Transport));
}

#[test]
fn fetch_not_configured_when_auth_empty() {
    let http = FakeHttp::new();
    // Queue no responses — fetch must not call http at all when not configured.
    let r = Claude.fetch(&auth_empty(), &http);
    assert!(!r.ok);
    assert!(!r.configured);
    assert!(matches!(r.error.unwrap().kind, ErrorKind::NotConfigured));
}
