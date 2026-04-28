# Disk I/O Rates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add live disk read/write throughput per second for matched agent processes everywhere process metrics are displayed, and make the dashboard activity sparkline reflect CPU plus disk activity.

**Architecture:** Keep OS sampling in `SysinfoScanner` and derive rates in `ProcessCorrelator`, which already persists across refreshes. Preserve existing cumulative disk counters and add explicit per-second fields to `ProcessMetrics`, then thread them through TUI, `--list`, and JSON output. In the dashboard refresh adapter, preserve short per-session histories and append a combined CPU/disk activity score for the existing `ACTIVITY` sparkline.

**Tech Stack:** Rust workspace, `sysinfo`, `serde`, `ratatui`, `insta`, existing `agtop-core` process correlation and `agtop-cli` rendering code.

---

## File Structure

- Modify `crates/agtop-core/src/process/mod.rs`: add rate fields to `ProcessMetrics`, store prior disk samples in `ProcessCorrelator`, enrich metrics after correlation, and test lifecycle/subagent propagation.
- Modify `crates/agtop-core/src/process/scanner.rs`: initialize new rate fields to `0.0` when raw OS samples are created.
- Modify `crates/agtop-core/src/process/correlator.rs`: update test fixtures that construct `ProcessMetrics`.
- Modify `crates/agtop-cli/src/fmt.rs`: add byte-per-second formatting helper.
- Modify `crates/agtop-cli/src/tui/column_config.rs`: add `DiskReadRate` and `DiskWriteRate` columns, labels, descriptions, widths, and default visibility.
- Modify `crates/agtop-cli/src/tui/widgets/session_table.rs`: render new classic table columns.
- Modify `crates/agtop-cli/src/tui/widgets/process_tab.rs`: render cumulative disk totals and per-second disk rates.
- Modify `crates/agtop-cli/src/tui/widgets/info_tab.rs`: render new columns in the info/configurable detail path.
- Modify `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs`: render new dashboard table columns.
- Modify `crates/agtop-cli/src/tui/screens/dashboard/info_process.rs`: render disk rates in dashboard process details.
- Modify `crates/agtop-cli/src/tui/refresh_adapter.rs`: update test fixtures that construct `ProcessMetrics`, preserve per-session activity history, and compute combined CPU/disk activity samples.
- Modify `crates/agtop-cli/src/main.rs`: print read/s and write/s in `--list`, keep JSON additive, and update JSON test fixture.
- Update snapshots under `crates/agtop-cli/tests/snapshots/` through `cargo insta test --accept` or targeted snapshot acceptance after verifying diffs.

---

### Task 1: Extend Core Metric Type And Fixtures

**Files:**
- Modify: `crates/agtop-core/src/process/mod.rs:47-60`
- Modify: `crates/agtop-core/src/process/scanner.rs:214-221`
- Modify: `crates/agtop-core/src/process/correlator.rs:1033-1146`
- Modify: `crates/agtop-cli/src/tui/refresh_adapter.rs:208-220, 345-357`
- Modify: `crates/agtop-cli/src/main.rs:725-731`

- [ ] **Step 1: Add failing assertions for propagated rate fields**

In `crates/agtop-core/src/process/mod.rs`, update `attach_process_info_propagates_parent_pid_to_subagent_children` constants and assertions:

```rust
        const FAKE_DISK_READ_RATE: f64 = 128.0;
        const FAKE_DISK_WRITE_RATE: f64 = 256.0;
```

Add the new fields to the `ProcessMetrics` literal in that test:

```rust
                    disk_read_bytes: FAKE_DISK_READ,
                    disk_written_bytes: FAKE_DISK_WRITE,
                    disk_read_bytes_per_sec: FAKE_DISK_READ_RATE,
                    disk_written_bytes_per_sec: FAKE_DISK_WRITE_RATE,
```

Add assertions after the existing parent CPU assertion:

```rust
        assert_eq!(
            p.process_metrics.as_ref().map(|m| m.disk_read_bytes_per_sec),
            Some(FAKE_DISK_READ_RATE)
        );
```

Add assertions inside the child loop after the existing memory assertion:

```rust
            assert_eq!(
                child
                    .process_metrics
                    .as_ref()
                    .map(|m| m.disk_written_bytes_per_sec),
                Some(FAKE_DISK_WRITE_RATE)
            );
```

