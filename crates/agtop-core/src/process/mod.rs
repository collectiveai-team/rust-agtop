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

pub(crate) mod argv_uuid;
pub(crate) mod correlator;
pub(crate) mod fd;
pub(crate) mod scanner;
pub(crate) mod transcript_paths;

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::process::correlator::correlate;
use crate::process::fd::{default_fd_scanner, FdScanner};
use crate::process::scanner::{Scanner, SysinfoScanner};
use crate::session::{SessionAnalysis, SessionSummary};

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

/// Live OS resource metrics for a matched process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessMetrics {
    /// Instantaneous CPU usage as a percentage (0.0-100.0 per core, as reported by sysinfo).
    pub cpu_percent: f32,
    /// Resident set size in bytes.
    pub memory_bytes: u64,
    /// Virtual memory size in bytes.
    pub virtual_memory_bytes: u64,
    /// Cumulative bytes read from disk since process start.
    pub disk_read_bytes: u64,
    /// Cumulative bytes written to disk since process start.
    pub disk_written_bytes: u64,
    /// Bytes read from disk per second since the previous sample.
    pub disk_read_bytes_per_sec: f64,
    /// Bytes written to disk per second since the previous sample.
    pub disk_written_bytes_per_sec: f64,
}

/// Per-session OS-process information attached after correlation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub liveness: Liveness,
    pub match_confidence: Confidence,
    pub parent_pid: Option<u32>,
    /// Live resource metrics; `None` when not yet sampled or process is stopped.
    pub metrics: Option<ProcessMetrics>,
}

/// Correlates a set of sessions to currently-running OS processes.
pub struct ProcessCorrelator {
    pub(crate) scanner: Box<dyn Scanner + Send + Sync>,
    pub(crate) fd_scanner: Box<dyn FdScanner + Send + Sync>,
    /// Previous snapshot, for emitting one transient `Stopped` frame when
    /// a matched process goes away.
    prior: HashMap<String, ProcessInfo>,
    /// Sessions that already emitted their `Stopped` frame last snapshot
    /// and should now disappear from the map.
    drop_next: std::collections::HashSet<String>,
    /// Prior disk I/O counters per PID for computing per-second rates.
    disk_samples: HashMap<u32, DiskIoSample>,
}

