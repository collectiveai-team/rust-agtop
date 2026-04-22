//! Integration tests for the Copilot provider.

use agtop_core::quota::http::FakeHttp;
use agtop_core::quota::providers::{copilot::Copilot, Provider};
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

fn copilot_body() -> Vec<u8> {
    std::fs::read(fixture("copilot/200_individual_unlimited.json")).unwrap()
}

#[test]
fn is_configured_true_when_access_present() {
    assert!(Copilot.is_configured(&auth_full()));
}

#[test]
fn is_configured_false_when_missing() {
    assert!(!Copilot.is_configured(&OpencodeAuth::empty()));
}

#[test]
fn fetch_uses_token_scheme_not_bearer() {
    let http = FakeHttp::new();
    http.push_ok(200, &copilot_body());
    let _ = Copilot.fetch(&auth_full(), &http);
    let req = http.last_request().unwrap();
    let auth_h = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .unwrap();
    assert_eq!(auth_h.1, "token COPILOT_ACCESS_PLACEHOLDER");
    assert!(!auth_h.1.starts_with("Bearer "));
}

#[test]
fn fetch_includes_editor_headers() {
    let http = FakeHttp::new();
    http.push_ok(200, &copilot_body());
    let _ = Copilot.fetch(&auth_full(), &http);
    let req = http.last_request().unwrap();
    let editor = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("editor-version"))
        .unwrap();
    assert_eq!(editor.1, "vscode/1.96.2");
    let api_ver = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-github-api-version"))
        .unwrap();
    assert_eq!(api_ver.1, "2025-04-01");
}

#[test]
fn fetch_success_returns_usage_and_meta() {
    let http = FakeHttp::new();
    http.push_ok(200, &copilot_body());
    let r = Copilot.fetch(&auth_full(), &http);
    assert!(r.ok);
    assert_eq!(r.provider_id, ProviderId::Copilot);
    let u = r.usage.unwrap();
    assert!(u.windows.contains_key("chat"));
    assert!(u.windows.contains_key("premium"));
    assert_eq!(r.meta.get("plan").map(String::as_str), Some("individual"));
}

#[test]
fn fetch_401_returns_http_error() {
    let http = FakeHttp::new();
    http.push_ok(401, b"{\"message\":\"Bad credentials\"}");
    let r = Copilot.fetch(&auth_full(), &http);
    assert!(matches!(
        r.error.unwrap().kind,
        ErrorKind::Http { status: 401, .. }
    ));
}

#[test]
fn fetch_not_configured_when_auth_empty() {
    let http = FakeHttp::new();
    let r = Copilot.fetch(&OpencodeAuth::empty(), &http);
    assert!(matches!(r.error.unwrap().kind, ErrorKind::NotConfigured));
}