- [ ] **Step 2: Run the targeted test to verify it fails**

Run: `cargo test -p agtop-core process::lifecycle_tests::attach_process_info_propagates_parent_pid_to_subagent_children`

Expected: compile failure because `ProcessMetrics` has no `disk_read_bytes_per_sec` or `disk_written_bytes_per_sec` fields.

- [ ] **Step 3: Add rate fields to `ProcessMetrics`**

In `crates/agtop-core/src/process/mod.rs`, replace the struct with:

```rust
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
```

- [ ] **Step 4: Initialize rate fields at raw scanner boundaries**

In `crates/agtop-core/src/process/scanner.rs`, update the `ProcessMetrics` literal in `SysinfoScanner::refresh()`:

```rust
            let metrics = Some(ProcessMetrics {
                cpu_percent: proc.cpu_usage(),
                memory_bytes: proc.memory(),
                virtual_memory_bytes: proc.virtual_memory(),
                disk_read_bytes: disk.total_read_bytes,
                disk_written_bytes: disk.total_written_bytes,
                disk_read_bytes_per_sec: 0.0,
                disk_written_bytes_per_sec: 0.0,
            });
```

- [ ] **Step 5: Update test fixture literals across core and CLI**

For every `ProcessMetrics { ... }` literal in these files, add the two rate fields with deterministic values:

```rust
disk_read_bytes_per_sec: 0.0,
disk_written_bytes_per_sec: 0.0,
```

Use non-zero values only where the test specifically asserts rate propagation. Files:

```text
crates/agtop-core/src/process/correlator.rs
crates/agtop-cli/src/tui/refresh_adapter.rs
crates/agtop-cli/src/main.rs
```

- [ ] **Step 6: Run targeted compile/test**

Run: `cargo test -p agtop-core process::lifecycle_tests::attach_process_info_propagates_parent_pid_to_subagent_children`

Expected: PASS.

- [ ] **Step 7: Commit**

Run:

```bash
git add crates/agtop-core/src/process/mod.rs crates/agtop-core/src/process/scanner.rs crates/agtop-core/src/process/correlator.rs crates/agtop-cli/src/tui/refresh_adapter.rs crates/agtop-cli/src/main.rs
git commit -m "feat(core): add disk io rate metric fields"
```

---

### Task 2: Derive Disk I/O Rates In The Correlator

**Files:**
- Modify: `crates/agtop-core/src/process/mod.rs:20-150`

- [ ] **Step 1: Write failing tests for first and second samples**

In `crates/agtop-core/src/process/mod.rs`, inside `lifecycle_tests`, add this helper before the tests:

```rust
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
```

Add this test:

```rust
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
```

- [ ] **Step 2: Write failing test for counter reset**

Add this test in the same module:

```rust
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p agtop-core disk_io_rates -- --nocapture`

Expected: tests compile but fail because the rates remain `0.0` on the second sample.

- [ ] **Step 4: Add prior disk sample state to `ProcessCorrelator`**

In `crates/agtop-core/src/process/mod.rs`, add imports near the top:

```rust
use std::time::Instant;
```

Add this private struct after `ProcessInfo`:

```rust
#[derive(Debug, Clone, Copy)]
struct DiskIoSample {
    read_bytes: u64,
    written_bytes: u64,
    sampled_at: Instant,
}
```

Add this field to `ProcessCorrelator`:

```rust
    disk_samples: HashMap<u32, DiskIoSample>,
```

Initialize it in `with_scanners`:

```rust
            disk_samples: HashMap::new(),
```

- [ ] **Step 5: Add rate enrichment helpers**

In the `impl ProcessCorrelator` block, before `snapshot`, add:

```rust
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
                if elapsed > 0.0
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

            self.disk_samples.insert(info.pid, current);
        }

        self.disk_samples.retain(|pid, _| live_pids.contains(pid));
    }
```

- [ ] **Step 6: Call rate enrichment during snapshots**

In `ProcessCorrelator::snapshot`, after `fresh` is created and before stopped-process handling, add:

```rust
        let now = Instant::now();
        self.enrich_disk_rates(&mut fresh, now);
```

