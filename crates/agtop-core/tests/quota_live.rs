//! Opt-in live smoke tests. Gated behind env var + `#[ignore]`.
//!
//! Run with:
//!   AGTOP_TEST_LIVE=1 cargo test -p agtop-core --test quota_live -- --ignored
//!
//! These tests hit real provider APIs with real credentials. They exist to
//! detect provider-side shape changes — failing means the provider changed
//! something, or the local credentials are stale. Never run in CI.

use agtop_core::quota::http::UreqClient;
use agtop_core::quota::providers::{claude::Claude, Provider};
use agtop_core::quota::OpencodeAuth;

fn live_enabled() -> bool {
    std::env::var("AGTOP_TEST_LIVE").ok().as_deref() == Some("1")
}

#[test]
#[ignore = "requires AGTOP_TEST_LIVE=1 and configured opencode auth for Claude"]
fn claude_live_call_returns_recognizable_shape() {
    if !live_enabled() {
        eprintln!("skipping: AGTOP_TEST_LIVE not set");
        return;
    }
    let auth = OpencodeAuth::load().expect("opencode auth.json present");
    if !Claude.is_configured(&auth) {
        eprintln!("skipping: Claude not configured in opencode auth.json");
        return;
    }

    let http = UreqClient::new();
    let r = Claude.fetch(&auth, &http);
    eprintln!("claude fetch result: ok={} error={:?}", r.ok, r.error);
    assert!(r.ok, "Claude live fetch failed: {:?}", r.error);
    let u = r.usage.expect("usage present");
    assert!(
        !u.windows.is_empty(),
        "expected at least one window (5h should always be present)"
    );
}
