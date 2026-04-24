//! Background session-refresh worker.
//!
//! A dedicated tokio current-thread runtime lives off-UI-thread and
//! periodically calls `analyze_all(...)`. Each successful snapshot is
//! published on a `tokio::sync::watch` channel. Failures are surfaced on
//! a separate oneshot-ish channel so the UI can render a footer error
//! without losing the last-good snapshot.
//!
//! Why tokio? The HANDOFF.md explicitly requests it; the runtime
//! handles timers and cooperative cancellation for us. The actual work
//! (`analyze_all`) is synchronous, so we wrap it in `spawn_blocking` to
//! keep the runtime reactor unblocked.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use agtop_core::process::ProcessCorrelator;

use agtop_core::pricing::Plan;
use agtop_core::quota::ProviderResult;

const QUOTA_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
use agtop_core::session::SessionAnalysis;
use agtop_core::{discover_all, plan_usage_all_from_summaries, Client, ClientKind};
use chrono::{DateTime, Utc};
use tokio::sync::watch;

/// Message the UI consumes from the refresh task.
#[derive(Debug, Clone)]
pub enum RefreshMsg {
    /// A fresh snapshot ready to swap into the App. The `u64` is a
    /// monotonic generation counter useful for debugging / tests;
    /// prefix it with `_` to quiet the dead-code warning while
    /// preserving the field name for Debug output.
    Snapshot {
        #[allow(dead_code)]
        generation: u64,
        analyses: Vec<agtop_core::session::SessionAnalysis>,
        plan_usage: Vec<agtop_core::PlanUsage>,
    },
    /// Analysis failed; the last good snapshot (if any) stays in place.
    /// The string is already formatted for display.
    Error {
        #[allow(dead_code)]
        generation: u64,
        message: String,
    },
    /// Fresh quota results from `fetch_all`.
    QuotaSnapshot {
        #[allow(dead_code)]
        generation: u64,
        results: Vec<ProviderResult>,
    },
    /// `fetch_all` orchestration failed before any per-provider results
    /// were produced.
    QuotaError {
        #[allow(dead_code)]
        generation: u64,
        message: String,
    },
}

/// Command issued by the UI to control the quota fetch loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaCmd {
    /// The user entered the quota pane. Start auto-refresh.
    Start,
    /// The user left the quota pane. Pause auto-refresh.
    Stop,
}

/// Handle returned to the UI. Holding this alive keeps the worker
/// running; dropping it sets the shutdown flag and then drops the
/// tokio runtime (which waits for any in-flight `spawn_blocking` task
/// to complete before tearing down).
pub struct RefreshHandle {
    rx: watch::Receiver<RefreshMsg>,
    manual_tx: watch::Sender<u64>,
    quota_trigger_tx: watch::Sender<QuotaCmd>,
    /// Signals the worker loop to stop after the current iteration.
    /// Set to `true` before we drop the runtime so the worker doesn't
    /// start another `analyze_all` call while the runtime is shutting down.
    shutdown: Arc<AtomicBool>,
    _runtime: tokio::runtime::Runtime,
}

impl Drop for RefreshHandle {
    fn drop(&mut self) {
        let _ = self.quota_trigger_tx.send(QuotaCmd::Stop);
        // Signal the worker to stop after its current `spawn_blocking`
        // finishes. This prevents a new `analyze_all` from starting
        // while the runtime is being torn down.
        self.shutdown.store(true, Ordering::Release);
    }
}

impl RefreshHandle {
    /// Non-blocking peek at the latest message. Returns `None` if
    /// nothing new has arrived since the last call. Use [`watch::Receiver::has_changed`]
    /// first to avoid redundant cloning on hot paths.
    pub fn try_recv(&mut self) -> Option<RefreshMsg> {
        if self.rx.has_changed().unwrap_or(false) {
            // `borrow_and_update` marks the value seen so the next
            // `has_changed` returns false until the sender posts again.
            let msg = self.rx.borrow_and_update().clone();
            Some(msg)
        } else {
            None
        }
    }