The beginning of `snapshot` should include:

```rust
        self.scanner.refresh();
        let mut fresh = correlate(self.scanner.as_ref(), self.fd_scanner.as_ref(), sessions);
        let now = Instant::now();
        self.enrich_disk_rates(&mut fresh, now);
```

- [ ] **Step 7: Run targeted tests**

Run: `cargo test -p agtop-core disk_io_rates -- --nocapture`

Expected: PASS.

- [ ] **Step 8: Run process module tests**

Run: `cargo test -p agtop-core process::`

Expected: PASS.

- [ ] **Step 9: Commit**

Run:

```bash
git add crates/agtop-core/src/process/mod.rs
git commit -m "feat(core): derive disk io rates"
```

---

### Task 3: Add Formatting Helper For Byte Rates

**Files:**
- Modify: `crates/agtop-cli/src/fmt.rs:116-120`

- [ ] **Step 1: Write failing formatter tests**

At the bottom of `crates/agtop-cli/src/fmt.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_rate_formats_optional_bytes_per_second() {
        assert_eq!(compact_rate_opt(None), "-");
        assert_eq!(compact_rate_opt(Some(0.0)), "0/s");
        assert_eq!(compact_rate_opt(Some(512.0)), "512/s");
        assert_eq!(compact_rate_opt(Some(1_280.0)), "1.3K/s");
        assert_eq!(compact_rate_opt(Some(1_250_000.0)), "1.2M/s");
    }
}
```

- [ ] **Step 2: Run the formatter test to verify it fails**

Run: `cargo test -p agtop-cli fmt::tests::compact_rate_formats_optional_bytes_per_second`

Expected: compile failure because `compact_rate_opt` is missing.

- [ ] **Step 3: Implement byte-rate formatting**

In `crates/agtop-cli/src/fmt.rs`, after `compact_opt`, add:

```rust
/// Format an optional byte-per-second rate using [`compact`] plus `/s`.
/// Returns `"-"` when the value is absent.
pub fn compact_rate_opt(value: Option<f64>) -> String {
    match value {
        Some(v) if v.is_finite() && v > 0.0 => format!("{}/s", compact(v.round() as u64)),
        Some(_) => "0/s".to_string(),
        None => "-".to_string(),
    }
}
```

- [ ] **Step 4: Run the formatter test**

Run: `cargo test -p agtop-cli fmt::tests::compact_rate_formats_optional_bytes_per_second`

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add crates/agtop-cli/src/fmt.rs
git commit -m "feat(cli): format disk io rates"
```

---

### Task 4: Populate Activity Sparkline From CPU Plus Disk

**Files:**
- Modify: `crates/agtop-cli/src/tui/refresh_adapter.rs:17-122`

- [ ] **Step 1: Add failing tests for combined activity samples**

In `crates/agtop-cli/src/tui/refresh_adapter.rs`, inside the existing `#[cfg(test)] mod tests`, add this helper after `analysis(id: &str)`:

```rust
    fn analysis_with_metrics(id: &str, cpu_percent: f32, read_rate: f64, write_rate: f64) -> SessionAnalysis {
        let mut a = analysis(id);
        a.process_metrics = Some(ProcessMetrics {
            cpu_percent,
            memory_bytes: 1024,
            virtual_memory_bytes: 2048,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
            disk_read_bytes_per_sec: read_rate,
            disk_written_bytes_per_sec: write_rate,
        });
        a
    }
```

Add this test:

```rust
    #[test]
    fn activity_samples_use_max_of_cpu_and_normalized_disk() {
        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();

        let disk_heavy = analysis_with_metrics("disk", 5.0, 5.0 * 1_048_576.0, 0.0);
        apply_analyses(
            &[disk_heavy],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        assert_eq!(sessions.rows.len(), 1);
        let sample = sessions.rows[0].activity_samples.last().copied().unwrap();
        assert!(
            (49.0..=51.0).contains(&sample),
            "5 MiB/s should normalize to about 50, got {sample}"
        );

        let cpu_heavy = analysis_with_metrics("disk", 80.0, 1.0 * 1_048_576.0, 0.0);
        apply_analyses(
            &[cpu_heavy],
            &mut header,
            &mut sessions,
            &mut quota,
            &mut aggregation,
            5,
        );

        assert_eq!(sessions.rows[0].activity_samples.len(), 2);
        assert_eq!(sessions.rows[0].activity_samples.last().copied(), Some(80.0));
    }
```

