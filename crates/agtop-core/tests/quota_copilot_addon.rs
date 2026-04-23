//! Integration tests for the Copilot Add-on provider. Sanity-checks that
//! addon shares Copilot's endpoint and auth scheme but produces only the
//! `premium` window.

use agtop_core::quota::http::FakeHttp;
use agtop_core::quota::providers::{copilot_addon::CopilotAddon, Provider};
use agtop_core::quota::types::ProviderId;
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
fn addon_is_not_configured_when_copilot_is_present() {
    // CopilotAddon suppresses itself when the base Copilot provider already
    // claims the same credential, to avoid duplicate rows in the quota pane.
    assert!(!CopilotAddon.is_configured(&auth_full()));
}

#[test]
fn addon_uses_same_endpoint_and_token_scheme() {
    let http = FakeHttp::new();
    http.push_ok(200, &copilot_body());
    let _ = CopilotAddon.fetch(&auth_full(), &http);
    let req = http.last_request().unwrap();
    assert_eq!(req.url, "https://api.github.com/copilot_internal/user");
    let auth_h = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .unwrap();
    assert!(auth_h.1.starts_with("token "));
}

#[test]
fn addon_returns_only_premium_window() {
    let http = FakeHttp::new();
    http.push_ok(200, &copilot_body());
    let r = CopilotAddon.fetch(&auth_full(), &http);
    assert!(r.ok);
    assert_eq!(r.provider_id, ProviderId::CopilotAddon);
    let u = r.usage.unwrap();
    assert_eq!(u.windows.len(), 1);
    assert!(u.windows.contains_key("premium"));
    // Explicitly verify the dropped windows:
    assert!(!u.windows.contains_key("chat"));
    assert!(!u.windows.contains_key("completions"));
}

#[test]
fn addon_meta_matches_copilot() {
    let http = FakeHttp::new();
    http.push_ok(200, &copilot_body());
    let r = CopilotAddon.fetch(&auth_full(), &http);
    assert_eq!(
        r.meta.get("plan").map(String::as_str),
        Some("GitHub Copilot Add-on · Individual")
    );
    assert_eq!(r.meta.get("login").map(String::as_str), Some("jedzill4"));
}