#[derive(Debug, Clone, Copy)]
struct DiskIoSample {
    read_bytes: u64,
    written_bytes: u64,
    sampled_at: Instant,
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
            disk_samples: HashMap::new(),
        }
    }

    fn enrich_disk_rates(&mut self, fresh: &mut HashMap<String, ProcessInfo>, now: Instant) {
        let mut live_pids = std::collections::HashSet::new();

        for info in fresh.values_mut() {
            if info.liveness != Liveness::Live {
                continue;
            }
            let Some(metrics) = info.metrics.as_mut() else {
                continue;
            };

            live_pids.insert(info.pid);
            let current = DiskIoSample {
                read_bytes: metrics.disk_read_bytes,
                written_bytes: metrics.disk_written_bytes,
                sampled_at: now,
            };

            if let Some(prior) = self.disk_samples.get(&info.pid) {
                let elapsed = now.duration_since(prior.sampled_at).as_secs_f64();
                if elapsed >= 0.001
                    && current.read_bytes >= prior.read_bytes
                    && current.written_bytes >= prior.written_bytes
                {
                    metrics.disk_read_bytes_per_sec =
                        (current.read_bytes - prior.read_bytes) as f64 / elapsed;
                    metrics.disk_written_bytes_per_sec =
                        (current.written_bytes - prior.written_bytes) as f64 / elapsed;
                } else {
                    metrics.disk_read_bytes_per_sec = 0.0;
                    metrics.disk_written_bytes_per_sec = 0.0;
                }
            } else {
                metrics.disk_read_bytes_per_sec = 0.0;
                metrics.disk_written_bytes_per_sec = 0.0;
            }

            // Always update the sample, even when rates are zeroed. On PID reuse or counter
            // reset this seeds the next cycle with the current counters so rates accumulate
            // normally from that point forward.
            self.disk_samples.insert(info.pid, current);
        }

        self.disk_samples.retain(|pid, _| live_pids.contains(pid));
    }

    pub fn snapshot(&mut self, sessions: &[SessionSummary]) -> HashMap<String, ProcessInfo> {
        // Fast path: nothing to correlate — skip the OS scan entirely.
        if sessions.is_empty() && self.prior.is_empty() {
            return HashMap::new();
        }
        self.scanner.refresh();
        let mut fresh = correlate(self.scanner.as_ref(), self.fd_scanner.as_ref(), sessions);
        let now = Instant::now();
        self.enrich_disk_rates(&mut fresh, now);

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
                        metrics: None,
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

/// Attach OS-process info to a slice of session analyses, propagating
/// each parent's match to its in-process subagent children.
///
/// Subagents (Claude `Task` tool, Codex `thread_spawn`, Gemini `<parent>/`
/// subagent transcripts) execute INSIDE the parent CLI process — there
/// is no separate OS PID for a subagent. This helper writes the parent's
/// PID/liveness/confidence onto every child so the TUI and JSON output
/// can show "this subagent is running on PID X" using the only PID that
/// actually exists.
///
/// Used by both the TUI refresh worker and the `--json` one-shot.
pub fn attach_process_info(
    info_map: &HashMap<String, ProcessInfo>,
    analyses: &mut [SessionAnalysis],
) {
    for a in analyses.iter_mut() {
        if let Some(info) = info_map.get(&a.summary.session_id) {
            a.pid = Some(info.pid);
            a.liveness = Some(info.liveness);
            a.match_confidence = Some(info.match_confidence);
            a.process_metrics = info.metrics.clone();
            // Note: propagation is one level deep only. Subagents run in-process
            // within their parent CLI, so there is no deeper nesting in practice.
            for child in &mut a.children {
                child.pid = Some(info.pid);
                child.liveness = Some(info.liveness);
                child.match_confidence = Some(info.match_confidence);
                child.process_metrics = info.metrics.clone();
            }
        }
    }
}

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
        )
    }

    fn analysis(id: &str) -> SessionAnalysis {
        use crate::session::{CostBreakdown, TokenTotals};
        SessionAnalysis::new(
            session(id, &format!("/tmp/{id}.jsonl")),
            TokenTotals::default(),
            CostBreakdown::default(),
            None,
            0,
            None,
            None,
            None,
            None,
            None,
        )
    }

    #[test]
    fn attach_process_info_propagates_parent_pid_to_subagent_children() {
        // Subagents run in-process within the parent CLI; they have no
        // PID of their own. After correlation we copy the parent's
        // match (PID + liveness + confidence) onto every child so the
        // TUI can show the actual OS process.
        const FAKE_CPU: f32 = 12.5;
        const FAKE_MEM_BYTES: u64 = 64 * 1024 * 1024;
        const FAKE_VMEM_BYTES: u64 = 512 * 1024 * 1024;
        const FAKE_DISK_READ: u64 = 1_024;
        const FAKE_DISK_WRITE: u64 = 2_048;
        const FAKE_DISK_READ_RATE: f64 = 128.0;
        const FAKE_DISK_WRITE_RATE: f64 = 256.0;

        let mut parent = analysis("parent-1");
        parent.children = vec![analysis("child-1"), analysis("child-2")];
        let mut analyses = vec![parent];

        let mut info_map = HashMap::new();
        info_map.insert(
            "parent-1".to_string(),
            ProcessInfo {
                pid: 4242,
                liveness: Liveness::Live,
                match_confidence: Confidence::High,
                parent_pid: Some(1),
                metrics: Some(ProcessMetrics {
                    cpu_percent: FAKE_CPU,
                    memory_bytes: FAKE_MEM_BYTES,
                    virtual_memory_bytes: FAKE_VMEM_BYTES,
                    disk_read_bytes: FAKE_DISK_READ,
                    disk_written_bytes: FAKE_DISK_WRITE,
                    disk_read_bytes_per_sec: FAKE_DISK_READ_RATE,
                    disk_written_bytes_per_sec: FAKE_DISK_WRITE_RATE,
                }),
            },
        );

        attach_process_info(&info_map, &mut analyses);

        let p = &analyses[0];
        assert_eq!(p.pid, Some(4242));
        assert_eq!(p.liveness, Some(Liveness::Live));
        assert_eq!(p.match_confidence, Some(Confidence::High));
        assert_eq!(
            p.process_metrics.as_ref().map(|m| m.cpu_percent),
            Some(FAKE_CPU)
        );
        assert_eq!(
            p.process_metrics
                .as_ref()
                .map(|m| m.disk_read_bytes_per_sec),
            Some(FAKE_DISK_READ_RATE)
        );
        assert_eq!(
            p.process_metrics
                .as_ref()
                .map(|m| m.disk_written_bytes_per_sec),
            Some(FAKE_DISK_WRITE_RATE)
        );
        for child in &p.children {
            assert_eq!(
                child.pid,
                Some(4242),
                "child {} must inherit parent's PID",
                child.summary.session_id
            );
            assert_eq!(child.liveness, Some(Liveness::Live));
            assert_eq!(child.match_confidence, Some(Confidence::High));
            assert_eq!(
                child.process_metrics.as_ref().map(|m| m.memory_bytes),
                Some(FAKE_MEM_BYTES)
            );
            assert_eq!(
                child
                    .process_metrics
                    .as_ref()
                    .map(|m| m.disk_written_bytes_per_sec),
                Some(FAKE_DISK_WRITE_RATE)
            );
            assert_eq!(
                child
                    .process_metrics
                    .as_ref()
                    .map(|m| m.disk_read_bytes_per_sec),
                Some(FAKE_DISK_READ_RATE)
            );
        }
    }

    #[test]
    fn attach_process_info_leaves_unmatched_children_alone() {
        // No match for parent → both parent and children stay at None.
        let mut parent = analysis("parent-1");
        parent.children = vec![analysis("child-1")];
        let mut analyses = vec![parent];

        attach_process_info(&HashMap::new(), &mut analyses);

        assert!(analyses[0].pid.is_none());
        assert!(analyses[0].children[0].pid.is_none());
    }

    fn candidate_with_disk(pid: u32, path: &str, read: u64, written: u64) -> Candidate {
        Candidate {
            pid,
            parent_pid: Some(1),
            binary: "claude".into(),
            argv: vec!["claude".into()],
            cwd: None,
            start_time: 1700000000,
            metrics: Some(ProcessMetrics {
                cpu_percent: 1.0,
                memory_bytes: 2,
                virtual_memory_bytes: 3,
                disk_read_bytes: read,
                disk_written_bytes: written,
                disk_read_bytes_per_sec: 0.0,
                disk_written_bytes_per_sec: 0.0,
            }),
        }
    }

    #[test]
    fn disk_io_rates_are_zero_on_first_sample_then_derived_from_deltas() {
        const SID: &str = "22222222-2222-4222-8222-222222222222";
        let path_str = format!("/tmp/{SID}.jsonl");
        let sessions = vec![session(SID, &path_str)];
        let path = PathBuf::from(&path_str);

        let scanner = Box::new(FakeScanner {
            processes: vec![candidate_with_disk(42, &path_str, 1_000, 2_000)],
        });
        let mut fd_map = std::collections::HashMap::new();
        fd_map.insert(42u32, vec![path.clone()]);
        let fd = Box::new(FakeFdScanner { map: fd_map });

        let mut c = ProcessCorrelator::with_scanners(scanner, fd);
        let first = c.snapshot(&sessions);
        let first_metrics = first.get(SID).and_then(|i| i.metrics.as_ref()).unwrap();
        assert_eq!(first_metrics.disk_read_bytes_per_sec, 0.0);
        assert_eq!(first_metrics.disk_written_bytes_per_sec, 0.0);

        std::thread::sleep(std::time::Duration::from_millis(20));
        c.scanner = Box::new(FakeScanner {
            processes: vec![candidate_with_disk(42, &path_str, 1_200, 2_400)],
        });
        let mut fd_map = std::collections::HashMap::new();
        fd_map.insert(42u32, vec![path]);
        c.fd_scanner = Box::new(FakeFdScanner { map: fd_map });

        let second = c.snapshot(&sessions);
        let second_metrics = second.get(SID).and_then(|i| i.metrics.as_ref()).unwrap();
        assert!(second_metrics.disk_read_bytes_per_sec > 0.0);
        assert!(second_metrics.disk_written_bytes_per_sec > second_metrics.disk_read_bytes_per_sec);
    }

    #[test]
    fn disk_io_rates_are_zero_when_counters_decrease() {
        const SID: &str = "33333333-3333-4333-8333-333333333333";
        let path_str = format!("/tmp/{SID}.jsonl");
        let sessions = vec![session(SID, &path_str)];
        let path = PathBuf::from(&path_str);

        let scanner = Box::new(FakeScanner {
            processes: vec![candidate_with_disk(77, &path_str, 5_000, 8_000)],
        });
        let mut fd_map = std::collections::HashMap::new();
        fd_map.insert(77u32, vec![path.clone()]);
        let fd = Box::new(FakeFdScanner { map: fd_map });

        let mut c = ProcessCorrelator::with_scanners(scanner, fd);
        let _ = c.snapshot(&sessions);

        std::thread::sleep(std::time::Duration::from_millis(20));
        c.scanner = Box::new(FakeScanner {
            processes: vec![candidate_with_disk(77, &path_str, 4_000, 7_000)],
        });
        let mut fd_map = std::collections::HashMap::new();
        fd_map.insert(77u32, vec![path]);
        c.fd_scanner = Box::new(FakeFdScanner { map: fd_map });

        let second = c.snapshot(&sessions);
        let metrics = second.get(SID).and_then(|i| i.metrics.as_ref()).unwrap();
        assert_eq!(metrics.disk_read_bytes_per_sec, 0.0);
        assert_eq!(metrics.disk_written_bytes_per_sec, 0.0);
    }

    #[test]
    fn stopped_is_emitted_once_then_drops() {
        // Tier B (fd UUID-in-path) requires UUID-shaped session ids, so
        // we use a canonical UUID here. The lifecycle behavior under
        // test (Live -> Stopped once -> dropped) is independent of which
        // tier produced the original match.
        const SID: &str = "11111111-1111-4111-8111-111111111111";
        let path_str = format!("/tmp/{SID}.jsonl");
        let sessions = vec![session(SID, &path_str)];
        let path = PathBuf::from(&path_str);

        // Cycle 1: process 42 holds session transcript open -> Live.
        let scanner = Box::new(FakeScanner {
            processes: vec![Candidate {
                pid: 42,
                parent_pid: Some(1),
                binary: "claude".into(),
                argv: vec!["claude".into()],
                cwd: None,
                start_time: 1700000000,
                metrics: None,
            }],
        });
        let mut fd_map = std::collections::HashMap::new();
        fd_map.insert(42u32, vec![path.clone()]);
        let fd = Box::new(FakeFdScanner { map: fd_map });

        let mut c = ProcessCorrelator::with_scanners(scanner, fd);
        let first = c.snapshot(&sessions);
        assert_eq!(first.get(SID).map(|i| i.liveness), Some(Liveness::Live));

        // Cycle 2: process 42 no longer in candidates -> Stopped once.
        c.scanner = Box::new(FakeScanner { processes: vec![] });
        c.fd_scanner = Box::new(FakeFdScanner {
            map: std::collections::HashMap::new(),
        });
        let second = c.snapshot(&sessions);
        assert_eq!(second.get(SID).map(|i| i.liveness), Some(Liveness::Stopped));
        assert!(second.get(SID).and_then(|i| i.metrics.as_ref()).is_none());

        // Cycle 3: dropped.
        let third = c.snapshot(&sessions);
        assert!(
            !third.contains_key(SID),
            "stopped entry should drop on next tick"
        );
    }
}
