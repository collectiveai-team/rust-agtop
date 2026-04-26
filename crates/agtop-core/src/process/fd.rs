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
#[allow(dead_code)]
pub(crate) trait FdScanner {
    fn open_paths(&self, pid: u32) -> Vec<PathBuf>;
}

// ── Linux: /proc via procfs ────────────────────────────────────────────
#[cfg(target_os = "linux")]
#[allow(dead_code)]
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
// NOTE: The libproc 0.14 safe API surface is limited. The full fd-path
// enumeration requires unsafe pointer access that is forbidden at workspace
// level. This implementation uses a safe subset; fd matching falls back to
// the scoring tier on macOS until a safe libproc binding is available.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub(crate) struct MacosFdScanner;

#[cfg(target_os = "macos")]
impl FdScanner for MacosFdScanner {
    fn open_paths(&self, _pid: u32) -> Vec<PathBuf> {
        // TODO: implement using libproc safe API once available.
        // Falls back to scoring tier.
        Vec::new()
    }
}

// ── Fallback for every other target (Windows, etc.) ───────────────────
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[allow(dead_code)]
pub(crate) struct NoopFdScanner;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl FdScanner for NoopFdScanner {
    fn open_paths(&self, _pid: u32) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Return the default `FdScanner` for the current platform.
#[allow(dead_code)]
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
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Test fake backed by an injected map of pid -> paths.
    #[derive(Default)]
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