- [ ] **Step 2: Add failing test for history cap and missing metrics**

Add this test in the same module:

```rust
    #[test]
    fn activity_samples_are_preserved_capped_and_zero_without_metrics() {
        let mut header = HeaderModel::default();
        let mut sessions = SessionsTable::default();
        let mut quota = QuotaPanel::default();
        let mut aggregation = AggregationState::default();

        for i in 0..35 {
            let analysis = if i == 34 {
                analysis("same")
            } else {
                analysis_with_metrics("same", i as f32, 0.0, 0.0)
            };
            apply_analyses(
                &[analysis],
                &mut header,
                &mut sessions,
                &mut quota,
                &mut aggregation,
                5,
            );
        }

        let samples = &sessions.rows[0].activity_samples;
        assert_eq!(samples.len(), 30);
        assert_eq!(samples.last().copied(), Some(0.0));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p agtop-cli activity_samples -- --nocapture`

Expected: tests compile but fail because `activity_samples` is always empty.

- [ ] **Step 4: Add activity constants and helper functions**

In `crates/agtop-cli/src/tui/refresh_adapter.rs`, after the imports, add:

```rust
const ACTIVITY_HISTORY_LIMIT: usize = 30;
const DISK_ACTIVITY_MAX_BYTES_PER_SEC: f64 = 10.0 * 1024.0 * 1024.0;

fn activity_sample(analysis: &SessionAnalysis) -> f32 {
    let Some(metrics) = analysis.process_metrics.as_ref() else {
        return 0.0;
    };
    let cpu_score = metrics.cpu_percent.clamp(0.0, 100.0);
    let disk_rate = metrics.disk_read_bytes_per_sec + metrics.disk_written_bytes_per_sec;
    let disk_score = if disk_rate.is_finite() && disk_rate > 0.0 {
        ((disk_rate / DISK_ACTIVITY_MAX_BYTES_PER_SEC) * 100.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    };
    cpu_score.max(disk_score)
}

fn next_activity_samples(previous: Option<&SessionRow>, analysis: &SessionAnalysis) -> Vec<f32> {
    let mut samples = previous
        .map(|row| row.activity_samples.clone())
        .unwrap_or_default();
    samples.push(activity_sample(analysis));
    if samples.len() > ACTIVITY_HISTORY_LIMIT {
        let drop_count = samples.len() - ACTIVITY_HISTORY_LIMIT;
        samples.drain(0..drop_count);
    }
    samples
}
```

- [ ] **Step 5: Preserve prior row histories in `apply_analyses`**

At the start of `apply_analyses`, after `normalized`, add:

```rust
    let previous_rows: std::collections::HashMap<String, SessionRow> = sessions
        .rows
        .iter()
        .map(|row| (row.analysis.summary.session_id.clone(), row.clone()))
        .collect();
```

In the top-level `flat_rows.push(SessionRow { ... })`, replace `activity_samples: vec![],` with:

```rust
            activity_samples: next_activity_samples(previous_rows.get(&a.summary.session_id), a),
```

In the child `flat_rows.push(SessionRow { ... })`, replace `activity_samples: vec![],` with:

```rust
                    activity_samples: next_activity_samples(
                        previous_rows.get(&child.summary.session_id),
                        child,
                    ),
```

- [ ] **Step 6: Run activity tests**

Run: `cargo test -p agtop-cli activity_samples -- --nocapture`

Expected: PASS.

- [ ] **Step 7: Commit**

Run:

```bash
git add crates/agtop-cli/src/tui/refresh_adapter.rs
git commit -m "feat(tui): plot cpu and disk activity"
```

---

### Task 5: Add TUI Columns And Process Details

**Files:**
- Modify: `crates/agtop-cli/src/tui/column_config.rs`
- Modify: `crates/agtop-cli/src/tui/widgets/session_table.rs:335-403`
- Modify: `crates/agtop-cli/src/tui/widgets/process_tab.rs:45-62`
- Modify: `crates/agtop-cli/src/tui/widgets/info_tab.rs:352-368`
- Modify: `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs:258-271`
- Modify: `crates/agtop-cli/src/tui/screens/dashboard/info_process.rs:17-49`

