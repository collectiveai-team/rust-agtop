//! Integration tests for the Google provider.
//!
//! These exercise the full multi-source, multi-endpoint `fetch_impl` path
//! including the fallback URL chain and partial-success aggregation.

use agtop_core::quota::http::FakeHttp;
use agtop_core::quota::providers::google::{fetch_impl, Google, SourceId};
use agtop_core::quota::providers::Provider;
use agtop_core::quota::types::ProviderId;
use agtop_core::quota::OpencodeAuth;
use serial_test::serial;
use std::path::PathBuf;

const NOW_MS: i64 = 1_777_075_200_000; // 2026-04-21T12:00:00Z

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures");
    p.push(name);
    p
}

fn auth_full() -> OpencodeAuth {
    OpencodeAuth::load_from(&fixture("auth/opencode_full.json")).unwrap()
}

fn load_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap()
}

#[test]
fn is_configured_true_when_gemini_entry_present() {
    assert!(Google.is_configured(&auth_full()));
}

#[test]
#[serial]
fn is_configured_false_when_no_sources() {
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");
    assert!(!Google.is_configured(&OpencodeAuth::empty()));
}

#[test]
#[serial]
fn fetch_gemini_only_queries_both_endpoints() {
    // Antigravity disabled by pointing the env override at a non-existent file.
    std::env::set_var(
        "AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS",
        "/tmp/does_not_exist_agtop_test",
    );
    let auth = auth_full();
    let http = FakeHttp::new();
    // Call 1: :retrieveUserQuota (Gemini).
    http.push_ok(200, &load_bytes("google/retrieveUserQuota_gemini.json"));
    // Call 2: :fetchAvailableModels — first sandbox fails, primary succeeds.
    http.push_ok(404, b"{}");
    http.push_ok(404, b"{}");
    http.push_ok(200, &load_bytes("google/fetchAvailableModels_gemini.json"));

    let r = fetch_impl(&auth, &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.provider_id, ProviderId::Google);
    let u = r.usage.unwrap();

    // Top-level windows empty — Google only emits per-model.
    assert!(u.windows.is_empty());
    // Both sources contribute models under "gemini/..." prefix.
    assert!(u.models.contains_key("gemini/gemini-2.5-pro"));
    assert!(u.models.contains_key("gemini/gemini-2.5-flash"));
    let pro = &u.models["gemini/gemini-2.5-pro"];
    assert!(pro.contains_key("daily"));
}

#[test]
#[serial]
fn fetch_aggregates_both_sources_when_both_configured() {
    // Point Antigravity at the fixture accounts file.
    let antigravity_path = fixture("google/antigravity_accounts.json");
    std::env::set_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS", &antigravity_path);

    let auth = auth_full();
    let http = FakeHttp::new();
    // Gemini path.
    http.push_ok(200, &load_bytes("google/retrieveUserQuota_gemini.json"));
    http.push_ok(200, &load_bytes("google/fetchAvailableModels_gemini.json"));
    // Antigravity path:
    // - Antigravity source has no access_token (its access_token is None by
    //   design — we don't refresh). So the Antigravity source produces an
    //   error and never calls the http client. Nothing to queue.

    let r = fetch_impl(&auth, &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");

    // Partial success: Gemini succeeded, Antigravity failed silently.
    assert!(r.ok);
    let u = r.usage.unwrap();
    assert!(u.models.contains_key("gemini/gemini-2.5-pro"));
    // Meta records both sources being present, even when one failed.
    let sources = r.meta.get("sources").unwrap();
    assert!(sources.contains("Gemini"));
    assert!(sources.contains("Antigravity"));
}

#[test]
#[serial]
fn fetch_all_sources_fail_returns_error() {
    std::env::set_var(
        "AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS",
        "/tmp/does_not_exist_agtop_test",
    );
    let auth = auth_full();
    let http = FakeHttp::new();
    // :retrieveUserQuota 401.
    http.push_ok(401, b"{\"error\":\"unauthorized\"}");
    // :fetchAvailableModels: all three fallback URLs return 401.
    http.push_ok(401, b"nope");
    http.push_ok(401, b"nope");
    http.push_ok(401, b"nope");

    let r = fetch_impl(&auth, &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");
    assert!(!r.ok);
    let err = r.error.unwrap();
    assert!(err.detail.contains("Gemini"));
}

#[test]
#[serial]
fn fetch_not_configured_when_empty_auth_and_no_file() {
    std::env::set_var(
        "AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS",
        "/tmp/does_not_exist_agtop_test",
    );
    let http = FakeHttp::new();
    let r = fetch_impl(&OpencodeAuth::empty(), &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");
    assert!(!r.ok);
    assert!(!r.configured);
}

#[test]
fn source_id_labels_are_stable_strings() {
    assert_eq!(SourceId::Gemini.label(), "gemini");
    assert_eq!(SourceId::Antigravity.label(), "antigravity");
}
