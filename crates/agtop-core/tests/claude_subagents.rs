//! Integration test for Claude subagent parent/child handling.
//!
//! Uses a tiny hand-rolled fixture tree under
//! `tests/fixtures/claude/<slug>/<uuid>.jsonl` + `<uuid>/subagents/*.jsonl`
//! that covers:
//!   - main-transcript per-requestId dedup (streaming rewrites)
//!   - two subagent files with distinct requestIds
//!   - parent analysis stays main-only while subagents surface via `children()`.

use std::path::PathBuf;

use agtop_core::providers::claude::ClaudeProvider;
use agtop_core::{Plan, Provider, ProviderKind, SessionSummary};

fn fixture_main_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude/project-slug/deadbeef-1234-5678-9abc-def012345678.jsonl")
}

fn summary_for_fixture() -> SessionSummary {
    SessionSummary::new(
        ProviderKind::Claude,
        None,
        "deadbeef-1234-5678-9abc-def012345678".into(),
        None,
        None,
        Some("claude-sonnet-4-5".into()),
        Some("/tmp/test".into()),
        fixture_main_path(),
        None,
        None,
        None,
        None,
    )
}

#[test]
fn parent_analysis_excludes_subagent_totals() {
    // Keep the built-in pricing path (some CI might not have a cache).
    let provider = ClaudeProvider::default();
    let summary = summary_for_fixture();

    let analysis = provider
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed");

    // Expected = main transcript only.
    //   main    : input=300 output=120 cache_read=70 cw_5m=15
    let t = &analysis.tokens;
    assert_eq!(t.input, 300, "input tokens");
    assert_eq!(t.output, 120, "output tokens");
    assert_eq!(t.cache_read, 70, "cache_read tokens");
    assert_eq!(t.cache_write_5m, 15, "cache_write_5m tokens");
    assert_eq!(t.cache_write_1h, 0);
    // Claude maps cache_read to cached_input for cost math.
    assert_eq!(t.cached_input, t.cache_read);
    assert_eq!(
        analysis.subagent_file_count, 0,
        "parent analysis stays direct-only"
    );

    let children = provider
        .children(&summary)
        .expect("children should succeed for subagent fixtures");
    assert_eq!(children.len(), 2, "subagents should surface via children()");
    assert_eq!(children[0].session_id, "agent-aaaa");
    assert_eq!(children[1].session_id, "agent-bbbb");

    // Cost must be > 0 on the retail plan with a known Claude rate card.
    assert!(!analysis.cost.included);
    assert!(
        analysis.cost.total > 0.0,
        "retail cost should be non-zero (got {})",
        analysis.cost.total
    );
}

#[test]
fn subagents_zero_when_directory_missing() {
    // A session without a sidechain directory should still analyse, with
    // `subagent_file_count == 0` and no errors. We simulate this by
    // pointing at a transcript whose uuid has no subagent dir — we reuse
    // the same fixture but lie about the session_id so the resolver looks
    // for a directory that doesn't exist.
    let provider = ClaudeProvider::default();
    let mut summary = summary_for_fixture();
    summary.session_id = "no-such-uuid-0000-0000-0000-000000000000".into();

    let analysis = provider
        .analyze(&summary, Plan::Retail)
        .expect("analyze should succeed even with no subagent dir");
    assert_eq!(analysis.subagent_file_count, 0);
    // Main-only totals — same as above minus the two sidechains.
    assert_eq!(analysis.tokens.input, 300);
    assert_eq!(analysis.tokens.output, 120);
}
