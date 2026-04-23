# Phase 2 — Refresh worker extension for quota

> **For agentic workers:** Implement tasks in order. Phase 1 must be committed first.

**Goal:** Extend `refresh.rs` with a second inner loop for quota fetches: start/stop via a new `watch` channel, publish results through the existing message bus, share the existing tokio runtime.

**Spec sections covered:** "Refresh worker extension", "Pane focus wiring" (worker side only).

---

## Task 1: Add `QuotaCmd` and `RefreshMsg::QuotaSnapshot` / `QuotaError` variants

**Files:**
- Modify: `crates/agtop-cli/src/tui/refresh.rs`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block in `crates/agtop-cli/src/tui/refresh.rs`:

```rust
    #[test]
    fn refresh_msg_has_quota_variants() {
        // Compilation check: the variants exist and carry the expected data.
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
        // QuotaCmd needs Copy so watch::channel<QuotaCmd> works cleanly.
        fn assert_copy<T: Copy>() {}
        assert_copy::<QuotaCmd>();
    }
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli refresh::tests::refresh_msg_has_quota_variants`
Expected: FAIL — variants and `QuotaCmd` not defined.

- [ ] **Step 3: Add `QuotaCmd` and extend `RefreshMsg`**

At the top of `crates/agtop-cli/src/tui/refresh.rs` (after the existing imports around line 24), add:

```rust
use agtop_core::quota::ProviderResult;
```

Then, right after the `RefreshMsg` enum definition (around line 46), add:

```rust
/// Command issued by the UI to control the quota fetch loop.
///
/// Sent over a separate `watch` channel from `manual_tx` so the worker
/// can distinguish a user-initiated immediate refresh (`manual_tx`) from
/// a pane-focus lifecycle event (`quota_trigger_tx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaCmd {
    /// The user entered the quota pane. Start auto-refresh.
    Start,
    /// The user left the quota pane. Pause auto-refresh.
    Stop,
}
```

Now extend `RefreshMsg`. Replace the existing enum (around line 27-46):

```rust
#[derive(Debug, Clone)]
pub enum RefreshMsg {
    Snapshot {
        #[allow(dead_code)]
        generation: u64,
        analyses: Vec<agtop_core::session::SessionAnalysis>,
        plan_usage: Vec<agtop_core::PlanUsage>,
    },
    Error {
        #[allow(dead_code)]
        generation: u64,
        message: String,
    },
}
```

With:

```rust
#[derive(Debug, Clone)]
pub enum RefreshMsg {
    Snapshot {
        #[allow(dead_code)]
        generation: u64,
        analyses: Vec<agtop_core::session::SessionAnalysis>,
        plan_usage: Vec<agtop_core::PlanUsage>,
    },
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
    /// `fetch_all` orchestration failed (e.g. auth file unreadable) before
    /// any per-provider results were produced. Per-provider errors travel
    /// inside `QuotaSnapshot` as `ProviderResult { ok: false, ... }` and do
    /// not use this variant.
    QuotaError {
        #[allow(dead_code)]
        generation: u64,
        message: String,
    },
}
```

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli refresh::tests::refresh_msg_has_quota_variants refresh::tests::quota_cmd_is_copy`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/refresh.rs
git commit -m "quota-tui(phase-2): add QuotaCmd enum and RefreshMsg::QuotaSnapshot/QuotaError variants"
```

---

## Task 2: Add `quota_trigger_tx` to `RefreshHandle` and construct it in `spawn`

**Files:**
- Modify: `crates/agtop-cli/src/tui/refresh.rs`

- [ ] **Step 1: Write the failing test**

Append to the same `tests` module:

```rust
    #[test]
    fn handle_exposes_quota_trigger() {
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};
        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        let handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(100)).expect("spawn");
        // Just verify the method compiles and returns Ok.
        handle.send_quota_cmd(QuotaCmd::Stop);
    }