    /// Ask the worker to run one refresh ASAP, outside the normal
    /// interval. Any in-flight refresh finishes normally; the next one
    /// fires immediately after.
    pub fn trigger_manual(&self) {
        // Incrementing any value the worker is `changed().await`-ing on
        // wakes it up. The value itself isn't meaningful.
        let v = *self.manual_tx.borrow();
        // `send` ignores channel-closed errors; the worker's end of
        // the channel is dropped only when the runtime is dropped, at
        // which point we don't care.
        let _ = self.manual_tx.send(v.wrapping_add(1));
    }

    pub fn send_quota_cmd(&self, cmd: QuotaCmd) {
        let _ = self.quota_trigger_tx.send(cmd);
    }
}

/// Hash the subset of summary fields that affects plan-usage output
/// (session_id + last_active). FxHasher is used for speed; the input
/// is in-process, so DoS resistance is unnecessary.
pub(crate) fn hash_summaries_for_cache(summaries: &[agtop_core::session::SessionSummary]) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    summaries.len().hash(&mut h);
    for s in summaries {
        s.session_id.hash(&mut h);
        // DateTime<Utc> does not implement Hash; use the unix timestamp components.
        match s.last_active {
            Some(ts) => {
                true.hash(&mut h);
                ts.timestamp().hash(&mut h);
                ts.timestamp_subsec_nanos().hash(&mut h);
            }
            None => {
                false.hash(&mut h);
            }
        }
    }
    h.finish()
}

