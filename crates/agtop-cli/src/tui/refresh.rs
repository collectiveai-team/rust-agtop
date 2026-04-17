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

use std::sync::Arc;
use std::time::Duration;

use agtop_core::pricing::Plan;
use agtop_core::{analyze_all, plan_usage_all, Provider};
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
/// running; dropping it shuts the runtime down cleanly via the
/// internal cancel token.
pub struct RefreshHandle {
    rx: watch::Receiver<RefreshMsg>,
    manual_tx: watch::Sender<u64>,
    _runtime: tokio::runtime::Runtime,
}

impl RefreshHandle {
    /// Non-blocking peek at the latest message. Returns `None` if
    /// nothing new has arrived since the last call. Use [`has_changed`]
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

    // Spawn the worker on the runtime. We use `spawn_blocking` for the
    // CPU/IO-bound `analyze_all` so timers keep ticking.
    runtime.spawn(async move {
        // `gen` is a reserved keyword in the 2024 edition; use
        // `generation` (also matches the message field name).
        let mut generation: u64 = 0;
        let mut manual_rx = manual_rx;
        loop {
            generation = generation.wrapping_add(1);
            let providers_inner = providers_arc.clone();
            let result = tokio::task::spawn_blocking(move || {
                let analyses = analyze_all(&providers_inner, plan);
                let plan_usage = plan_usage_all(&providers_inner);
                (analyses, plan_usage)
            })
            .await;
            let msg = match result {
                Ok((analyses, plan_usage)) => RefreshMsg::Snapshot {
                    generation,
                    analyses,
                    plan_usage,
                },
                Err(e) => RefreshMsg::Error {
                    generation,
                    message: format!("analyze_all panicked: {e}"),
                },
            };
            // `send` only errors when all receivers are dropped, which
            // means the UI is gone and we should exit.
            if tx.send(msg).is_err() {
                break;
            }

            // Wait for either the tick deadline or a manual trigger.
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
        _runtime: runtime,
    })
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
