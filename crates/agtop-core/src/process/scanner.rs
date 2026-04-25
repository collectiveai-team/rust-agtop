//! Process enumeration: narrow OS process table to candidate agent CLIs.

use std::path::PathBuf;

/// One candidate process that might be running an agent CLI.
#[allow(dead_code)]
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
        let direct = DIRECT.contains(&binary);
        // Gemini CLI runs under node; disambiguate via argv.
        let node_hosted_gemini = binary == "node" && argv.iter().any(|a| a.contains("gemini"));
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
    /// Examples kept out:
    ///   * `codex app-server`            (VSCode/IDE daemon)
    ///   * `opencode serve --port ...`   (Bun headless server)
    ///   * `claude mcp-server ...`       (MCP transport)
    ///   * `claude --input-format stream-json ...`  (IDE bridge helper)
    ///   * any `*.app/` macOS bundle path
    fn is_excluded_invocation(argv: &[String]) -> bool {
        let joined = argv.join(" ");
        let lower = joined.to_lowercase();
        // macOS .app bundle paths (Claude Desktop, Codex Desktop, etc).
        if lower.contains(".app/") || lower.contains(".app\\") {
            return true;
        }
        // Long-running server / daemon subcommands. Match as
        // whitespace-bounded tokens to avoid false positives in paths.
        const DAEMON_TOKENS: &[&str] = &[
            "app-server",
            "mcp-server",
            "mcp-stdio",
            "serve",
            "server",
            "daemon",
        ];
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
    }

    #[test]
    fn is_known_cli_rejects_random_binary() {
        assert!(!SysinfoScanner::is_known_cli("bash", &[]));
    }

    #[test]
    fn is_known_cli_rejects_daemons() {
        // codex VSCode IDE backend
        assert!(!SysinfoScanner::is_known_cli(
            "codex",
            &["codex".into(), "app-server".into(), "--analytics".into()]
        ));
        // opencode headless server
        assert!(!SysinfoScanner::is_known_cli(
            "opencode",
            &[
                "opencode".into(),
                "serve".into(),
                "--port".into(),
                "39241".into()
            ]
        ));
        // claude MCP transport
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