/// Spawn a refresh worker. Returns a handle the UI can poll.
///
/// - `clients`: shared with the worker via `Arc`. `Client` is
///   already `Send + Sync`.
/// - `enabled`: shared set of enabled client kinds; the worker
///   consults this on every cycle so toggles take effect immediately.
/// - `plan`: billing plan, passed through to `analyze_all`.
/// - `interval`: how long the worker sleeps between automatic refreshes.
///   Manual refreshes via [`RefreshHandle::trigger_manual`] bypass the
///   sleep. Zero is clamped to 1s to avoid a busy-loop.
pub fn spawn(
    clients: Vec<Arc<dyn Client>>,
    enabled: Arc<RwLock<HashSet<ClientKind>>>,
    plan: Plan,
    interval: Duration,
) -> std::io::Result<RefreshHandle> {
    let interval = if interval.is_zero() {
        Duration::from_secs(1)
    } else {
        interval
    };

    // Multi-thread runtime with a single worker: a current-thread
    // runtime would only drive tasks when someone calls `block_on`,
    // which would block the UI. One dedicated worker thread is enough
    // for this workload (one periodic `analyze_all` + one manual
    // trigger channel).
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .thread_name("agtop-refresh")
        .build()?;

    // Kick off an initial "loading" state so the UI has something to
    // render before the first snapshot completes. Marking it as
    // generation 0 distinguishes it from any real snapshot.
    let initial = RefreshMsg::Error {
        generation: 0,
        message: "loading…".into(),
    };
    let (tx, rx) = watch::channel(initial);
    let (manual_tx, manual_rx) = watch::channel::<u64>(0);
    let (quota_trigger_tx, quota_trigger_rx) = watch::channel(QuotaCmd::Stop);

    let clients_arc = clients.clone();
    // Shutdown flag: `RefreshHandle::drop` sets this to `true` so the
    // worker knows not to start another `analyze_all` iteration after
    // its current one finishes.
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_worker = Arc::clone(&shutdown);
    let enabled_worker = Arc::clone(&enabled);

    // Clone tx before it is captured by the session-loop spawn below.
    // The quota loop uses this clone to publish QuotaSnapshot / QuotaError.
    let tx_quota = tx.clone();

    // Spawn the worker on the runtime. We use `spawn_blocking` for the
    // CPU/IO-bound `analyze_all` so timers keep ticking.
    runtime.spawn(async move {
        // `gen` is a reserved keyword in the 2024 edition; use
        // `generation` (also matches the message field name).
        let mut generation: u64 = 0;
        let mut manual_rx = manual_rx;
        let mut session_cache: SessionCache = HashMap::new();
        let mut project_name_cache: ProjectNameCache = HashMap::new();
        let mut plan_cache_key: Option<u64> = None;
        let mut plan_cache_val: Vec<agtop_core::PlanUsage> = Vec::new();
        let mut correlator = ProcessCorrelator::new();
        loop {
            // Check shutdown flag before starting any new analysis.
            if shutdown_worker.load(Ordering::Acquire) {
                break;
            }

            // Record when this iteration starts so we can measure work duration.
            let iter_started = std::time::Instant::now();

            // Take a local snapshot of the enabled set so the work below
            // doesn't hold the lock across .await points.
            let enabled_snap = {
                match enabled_worker.read() {
                    Ok(guard) => guard.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                }
            };
            let live: Vec<Arc<dyn Client>> = clients_arc
                .iter()
                .filter(|client| enabled_snap.contains(&client.kind()))
                .cloned()
                .collect();

            generation = generation.wrapping_add(1);
            // Move the caches into the blocking task; recover them in the result.
            let cache_in = std::mem::take(&mut session_cache);
            let project_cache_in = std::mem::take(&mut project_name_cache);
            let plan_cache_key_in = plan_cache_key;
            let plan_cache_val_in = plan_cache_val.clone();
            let result = tokio::task::spawn_blocking(move || {
                let summaries = discover_all(&live);
                let (analyses, cache_out, project_cache_out) =
                    cached_analyze_all(&live, &summaries, plan, cache_in, project_cache_in);

                let new_key = hash_summaries_for_cache(&summaries);
                let (plan_usage, new_cache_key, new_cache_val) =
                    if Some(new_key) == plan_cache_key_in {
                        // Summaries unchanged — reuse cached plan usage.
                        (
                            plan_cache_val_in.clone(),
                            plan_cache_key_in,
                            plan_cache_val_in,
                        )
                    } else {
                        let v = plan_usage_all_from_summaries(&live, &summaries);
                        (v.clone(), Some(new_key), v)
                    };

                (
                    analyses,
                    plan_usage,
                    cache_out,
                    project_cache_out,
                    new_cache_key,
                    new_cache_val,
                )
            })
            .await;
            let msg = match result {
                Ok((mut analyses, plan_usage, cache_out, project_cache_out, new_key, new_val)) => {
                    session_cache = cache_out;
                    project_name_cache = project_cache_out;
                    plan_cache_key = new_key;
                    plan_cache_val = new_val;

                    // Attach OS-process info.
                    let summaries: Vec<_> = analyses.iter().map(|a| a.summary.clone()).collect();
                    let info_map = correlator.snapshot(&summaries);
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
                Err(e) => {
                    // Cache was moved into the panicking task and is lost.
                    // session_cache is already an empty HashMap (from mem::take),
                    // so the next cycle rebuilds cleanly.
                    RefreshMsg::Error {
                        generation,
                        message: format!("analyze_all panicked: {e}"),
                    }
                }
            };
            // `send` only errors when all receivers are dropped, which
            // means the UI is gone and we should exit.
            if tx.send(msg).is_err() {
                break;
            }

            let work_elapsed = iter_started.elapsed();
            // Cap worker CPU at ~50%: sleep at least `work_elapsed * 2`,
            // but never less than the configured interval.
            let wait = interval.max(work_elapsed.saturating_mul(2));

            // Wait for either the tick deadline or a manual trigger.
            // Also bail immediately if the shutdown flag was set while
            // we were sleeping.
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                changed = manual_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // ── Quota inner loop ──────────────────────────────────────────────────────
    let mut quota_trigger_rx = quota_trigger_rx;
    let mut manual_rx_quota = manual_tx.subscribe();
    let shutdown_quota = Arc::clone(&shutdown);

    runtime.spawn(async move {
        let mut quota_generation: u64 = 0;
        loop {
            if shutdown_quota.load(Ordering::Acquire) {
                break;
            }

            // Idle: wait for Start.
            loop {
                if shutdown_quota.load(Ordering::Acquire) {
                    return;
                }
                match *quota_trigger_rx.borrow() {
                    QuotaCmd::Start => break,
                    QuotaCmd::Stop => {}
                }
                if quota_trigger_rx.changed().await.is_err() {
                    return;
                }
            }

            // Active: fetch-and-publish loop. Exits on Stop or shutdown.
            'active: loop {
                if shutdown_quota.load(Ordering::Acquire) {
                    return;
                }
                quota_generation = quota_generation.wrapping_add(1);

                let result = tokio::task::spawn_blocking(|| {
                    let config = match agtop_core::quota::QuotaConfig::load(None) {
                        Ok(c) => c,
                        Err(e) => {
                            return Err(format!("QuotaConfig::load failed: {e}"));
                        }
                    };
                    let auth = match &config.opencode_auth_path {
                        Some(p) => agtop_core::quota::OpencodeAuth::load_from(p)
                            .unwrap_or_else(|_| agtop_core::quota::OpencodeAuth::empty()),
                        None => agtop_core::quota::OpencodeAuth::load()
                            .unwrap_or_else(|_| agtop_core::quota::OpencodeAuth::empty()),
                    };
                    let http = agtop_core::quota::UreqClient::new();
                    Ok(agtop_core::quota::fetch_all(&auth, &http, &config))
                })
                .await;

                let msg = match result {
                    Ok(Ok(results)) => RefreshMsg::QuotaSnapshot {
                        generation: quota_generation,
                        results,
                    },
                    Ok(Err(err_msg)) => RefreshMsg::QuotaError {
                        generation: quota_generation,
                        message: err_msg,
                    },
                    Err(join_err) => RefreshMsg::QuotaError {
                        generation: quota_generation,
                        message: format!("quota fetch task panicked: {join_err}"),
                    },
                };
                if tx_quota.send(msg).is_err() {
                    return;
                }

                tokio::select! {
                    _ = tokio::time::sleep(QUOTA_REFRESH_INTERVAL) => {}
                    changed = quota_trigger_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        match *quota_trigger_rx.borrow() {
                            QuotaCmd::Stop => break 'active,
                            QuotaCmd::Start => {}
                        }
                    }
                    changed = manual_rx_quota.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });

    Ok(RefreshHandle {
        rx,
        manual_tx,
        quota_trigger_tx,
        shutdown,
        _runtime: runtime,
    })
}

/// Cache entry: (last_active timestamp, pre-computed analysis).
type SessionCache = HashMap<String, (Option<DateTime<Utc>>, SessionAnalysis)>;

/// Cache of resolved project names keyed by canonical cwd path.
type ProjectNameCache = HashMap<std::path::PathBuf, Option<String>>;

/// Like `analyze_all_from_summaries` but skips sessions whose
/// `last_active` timestamp hasn't changed since the previous refresh,
/// reusing the cached `SessionAnalysis` instead.
///
/// Also resolves project names via `agtop_core::project::resolve_project_name`,
/// caching results across cycles so git is only invoked when a new cwd appears.
///
/// Returns the new analyses vec and the updated caches (to be stored by
/// the caller for the next cycle). Prunes cache entries for sessions no
/// longer present in `summaries`.
fn cached_analyze_all(
    clients: &[Arc<dyn Client>],
    summaries: &[agtop_core::session::SessionSummary],
    plan: agtop_core::pricing::Plan,
    mut cache: SessionCache,
    mut project_cache: ProjectNameCache,
) -> (Vec<SessionAnalysis>, SessionCache, ProjectNameCache) {
    use std::collections::HashSet;

    // Remove entries for sessions that no longer exist.
    let live_ids: HashSet<&str> = summaries.iter().map(|s| s.session_id.as_str()).collect();
    cache.retain(|id, _| live_ids.contains(id.as_str()));

    // Pre-resolve project names for all unique cwds (cache misses only).
    for summary in summaries {
        if let Some(cwd) = &summary.cwd {
            let key = std::path::PathBuf::from(cwd);
            project_cache
                .entry(key.clone())
                .or_insert_with(|| agtop_core::project::resolve_project_name(&key));
        }
    }

    let mut out = Vec::with_capacity(summaries.len());

    for summary in summaries {
        // Cache hit: session unchanged since last refresh.
        if let Some((cached_ts, mut cached_analysis)) = cache.get(&summary.session_id).cloned() {
            if cached_ts == summary.last_active {
                // Inject the (possibly newly-resolved) project name.
                if cached_analysis.project_name.is_none() {
                    if let Some(cwd) = &summary.cwd {
                        let key = std::path::PathBuf::from(cwd);
                        cached_analysis.project_name =
                            project_cache.get(&key).and_then(|v| v.clone());
                    }
                }
                // Note: children are cached with the parent; if only a child's
                // last_active changes while the parent's is stable, the stale
                // child data is served until the parent itself is re-analyzed.
                out.push(cached_analysis);
                continue;
            }
        }

        // Cache miss or stale: re-analyze.
        let client = match clients
            .iter()
            .find(|candidate| candidate.kind() == summary.client)
        {
            Some(client) => client,
            None => continue,
        };
        match client.analyze(summary, plan) {
            Ok(mut analysis) => {
                if let Some(cwd) = &summary.cwd {
                    let key = std::path::PathBuf::from(cwd);
                    analysis.project_name = project_cache.get(&key).and_then(|v| v.clone());
                }

                match client.children(summary) {
                    Ok(child_summaries) => {
                        for child_summary in &child_summaries {
                            let child_client = clients
                                .iter()
                                .find(|candidate| candidate.kind() == child_summary.client);
                            let child_client = match child_client {
                                Some(client) => client,
                                None => {
                                    tracing::debug!(
                                        child = child_summary.session_id.as_str(),
                                        "no client for child session"
                                    );
                                    continue;
                                }
                            };
                            match child_client.analyze(child_summary, plan) {
                                Ok(child_analysis) => {
                                    analysis.children.push(child_analysis);
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        child = child_summary.session_id.as_str(),
                                        error = %e,
                                        "child analyze failed, skipping"
                                    );
                                }
                            }
                        }
                        analysis.subagent_file_count = child_summaries.len();
                    }
                    Err(e) => {
                        tracing::warn!(
                            parent = summary.session_id.as_str(),
                            error = %e,
                            "children() failed, treating as empty"
                        );
                    }
                }

                cache.insert(
                    summary.session_id.clone(),
                    (summary.last_active, analysis.clone()),
                );
                out.push(analysis);
            }
            Err(e) => tracing::warn!(
                session = summary.session_id.as_str(),
                error = %e,
                "analyze failed"
            ),
        }
    }

    (out, cache, project_cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These three quota-loop integration tests each spawn a real tokio runtime
    // that competes for blocking threads with other parallel tests.  Acquire
    // this lock at the start of each test so they run sequentially even when
    // `cargo test` uses multiple threads (the default).  This is equivalent to
    // `#[serial]` from the `serial_test` crate without adding a dependency.
    static QUOTA_LOOP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Smoke test: the worker should publish *something* within a short
    /// window. We use an empty client list to keep the test hermetic
    /// — a real `default_clients()` run can take multiple seconds
    /// against a ~/.claude tree with hundreds of sessions (which isn't
    /// what we're testing here).
    #[test]
    fn worker_publishes_initial_snapshot() {
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};
        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(
            agtop_core::ClientKind::all()
                .iter()
                .copied()
                .collect::<HashSet<_>>(),
        ));
        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(50)).expect("spawn worker");

        // Poll up to ~5s for a non-loading message.
        let start = std::time::Instant::now();
        let mut got_snapshot = false;
        let mut last_msg_kind = "(none)";
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(msg) = handle.try_recv() {
                last_msg_kind = match msg {
                    RefreshMsg::Snapshot { .. } => {
                        got_snapshot = true;
                        break;
                    }
                    RefreshMsg::Error { .. } => "error",
                    RefreshMsg::QuotaSnapshot { .. } => "quota-snapshot",
                    RefreshMsg::QuotaError { .. } => "quota-error",
                };
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            got_snapshot,
            "refresh worker produced no Snapshot within 5s (last msg: {last_msg_kind})"
        );
    }

    /// The worker must sleep at least `work_duration * 2` between cycles
    /// so it never exceeds ~50% CPU. We simulate slow work by registering
    /// a mock client whose list_sessions sleeps for 300 ms.
    #[test]
    fn adaptive_sleep_scales_with_work_duration() {
        use agtop_core::{session::SessionSummary, ClientKind};
        use std::collections::HashSet;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, RwLock};

        #[derive(Debug)]
        struct SlowClient {
            calls: Arc<AtomicUsize>,
        }
        impl Client for SlowClient {
            fn kind(&self) -> ClientKind {
                ClientKind::Claude
            }
            fn list_sessions(&self) -> agtop_core::Result<Vec<SessionSummary>> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(300));
                Ok(vec![])
            }
            fn analyze(
                &self,
                _s: &SessionSummary,
                _p: Plan,
            ) -> agtop_core::Result<agtop_core::session::SessionAnalysis> {
                unreachable!("no sessions")
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let clients: Vec<Arc<dyn Client>> = vec![Arc::new(SlowClient {
            calls: Arc::clone(&calls),
        })];
        let enabled = Arc::new(RwLock::new(
            std::iter::once(ClientKind::Claude).collect::<HashSet<_>>(),
        ));

        // Very short interval (10ms) — work time (300ms) should dominate.
        let _handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(10)).expect("spawn");

        // Run for ~1s. Without adaptive sleep: ≥3 calls (300ms each, back-to-back).
        // With adaptive sleep (wait = max(10ms, 600ms) = 600ms): ~1 call.
        std::thread::sleep(Duration::from_millis(1_000));
        let n = calls.load(Ordering::SeqCst);
        assert!(
            n <= 2,
            "worker called list_sessions {n} times in 1s — adaptive sleep is not capping work"
        );
    }

    /// The worker must consult the shared enabled set each cycle, not
    /// just at startup. Empty set → snapshot has zero sessions.
    #[test]
    fn worker_respects_empty_enabled_set() {
        use agtop_core::ClientKind;
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled: Arc<RwLock<HashSet<ClientKind>>> = Arc::new(RwLock::new(HashSet::new()));

        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(50)).expect("spawn worker");

        // Poll for up to 2s. With zero clients + empty enabled set, we
        // should still get an initial Snapshot message (an empty one).
        let start = std::time::Instant::now();
        let mut saw_empty_snapshot = false;
        while start.elapsed() < Duration::from_secs(2) {
            if let Some(RefreshMsg::Snapshot {
                analyses,
                plan_usage,
                ..
            }) = handle.try_recv()
            {
                if analyses.is_empty() && plan_usage.is_empty() {
                    saw_empty_snapshot = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_empty_snapshot, "expected empty snapshot");
    }

    #[test]
    fn plan_usage_cache_hashing_is_stable() {
        // Verify the function exists and returns a u64.
        // Verify two empty slices hash equal (deterministic).
        let h_empty1 = hash_summaries_for_cache(&[]);
        let h_empty2 = hash_summaries_for_cache(&[]);
        assert_eq!(h_empty1, h_empty2, "same input must hash equal");
    }

    #[test]
    fn cached_analyze_all_populates_children_on_cache_miss() {
        use agtop_core::session::{CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals};
        use agtop_core::ClientKind;
        use std::path::PathBuf;

        let parent_summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "parent-1".into(),
            None,
            None,
            Some("claude-3".into()),
            Some("/tmp/proj".into()),
            PathBuf::from("/tmp/proj/parent.jsonl"),
            None,
            None,
            None,
            None,
        );
        let child_summary = SessionSummary::new(
            ClientKind::Claude,
            None,
            "child-1".into(),
            None,
            None,
            Some("claude-3".into()),
            Some("/tmp/proj".into()),
            PathBuf::from("/tmp/proj/child.jsonl"),
            None,
            None,
            None,
            None,
        );

        #[derive(Debug)]
        struct MockClient {
            child: SessionSummary,
        }
        impl Client for MockClient {
            fn kind(&self) -> ClientKind {
                ClientKind::Claude
            }
            fn list_sessions(&self) -> agtop_core::Result<Vec<SessionSummary>> {
                Ok(vec![])
            }
            fn analyze(
                &self,
                summary: &SessionSummary,
                _plan: Plan,
            ) -> agtop_core::Result<SessionAnalysis> {
                Ok(SessionAnalysis::new(
                    summary.clone(),
                    TokenTotals::default(),
                    CostBreakdown::default(),
                    summary.model.clone(),
                    0,
                    None,
                    None,
                    None,
                    None,
                    None,
                ))
            }
            fn children(&self, parent: &SessionSummary) -> agtop_core::Result<Vec<SessionSummary>> {
                if parent.session_id == "parent-1" {
                    Ok(vec![self.child.clone()])
                } else {
                    Ok(vec![])
                }
            }
        }

        let client: Arc<dyn Client> = Arc::new(MockClient {
            child: child_summary,
        });
        let summaries = vec![parent_summary];
        let cache = SessionCache::new();
        let project_cache = ProjectNameCache::new();

        let (analyses, _cache_out, _project_cache_out) =
            cached_analyze_all(&[client], &summaries, Plan::Retail, cache, project_cache);

        assert_eq!(analyses.len(), 1, "should have one parent analysis");
        let parent = &analyses[0];
        assert_eq!(
            parent.children.len(),
            1,
            "parent should have one child analysis"
        );
        assert_eq!(
            parent.children[0].summary.session_id, "child-1",
            "child session_id should match"
        );
        assert_eq!(
            parent.subagent_file_count, 1,
            "subagent_file_count should equal number of children"
        );
    }

    #[test]
    fn handle_exposes_quota_trigger() {
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};
        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        let handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(100)).expect("spawn");
        handle.send_quota_cmd(QuotaCmd::Stop);
    }

    #[test]
    fn refresh_msg_has_quota_variants() {
        let _ = RefreshMsg::QuotaSnapshot {
            generation: 1,
            results: Vec::new(),
        };
        let _ = RefreshMsg::QuotaError {
            generation: 1,
            message: "boom".into(),
        };
    }

    #[test]
    fn quota_cmd_is_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<QuotaCmd>();
    }

    #[test]
    fn quota_loop_honors_manual_trigger() {
        let _lock = QUOTA_LOOP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        // Long interval → any extra snapshot must come from a manual trigger.
        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_secs(120)).expect("spawn");

        handle.send_quota_cmd(QuotaCmd::Start);

        // Consume initial snapshot.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Some(RefreshMsg::QuotaSnapshot { .. }) = handle.try_recv() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        handle.trigger_manual();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut got_second = false;
        while std::time::Instant::now() < deadline {
            if let Some(RefreshMsg::QuotaSnapshot { .. }) = handle.try_recv() {
                got_second = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            got_second,
            "manual trigger did not produce a second QuotaSnapshot"
        );
    }

    #[test]
    fn quota_loop_publishes_snapshot_after_start() {
        let _lock = QUOTA_LOOP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        // With no configured providers (default empty auth), fetch_all returns
        // an empty Vec which is still a valid QuotaSnapshot.
        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(100)).expect("spawn");

        handle.send_quota_cmd(QuotaCmd::Start);

        let start = std::time::Instant::now();
        let mut got = false;
        while start.elapsed() < Duration::from_secs(15) {
            if let Some(msg) = handle.try_recv() {
                if matches!(msg, RefreshMsg::QuotaSnapshot { .. }) {
                    got = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(got, "expected QuotaSnapshot within 15s");
    }

    #[test]
    fn quota_loop_stops_on_stop_cmd() {
        let _lock = QUOTA_LOOP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        // With no configured providers, fetch_all returns an empty QuotaSnapshot
        // very quickly. We use that to verify the Stop command halts the loop.
        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(100)).expect("spawn");

        handle.send_quota_cmd(QuotaCmd::Start);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut saw_first = false;
        while std::time::Instant::now() < deadline {
            if let Some(RefreshMsg::QuotaSnapshot { .. }) = handle.try_recv() {
                saw_first = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_first, "setup failed — no initial QuotaSnapshot");
        handle.send_quota_cmd(QuotaCmd::Stop);

        // Give the loop time to process Stop. Allow at most 1 extra snapshot
        // for the case where a fetch was already in-flight when Stop arrived.
        std::thread::sleep(Duration::from_millis(2000));
        let mut extra = 0;
        while let Some(msg) = handle.try_recv() {
            if matches!(msg, RefreshMsg::QuotaSnapshot { .. }) {
                extra += 1;
            }
        }

        // Wait another interval to confirm no further publishing occurs.
        std::thread::sleep(Duration::from_millis(2000));
        while let Some(msg) = handle.try_recv() {
            if matches!(msg, RefreshMsg::QuotaSnapshot { .. }) {
                extra += 1;
            }
        }

        assert!(
            extra <= 1,
            "stopped loop kept publishing ({extra} extra beyond allowed 1)"
        );
    }
}