```

- [ ] **Step 2: Run tests — expect failure**

Run: `cargo test -p agtop-cli refresh::tests::handle_exposes_quota_trigger`
Expected: FAIL — `send_quota_cmd` not defined.

- [ ] **Step 3: Extend `RefreshHandle`**

Find the `RefreshHandle` struct (around line 52) and replace:

```rust
pub struct RefreshHandle {
    rx: watch::Receiver<RefreshMsg>,
    manual_tx: watch::Sender<u64>,
    /// Signals the worker loop to stop after the current iteration.
    /// Set to `true` before we drop the runtime so the worker doesn't
    /// start another `analyze_all` call while the runtime is shutting down.
    shutdown: Arc<AtomicBool>,
    _runtime: tokio::runtime::Runtime,
}
```

With:

```rust
pub struct RefreshHandle {
    rx: watch::Receiver<RefreshMsg>,
    manual_tx: watch::Sender<u64>,
    /// Tells the quota inner loop to start/stop auto-refreshing.
    quota_trigger_tx: watch::Sender<QuotaCmd>,
    /// Signals the worker loop to stop after the current iteration.
    /// Set to `true` before we drop the runtime so the worker doesn't
    /// start another `analyze_all` call while the runtime is shutting down.
    shutdown: Arc<AtomicBool>,
    _runtime: tokio::runtime::Runtime,
}
```

Update `Drop for RefreshHandle` (around line 62) to also send `Stop` before setting shutdown, so the quota loop exits the select cleanly:

```rust
impl Drop for RefreshHandle {
    fn drop(&mut self) {
        // Tell the quota loop to exit its select. Ignore errors — if
        // receivers are gone the worker is already tearing down.
        let _ = self.quota_trigger_tx.send(QuotaCmd::Stop);
        // Signal all loops to stop after their current iteration.
        self.shutdown.store(true, Ordering::Release);
    }
}
```

Add a public method on `impl RefreshHandle` (around line 88 after `trigger_manual`):

```rust
    /// Send a quota command to the worker. Starts / stops the quota
    /// auto-refresh loop. Errors are ignored — channel closure means the
    /// worker has shut down and the command is moot.
    pub fn send_quota_cmd(&self, cmd: QuotaCmd) {
        let _ = self.quota_trigger_tx.send(cmd);
    }
```

Now update `spawn` to create the trigger channel and include it in the returned `RefreshHandle`. Find the return block (around line 290) and replace:

```rust
    Ok(RefreshHandle {
        rx,
        manual_tx,
        shutdown,
        _runtime: runtime,
    })
```

With:

```rust
    let (quota_trigger_tx, _quota_trigger_rx_unused) = watch::channel(QuotaCmd::Stop);
    // NOTE: the quota inner loop (added below) takes a `subscribe()`-derived
    // receiver so multiple worker tasks could observe the same stream if needed.

    Ok(RefreshHandle {
        rx,
        manual_tx,
        quota_trigger_tx,
        shutdown,
        _runtime: runtime,
    })
```

Also add `QuotaCmd` to the imports in the test module header (line 172 inside `mod tests`) — append:

```rust
    use super::QuotaCmd;
```

Wait — `tests` is already `use super::*;`, so this is free. No extra import needed.

- [ ] **Step 4: Run tests — expect success**

Run: `cargo test -p agtop-cli refresh::tests::handle_exposes_quota_trigger`
Expected: PASS.

Also run the full suite to make sure the existing tests still pass:

Run: `cargo test -p agtop-cli`
Expected: PASS (including worker_publishes_initial_snapshot etc.).

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/refresh.rs
git commit -m "quota-tui(phase-2): add quota_trigger_tx channel to RefreshHandle"
```

---

## Task 3: Implement the quota inner loop

**Files:**
- Modify: `crates/agtop-cli/src/tui/refresh.rs`

- [ ] **Step 1: Understand what we're building**

The existing `spawn` function launches a single async task that runs the session-analysis loop. We need a SECOND async task on the same runtime that:

1. Waits for `QuotaCmd::Start` on `quota_trigger_rx`.
2. Runs `quota::fetch_all` in `spawn_blocking`.
3. Publishes a `QuotaSnapshot` (or `QuotaError`) on the existing `tx` sender.
4. Enters a `select!` with: timer (60s), trigger change (Stop ⇒ break inner loop; Start ⇒ immediate refetch), manual_rx, shutdown.

The `tx` is a `watch::Sender<RefreshMsg>`; `watch` senders can be `clone()`d. Clone it for the new task.

- [ ] **Step 2: Write the failing test**

Append to the `tests` module:

