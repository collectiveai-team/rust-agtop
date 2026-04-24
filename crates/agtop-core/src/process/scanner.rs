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
pub(crate) mod tests {
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
