# Session PID Tracking — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect whether the agent CLI process that produced each session is still running; display its OS PID + liveness state in the TUI and JSON output.

**Architecture:** New `agtop-core::process` module that takes a slice of `SessionSummary` and returns a `HashMap<session_id, ProcessInfo>`. Two-tier matching: fd-based (definitive, via `procfs` on Linux / `libproc` on macOS) with fallback scoring on cwd+binary+start-time. Core stays TUI-free; CLI refresh layer calls the correlator once per tick and attaches `ProcessInfo` to each `SessionAnalysis` before rendering.

**Tech Stack:** Rust 2021, sysinfo 0.38, procfs 0.18 (Linux only), libproc 0.14 (macOS only). Existing crates: ratatui, chrono, serde, rayon, tracing. Workspace lint `unsafe_code = "forbid"` — applies to our code only, crates wrap their own unsafe.

**Spec:** `docs/specs/2026-04-24-session-pid-tracking-design.md`

---

## File structure

Each file has one clear responsibility; files that change together stay together.

**Created:**
- `crates/agtop-core/src/process/mod.rs` — public API: `ProcessCorrelator`, `ProcessInfo`, `Liveness`, `Confidence`
- `crates/agtop-core/src/process/scanner.rs` — `Scanner` trait + `Candidate` struct + `SysinfoScanner` impl
- `crates/agtop-core/src/process/fd.rs` — `FdScanner` trait + `LinuxFdScanner`, `MacosFdScanner`, `NoopFdScanner`
- `crates/agtop-core/src/process/transcript_paths.rs` — per-client expected-open-paths + expected-binary-names
- `crates/agtop-core/src/process/correlator.rs` — matching algorithm, consumes Scanner + FdScanner
- `crates/agtop-core/tests/process_integration.rs` — end-to-end test using a real temp file held open by this process

**Modified:**
- `Cargo.toml` — add sysinfo / procfs / libproc to `[workspace.dependencies]`
- `crates/agtop-core/Cargo.toml` — add the three deps with cfg gates
- `crates/agtop-core/src/lib.rs` — declare `pub mod process;` and re-export the public types
- `crates/agtop-core/src/session.rs` — add `pid`, `liveness` fields to `SessionAnalysis` + update `SessionAnalysis::new`
- `crates/agtop-cli/src/tui/refresh.rs` — instantiate `ProcessCorrelator` once, call `snapshot()` per refresh, attach to analyses
- `crates/agtop-cli/src/tui/column_config.rs` — add `ColumnId::Pid` variant, label, fixed_width, description
- `crates/agtop-cli/src/tui/widgets/session_table.rs` — render `ColumnId::Pid` cell
- `crates/agtop-cli/src/tui/widgets/info_tab.rs` — handle `ColumnId::Pid` in `column_line`, add "PID" and "Match" rows
- `README.md` — document the feature + platform matrix

---

## Important conventions

- **unsafe_code = "forbid"** at workspace level — all our code MUST compile without `unsafe`. `libproc` / `procfs` wrap their own unsafe internally; we just call safe APIs.
- **Never panic in `process/`** — the correlator's public `snapshot` must never fail a refresh. Always log errors at `debug!` and return degraded data.
- **Project uses `rtk` prefix for git in some environments** — normal `cargo` and `git` commands work fine in the worktree. The plan uses plain commands.
- **Commit often** — one commit per Task (TDD red/green pair + cleanup counts as one unit).

---

## Task 1: Add dependencies

**Files:**
- Modify: `Cargo.toml` (workspace dependencies section)
- Modify: `crates/agtop-core/Cargo.toml` (crate dependencies section)

- [ ] **Step 1: Add workspace dependencies**

Edit `Cargo.toml`. Add these lines to `[workspace.dependencies]`, grouped with a comment block explaining why:

```toml
# Process enumeration for session<->OS correlation (issue #23).
# sysinfo is cross-platform; procfs/libproc give platform-specific fd
# listings that sysinfo does not expose.
sysinfo = { version = "0.38", default-features = false, features = ["system"] }
procfs = "0.18"
libproc = "0.14"
```

- [ ] **Step 2: Wire them into agtop-core**

Edit `crates/agtop-core/Cargo.toml`. Add under `[dependencies]`:

```toml
sysinfo.workspace = true

[target.'cfg(target_os = "linux")'.dependencies]
procfs.workspace = true

[target.'cfg(target_os = "macos")'.dependencies]
libproc.workspace = true
```

- [ ] **Step 3: Verify everything still builds**

Run: `cargo build -p agtop-core`
Expected: compiles cleanly (crates download + compile on first run; no warnings in our code).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/agtop-core/Cargo.toml
git commit -m "build(agtop-core): add sysinfo/procfs/libproc for #23"
```

---

## Task 2: Scaffold the process module (empty public types)

This task creates the module skeleton so subsequent tasks have something to hang tests on. No logic yet.

**Files:**
- Create: `crates/agtop-core/src/process/mod.rs`
- Modify: `crates/agtop-core/src/lib.rs`

- [ ] **Step 1: Create `crates/agtop-core/src/process/mod.rs`**

```rust
//! Session <-> OS process correlation.
//!
//! See `docs/specs/2026-04-24-session-pid-tracking-design.md`.
//!
//! Public entry point is [`ProcessCorrelator`]. Construct one once, call
//! [`ProcessCorrelator::snapshot`] per refresh with the sessions you
//! want correlated, attach the returned [`ProcessInfo`] to each
//! `SessionAnalysis` before rendering.
//!
//! This module must never panic or return `Err` from `snapshot` — it's
//! a best-effort observability feature layered on top of the core
//! session display. All errors are logged at `debug!` and degraded away.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::session::SessionSummary;

/// How certain we are about a given PID-to-session match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// The matched process was observed holding the session's transcript file open.
    High,
    /// Matched on cwd + binary + start-time overlap, unambiguously.
    Medium,
}

/// Whether the matched process is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Liveness {
    Live,
    Stopped,
}

/// Per-session OS-process information attached after correlation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub liveness: Liveness,
    pub match_confidence: Confidence,
    pub parent_pid: Option<u32>,
}

/// Correlates a set of sessions to currently-running OS processes.
pub struct ProcessCorrelator {
    // Fields added in later tasks.
    _placeholder: (),
}

impl Default for ProcessCorrelator {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessCorrelator {
    pub fn new() -> Self {
        Self { _placeholder: () }
    }

