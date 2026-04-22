//! Integration tests for the z.ai provider.

use agtop_core::quota::http::FakeHttp;
use agtop_core::quota::providers::{zai::Zai, Provider};
use agtop_core::quota::types::{ErrorKind, ProviderId, UsageExtra};
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

fn zai_body() -> Vec<u8> {
    std::fs::read(fixture("zai/200_lite_both_windows.json")).unwrap()
}

#[test]
fn is_configured_true_when_key_present() {
    assert!(Zai.is_configured(&auth_full()));
}

#[test]
fn is_configured_false_when_missing() {
    assert!(!Zai.is_configured(&OpencodeAuth::empty()));
}

#[test]
fn fetch_uses_bearer_with_key() {
    let http = FakeHttp::new();
    http.push_ok(200, &zai_body());
    let _ = Zai.fetch(&auth_full(), &http);
    let req = http.last_request().unwrap();
    assert_eq!(req.url, "https://api.z.ai/api/monitor/usage/quota/limit");
    let auth_h = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .unwrap();
    assert_eq!(auth_h.1, "Bearer ZAI_KEY_PLACEHOLDER");
}

#[test]
fn fetch_success_returns_usage_meta_and_extras() {
    let http = FakeHttp::new();
    http.push_ok(200, &zai_body());
    let r = Zai.fetch(&auth_full(), &http);
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.provider_id, ProviderId::Zai);

    let u = r.usage.unwrap();
    // Both tokens windows surfaced.
    assert!(u.windows.contains_key("5h"));
    assert!(u.windows.contains_key("monthly"));
    // Web-tools extras present.
    assert!(matches!(
        u.extras.get("web-tools"),
        Some(UsageExtra::PerToolCounts { .. })
    ));
    // Meta has level.
    assert_eq!(r.meta.get("level").map(String::as_str), Some("lite"));
}

#[test]
fn fetch_401_returns_http_error() {
    let http = FakeHttp::new();
    http.push_ok(401, b"{\"msg\":\"unauthorized\"}");
    let r = Zai.fetch(&auth_full(), &http);
    assert!(matches!(
        r.error.unwrap().kind,
        ErrorKind::Http { status: 401, .. }
    ));
}

#[test]
fn fetch_transport_error_returns_transport_kind() {
    use agtop_core::quota::http::TransportError;
    let http = FakeHttp::new();
    http.push_err(TransportError::Timeout);
    let r = Zai.fetch(&auth_full(), &http);
    assert!(matches!(r.error.unwrap().kind, ErrorKind::Transport));
}

#[test]
fn fetch_not_configured_when_auth_empty() {
    let http = FakeHttp::new();
    let r = Zai.fetch(&OpencodeAuth::empty(), &http);
    assert!(matches!(r.error.unwrap().kind, ErrorKind::NotConfigured));
}
