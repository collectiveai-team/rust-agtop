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
}
