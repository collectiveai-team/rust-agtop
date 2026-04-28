//! Process enumeration: narrow OS process table to candidate agent CLIs.

use std::path::PathBuf;

use crate::process::ProcessMetrics;

/// One candidate process that might be running an agent CLI.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
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
    /// Live resource metrics sampled when the candidate was enumerated.
    /// `None` if the OS could not provide the data for this process.
    pub metrics: Option<ProcessMetrics>,
}

/// True if `needle` appears in `haystack` bounded on both sides by either
/// a string boundary or a non-alphanumeric, non-`-` character. Used to
/// match shell tokens (`serve`, `app-server`, …) without firing on
/// substring hits inside paths like `/usr/share/server-data/`.
fn contains_token(haystack: &str, needle: &str) -> bool {
    let mut start = 0usize;
    while let Some(pos) = haystack[start..].find(needle) {
        let abs = start + pos;
        let before = if abs == 0 {
            None
        } else {
            haystack[..abs].chars().next_back()
        };
        let after = haystack[abs + needle.len()..].chars().next();
        let bound = |c: Option<char>| match c {
            None => true,
            Some(ch) => !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'),
        };
        if bound(before) && bound(after) {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

/// OS process enumeration.
#[allow(dead_code)]
pub(crate) trait Scanner {
    /// Re-read the OS process table. Must be called before `candidates()`
    /// each snapshot.
    fn refresh(&mut self);
    fn candidates(&self) -> &[Candidate];
}

/// Default `Scanner` backed by the `sysinfo` crate.
///
/// Only candidates whose executable name is in the known-CLIs set are
/// returned, keeping per-snapshot cost proportional to running agents
/// rather than total system processes.
#[allow(dead_code)]
pub(crate) struct SysinfoScanner {
    system: sysinfo::System,
    candidates: Vec<Candidate>,
}

#[allow(dead_code)]
impl SysinfoScanner {
    pub(crate) fn new() -> Self {
        // `new_with_specifics(ProcessRefreshKind::everything())` is called on
        // each refresh (not here) to get full metrics including disk I/O.
        // Initialize with an empty system; the heavy work happens in refresh().
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
        let direct = DIRECT.contains(&binary);
        // Gemini CLI runs under node; disambiguate via argv. The Linux
        // kernel exposes the main thread's `comm` (which Node.js renames
        // via prctl on recent versions). Two distinct forms exist in the
        // wild:
        //   * Node v25.x:  "node-MainThread"
        //   * Node v24.x:  bare "MainThread"  (no prefix)
        // Accept all three. The argv-mentions-"gemini" gate keeps random
        // node processes that renamed themselves to "MainThread" out.
        let is_node_host =
            binary == "node" || binary == "node-MainThread" || binary == "MainThread";
        let node_hosted_gemini = is_node_host && argv.iter().any(|a| a.contains("gemini"));
        if !direct && !node_hosted_gemini {
            return false;
        }
        // Reject long-running daemons / IDE bridges / desktop bundles —
        // these are not interactive sessions and would otherwise be
        // false-positive matched by cwd-tier scoring.
        if Self::is_excluded_invocation(argv) {
            return false;
        }
        true
    }

    /// Heuristics that mark a process as NOT an interactive session even
    /// though its binary name would qualify. Matched against the joined
    /// argv (case-insensitive substring).
    ///
    /// We deliberately DO NOT exclude `serve`, `server`, or `app-server`
    /// here: on Linux+VSCode setups, the user-facing OpenCode is the
    /// `opencode serve` daemon (the webview talks HTTP to it), and the
    /// user-facing Codex IDE backend is `codex app-server`. Banning those
    /// at scanner-time prevents ANY OpenCode/Codex session from ever
    /// matching a PID. The "all sessions same PID" regression for
    /// SQLite-backed clients is prevented elsewhere: `paths_for` returns
    /// an empty list for OpenCode/Antigravity (so fd-tier never fans out
    /// across sessions), and the cwd-tier's `(cwd, client)` recency
    /// dedup picks one session per (cwd, daemon-PID) pair.
    ///
    /// Examples that ARE kept out:
    ///   * `claude mcp-server ...`       (MCP transport child)
    ///   * `claude --input-format stream-json ...`  (IDE bridge helper)
    ///   * anything advertising itself as a `daemon` subcommand
    ///   * any `*.app/` macOS bundle path (desktop apps, not CLIs)
    fn is_excluded_invocation(argv: &[String]) -> bool {
        let joined = argv.join(" ");
        let lower = joined.to_lowercase();
        // macOS .app bundle paths (Claude Desktop, Codex Desktop, etc).
        if lower.contains(".app/") || lower.contains(".app\\") {
            return true;
        }
        // MCP transport children and explicit daemon subcommands. Match
        // as whitespace-bounded tokens to avoid false positives in paths.
        const DAEMON_TOKENS: &[&str] = &["mcp-server", "mcp-stdio", "daemon"];
        for tok in DAEMON_TOKENS {
            if contains_token(&lower, tok) {
                return true;
            }
        }
        // Claude IDE bridge helper: stream-json IO mode is never an
        // interactive session.
        if lower.contains("--input-format stream-json")
            || lower.contains("--input-format=stream-json")
        {
            return true;
        }
        false
    }
}

impl Scanner for SysinfoScanner {
    fn refresh(&mut self) {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        // Tell sysinfo to refresh the process list. We don't need disk IO.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );

        self.candidates.clear();
        for (pid, proc) in self.system.processes() {
            // Skip individual threads. On Linux, sysinfo enumerates each
            // thread (TID) under /proc/<pid>/task as if it were a separate
            // process. Threads that did not rename themselves via
            // prctl(PR_SET_NAME) inherit the parent's `comm`, so a single
            // multi-threaded `claude` produces multiple Candidate entries
            // with identical (binary, cwd, argv, start_time) — which the
            // score-tier sees as ties and refuses to match. We only want
            // the thread group leader (TGID == TID), which sysinfo
            // identifies as `thread_kind() == None`.
            if proc.thread_kind().is_some() {
                continue;
            }
            let raw_binary = proc.name().to_string_lossy().into_owned();
            let argv: Vec<String> = proc
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            if !Self::is_known_cli(&raw_binary, &argv) {
                continue;
            }
            // Normalize the binary name. Recent Node.js renames its main
            // thread comm via prctl, surfacing as `name()` in sysinfo.
            // Two forms seen in the wild:
            //   * Node v25.x emits "node-MainThread"
            //   * Node v24.x emits bare "MainThread"
            // The downstream score tier compares against
            // `expected_binaries(client)` which lists "node", so we
            // collapse both renames here to keep the rest of the pipeline
            // ignorant of kernel/comm quirks.
            let binary = if raw_binary == "node-MainThread" || raw_binary == "MainThread" {
                "node".to_string()
            } else {
                raw_binary
            };
            let disk = proc.disk_usage();
            let metrics = Some(ProcessMetrics {
                cpu_percent: proc.cpu_usage(),
                memory_bytes: proc.memory(),
                virtual_memory_bytes: proc.virtual_memory(),
                disk_read_bytes: disk.total_read_bytes,
                disk_written_bytes: disk.total_written_bytes,
                // Rate fields are always 0.0 here; rates are derived by ProcessCorrelator which
                // has access to prior samples and elapsed time.
                disk_read_bytes_per_sec: 0.0,
                disk_written_bytes_per_sec: 0.0,
            });
            self.candidates.push(Candidate {
                pid: pid.as_u32(),
                parent_pid: proc.parent().map(|p| p.as_u32()),
                binary,
                argv,
                cwd: proc.cwd().map(|p| p.to_path_buf()),
                start_time: proc.start_time(),
                metrics,
            });
        }
    }

    fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::SysinfoScanner;
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
                metrics: None,
            }],
        };
        s.refresh();
        assert_eq!(s.candidates().len(), 1);
        assert_eq!(s.candidates()[0].pid, 42);
    }

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
        // Recent Node.js renames its main thread comm to "node-MainThread"
        // via prctl. sysinfo surfaces that as binary; we must accept it
        // when argv discloses gemini, and reject it otherwise.
        assert!(SysinfoScanner::is_known_cli(
            "node-MainThread",
            &["node".into(), "/opt/gemini/bin/gemini".into()]
        ));
        assert!(!SysinfoScanner::is_known_cli(
            "node-MainThread",
            &["node".into(), "/home/app/server.js".into()]
        ));
    }

    /// Node.js v24.x (bundled with nvm and many distros) renames the main
    /// thread to bare `"MainThread"` — without the `node-` prefix that
    /// v25.x uses. Live evidence from a user's machine running gemini
    /// under nvm Node v24.7.0:
    ///
    /// ```text
    /// pid=1548521 ppid=618724 comm=MainThread cwd=/home/rbarriga
    ///   cmd=node /home/rbarriga/.nvm/versions/node/v24.7.0/bin/gemini
    /// ```
    ///
    /// Without this acceptance the scanner produces zero candidates for
    /// gemini-under-nvm-node-24, breaking PID matching for every Gemini
    /// session on those setups regardless of how perfect the correlator
    /// is.
    ///
    /// We still gate on argv mentioning "gemini" to avoid matching
    /// arbitrary node processes that happened to rename their main
    /// thread to `"MainThread"`.
    #[test]
    fn is_known_cli_accepts_bare_mainthread_running_gemini() {
        assert!(SysinfoScanner::is_known_cli(
            "MainThread",
            &[
                "node".into(),
                "/home/user/.nvm/versions/node/v24.7.0/bin/gemini".into()
            ]
        ));
        assert!(!SysinfoScanner::is_known_cli(
            "MainThread",
            &["node".into(), "/home/app/server.js".into()]
        ));
    }

    #[test]
    fn is_known_cli_rejects_random_binary() {
        assert!(!SysinfoScanner::is_known_cli("bash", &[]));
    }

    #[test]
    fn is_known_cli_rejects_daemons() {
        // claude MCP transport child
        assert!(!SysinfoScanner::is_known_cli(
            "claude",
            &["claude".into(), "mcp-server".into()]
        ));
        // claude stream-json IDE bridge
        assert!(!SysinfoScanner::is_known_cli(
            "claude",
            &[
                "claude".into(),
                "--output-format".into(),
                "stream-json".into(),
                "--input-format".into(),
                "stream-json".into(),
            ]
        ));
    }

    #[test]
    fn is_known_cli_accepts_user_facing_serve_and_app_server() {
        // On Linux+VSCode, the user-facing OpenCode is `opencode serve`
        // (the webview talks HTTP to it). Banning `serve` would prevent
        // ANY OpenCode session from matching. fd-tier prevents the
        // shared-DB fan-out (via `paths_for` returning empty for
        // OpenCode), and cwd-tier recency-dedup picks one session per
        // (cwd, daemon-PID) pair.
        assert!(SysinfoScanner::is_known_cli(
            "opencode",
            &[
                "opencode".into(),
                "serve".into(),
                "--port".into(),
                "39241".into()
            ]
        ));
        // Same reasoning for `codex app-server` (Codex IDE backend).
        assert!(SysinfoScanner::is_known_cli(
            "codex",
            &["codex".into(), "app-server".into(), "--analytics".into()]
        ));
    }

    #[test]
    fn is_known_cli_rejects_macos_app_bundles() {
        assert!(!SysinfoScanner::is_known_cli(
            "claude",
            &["/Applications/Claude.app/Contents/MacOS/claude".into()]
        ));
    }

    #[test]
    fn is_known_cli_accepts_session_invocations() {
        // claude resume <uuid>
        assert!(SysinfoScanner::is_known_cli(
            "claude",
            &[
                "claude".into(),
                "--resume".into(),
                "12345678-1234-1234-1234-123456789abc".into()
            ]
        ));
        // codex resume <uuid> (positional)
        assert!(SysinfoScanner::is_known_cli(
            "codex",
            &[
                "codex".into(),
                "resume".into(),
                "12345678-1234-1234-1234-123456789abc".into()
            ]
        ));
        // opencode run -s <uuid>
        assert!(SysinfoScanner::is_known_cli(
            "opencode",
            &[
                "opencode".into(),
                "run".into(),
                "-s".into(),
                "12345678-1234-1234-1234-123456789abc".into()
            ]
        ));
    }

    #[test]
    fn contains_token_respects_word_boundaries() {
        // positive
        assert!(super::contains_token("opencode serve --port 1", "serve"));
        assert!(super::contains_token("codex app-server foo", "app-server"));
        assert!(super::contains_token("daemon", "daemon"));
        // negative — token embedded in identifier or path
        assert!(!super::contains_token("/usr/share/serverpkg/x", "server"));
        assert!(!super::contains_token("preserved", "serve"));
        assert!(!super::contains_token("observer", "serve"));
    }
}