- [ ] **Step 1: Add failing column-config test**

In `crates/agtop-cli/src/tui/column_config.rs`, add this test module at the bottom if one does not already exist, or add the test to the existing module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_rate_columns_are_visible_by_default() {
        let visible = default_visible_v2();
        assert!(visible.contains(&ColumnId::DiskReadRate));
        assert!(visible.contains(&ColumnId::DiskWriteRate));
        assert_eq!(ColumnId::DiskReadRate.label(), "R/s");
        assert_eq!(ColumnId::DiskWriteRate.label(), "W/s");
    }
}
```

- [ ] **Step 2: Run the column-config test to verify it fails**

Run: `cargo test -p agtop-cli tui::column_config::tests::disk_rate_columns_are_visible_by_default`

Expected: compile failure because `DiskReadRate` and `DiskWriteRate` are missing.

- [ ] **Step 3: Add column identifiers**

In `ColumnId`, add after `DiskWritten`:

```rust
    /// Live disk read throughput for the matched process.
    DiskReadRate,
    /// Live disk write throughput for the matched process.
    DiskWriteRate,
```

In `ColumnId::all()`, add after `ColumnId::DiskWritten`:

```rust
            ColumnId::DiskReadRate,
            ColumnId::DiskWriteRate,
```

In `label()`, add:

```rust
            ColumnId::DiskReadRate => "R/s",
            ColumnId::DiskWriteRate => "W/s",
```

In `description()`, add:

```rust
            ColumnId::DiskReadRate => "Live disk read throughput of the matched process",
            ColumnId::DiskWriteRate => "Live disk write throughput of the matched process",
```

In `fixed_width()`, add:

```rust
            ColumnId::DiskReadRate => Some(8),
            ColumnId::DiskWriteRate => Some(8),
```

In `sort_col()`, add:

```rust
            ColumnId::DiskReadRate => None,
            ColumnId::DiskWriteRate => None,
```

In `default_visible_v2()`, add after `ColumnId::Memory`:

```rust
        ColumnId::DiskReadRate,
        ColumnId::DiskWriteRate,
```

- [ ] **Step 4: Render columns in the classic session table**

In `crates/agtop-cli/src/tui/widgets/session_table.rs`, add match arms after `ColumnId::DiskWritten`:

```rust
            ColumnId::DiskReadRate => Cell::from(crate::fmt::compact_rate_opt(
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_read_bytes_per_sec),
            )),
            ColumnId::DiskWriteRate => Cell::from(crate::fmt::compact_rate_opt(
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_written_bytes_per_sec),
            )),
```

- [ ] **Step 5: Render rates in the classic Process tab**

In `crates/agtop-cli/src/tui/widgets/process_tab.rs`, after `disk_w`, add:

```rust
                let disk_r_rate = fmt::compact_rate_opt(
                    a.process_metrics
                        .as_ref()
                        .map(|m| m.disk_read_bytes_per_sec),
                );
                let disk_w_rate = fmt::compact_rate_opt(
                    a.process_metrics
                        .as_ref()
                        .map(|m| m.disk_written_bytes_per_sec),
                );
```

Add lines after `disk_written`:

```rust
                    super::kv_line("disk_read/s", disk_r_rate),
                    super::kv_line("disk_written/s", disk_w_rate),
```

- [ ] **Step 6: Render rates in info/configurable detail path**

In `crates/agtop-cli/src/tui/widgets/info_tab.rs`, add match arms near the existing disk arms:

```rust
        ColumnId::DiskReadRate => kv_line(
            "disk_read/s",
            crate::fmt::compact_rate_opt(
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_read_bytes_per_sec),
            ),
        ),
        ColumnId::DiskWriteRate => kv_line(
            "disk_written/s",
            crate::fmt::compact_rate_opt(
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_written_bytes_per_sec),
            ),
        ),