```rust
    /// With a Fake HttpClient configured to return 200 OK for all
    /// providers, starting the quota loop should yield a QuotaSnapshot
    /// within a short window. (We don't care about the inner Vec<ProviderResult>
    /// contents — those are tested in agtop-core.)
    ///
    /// Hermetic setup: no opencode auth file on disk; all providers report
    /// NotConfigured and are filtered out. The result is an empty Vec<ProviderResult>.
    /// That's still a QuotaSnapshot, which is what we're asserting.
    #[test]
    fn quota_loop_publishes_snapshot_after_start() {
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        // Point AGTOP_QUOTA_OPENCODE_AUTH_PATH at a nonexistent file so
        // OpencodeAuth::load returns NotFound → treated as empty auth.
        // All providers are unconfigured → fetch_all returns Vec::new().
        let nonexistent = std::env::temp_dir().join("agtop-refresh-test-no-auth.json");
        let _ = std::fs::remove_file(&nonexistent);
        std::env::set_var("AGTOP_QUOTA_OPENCODE_AUTH_PATH", &nonexistent);

        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(100)).expect("spawn");

        handle.send_quota_cmd(QuotaCmd::Start);

        let start = std::time::Instant::now();
        let mut got = false;
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(msg) = handle.try_recv() {
                if matches!(msg, RefreshMsg::QuotaSnapshot { .. }) {
                    got = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        std::env::remove_var("AGTOP_QUOTA_OPENCODE_AUTH_PATH");
        assert!(got, "expected QuotaSnapshot within 5s");
    }

    #[test]
    fn quota_loop_stops_on_stop_cmd() {
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        let nonexistent = std::env::temp_dir().join("agtop-refresh-test-stop.json");
        let _ = std::fs::remove_file(&nonexistent);
        std::env::set_var("AGTOP_QUOTA_OPENCODE_AUTH_PATH", &nonexistent);

        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_millis(100)).expect("spawn");

        // Start, consume the initial snapshot, then stop.
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

        // Wait longer than the quota interval; no new QuotaSnapshot should appear.
        std::thread::sleep(Duration::from_millis(1500));
        let mut extra = 0;
        while let Some(msg) = handle.try_recv() {
            if matches!(msg, RefreshMsg::QuotaSnapshot { .. }) {
                extra += 1;
            }
        }

        std::env::remove_var("AGTOP_QUOTA_OPENCODE_AUTH_PATH");
        assert_eq!(extra, 0, "stopped loop kept publishing ({extra} extra)");
    }
```

NOTE: these tests set a process-wide env var. Run them with `--test-threads=1` if they interfere with other tests in the same binary. For now they use unique file names to avoid collisions; Cargo's default test parallelism will generally be fine because the env var manipulation is bracketed.

- [ ] **Step 3: Run tests — expect failure**

Run: `cargo test -p agtop-cli refresh::tests::quota_loop -- --test-threads=1`
Expected: FAIL — no QuotaSnapshot produced.

- [ ] **Step 4: Implement the quota inner loop**

Add this constant near the top of `crates/agtop-cli/src/tui/refresh.rs` (after existing imports, around line 20):

```rust
/// Default quota refresh interval. Not configurable in v1 per the spec.
const QUOTA_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
```

Inside `spawn()`, after the existing `runtime.spawn(async move { ... })` block but BEFORE the `Ok(RefreshHandle { ... })` return, add:

```rust
    // ── Quota inner loop ────────────────────────────────────────────────
    //
    // Independent from the session analysis loop. Drives quota::fetch_all
    // on-demand when the user enters the quota pane, then every
    // QUOTA_REFRESH_INTERVAL while the pane stays active.
    //
    // Channels:
    //   - quota_trigger_rx (new, subscribed here): Start/Stop pane focus.
    //   - manual_rx_quota (clone of the session loop's manual channel):
    //     shares the `r`-key refresh trigger.
    //   - tx_quota (clone of the session snapshot tx): publishes
    //     QuotaSnapshot / QuotaError on the same message bus the UI
    //     already drains.
    let tx_quota = tx.clone();
    let mut quota_trigger_rx = quota_trigger_tx.subscribe();
    let mut manual_rx_quota = manual_tx.subscribe();
    let shutdown_quota = Arc::clone(&shutdown);

    runtime.spawn(async move {
        let mut quota_generation: u64 = 0;
        loop {
            if shutdown_quota.load(Ordering::Acquire) {
                break;
            }

            // Idle: wait for a Start command.
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

                // Spawn blocking quota::fetch_all. Load config+auth inside
                // the blocking task to avoid filesystem work on the reactor.
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

                // Wait for the next cycle.
                tokio::select! {
                    _ = tokio::time::sleep(QUOTA_REFRESH_INTERVAL) => {
                        // Loop: natural refresh.
                    }
                    changed = quota_trigger_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        match *quota_trigger_rx.borrow() {
                            QuotaCmd::Stop => break 'active,
                            QuotaCmd::Start => {
                                // Repeat-Start = immediate re-fetch.
                            }
                        }
                    }
                    changed = manual_rx_quota.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // Manual refresh: fall through to the top of the loop.
                    }
                }
            }
        }
    });
```

- [ ] **Step 5: Run tests — expect success**

Run: `cargo test -p agtop-cli refresh::tests::quota_loop -- --test-threads=1`
Expected: PASS.

Run full suite:
Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

The `--test-threads=1` is required because the quota tests manipulate a process-wide env var.

- [ ] **Step 6: Commit**

```bash
git add crates/agtop-cli/src/tui/refresh.rs
git commit -m "quota-tui(phase-2): implement quota inner loop with Start/Stop lifecycle"
```

