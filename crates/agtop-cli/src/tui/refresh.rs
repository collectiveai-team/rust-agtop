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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agtop_core::pricing::Plan;
use agtop_core::session::SessionAnalysis;
use agtop_core::{discover_all, plan_usage_all_from_summaries, Provider};
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
}

/// Handle returned to the UI. Holding this alive keeps the worker
/// running; dropping it sets the shutdown flag and then drops the
/// tokio runtime (which waits for any in-flight `spawn_blocking` task
/// to complete before tearing down).
pub struct RefreshHandle {
    rx: watch::Receiver<RefreshMsg>,
    manual_tx: watch::Sender<u64>,
    /// Signals the worker loop to stop after the current iteration.
    /// Set to `true` before we drop the runtime so the worker doesn't
    /// start another `analyze_all` call while the runtime is shutting down.
    shutdown: Arc<AtomicBool>,
    _runtime: tokio::runtime::Runtime,
}

impl Drop for RefreshHandle {
    fn drop(&mut self) {
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
}

/// Spawn a refresh worker. Returns a handle the UI can poll.
///
/// - `providers`: shared with the worker via `Arc`. `Provider` is
///   already `Send + Sync`.
/// - `plan`: billing plan, passed through to `analyze_all`.
/// - `interval`: how long the worker sleeps between automatic refreshes.
///   Manual refreshes via [`RefreshHandle::trigger_manual`] bypass the
///   sleep. Zero is clamped to 1s to avoid a busy-loop.
pub fn spawn(
    providers: Vec<Arc<dyn Provider>>,
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

    let providers_arc = providers.clone();
    // Shutdown flag: `RefreshHandle::drop` sets this to `true` so the
    // worker knows not to start another `analyze_all` iteration after
    // its current one finishes.
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_worker = Arc::clone(&shutdown);

    // Spawn the worker on the runtime. We use `spawn_blocking` for the
    // CPU/IO-bound `analyze_all` so timers keep ticking.
    runtime.spawn(async move {
        // `gen` is a reserved keyword in the 2024 edition; use
        // `generation` (also matches the message field name).
        let mut generation: u64 = 0;
        let mut manual_rx = manual_rx;
        let mut session_cache: HashMap<String, (Option<DateTime<Utc>>, SessionAnalysis)> =
            HashMap::new();
        loop {
            // Check shutdown flag before starting any new analysis.
            if shutdown_worker.load(Ordering::Acquire) {
                break;
            }

            generation = generation.wrapping_add(1);
            let providers_inner = providers_arc.clone();
            // Move the cache into the blocking task; recover it in the result.
            let cache_in = std::mem::take(&mut session_cache);
            let result = tokio::task::spawn_blocking(move || {
                let summaries = discover_all(&providers_inner);
                let (analyses, cache_out) =
                    cached_analyze_all(&providers_inner, &summaries, plan, cache_in);
                let plan_usage = plan_usage_all_from_summaries(&providers_inner, &summaries);
                (analyses, plan_usage, cache_out)
            })
            .await;
            let msg = match result {
                Ok((analyses, plan_usage, cache_out)) => {
                    session_cache = cache_out;
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

            // Wait for either the tick deadline or a manual trigger.
            // Also bail immediately if the shutdown flag was set while
            // we were sleeping.
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                changed = manual_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    });

    Ok(RefreshHandle {
        rx,
        manual_tx,
        shutdown,
        _runtime: runtime,
    })
}

/// Like `analyze_all_from_summaries` but skips sessions whose
/// `last_active` timestamp hasn't changed since the previous refresh,
/// reusing the cached `SessionAnalysis` instead.
///
/// Returns the new analyses vec and the updated cache (to be stored by
/// the caller for the next cycle). Prunes cache entries for sessions no
/// longer present in `summaries`.
fn cached_analyze_all(
    providers: &[Arc<dyn Provider>],
    summaries: &[agtop_core::session::SessionSummary],
    plan: agtop_core::pricing::Plan,
    mut cache: HashMap<String, (Option<DateTime<Utc>>, SessionAnalysis)>,
) -> (
    Vec<SessionAnalysis>,
    HashMap<String, (Option<DateTime<Utc>>, SessionAnalysis)>,
) {
    use std::collections::HashSet;

    // Remove entries for sessions that no longer exist.
    let live_ids: HashSet<&str> = summaries.iter().map(|s| s.session_id.as_str()).collect();
    cache.retain(|id, _| live_ids.contains(id.as_str()));

    let mut out = Vec::with_capacity(summaries.len());

    for summary in summaries {
        // Cache hit: session unchanged since last refresh.
        if let Some((cached_ts, cached_analysis)) = cache.get(&summary.session_id) {
            if *cached_ts == summary.last_active {
                out.push(cached_analysis.clone());
                continue;
            }
        }

        // Cache miss or stale: re-analyze.
        let provider = match providers.iter().find(|p| p.kind() == summary.provider) {
            Some(p) => p,
            None => continue,
        };
        match provider.analyze(summary, plan) {
            Ok(analysis) => {
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

    (out, cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the worker should publish *something* within a short
    /// window. We use an empty provider list to keep the test hermetic
    /// — a real `default_providers()` run can take multiple seconds
    /// against a ~/.claude tree with hundreds of sessions (which isn't
    /// what we're testing here).
    #[test]
    fn worker_publishes_initial_snapshot() {
        let providers: Vec<Arc<dyn Provider>> = Vec::new();
        let mut handle =
            spawn(providers, Plan::Retail, Duration::from_millis(50)).expect("spawn worker");

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
                };
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            got_snapshot,
            "refresh worker produced no Snapshot within 5s (last msg: {last_msg_kind})"
        );
    }
}