```

- [ ] **Step 7: Render rates in dashboard sessions table**

In `crates/agtop-cli/src/tui/screens/dashboard/sessions.rs`, add match arms after `ColumnId::Memory`:

```rust
            ColumnId::DiskReadRate => Cell::from(
                row.analysis
                    .process_metrics
                    .as_ref()
                    .map(|m| crate::fmt::compact_rate_opt(Some(m.disk_read_bytes_per_sec)))
                    .unwrap_or_else(|| "—".into()),
            ),
            ColumnId::DiskWriteRate => Cell::from(
                row.analysis
                    .process_metrics
                    .as_ref()
                    .map(|m| crate::fmt::compact_rate_opt(Some(m.disk_written_bytes_per_sec)))
                    .unwrap_or_else(|| "—".into()),
            ),
```

- [ ] **Step 8: Render rates in dashboard process details**

In `crates/agtop-cli/src/tui/screens/dashboard/info_process.rs`, add local values after `resident`:

```rust
    let disk_read_rate = a.process_metrics
        .as_ref()
        .map(|m| crate::fmt::compact_rate_opt(Some(m.disk_read_bytes_per_sec)))
        .unwrap_or_else(|| "—".into());
    let disk_write_rate = a.process_metrics
        .as_ref()
        .map(|m| crate::fmt::compact_rate_opt(Some(m.disk_written_bytes_per_sec)))
        .unwrap_or_else(|| "—".into());
```

Add lines after Resident mem:

```rust
        Line::from(vec![
            Span::styled("  Disk read/s  ", Style::default().fg(theme.fg_muted)),
            Span::styled(disk_read_rate, Style::default().fg(theme.fg_default)),
        ]),
        Line::from(vec![
            Span::styled("  Disk write/s ", Style::default().fg(theme.fg_muted)),
            Span::styled(disk_write_rate, Style::default().fg(theme.fg_default)),
        ]),
```

- [ ] **Step 9: Run TUI-focused tests**

Run: `cargo test -p agtop-cli tui::column_config::tests::disk_rate_columns_are_visible_by_default`

Expected: PASS.

Run: `cargo test -p agtop-cli --tests sessions_table_snapshot config_snapshot`

Expected: snapshot failures may occur because default visible columns changed; inspect before accepting.

- [ ] **Step 10: Commit**

Run:

```bash
git add crates/agtop-cli/src/tui/column_config.rs crates/agtop-cli/src/tui/widgets/session_table.rs crates/agtop-cli/src/tui/widgets/process_tab.rs crates/agtop-cli/src/tui/widgets/info_tab.rs crates/agtop-cli/src/tui/screens/dashboard/sessions.rs crates/agtop-cli/src/tui/screens/dashboard/info_process.rs
git commit -m "feat(tui): show disk io rates"
```

---

### Task 6: Add `--list` And JSON Coverage

**Files:**
- Modify: `crates/agtop-cli/src/main.rs:457-590, 691-745`

- [ ] **Step 1: Update JSON test fixture and assertions**

In `json_session_display_state_and_process_metrics`, add rate fields to the `ProcessMetrics` literal:

```rust
            disk_read_bytes_per_sec: 45.0,
            disk_written_bytes_per_sec: 6.0,
```

Add this assertion after the existing `disk_written_bytes` assertion:

```rust
        assert_eq!(
            json.process_metrics
                .as_ref()
                .map(|m| m.disk_read_bytes_per_sec),
            Some(45.0)
        );
```

- [ ] **Step 2: Run JSON test**

Run: `cargo test -p agtop-cli json_output_tests::json_session_display_state_and_process_metrics`

Expected: PASS, because JSON uses `ProcessMetrics` directly and the fixture now includes the additive fields.

- [ ] **Step 3: Add `--list` rate strings**

In `render_table`, update the table width comment and constant:

```rust
    // 16 columns plus 15 two-space gaps.
    const TABLE_WIDTH: usize = 197;
```

Update the header format string to include two new right-aligned columns after `MEM`:

```rust
        "{:<10}  {:<16}  {:<10}  {:<16}  {:>4}  {:<20}  {:<18}  {:>9}  {:>9}  {:>9}  {:>8}  {:>7}  {:>6}  {:>7}  {:>8}  {:>8}",
```

Add header labels after `"MEM"`:

```rust
        "R/s",
        "W/s"