---

## Task 4: Manual-refresh integration test

**Files:**
- Modify: `crates/agtop-cli/src/tui/refresh.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module:

```rust
    #[test]
    fn quota_loop_honors_manual_trigger() {
        use std::collections::HashSet;
        use std::sync::{Arc, RwLock};

        let nonexistent = std::env::temp_dir().join("agtop-refresh-test-manual.json");
        let _ = std::fs::remove_file(&nonexistent);
        std::env::set_var("AGTOP_QUOTA_OPENCODE_AUTH_PATH", &nonexistent);

        let clients: Vec<Arc<dyn Client>> = Vec::new();
        let enabled = Arc::new(RwLock::new(HashSet::new()));
        // Long interval → any extra snapshot must come from a manual trigger.
        let mut handle =
            spawn(clients, enabled, Plan::Retail, Duration::from_secs(120)).expect("spawn");

        handle.send_quota_cmd(QuotaCmd::Start);

        // Consume initial snapshot.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Some(RefreshMsg::QuotaSnapshot { .. }) = handle.try_recv() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // Trigger a manual refresh; a second QuotaSnapshot should appear
        // well before the 120-second auto interval elapses.
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

        std::env::remove_var("AGTOP_QUOTA_OPENCODE_AUTH_PATH");
        assert!(got_second, "manual trigger did not produce a second QuotaSnapshot");
    }
```

- [ ] **Step 2: Run test — expect success**

The quota loop was already designed to listen on `manual_rx_quota.changed()`. This test just confirms the wiring works end-to-end.

Run: `cargo test -p agtop-cli refresh::tests::quota_loop_honors_manual_trigger -- --test-threads=1`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/tui/refresh.rs
git commit -m "quota-tui(phase-2): test quota loop honors manual trigger"
```

---

## Task 5: Event-loop drains `QuotaSnapshot` / `QuotaError` into App

**Files:**
- Modify: `crates/agtop-cli/src/tui/mod.rs`

- [ ] **Step 1: Understand the change**

The event loop in `tui::mod.rs` has a drain block around line 189-198:

```rust
while let Some(msg) = handle.try_recv() {
    match msg {
        RefreshMsg::Snapshot { analyses, plan_usage, .. } => app.set_snapshot(...),
        RefreshMsg::Error { message, .. } => app.set_refresh_error(message),
    }
}
```

This match is no longer exhaustive after Phase 2. We need to handle the two new variants.

- [ ] **Step 2: Write the failing test**

No new unit test here — this is glue code. We'll verify in Phase 5 with an end-to-end render test. For now just ensure the match is exhaustive so the build stays green.

- [ ] **Step 3: Update the drain block**

In `crates/agtop-cli/src/tui/mod.rs` around line 189, replace:

```rust
        while let Some(msg) = handle.try_recv() {
            match msg {
                RefreshMsg::Snapshot {
                    analyses,
                    plan_usage,
                    ..
                } => app.set_snapshot(analyses, plan_usage),
                RefreshMsg::Error { message, .. } => app.set_refresh_error(message),
            }
        }
```

With:

```rust
        while let Some(msg) = handle.try_recv() {
            match msg {
                RefreshMsg::Snapshot {
                    analyses,
                    plan_usage,
                    ..
                } => app.set_snapshot(analyses, plan_usage),
                RefreshMsg::Error { message, .. } => app.set_refresh_error(message),
                RefreshMsg::QuotaSnapshot { results, .. } => app.apply_quota_results(results),
                RefreshMsg::QuotaError { message, .. } => app.set_quota_error(message),
            }
        }
```

- [ ] **Step 4: Run full suite**

Run: `cargo test -p agtop-cli -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-cli/src/tui/mod.rs
git commit -m "quota-tui(phase-2): drain QuotaSnapshot/QuotaError in event loop"
```

---

## Task 6: Clippy pass

- [ ] **Step 1: Run clippy**

Run: `cargo clippy -p agtop-cli -- -D warnings`
Expected: no warnings.

Common issues:
- Unused `QuotaCmd::Stop` arm warning in the idle-wait loop → suppress with `_ => {}` if the compiler flags it, or restructure the match.
- `manual_rx_quota` field of `unused_must_use` on `changed()` → already handled via `if changed.is_err()`.

If something needs `#[allow]`, keep it tight and document why inline.

- [ ] **Step 2: Commit any fixes**

```bash
git add crates/agtop-cli/src/tui/refresh.rs
git commit -m "quota-tui(phase-2): clippy fixes for quota loop"
```

Phase 2 complete. The worker publishes quota snapshots; the UI drains them into `App::quota_slots`. Nothing visible in the TUI yet — that's Phases 3 and 4.
