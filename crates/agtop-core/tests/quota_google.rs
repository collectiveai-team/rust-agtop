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
    std::env::set_var(
        "AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS",
        "/tmp/does_not_exist_agtop_test",
    );
    std::env::set_var(
        "AGTOP_QUOTA_GEMINI_CLI_CREDS",
        "/tmp/does_not_exist_agtop_gemini",
    );
    assert!(!Google.is_configured(&OpencodeAuth::empty()));
    std::env::remove_var("AGTOP_QUOTA_GEMINI_CLI_CREDS");
}

#[test]
#[serial]
fn fetch_gemini_free_tier_surfaces_tier_and_project() {
    // Antigravity disabled by pointing the env override at a non-existent file.
    std::env::set_var(
        "AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS",
        "/tmp/does_not_exist_agtop_test",
    );
    let auth = auth_full();
    let http = FakeHttp::new();
    // Free-tier flow: loadCodeAssist only. No quota buckets fetched since
    // there's no paidTier. Mirrors what Gemini CLI itself does.
    http.push_ok(200, &load_bytes("google/loadCodeAssist_free_tier.json"));

    let r = fetch_impl(&auth, &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.provider_id, ProviderId::Google);
    let u = r.usage.as_ref().unwrap();
    assert!(u.windows.is_empty());
    // Free-tier has no per-model quota windows — Gemini CLI itself skips the
    // retrieveUserQuota call when there's no onboarded paid project.
    assert!(u.models.is_empty());
    // Meta surfaces the tier label and project id so the TUI can render a
    // helpful row instead of an error.
    assert_eq!(
        r.meta.get("tier").map(String::as_str),
        Some("Gemini Code Assist for individuals")
    );
    assert_eq!(
        r.meta.get("project_id").map(String::as_str),
        Some("pure-pentameter-m2hpz")
    );
    assert!(r
        .meta
        .get("plan")
        .unwrap()
        .contains("Gemini Code Assist for individuals"));
}

#[test]
#[serial]
fn fetch_gemini_paid_tier_fetches_quota_buckets() {
    std::env::set_var(
        "AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS",
        "/tmp/does_not_exist_agtop_test",
    );
    let auth = auth_full();
    let http = FakeHttp::new();
    // Paid-tier flow: loadCodeAssist, then retrieveUserQuota using the
    // onboarded project id.
    http.push_ok(200, &load_bytes("google/loadCodeAssist_paid_tier.json"));
    http.push_ok(200, &load_bytes("google/retrieveUserQuota_gemini.json"));

    let r = fetch_impl(&auth, &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");

    assert!(r.ok, "{:?}", r.error);
    let u = r.usage.unwrap();
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
    // Free-tier loadCodeAssist for the Gemini source. Antigravity source has
    // no access token (we don't refresh), so it never hits the http client.
    http.push_ok(200, &load_bytes("google/loadCodeAssist_free_tier.json"));

    let r = fetch_impl(&auth, &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");

    // Partial success: Gemini succeeded, Antigravity contributes nothing.
    assert!(r.ok);
    // Meta records both sources being present, even when one contributed
    // no data.
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
    // :loadCodeAssist returns 401 — token expired. Our policy is to surface
    // the HTTP status rather than refresh.
    http.push_ok(401, b"{\"error\":\"unauthorized\"}");

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
    std::env::set_var(
        "AGTOP_QUOTA_GEMINI_CLI_CREDS",
        "/tmp/does_not_exist_agtop_gemini",
    );
    let http = FakeHttp::new();
    let r = fetch_impl(&OpencodeAuth::empty(), &http, NOW_MS);
    std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");
    std::env::remove_var("AGTOP_QUOTA_GEMINI_CLI_CREDS");
    assert!(!r.ok);
    assert!(!r.configured);
}

#[test]
fn source_id_labels_are_stable_strings() {
    assert_eq!(SourceId::Gemini.label(), "gemini");
    assert_eq!(SourceId::Antigravity.label(), "antigravity");
}