```

After `mem_str`, add:

```rust
        let disk_read_rate_str = fmt::compact_rate_opt(
            a.and_then(|a| {
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_read_bytes_per_sec)
            }),
        );
        let disk_write_rate_str = fmt::compact_rate_opt(
            a.and_then(|a| {
                a.process_metrics
                    .as_ref()
                    .map(|m| m.disk_written_bytes_per_sec)
            }),
        );
```

Update the row format string to match the header and pass the new values after `mem_str`:

```rust
            "{:<10}  {:<16}  {:<10}  {:<16}  {:>4}  {:<20}  {:<18}  {:>9}  {:>9}  {:>9}  {:>8}  {:>7}  {:>6}  {:>7}  {:>8}  {:>8}",
```

```rust
            disk_read_rate_str,
            disk_write_rate_str,
```

- [ ] **Step 4: Run CLI unit tests**

Run: `cargo test -p agtop-cli json_output_tests::json_session_display_state_and_process_metrics fmt::tests::compact_rate_formats_optional_bytes_per_second`

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add crates/agtop-cli/src/main.rs
git commit -m "feat(cli): include disk io rates in outputs"
```

---

### Task 7: Update Snapshots, Docs, And Verify

**Files:**
- Modify: `README.md:21-56`
- Modify: `crates/agtop-cli/tests/snapshots/*.snap` as generated by verified snapshot acceptance

- [ ] **Step 1: Update README process tracking text**

In `README.md`, replace line 27 text with:

```markdown
state (`live` / `stopped`), match confidence, and live resource metrics
(CPU usage, resident memory, virtual memory, cumulative disk I/O, and disk I/O throughput).
```

Replace lines 50-54 with:

```markdown
**v0.4** adds live process metrics. Each matched session now surfaces CPU
usage, resident/virtual memory, cumulative disk I/O, and disk read/write
throughput sampled from the OS on every refresh. The dashboard `ACTIVITY`
sparkline reflects the stronger of CPU load or normalized disk throughput.
TUI columns `CPU`, `MEM`, `R/s`, and `W/s` are visible by default (plus `VSZ`,
`DISK R`, `DISK W` in the Config tab); a dedicated **Process** bottom-panel
tab shows all metrics with room to read them. The `--list` table gains `PID`,
`CPU`, `MEM`, `R/s`, and `W/s` columns; `--json` includes the expanded
`process_metrics` field.
```

- [ ] **Step 2: Run full formatting**

Run: `cargo fmt --all --check`

Expected: PASS. If it fails, run `cargo fmt --all`, then rerun `cargo fmt --all --check`.

- [ ] **Step 3: Run targeted tests before accepting snapshots**

Run: `cargo test -p agtop-core process::`

Expected: PASS.

Run: `cargo test -p agtop-cli fmt::tests::compact_rate_formats_optional_bytes_per_second json_output_tests::json_session_display_state_and_process_metrics`

Expected: PASS.

- [ ] **Step 4: Run snapshot tests and inspect failures**

Run: `cargo test -p agtop-cli --tests sessions_table_snapshot config_snapshot header_snapshot quota_panel_snapshot`

Expected: snapshot failures only where added rate columns change visible output.

- [ ] **Step 5: Accept intentional snapshot changes**

Run: `cargo insta test -p agtop-cli --accept`

Expected: snapshots update. Inspect the resulting diff with `git diff -- crates/agtop-cli/tests/snapshots` and verify changes are limited to disk-rate columns/details.

- [ ] **Step 6: Run full verification**

Run: `cargo test`

Expected: PASS.

Run: `cargo clippy -- -D warnings`

Expected: PASS.

- [ ] **Step 7: Commit final verification updates**

Run:

```bash
git add README.md crates/agtop-cli/tests/snapshots
git commit -m "docs: describe disk io rate metrics"
```

---

## Self-Review Notes

- Spec coverage: core rate derivation is covered by Tasks 1-2; activity sparkline behavior is covered by Task 4; presentation everywhere is covered by Tasks 3 and 5-7; additive JSON compatibility is covered by Task 6; error handling for first samples, counter resets, and stopped processes is covered by Task 2.
- Placeholder scan: no deferred implementation text is required to execute the plan.
- Type consistency: the plan uses `disk_read_bytes_per_sec` and `disk_written_bytes_per_sec` consistently across `ProcessMetrics`, formatters, TUI columns, `--list`, and JSON tests.
