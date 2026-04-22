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

#[test]
#[ignore = "requires AGTOP_TEST_LIVE=1 and configured opencode auth for OpenAI/Codex"]
fn codex_live_call_returns_recognizable_shape() {
    use agtop_core::quota::providers::codex::Codex;
    if !live_enabled() {
        eprintln!("skipping: AGTOP_TEST_LIVE not set");
        return;
    }
    let auth = OpencodeAuth::load().expect("opencode auth.json present");
    if !Codex.is_configured(&auth) {
        eprintln!("skipping: Codex not configured in opencode auth.json");
        return;
    }
    let http = UreqClient::new();
    let r = Codex.fetch(&auth, &http);
    eprintln!("codex fetch result: ok={} error={:?}", r.ok, r.error);
    // 401 is expected behaviour when session token expired; don't assert ok.
    if r.ok {
        let u = r.usage.expect("usage present");
        assert!(
            u.windows.contains_key("5h") || u.windows.contains_key("credits"),
            "expected 5h or credits window"
        );
    }
}

#[test]
#[ignore = "requires AGTOP_TEST_LIVE=1 and configured opencode auth for GitHub Copilot"]
fn copilot_live_call_returns_recognizable_shape() {
    use agtop_core::quota::providers::copilot::Copilot;
    if !live_enabled() {
        eprintln!("skipping: AGTOP_TEST_LIVE not set");
        return;
    }
    let auth = OpencodeAuth::load().expect("opencode auth.json present");
    if !Copilot.is_configured(&auth) {
        eprintln!("skipping: Copilot not configured in opencode auth.json");
        return;
    }
    let http = UreqClient::new();
    let r = Copilot.fetch(&auth, &http);
    eprintln!("copilot fetch result: ok={} error={:?}", r.ok, r.error);
    assert!(r.ok, "Copilot live fetch failed: {:?}", r.error);
    let u = r.usage.expect("usage present");
    assert!(
        !u.windows.is_empty(),
        "expected at least one window (chat/completions/premium)"
    );
}

#[test]
#[ignore = "requires AGTOP_TEST_LIVE=1 and configured opencode auth for Copilot Add-on"]
fn copilot_addon_live_call_returns_premium_only() {
    use agtop_core::quota::providers::copilot_addon::CopilotAddon;
    if !live_enabled() {
        eprintln!("skipping: AGTOP_TEST_LIVE not set");
        return;
    }
    let auth = OpencodeAuth::load().expect("opencode auth.json present");
    if !CopilotAddon.is_configured(&auth) {
        eprintln!("skipping: Copilot Add-on not configured");
        return;
    }
    let http = UreqClient::new();
    let r = CopilotAddon.fetch(&auth, &http);
    eprintln!(
        "copilot-addon fetch result: ok={} error={:?}",
        r.ok, r.error
    );
    assert!(r.ok, "Copilot Add-on live fetch failed: {:?}", r.error);
    let u = r.usage.expect("usage present");
    // Premium window must be present; no others should be.
    assert!(u.windows.contains_key("premium"));
    assert!(!u.windows.contains_key("chat"));
    assert!(!u.windows.contains_key("completions"));
}

#[test]
#[ignore = "requires AGTOP_TEST_LIVE=1 and configured opencode auth for z.ai"]
fn zai_live_call_returns_recognizable_shape() {
    use agtop_core::quota::providers::zai::Zai;
    if !live_enabled() {
        eprintln!("skipping: AGTOP_TEST_LIVE not set");
        return;
    }
    let auth = OpencodeAuth::load().expect("opencode auth.json present");
    if !Zai.is_configured(&auth) {
        eprintln!("skipping: z.ai not configured in opencode auth.json");
        return;
    }
    let http = UreqClient::new();
    let r = Zai.fetch(&auth, &http);
    eprintln!("zai fetch result: ok={} error={:?}", r.ok, r.error);
    assert!(r.ok, "z.ai live fetch failed: {:?}", r.error);
    let u = r.usage.expect("usage present");
    // At least one of: a tokens window, or the web-tools extras block.
    assert!(
        !u.windows.is_empty() || u.extras.contains_key("web-tools"),
        "expected either a window or a web-tools extras entry"
    );
}