    /// Refresh OS-process state and match against the given sessions.
    /// Returns a map keyed by `session_id`; sessions with no match are
    /// absent from the map. Never panics, never returns Err.
    pub fn snapshot(&mut self, _sessions: &[SessionSummary]) -> HashMap<String, ProcessInfo> {
        HashMap::new()
    }
}
```

- [ ] **Step 2: Wire into `crates/agtop-core/src/lib.rs`**

Find the `pub mod` lines (around lines 12-22). Add immediately after `pub mod project;`:

```rust
pub mod process;
```

Find the `pub use` block (around lines 24-32). Add at the end:

```rust
pub use process::{Confidence, Liveness, ProcessCorrelator, ProcessInfo};
```

- [ ] **Step 3: Run tests to verify compile**

Run: `cargo test -p agtop-core --lib`
Expected: all existing tests still pass; no new tests yet.

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-core/src/process/mod.rs crates/agtop-core/src/lib.rs
git commit -m "feat(process): scaffold ProcessCorrelator module (#23)"
```

---

## Task 3: Scanner trait + Candidate struct (test-first)

**Files:**
- Create: `crates/agtop-core/src/process/scanner.rs`
- Modify: `crates/agtop-core/src/process/mod.rs` (declare submodule)

- [ ] **Step 1: Write the failing test**

Create `crates/agtop-core/src/process/scanner.rs`:

```rust
//! Process enumeration: narrow OS process table to candidate agent CLIs.

use std::path::PathBuf;

/// One candidate process that might be running an agent CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    /// Binary file name, e.g. `"claude"`, `"codex"`, `"node"`.
    pub binary: String,
    /// Full argv including argv[0]. Used to disambiguate wrapper
    /// launchers (gemini-cli runs under `node`).
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Process start time, unix epoch seconds.
    pub start_time: u64,
}

/// OS process enumeration.
pub(crate) trait Scanner {
    /// Re-read the OS process table. Must be called before `candidates()`
    /// each snapshot.
    fn refresh(&mut self);
    fn candidates(&self) -> &[Candidate];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manually-populated Scanner used by higher-level correlator tests.
    pub(crate) struct FakeScanner {
        pub processes: Vec<Candidate>,
    }

    impl Scanner for FakeScanner {
        fn refresh(&mut self) {}
        fn candidates(&self) -> &[Candidate] {
            &self.processes
        }
    }

    #[test]
    fn fake_scanner_returns_injected_processes() {
        let mut s = FakeScanner {
            processes: vec![Candidate {
                pid: 42,
                parent_pid: Some(1),
                binary: "claude".to_string(),
                argv: vec!["claude".to_string()],
                cwd: Some(PathBuf::from("/home/test")),
                start_time: 1700000000,
            }],
        };
        s.refresh();
        assert_eq!(s.candidates().len(), 1);
        assert_eq!(s.candidates()[0].pid, 42);
    }
}
```

Declare the submodule in `crates/agtop-core/src/process/mod.rs` by adding at the top of the file (after the module doc-comment):

```rust
pub(crate) mod scanner;
```

- [ ] **Step 2: Run test to verify it passes (it's a smoke test)**

Run: `cargo test -p agtop-core --lib process::scanner`
Expected: `fake_scanner_returns_injected_processes` PASSES.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-core/src/process/
git commit -m "feat(process): add Scanner trait + Candidate struct (#23)"
```

---

## Task 4: Implement SysinfoScanner

**Files:**
- Modify: `crates/agtop-core/src/process/scanner.rs`

- [ ] **Step 1: Add the real implementation above the `#[cfg(test)]` block**

Insert between the `trait Scanner` and `#[cfg(test)]`:

```rust
/// Default `Scanner` backed by the `sysinfo` crate.
///
/// Only candidates whose executable name is in the known-CLIs set are
/// returned, keeping per-snapshot cost proportional to running agents
/// rather than total system processes.
pub(crate) struct SysinfoScanner {
    system: sysinfo::System,
    candidates: Vec<Candidate>,
}

impl SysinfoScanner {
    pub(crate) fn new() -> Self {
        // `new_with_specifics(ProcessRefreshKind::everything())` is heavier
        // than we need — we don't want disk IO / network stats. Start
        // minimal and refresh just the process list on each call.
        let system = sysinfo::System::new();
        Self {
            system,
            candidates: Vec::new(),
        }
    }

    fn is_known_cli(binary: &str, argv: &[String]) -> bool {
        const DIRECT: &[&str] = &[
            "claude",
            "codex",
            "gemini",
            "opencode",
            "copilot",
            "gh-copilot",
            "cursor",
            "cursor-agent",
            "antigravity",
        ];
        if DIRECT.iter().any(|&known| binary == known) {
            return true;
        }
        // Gemini CLI runs under node; disambiguate via argv.
        if binary == "node" && argv.iter().any(|a| a.contains("gemini")) {
            return true;
        }
        false
    }
}

impl Scanner for SysinfoScanner {
    fn refresh(&mut self) {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        // Tell sysinfo to refresh the process list. We don't need disk IO.
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::everything());

        self.candidates.clear();
        for (pid, proc) in self.system.processes() {
            let binary = proc.name().to_string_lossy().into_owned();
            let argv: Vec<String> = proc
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            if !Self::is_known_cli(&binary, &argv) {
                continue;
            }
            self.candidates.push(Candidate {
                pid: pid.as_u32(),
                parent_pid: proc.parent().map(|p| p.as_u32()),
                binary,
                argv,
                cwd: proc.cwd().map(|p| p.to_path_buf()),
                start_time: proc.start_time(),
            });
        }
    }

    fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }
}
```

- [ ] **Step 2: Add tests for the binary filter**

Append to the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn is_known_cli_accepts_direct_binaries() {
        for binary in &["claude", "codex", "opencode", "cursor", "gemini"] {
            assert!(SysinfoScanner::is_known_cli(binary, &[]));
        }
    }

    #[test]
    fn is_known_cli_accepts_node_only_when_running_gemini() {
        assert!(SysinfoScanner::is_known_cli(
            "node",
            &["node".into(), "/opt/gemini/bin/gemini".into()]
        ));
        assert!(!SysinfoScanner::is_known_cli(
            "node",
            &["node".into(), "/home/app/server.js".into()]
        ));
    }

    #[test]
    fn is_known_cli_rejects_random_binary() {
        assert!(!SysinfoScanner::is_known_cli("bash", &[]));
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p agtop-core --lib process::scanner`
Expected: 4 tests pass (the smoke test from task 3 + 3 new filter tests).

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-core/src/process/scanner.rs
git commit -m "feat(process): add SysinfoScanner with known-CLI filter (#23)"
```

---

## Task 5: FdScanner trait + cfg-gated impls

**Files:**
- Create: `crates/agtop-core/src/process/fd.rs`
- Modify: `crates/agtop-core/src/process/mod.rs` (declare submodule)

- [ ] **Step 1: Create `crates/agtop-core/src/process/fd.rs`**

```rust
//! Per-pid open-file enumeration.
//!
//! Needed to disambiguate concurrent agents in the same working
//! directory: the CLI holds its transcript file open for the session's
//! lifetime, so the process owning that fd is definitively the one
//! running the session.

use std::path::PathBuf;

/// List the open file paths for a given PID.
///
/// Implementations MUST NOT panic. Returning an empty `Vec` is the
/// correct response to "process gone", "permission denied", and any
/// other error condition; the correlator will fall back to scoring.
pub(crate) trait FdScanner {
    fn open_paths(&self, pid: u32) -> Vec<PathBuf>;
}

// ── Linux: /proc via procfs ────────────────────────────────────────────
#[cfg(target_os = "linux")]
pub(crate) struct LinuxFdScanner;

#[cfg(target_os = "linux")]
impl FdScanner for LinuxFdScanner {
    fn open_paths(&self, pid: u32) -> Vec<PathBuf> {
        let pid_i32 = match i32::try_from(pid) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let proc = match procfs::process::Process::new(pid_i32) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(pid, error = %e, "procfs: cannot open process");
                return Vec::new();
            }
        };
        let fds = match proc.fd() {
            Ok(it) => it,
            Err(e) => {
                tracing::debug!(pid, error = %e, "procfs: cannot list fds");
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for fd in fds.flatten() {
            if let procfs::process::FDTarget::Path(p) = fd.target {
                out.push(p);
            }
        }
        out
    }
}

// ── macOS: libproc ─────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
pub(crate) struct MacosFdScanner;

#[cfg(target_os = "macos")]
impl FdScanner for MacosFdScanner {
    fn open_paths(&self, pid: u32) -> Vec<PathBuf> {
        use libproc::file_info::{pidfdinfo, ListFDs, ProcFDType};
        use libproc::proc_pid::listpidinfo;
        use libproc::net_info::VnodeFdInfoWithPath;

        let pid_i32 = match i32::try_from(pid) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let fds = match listpidinfo::<ListFDs>(pid_i32, 1024) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(pid, error = %e, "libproc: cannot list fds");
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for fd in fds {
            if fd.proc_fdtype != ProcFDType::VNode as u32 {
                continue;
            }
            match pidfdinfo::<VnodeFdInfoWithPath>(pid_i32, fd.proc_fd) {
                Ok(info) => {
                    let path_str = unsafe {
                        std::ffi::CStr::from_ptr(info.pvip.vip_path.as_ptr())
                    };
                    if let Ok(s) = path_str.to_str() {
                        if !s.is_empty() {
                            out.push(PathBuf::from(s));
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(pid, fd = fd.proc_fd, error = %e, "libproc: pidfdinfo failed");
                }
            }
        }
        out
    }
}

// ── Fallback for every other target (Windows, etc.) ───────────────────
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) struct NoopFdScanner;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl FdScanner for NoopFdScanner {
    fn open_paths(&self, _pid: u32) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Return the default `FdScanner` for the current platform.
pub(crate) fn default_fd_scanner() -> Box<dyn FdScanner + Send + Sync> {
    #[cfg(target_os = "linux")]
    {
        Box::new(LinuxFdScanner)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(MacosFdScanner)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Box::new(NoopFdScanner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Test fake backed by an injected map of pid -> paths.
    pub(crate) struct FakeFdScanner {
        pub map: HashMap<u32, Vec<PathBuf>>,
    }

    impl FdScanner for FakeFdScanner {
        fn open_paths(&self, pid: u32) -> Vec<PathBuf> {
            self.map.get(&pid).cloned().unwrap_or_default()
        }
    }

    #[test]
    fn fake_returns_injected_paths() {
        let mut m = HashMap::new();
        m.insert(
            42,
            vec![PathBuf::from("/tmp/a.jsonl"), PathBuf::from("/tmp/b.log")],
        );
        let fake = FakeFdScanner { map: m };
        assert_eq!(fake.open_paths(42).len(), 2);
        assert_eq!(fake.open_paths(99).len(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scanner_finds_own_open_file() {
        // Open a temp file and confirm LinuxFdScanner sees it on our own pid.
        let tf = tempfile::NamedTempFile::new().expect("tempfile");
        let pid = std::process::id();
        let scanner = LinuxFdScanner;
        let paths = scanner.open_paths(pid);
        let canon = std::fs::canonicalize(tf.path()).unwrap();
        let found = paths.iter().any(|p| *p == canon || *p == tf.path());
        assert!(
            found,
            "expected own open temp file {} in fd list ({} entries)",
            tf.path().display(),
            paths.len()
        );
    }
}
```

- [ ] **Step 2: Declare the submodule in `process/mod.rs`**

Add below the existing `pub(crate) mod scanner;` line:

```rust
pub(crate) mod fd;
```

- [ ] **Step 3: Run tests on Linux**

Run: `cargo test -p agtop-core --lib process::fd`
Expected (on Linux): 2 tests pass (fake + self-scan). The self-scan test proves the Linux scanner works against a live process.

On macOS this test is cfg-gated off; the macOS implementation is exercised by the integration test in Task 9.

- [ ] **Step 4: Verify the code compiles on all platforms**

Run: `cargo check -p agtop-core`
Expected: no errors, no warnings. If macOS/Linux libproc API differs from this file, fix inline based on the actual signatures reported by `cargo check`. (The `libproc` 0.14 API is stable; `pidfdinfo` and `listpidinfo` are the canonical entry points.)

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-core/src/process/fd.rs crates/agtop-core/src/process/mod.rs
git commit -m "feat(process): add FdScanner with Linux/macOS implementations (#23)"
```

---

## Task 6: transcript_paths — per-client expected open paths

**Files:**
- Create: `crates/agtop-core/src/process/transcript_paths.rs`
- Modify: `crates/agtop-core/src/process/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/agtop-core/src/process/transcript_paths.rs`:

```rust
//! Per-client knowledge: given a `SessionSummary`, which OS-level file
//! paths should a live agent process have open? And which binary names
//! does that client usually run under?
//!
//! Keeping this table next to the correlator (rather than in each client
//! parser) avoids teaching every client about OS processes.

use std::path::PathBuf;

use crate::session::{ClientKind, SessionSummary};

/// Return the set of file paths that a process running `session` is
/// expected to be holding open.
pub(crate) fn paths_for(session: &SessionSummary) -> Vec<PathBuf> {
    match session.client {
        // JSONL transcripts: the file itself is held open for writes.
        ClientKind::Claude
        | ClientKind::Codex
        | ClientKind::GeminiCli
        | ClientKind::Copilot
        | ClientKind::Cursor => vec![session.data_path.clone()],

        // SQLite-backed clients: the DB plus WAL+SHM are open while the
        // process is writing. WAL is the most reliable signal because
        // it's created the moment a write begins.
        ClientKind::OpenCode | ClientKind::Antigravity => {
            let base = session.data_path.clone();
            let mut out = Vec::with_capacity(3);
            out.push(base.clone());
            out.push(append_suffix(&base, "-wal"));
            out.push(append_suffix(&base, "-shm"));
            out
        }
    }
}

/// Return the binary names we expect for `client`. Used to boost match
/// scores in the fallback tier.
pub(crate) fn expected_binaries(client: ClientKind) -> &'static [&'static str] {
    match client {
        ClientKind::Claude => &["claude"],
        ClientKind::Codex => &["codex"],
        ClientKind::GeminiCli => &["gemini", "node"],
        ClientKind::OpenCode => &["opencode"],
        ClientKind::Copilot => &["copilot", "gh-copilot"],
        ClientKind::Cursor => &["cursor", "cursor-agent"],
        ClientKind::Antigravity => &["antigravity"],
    }
}

fn append_suffix(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn summary(client: ClientKind, data_path: &str) -> SessionSummary {
        SessionSummary::new(
            client,
            None,
            "id".into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            None,
            PathBuf::from(data_path),
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn jsonl_clients_expect_the_transcript_path_itself() {
        for client in [
            ClientKind::Claude,
            ClientKind::Codex,
            ClientKind::GeminiCli,
            ClientKind::Copilot,
            ClientKind::Cursor,
        ] {
            let s = summary(client, "/tmp/session.jsonl");
            let paths = paths_for(&s);
            assert_eq!(paths, vec![PathBuf::from("/tmp/session.jsonl")], "{:?}", client);
        }
    }

    #[test]
    fn sqlite_clients_expect_db_wal_shm_triple() {
        let s = summary(ClientKind::OpenCode, "/tmp/storage.db");
        let paths = paths_for(&s);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/storage.db"),
                PathBuf::from("/tmp/storage.db-wal"),
                PathBuf::from("/tmp/storage.db-shm"),
            ]
        );
    }

    #[test]
    fn expected_binaries_has_entry_for_every_client_kind() {
        for &client in ClientKind::all() {
            assert!(
                !expected_binaries(client).is_empty(),
                "no expected binaries for {:?}",
                client
            );
        }
    }
}
```

- [ ] **Step 2: Declare in `process/mod.rs`**

Add below the existing `pub(crate) mod fd;`:

```rust
pub(crate) mod transcript_paths;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p agtop-core --lib process::transcript_paths`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-core/src/process/transcript_paths.rs crates/agtop-core/src/process/mod.rs
git commit -m "feat(process): add per-client transcript_paths table (#23)"
```

---

## Task 7: Correlator — fd-match tier (test-first)

**Files:**
- Create: `crates/agtop-core/src/process/correlator.rs`
- Modify: `crates/agtop-core/src/process/mod.rs`

- [ ] **Step 1: Write the failing test FIRST**

Create `crates/agtop-core/src/process/correlator.rs`:

```rust
//! Matching algorithm: sessions -> running OS processes.
//!
//! Strategy:
//! 1. Fd-match tier (High confidence): build `open_path -> pid` index
//!    from candidate processes; any session whose transcript path is in
//!    the index matches unambiguously.
//! 2. Score tier (Medium confidence): for unmatched sessions, score
//!    candidates on binary + cwd + start-time and accept a unique
//!    high-score winner.

use std::collections::{HashMap, HashSet};

use crate::process::fd::FdScanner;
use crate::process::scanner::{Candidate, Scanner};
use crate::process::transcript_paths::{expected_binaries, paths_for};
use crate::process::{Confidence, Liveness, ProcessInfo};
use crate::session::SessionSummary;

/// Run one correlation pass.
///
/// Does not track prior snapshots; `Liveness` is always `Live` here.
/// Stopped-state emission lives on `ProcessCorrelator` (task 9).
pub(crate) fn correlate(
    scanner: &dyn Scanner,
    fd_scanner: &dyn FdScanner,
    sessions: &[SessionSummary],
) -> HashMap<String, ProcessInfo> {
    let candidates = scanner.candidates();
    let mut out = HashMap::new();

    // Build reverse index path -> pid, but only for paths any session
    // actually wants; this caps fd-reads at O(candidates) regardless of
    // how many files each candidate holds.
    let interesting: HashSet<std::path::PathBuf> = sessions
        .iter()
        .flat_map(paths_for)
        .collect();

    let mut path_to_pid: HashMap<std::path::PathBuf, &Candidate> = HashMap::new();
    for c in candidates {
        let paths = fd_scanner.open_paths(c.pid);
        for p in paths {
            if interesting.contains(&p) {
                path_to_pid.insert(p, c);
            }
        }
    }

    for session in sessions {
        // Fd-match tier
        let matched = paths_for(session)
            .into_iter()
            .find_map(|p| path_to_pid.get(&p).copied());
        if let Some(c) = matched {
            out.insert(
                session.session_id.clone(),
                ProcessInfo {
                    pid: c.pid,
                    liveness: Liveness::Live,
                    match_confidence: Confidence::High,
                    parent_pid: c.parent_pid,
                },
            );
        }
        // Score tier added in task 8.
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::fd::tests::FakeFdScanner;
    use crate::process::scanner::tests::FakeScanner;
    use crate::session::ClientKind;
    use chrono::Utc;
    use std::path::PathBuf;

    fn claude_session(id: &str, path: &str) -> SessionSummary {
        SessionSummary::new(
            ClientKind::Claude,
            None,
            id.into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from(path),
            None,
            None,
            None,
            None,
        )
    }

    fn candidate(pid: u32, binary: &str, cwd: &str) -> Candidate {
        Candidate {
            pid,
            parent_pid: Some(1),
            binary: binary.into(),
            argv: vec![binary.into()],
            cwd: Some(PathBuf::from(cwd)),
            start_time: 1700000000,
        }
    }

    #[test]
    fn fd_match_produces_high_confidence() {
        let scanner = FakeScanner {
            processes: vec![candidate(42, "claude", "/home/user/proj")],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(42u32, vec![PathBuf::from("/tmp/s1.jsonl")]);
        let fd = FakeFdScanner { map: fd_map };

        let sessions = vec![claude_session("s1", "/tmp/s1.jsonl")];
        let result = correlate(&scanner, &fd, &sessions);

        let info = result.get("s1").expect("s1 must be matched");
        assert_eq!(info.pid, 42);
        assert_eq!(info.liveness, Liveness::Live);
        assert_eq!(info.match_confidence, Confidence::High);
    }

    #[test]
    fn fd_match_disambiguates_two_claudes_in_same_cwd() {
        // Two claude processes in the same cwd. Only one holds s1.jsonl open.
        let scanner = FakeScanner {
            processes: vec![
                candidate(42, "claude", "/home/user/proj"),
                candidate(43, "claude", "/home/user/proj"),
            ],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(42u32, vec![PathBuf::from("/tmp/s1.jsonl")]);
        fd_map.insert(43u32, vec![PathBuf::from("/tmp/s2.jsonl")]);
        let fd = FakeFdScanner { map: fd_map };

        let sessions = vec![
            claude_session("s1", "/tmp/s1.jsonl"),
            claude_session("s2", "/tmp/s2.jsonl"),
        ];
        let result = correlate(&scanner, &fd, &sessions);

        assert_eq!(result.get("s1").map(|i| i.pid), Some(42));
        assert_eq!(result.get("s2").map(|i| i.pid), Some(43));
    }

    #[test]
    fn no_fd_match_yields_no_entry_yet() {
        // Score tier not wired yet; without fd match, no entry.
        let scanner = FakeScanner {
            processes: vec![candidate(42, "claude", "/home/user/proj")],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        let sessions = vec![claude_session("s1", "/tmp/s1.jsonl")];
        let result = correlate(&scanner, &fd, &sessions);
        assert!(result.is_empty());
    }
}
```

- [ ] **Step 2: Declare submodule in `process/mod.rs`**

Add below the existing submodule declarations:

```rust
pub(crate) mod correlator;
```

**Also** update the `tests` visibility for cross-module access: in `scanner.rs` change `mod tests` to `pub(crate) mod tests`, and same for `fd.rs`. This is what lets `correlator::tests` import `FakeScanner` and `FakeFdScanner`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p agtop-core --lib process::correlator`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-core/src/process/correlator.rs crates/agtop-core/src/process/mod.rs crates/agtop-core/src/process/scanner.rs crates/agtop-core/src/process/fd.rs
git commit -m "feat(process): correlator fd-match tier (#23)"
```

---

## Task 8: Correlator — score tier fallback

**Files:**
- Modify: `crates/agtop-core/src/process/correlator.rs`

- [ ] **Step 1: Add the scoring logic to `correlate`**

Replace the `// Score tier added in task 8.` comment in `correlate` with the real implementation. The whole for-loop body becomes:

```rust
    for session in sessions {
        // Fd-match tier
        let matched = paths_for(session)
            .into_iter()
            .find_map(|p| path_to_pid.get(&p).copied());
        if let Some(c) = matched {
            out.insert(
                session.session_id.clone(),
                ProcessInfo {
                    pid: c.pid,
                    liveness: Liveness::Live,
                    match_confidence: Confidence::High,
                    parent_pid: c.parent_pid,
                },
            );
            continue;
        }

        // Score tier: find the best candidate via cwd + binary + time.
        let mut best: Option<(u32, u32, Option<u32>)> = None; // (score, pid, parent)
        let mut tie = false;
        for c in candidates {
            let score = score_candidate(c, session);
            if score < 2 {
                continue;
            }
            match best {
                None => best = Some((score, c.pid, c.parent_pid)),
                Some((s, _, _)) if score > s => {
                    best = Some((score, c.pid, c.parent_pid));
                    tie = false;
                }
                Some((s, _, _)) if score == s => {
                    tie = true;
                }
                _ => {}
            }
        }
        if let (Some((_, pid, parent_pid)), false) = (best, tie) {
            out.insert(
                session.session_id.clone(),
                ProcessInfo {
                    pid,
                    liveness: Liveness::Live,
                    match_confidence: Confidence::Medium,
                    parent_pid,
                },
            );
        }
    }
```

- [ ] **Step 2: Add the `score_candidate` helper at module level**

Insert below `correlate`:

```rust
/// Score a candidate process against a session. Each criterion adds 1.
/// Returns 0..=3.
fn score_candidate(c: &Candidate, s: &SessionSummary) -> u32 {
    let mut score = 0;

    // Binary matches an expected name for this client.
    if expected_binaries(s.client).iter().any(|&b| c.binary == b) {
        score += 1;
    }

    // cwd exact match.
    if let (Some(cc), Some(sc)) = (&c.cwd, &s.cwd) {
        if cc.as_os_str() == std::ffi::OsStr::new(sc) {
            score += 1;
        }
    }

    // Process start time falls inside the session's observed window.
    if let (Some(started), Some(last_active)) = (s.started_at, s.last_active) {
        let started = started.timestamp() as u64;
        let last_active = last_active.timestamp() as u64;
        if c.start_time >= started && c.start_time <= last_active {
            score += 1;
        }
    }

    score
}
```

- [ ] **Step 3: Add tests for the score tier**

Append to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn score_tier_matches_medium_confidence_when_unique() {
        // No fd info. Candidate cwd + binary + start_time all line up.
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::Claude,
            None,
            "s1".into(),
            Some(now - chrono::Duration::minutes(10)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/tmp/s1.jsonl"),
            None,
            None,
            None,
            None,
        );
        let mut c = candidate(42, "claude", "/home/user/proj");
        c.start_time = (now - chrono::Duration::minutes(5)).timestamp() as u64;

        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner { map: HashMap::new() };

        let result = correlate(&scanner, &fd, &[s]);
        let info = result.get("s1").expect("s1 must be matched");
        assert_eq!(info.pid, 42);
        assert_eq!(info.match_confidence, Confidence::Medium);
    }

    #[test]
    fn score_tier_refuses_ambiguous_tie() {
        // Two candidates that score identically => no match.
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::Claude,
            None,
            "s1".into(),
            Some(now - chrono::Duration::minutes(10)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/tmp/s1.jsonl"),
            None,
            None,
            None,
            None,
        );
        let mut c1 = candidate(42, "claude", "/home/user/proj");
        let mut c2 = candidate(43, "claude", "/home/user/proj");
        c1.start_time = (now - chrono::Duration::minutes(5)).timestamp() as u64;
        c2.start_time = (now - chrono::Duration::minutes(3)).timestamp() as u64;

        let scanner = FakeScanner { processes: vec![c1, c2] };
        let fd = FakeFdScanner { map: HashMap::new() };

        let result = correlate(&scanner, &fd, &[s]);
        assert!(result.is_empty(), "ambiguous match should not be emitted");
    }

    #[test]
    fn score_tier_rejects_low_score() {
        // Only binary matches; cwd mismatch, no time overlap => score 1 => reject.
        let s = claude_session("s1", "/tmp/s1.jsonl");
        let c = candidate(42, "claude", "/elsewhere");
        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner { map: HashMap::new() };
        let result = correlate(&scanner, &fd, &[s]);
        assert!(result.is_empty());
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agtop-core --lib process::correlator`
Expected: 6 tests pass (3 original + 3 new).

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-core/src/process/correlator.rs
git commit -m "feat(process): correlator score-tier fallback (#23)"
```

---

## Task 9: Wire ProcessCorrelator::snapshot + Stopped emission

**Files:**
- Modify: `crates/agtop-core/src/process/mod.rs`
- Create: `crates/agtop-core/tests/process_integration.rs`

- [ ] **Step 1: Replace the stub ProcessCorrelator**

In `crates/agtop-core/src/process/mod.rs`, the submodule declarations (`scanner`, `fd`, `transcript_paths`, `correlator`) are already in place from Tasks 3/5/6/7. You are now replacing **only** the stub `ProcessCorrelator` struct + impl with the real one. Add these imports at the top (below the existing ones) and replace the struct + impl:

```rust
use crate::process::correlator::correlate;
use crate::process::fd::{default_fd_scanner, FdScanner};
use crate::process::scanner::{Scanner, SysinfoScanner};

// (Keep the existing Confidence / Liveness / ProcessInfo definitions unchanged.)

pub struct ProcessCorrelator {
    scanner: Box<dyn Scanner + Send + Sync>,
    fd_scanner: Box<dyn FdScanner + Send + Sync>,
    /// Previous snapshot, for emitting one transient `Stopped` frame when
    /// a matched process goes away.
    prior: HashMap<String, ProcessInfo>,
    /// Sessions that already emitted their `Stopped` frame last snapshot
    /// and should now disappear from the map.
    drop_next: std::collections::HashSet<String>,
}

impl Default for ProcessCorrelator {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessCorrelator {
    pub fn new() -> Self {
        Self::with_scanners(Box::new(SysinfoScanner::new()), default_fd_scanner())
    }

    pub(crate) fn with_scanners(
        scanner: Box<dyn Scanner + Send + Sync>,
        fd_scanner: Box<dyn FdScanner + Send + Sync>,
    ) -> Self {
        Self {
            scanner,
            fd_scanner,
            prior: HashMap::new(),
            drop_next: std::collections::HashSet::new(),
        }
    }

    pub fn snapshot(&mut self, sessions: &[SessionSummary]) -> HashMap<String, ProcessInfo> {
        self.scanner.refresh();
        let mut fresh = correlate(self.scanner.as_ref(), self.fd_scanner.as_ref(), sessions);

        // Live candidate PIDs we saw this cycle; used to decide whether a
        // previously-matched session's pid is gone.
        let alive_pids: std::collections::HashSet<u32> =
            self.scanner.candidates().iter().map(|c| c.pid).collect();

        // For sessions matched previously but not this cycle: if the prior
        // pid is no longer alive AND we haven't already sent the Stopped
        // frame, emit one Stopped frame.
        let mut new_drop_next = std::collections::HashSet::new();
        for (sid, prior_info) in &self.prior {
            if fresh.contains_key(sid) {
                continue;
            }
            if self.drop_next.contains(sid) {
                continue; // already emitted Stopped last time; drop now.
            }
            if !alive_pids.contains(&prior_info.pid) {
                fresh.insert(
                    sid.clone(),
                    ProcessInfo {
                        pid: prior_info.pid,
                        liveness: Liveness::Stopped,
                        match_confidence: prior_info.match_confidence,
                        parent_pid: prior_info.parent_pid,
                    },
                );
                new_drop_next.insert(sid.clone());
            }
        }

        self.drop_next = new_drop_next;
        self.prior = fresh.clone();
        fresh
    }
}
```

- [ ] **Step 2: Add a test for the Stopped lifecycle**

Append to the existing tests in `process/correlator.rs` (or put a new test in `process/mod.rs`'s `#[cfg(test)] mod tests`):

In `crates/agtop-core/src/process/mod.rs`, add at the end:

```rust
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::process::fd::tests::FakeFdScanner;
    use crate::process::scanner::tests::FakeScanner;
    use crate::process::scanner::Candidate;
    use crate::session::ClientKind;
    use chrono::Utc;
    use std::path::PathBuf;

    fn session(id: &str, path: &str) -> SessionSummary {
        SessionSummary::new(
            ClientKind::Claude,
            None,
            id.into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            None,
            PathBuf::from(path),
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn stopped_is_emitted_once_then_drops() {
        let sessions = vec![session("s1", "/tmp/s1.jsonl")];
        let path = PathBuf::from("/tmp/s1.jsonl");

        // Cycle 1: process 42 holds s1 open -> Live.
        let scanner = Box::new(FakeScanner {
            processes: vec![Candidate {
                pid: 42,
                parent_pid: Some(1),
                binary: "claude".into(),
                argv: vec!["claude".into()],
                cwd: None,
                start_time: 1700000000,
            }],
        });
        let mut fd_map = std::collections::HashMap::new();
        fd_map.insert(42u32, vec![path.clone()]);
        let fd = Box::new(FakeFdScanner { map: fd_map });

        let mut c = ProcessCorrelator::with_scanners(scanner, fd);
        let first = c.snapshot(&sessions);
        assert_eq!(first.get("s1").map(|i| i.liveness), Some(Liveness::Live));

        // Cycle 2: process 42 no longer in candidates -> Stopped once.
        c.scanner = Box::new(FakeScanner { processes: vec![] });
        c.fd_scanner = Box::new(FakeFdScanner {
            map: std::collections::HashMap::new(),
        });
        let second = c.snapshot(&sessions);
        assert_eq!(second.get("s1").map(|i| i.liveness), Some(Liveness::Stopped));

        // Cycle 3: dropped.
        let third = c.snapshot(&sessions);
        assert!(third.get("s1").is_none(), "stopped entry should drop on next tick");
    }
}
```

Add fields `pub scanner` and `pub fd_scanner` to `ProcessCorrelator` only if needed for the test; otherwise add a `#[cfg(test)] pub(crate) fn replace_scanners(...)` helper. The example above directly mutates the fields, which is fine inside the same module.

- [ ] **Step 3: Integration test — real process, real fd scanner (Linux/macOS)**

Create `crates/agtop-core/tests/process_integration.rs`:

```rust
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
```

This is a manual harness — the real protection comes from the unit tests in Task 5 (`linux_scanner_finds_own_open_file`) which DO run in normal `cargo test`.

- [ ] **Step 4: Run the full test suite**

Run: `cargo test -p agtop-core`
Expected: all previous tests still pass; new lifecycle test passes. Integration test is skipped (env var not set).

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-core/src/process/mod.rs crates/agtop-core/tests/process_integration.rs
git commit -m "feat(process): ProcessCorrelator with Stopped-state lifecycle (#23)"
```

---

## Task 10: Add `pid` + `liveness` fields to SessionAnalysis

**Files:**
- Modify: `crates/agtop-core/src/session.rs`

- [ ] **Step 1: Add the fields**

Locate `#[non_exhaustive]` struct `SessionAnalysis` (around lines 180-231). Add these fields just before the closing brace, keeping the `#[serde(default)]` pattern:

```rust
    /// OS PID of the agent CLI process currently running this session.
    /// `None` when no match was established.
    #[serde(default)]
    pub pid: Option<u32>,
    /// Whether the matched process is currently live or has just exited.
    /// `None` when no match was established.
    #[serde(default)]
    pub liveness: Option<crate::process::Liveness>,
```

- [ ] **Step 2: Update `SessionAnalysis::new`**

`SessionAnalysis::new` builds the struct with defaults for newer fields. Add `pid: None` and `liveness: None` alongside `children: Vec::new()` / `project_name: None`:

```rust
        Self {
            summary,
            tokens,
            cost,
            effective_model,
            subagent_file_count,
            tool_call_count,
            duration_secs,
            context_used_pct,
            context_used_tokens,
            context_window,
            children: Vec::new(),
            agent_turns: None,
            user_turns: None,
            project_name: None,
            pid: None,
            liveness: None,
        }
```

- [ ] **Step 3: Verify compile**

Run: `cargo build -p agtop-core`
Expected: compiles. If any existing construction sites break, add the two `None` fields there.

Run: `cargo test -p agtop-core`
Expected: all tests still pass (serde default keeps old JSON working).

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-core/src/session.rs
git commit -m "feat(session): add pid + liveness fields to SessionAnalysis (#23)"
```

---

## Task 11: Wire correlator into CLI refresh worker

**Files:**
- Modify: `crates/agtop-cli/src/tui/refresh.rs`

- [ ] **Step 1: Locate the analyze-all spawn_blocking in refresh.rs**

Read `crates/agtop-cli/src/tui/refresh.rs` around lines 220-305 to understand the current structure. The spawn_blocking produces `(analyses, plan_usage, cache_out, project_cache_out, new_key, new_val)`. The returned `analyses` are published directly on the watch channel.

- [ ] **Step 2: Add a module-level ProcessCorrelator**

Near the other imports (around line 25), add:

```rust
use agtop_core::process::ProcessCorrelator;
use std::sync::Mutex;
```

The worker is single-threaded per refresh loop (spawn_blocking is serialized). Still, to keep the `ProcessCorrelator` alive across iterations of the async loop, hold it in an `Arc<Mutex<ProcessCorrelator>>` owned by the loop's outer scope.

- [ ] **Step 3: Instantiate once before the loop**

Find the `loop {` that drives session refreshes. Before it, add:

```rust
let correlator = Arc::new(Mutex::new(ProcessCorrelator::new()));
```

- [ ] **Step 4: Run snapshot after analyze_all returns, before sending**

After the spawn_blocking returns `analyses` but before `RefreshMsg::Snapshot { ... }` is constructed, decorate analyses with process info:

```rust
let msg = match result {
    Ok((mut analyses, plan_usage, cache_out, project_cache_out, new_key, new_val)) => {
        session_cache = cache_out;
        project_name_cache = project_cache_out;
        plan_cache_key = new_key;
        plan_cache_val = new_val;

        // Attach OS-process info.
        let summaries: Vec<_> =
            analyses.iter().map(|a| a.summary.clone()).collect();
        let info_map = match correlator.lock() {
            Ok(mut c) => c.snapshot(&summaries),
            Err(poisoned) => poisoned.into_inner().snapshot(&summaries),
        };
        for a in &mut analyses {
            if let Some(info) = info_map.get(&a.summary.session_id) {
                a.pid = Some(info.pid);
                a.liveness = Some(info.liveness);
            }
        }

        RefreshMsg::Snapshot {
            generation,
            analyses,
            plan_usage,
        }
    }
    Err(e) => RefreshMsg::Error {
        generation,
        message: format!("analyze_all panicked: {e}"),
    },
};
```

Note: the existing analyses vec is moved into the match arm; keep the `mut` binding so we can mutate entries.

- [ ] **Step 5: Build the CLI**

Run: `cargo build -p agtop-cli`
Expected: compiles. If clippy complains about `Arc<Mutex<...>>` being overkill because the loop is single-task, that's fine — leave it because the snapshot call takes `&mut self` and the async loop borrow rules need it.

- [ ] **Step 6: Run the TUI smoke test**

Run: `cargo run -p agtop-cli -- --list`
Expected: same output as before (--list doesn't show new fields yet).

Run: `cargo run -p agtop-cli -- --json | head -50`
Expected: JSON now includes `"pid"` and `"liveness"` (possibly null) on each session object.

- [ ] **Step 7: Commit**

```bash
git add crates/agtop-cli/src/tui/refresh.rs
git commit -m "feat(cli): wire ProcessCorrelator into refresh worker (#23)"
```

---

## Task 12: Add `ColumnId::Pid` to column_config

**Files:**
- Modify: `crates/agtop-cli/src/tui/column_config.rs`

- [ ] **Step 1: Add the enum variant**

In `crates/agtop-cli/src/tui/column_config.rs`, add `Pid,` to the `ColumnId` enum (alphabetize near `Project` or append — pick based on existing order; append is safest):

```rust
pub enum ColumnId {
    // ... existing variants ...
    SessionName,
    Pid,
}
```

- [ ] **Step 2: Update `ColumnId::all()`**

Append `ColumnId::Pid,` to the array returned by `all()`.

- [ ] **Step 3: Update `label`, `description`, `fixed_width`, `sort_col`**

Add arms to each match:

```rust
// label
ColumnId::Pid => "PID",
// description
ColumnId::Pid => "OS process ID of the live agent CLI",
// fixed_width
ColumnId::Pid => Some(7),     // 6 digits + padding
// sort_col
ColumnId::Pid => None,        // not sortable
```

- [ ] **Step 4: Make Pid visible by default**

In `impl Default for ColumnConfig`, the `matches!(...)` expression lists default-visible columns. Add `ColumnId::Pid` to that list.

Also, in the test `default_visible_columns_match_design` in the same file, add `ColumnId::Pid` to the default-visible assertion block.

- [ ] **Step 5: Run tests**

Run: `cargo test -p agtop-cli --lib column_config`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/agtop-cli/src/tui/column_config.rs
git commit -m "feat(tui): add Pid column to column_config (#23)"
```

---

## Task 13: Render Pid cell in session table

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/session_table.rs`

- [ ] **Step 1: Extend the `match col_id` block**

In `row_for` (around line 286), add an arm for `ColumnId::Pid`:

```rust
ColumnId::Pid => Cell::from(
    match (a.pid, a.liveness) {
        (Some(pid), Some(agtop_core::process::Liveness::Live)) => pid.to_string(),
        (Some(pid), Some(agtop_core::process::Liveness::Stopped)) => {
            format!("{pid}†")
        }
        _ => "-".into(),
    },
),
```

The `†` dagger is a single-column marker that signals "process just exited" without adding a second cell. It's consistent with how other columns use `-` for unknown.

- [ ] **Step 2: Build and run visually**

Run: `cargo build -p agtop-cli`
Expected: clean build.

Run: `cargo run -p agtop-cli` (interactive — Ctrl-C when done)
Expected: a `PID` column is now visible. When an agent CLI is running, its PID shows; otherwise `-`.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/session_table.rs
git commit -m "feat(tui): render PID column in session table (#23)"
```

---

## Task 14: Render PID + Match rows in info panel

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/info_tab.rs`

- [ ] **Step 1: Extend `column_line`**

In `column_line` (search for `fn column_line`), add the new arm for `ColumnId::Pid`:

```rust
ColumnId::Pid => kv_line(
    "pid",
    match (a.pid, a.liveness) {
        (Some(pid), Some(agtop_core::process::Liveness::Live)) => {
            format!("{pid} (live)")
        }
        (Some(pid), Some(agtop_core::process::Liveness::Stopped)) => {
            format!("{pid} (stopped)")
        }
        _ => "-".into(),
    },
),
```

- [ ] **Step 2: Add a Match row**

After the `lines.extend(ColumnId::all().iter().map(...))` call (around line 53), add:

```rust
// Match confidence — only shown when we have a pid.
if a.pid.is_some() {
    // We only have confidence in the correlator's returned ProcessInfo,
    // not in SessionAnalysis. For now, surface liveness-derived wording.
    let match_note = match a.liveness {
        Some(agtop_core::process::Liveness::Live) => "process live",
        Some(agtop_core::process::Liveness::Stopped) => "process exited",
        None => "-",
    };
    lines.push(kv_line("match", match_note.into()));
}
```

Note: the spec mentioned showing `fd` vs `cwd+argv`. To expose that, `SessionAnalysis` would need to carry `match_confidence` too. That's a follow-up; for now liveness is enough signal in the info panel.

- [ ] **Step 3: Build + visual check**

Run: `cargo run -p agtop-cli`
Expected: select a live session → info panel shows `pid: <pid> (live)` and `match: process live`.

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/info_tab.rs
git commit -m "feat(tui): show PID + liveness in info panel (#23)"
```

---

## Task 15: Propagate match_confidence through to SessionAnalysis (scope-creep fix)

This was listed as a follow-up in Task 14 but is worth doing now so the info panel can show the spec's intended "Match: fd | cwd+argv" wording.

**Files:**
- Modify: `crates/agtop-core/src/session.rs`
- Modify: `crates/agtop-cli/src/tui/refresh.rs`
- Modify: `crates/agtop-cli/src/tui/widgets/info_tab.rs`

- [ ] **Step 1: Add field to `SessionAnalysis`**

In `crates/agtop-core/src/session.rs`, alongside `pid` and `liveness`:

```rust
    /// How we matched the PID. `None` when no match.
    #[serde(default)]
    pub match_confidence: Option<crate::process::Confidence>,
```

Update `SessionAnalysis::new` to initialize `match_confidence: None`.

- [ ] **Step 2: Populate from correlator result**

In `crates/agtop-cli/src/tui/refresh.rs`, in the decoration loop, add:

```rust
a.match_confidence = Some(info.match_confidence);
```

- [ ] **Step 3: Update the info panel "match" line**

Replace the Task 14 match-note logic with the real confidence label:

```rust
if a.pid.is_some() {
    let label = match a.match_confidence {
        Some(agtop_core::process::Confidence::High) => "fd",
        Some(agtop_core::process::Confidence::Medium) => "cwd+argv",
        None => "-",
    };
    lines.push(kv_line("match", label.into()));
}
```

- [ ] **Step 4: Build + tests**

Run: `cargo build -p agtop-cli && cargo test`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-core/src/session.rs crates/agtop-cli/src/tui/refresh.rs crates/agtop-cli/src/tui/widgets/info_tab.rs
git commit -m "feat(tui): show match confidence (fd vs cwd+argv) in info panel (#23)"
```

---

## Task 16: End-to-end verification

**Files:** none (verification only)

- [ ] **Step 1: Full test suite**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 2: Clippy clean**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. Fix any that appear; commit separately if non-trivial.

- [ ] **Step 3: Manual TUI smoke test with a live agent**

In another terminal, start an agent CLI (e.g. `claude`, `codex`) in a project directory. Let it write at least one turn.

Run: `cargo run -p agtop-cli`
Expected:
- The session for the live agent shows a PID in the PID column.
- Selecting it shows `pid: <n> (live)` and `match: fd` in the info panel.
- Kill the agent CLI. On the next refresh tick, the row shows `<pid>†` briefly, then the next tick it shows `-`.

- [ ] **Step 4: JSON output check**

Run: `cargo run -p agtop-cli -- --json | jq '.[] | select(.pid != null) | {session_id: .summary.session_id, pid, liveness, match_confidence}'`
Expected: live sessions emit `{pid, liveness: "live", match_confidence: "high"}`.

- [ ] **Step 5: Cross-platform sanity**

If macOS is available: repeat Step 3 on macOS. PID should show for a live agent; match should be `fd`.

---

## Task 17: README + platform matrix

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a Process tracking section**

Insert below the `## Supported agents` section:

```markdown
## Process tracking

When an agent CLI is running, agtop correlates its OS process to the
session transcript it's writing. The session table's `PID` column shows
the live PID; the info panel shows liveness state (`live` / `stopped`)
and how the match was established.

Correlation uses the transcript file held open by the CLI as the primary
signal; when that's unavailable, agtop falls back to scoring on
`cwd`, binary name, and start-time overlap.

### Platform matrix

| Platform | Process enum | Fd enum (definitive) | Score fallback |
|----------|:-:|:-:|:-:|
| Linux    | ✅ | ✅ (/proc)    | ✅ |
| macOS    | ✅ | ✅ (libproc)  | ✅ |
| Windows  | ✅ | ❌           | ✅ |

On Windows the score-only fallback still works but may be ambiguous
when multiple agents run in the same cwd.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): document session PID tracking (#23)"
```

---

## Task 18: Final self-review + open PR prep

- [ ] **Step 1: Ensure every spec requirement has a task**

Open `docs/specs/2026-04-24-session-pid-tracking-design.md` and check each section against this plan:

- Public API → Task 2, 9, 10, 15
- Scanner trait + sysinfo impl → Tasks 3, 4
- FdScanner trait + platform impls → Task 5
- transcript_paths → Task 6
- Correlation algorithm (both tiers + Stopped emission) → Tasks 7, 8, 9
- JSON fields → Tasks 10, 15
- TUI info panel → Tasks 14, 15
- TUI sessions-table PID column → Tasks 12, 13
- README + platform matrix → Task 17

If anything is missing, add a task.

- [ ] **Step 2: Push feature branch**

```bash
git push -u origin feature/session-pid-tracking
```

- [ ] **Step 3: Open the PR**

```bash
gh pr create --title "feat(process): track session PIDs and liveness (#23)" --body "$(cat <<'EOF'
## Summary
- Correlate agent CLI OS processes to sessions via fd-match + cwd/binary/time scoring
- Surface PID + liveness in the TUI (new column + info panel rows) and JSON output
- Works identically on Linux (procfs) and macOS (libproc); graceful degrade on Windows

## Spec
docs/specs/2026-04-24-session-pid-tracking-design.md

Closes #23
EOF
)"
```

---

## Post-plan: Self-review log (fill in during execution)

Mark the following as you complete execution:

- [ ] All 18 tasks completed
- [ ] `cargo test --workspace` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` green
- [ ] Manual smoke test with at least one agent CLI running
- [ ] PR opened and linked to #23
