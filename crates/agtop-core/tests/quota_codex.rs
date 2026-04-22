//! Integration tests for the Codex provider.

use agtop_core::quota::http::FakeHttp;
use agtop_core::quota::providers::{codex::Codex, Provider};
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

fn codex_sample() -> Vec<u8> {
    std::fs::read(fixture("codex/200_sample.json")).unwrap()
}

fn codex_401() -> Vec<u8> {
    std::fs::read(fixture("codex/401_token_rejected.json")).unwrap()
}

#[test]
fn is_configured_true_when_access_present() {
    assert!(Codex.is_configured(&auth_full()));
}

#[test]
fn is_configured_false_when_missing() {
    assert!(!Codex.is_configured(&OpencodeAuth::empty()));
}

#[test]
fn fetch_sends_bearer_and_account_id_headers() {
    let auth = auth_full();
    let http = FakeHttp::new();
    http.push_ok(200, &codex_sample());

    let _ = Codex.fetch(&auth, &http);
    let req = http.last_request().unwrap();
    assert_eq!(req.url, "https://chatgpt.com/backend-api/wham/usage");

    let auth_h = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .unwrap();
    assert_eq!(auth_h.1, "Bearer OPENAI_ACCESS_PLACEHOLDER");

    let acct = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("chatgpt-account-id"))
        .unwrap();
    assert_eq!(acct.1, "00000000-0000-0000-0000-000000000000");
}

#[test]
fn fetch_omits_account_id_when_entry_lacks_it() {
    use agtop_core::quota::AuthEntry;
    // Build an OpencodeAuth by hand via load_from on a synthesized file.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(
        &path,
        br#"{"openai":{"type":"oauth","access":"NO_ACCT_TOKEN"}}"#,
    )
    .unwrap();
    let auth = OpencodeAuth::load_from(&path).unwrap();

    let http = FakeHttp::new();
    http.push_ok(
        200,
        b"{\"rate_limit\":{\"primary_window\":{\"used_percent\":1.0}}}",
    );
    let _ = Codex.fetch(&auth, &http);
    let req = http.last_request().unwrap();
    assert!(req
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("chatgpt-account-id")));
    // Silence unused import warning.
    let _ = std::marker::PhantomData::<AuthEntry>;
}

#[test]
fn fetch_success_returns_usage() {
    let http = FakeHttp::new();
    http.push_ok(200, &codex_sample());
    let r = Codex.fetch(&auth_full(), &http);
    assert!(r.ok);
    assert_eq!(r.provider_id, ProviderId::Codex);
    assert!(r.usage.unwrap().windows.contains_key("5h"));
}

#[test]
fn fetch_401_returns_http_error_with_body() {
    let http = FakeHttp::new();
    http.push_ok(401, &codex_401());
    let r = Codex.fetch(&auth_full(), &http);
    assert!(!r.ok);
    let err = r.error.unwrap();
    assert!(matches!(err.kind, ErrorKind::Http { status: 401, .. }));
    assert!(err.detail.contains("signing in again"));
}

#[test]
fn fetch_429_parses_retry_after() {
    let http = FakeHttp::new();
    http.push_ok_with_headers(
        429,
        vec![("Retry-After".to_string(), "60".to_string())],
        b"rate limit",
    );
    let r = Codex.fetch(&auth_full(), &http);
    assert!(!r.ok);
    match r.error.unwrap().kind {
        ErrorKind::Http {
            status: 429,
            retry_after,
        } => assert_eq!(retry_after, Some(60)),
        other => panic!("wrong kind: {:?}", other),
    }
}

#[test]
fn fetch_transport_error_returns_transport_kind() {
    use agtop_core::quota::http::TransportError;
    let http = FakeHttp::new();
    http.push_err(TransportError::Timeout);
    let r = Codex.fetch(&auth_full(), &http);
    assert!(matches!(r.error.unwrap().kind, ErrorKind::Transport));
}

#[test]
fn fetch_not_configured_when_auth_empty() {
    let http = FakeHttp::new();
    let r = Codex.fetch(&OpencodeAuth::empty(), &http);
    assert!(!r.configured);
    assert!(matches!(r.error.unwrap().kind, ErrorKind::NotConfigured));
}
