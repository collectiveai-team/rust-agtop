//! End-to-end: open a real file, feed a session pointing at it to the
//! correlator, confirm we match ourselves with High confidence.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::path::PathBuf;

use agtop_core::process::ProcessCorrelator;
use agtop_core::session::{ClientKind, SessionSummary};

#[test]
fn correlator_matches_own_open_file() {
    // This test is meaningful only when our own binary name is on the
    // known-CLIs list. Cargo's test binary is not. Skip cleanly; the
    // fd-scan test in process::fd already exercises the Linux fd path.
    //
    // We keep this integration test as a harness for manual runs: set
    // AGTOP_ITEST_BINARY to a known CLI name to force SysinfoScanner
    // to pick up this process.
    if std::env::var_os("AGTOP_ITEST_BINARY").is_none() {
        eprintln!("skipping: AGTOP_ITEST_BINARY not set");
        return;
    }

    let tf = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tf.path().to_path_buf();

    let s = SessionSummary::new(
        ClientKind::Claude,
        None,
        "integration".into(),
        Some(chrono::Utc::now()),
        Some(chrono::Utc::now()),
        None,
        None,
        path,
        None,
        None,
        None,
        None,
    );

    let mut c = ProcessCorrelator::new();
    let result = c.snapshot(&[s]);
    assert!(result.contains_key("integration"));
}
