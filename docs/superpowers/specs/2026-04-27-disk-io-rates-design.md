# Disk I/O Rates Design

## Goal

Track live disk read and write throughput for matched agent processes, alongside the existing CPU, memory, and cumulative disk I/O metrics. The rates should be visible everywhere process metrics are exposed: TUI process details, configurable/default session table columns, `--list`, and JSON output. The dashboard activity sparkline should also reflect disk activity, not only CPU-oriented activity.

## Current State

`agtop-core::process::SysinfoScanner` already samples cumulative disk counters from `sysinfo::Process::disk_usage()` and stores them in `ProcessMetrics` as `disk_read_bytes` and `disk_written_bytes`. Those cumulative counters are attached to `SessionAnalysis` by `ProcessCorrelator` and rendered in optional TUI columns plus the Process tab.

The missing behavior is per-second throughput. A rate cannot be derived from a single process sample; it needs a prior sample and elapsed time between refreshes.

The dashboard sessions table already has an `ACTIVITY` sparkline column, but `refresh_adapter` currently populates `activity_samples` with an empty vector for every row. That makes the column available but not useful for live process activity.

## Approach

Compute disk I/O rates in `ProcessCorrelator`, not in individual UI layers.

`ProcessMetrics` will keep the existing cumulative counters and gain two explicit rate fields:

- `disk_read_bytes_per_sec: f64`
- `disk_written_bytes_per_sec: f64`

The scanner remains responsible only for collecting instantaneous OS data. The correlator already persists state across refreshes, so it is the right boundary for deriving deltas from cumulative counters.

The dashboard activity sparkline will use a combined activity score. On each refresh, the TUI adapter computes one sample per session from the current metrics:

```text
disk_rate = disk_read_bytes_per_sec + disk_written_bytes_per_sec
disk_score = min(100, disk_rate / 10 MiB/s * 100)
activity = max(cpu_percent clamped to 0..100, disk_score)
```

The fixed `10 MiB/s == 100` ceiling is intentionally simple and stable. It avoids per-row autoscaling that would make older samples change meaning as new spikes arrive.

## Data Flow

1. `SysinfoScanner::refresh()` samples CPU, memory, virtual memory, and cumulative disk counters for candidate agent processes.
2. `ProcessCorrelator::snapshot()` records the current sample time and compares each live candidate's cumulative disk counters with the previous sample for the same PID.
3. If a prior sample exists, elapsed time is positive, and counters did not decrease, the correlator computes bytes per second from the counter delta.
4. If this is the first observation for a PID, elapsed time is zero, counters decreased, or the process is stopped, the rate fields are set to `0.0`.
5. `attach_process_info()` propagates the enriched metrics to parent sessions and in-process subagent rows exactly as it does today.
6. `agtop-cli::tui::refresh_adapter` preserves each row's prior `activity_samples` by session id, appends the combined CPU/disk activity sample for the current refresh, and caps history to 30 samples.

## Presentation

The new rates are displayed as compact byte-per-second values, for example `0/s`, `512/s`, `1.2K/s`, or `4.8M/s`.

- The Process tab shows both cumulative totals (`disk_read`, `disk_written`) and rates (`disk_read/s`, `disk_written/s`).
- The session table adds read/s and write/s column identifiers near CPU and memory. They are visible by default because the requested scope is "everywhere."
- The dashboard `ACTIVITY` sparkline remains a single column, but its samples become a combined CPU/disk busy score. CPU-bound and disk-bound sessions both produce visible spikes.
- The Config tab exposes the new columns like other table columns.
- `--list` prints read/s and write/s next to PID, CPU, and MEM.
- `--json` includes the new fields inside `process_metrics`, preserving the existing cumulative fields.

## Error Handling

Disk I/O rates are best-effort process observability. The feature must not cause refresh failures or panics.

- Missing metrics remain `None` at the `ProcessInfo` level as they do today.
- First samples and invalid deltas produce `0.0` rates rather than `None`, because the process metrics object is present but there is not enough history yet.
- Counter decreases are treated as process replacement, OS reset, or unsupported counter behavior and produce `0.0` for that sample.
- Stopped processes keep `metrics: None`, so they do not show stale rates.
- Rows without live metrics append `0.0` to their activity history so the sparkline naturally decays as new refreshes arrive.

## Compatibility

Existing JSON fields remain unchanged. Adding fields to `ProcessMetrics` is an additive JSON change. Existing column configuration files continue to deserialize because new columns are opt-in through the default/all column definitions, not required in persisted configs.

The activity sparkline change is presentation-only. It does not change persisted session data or JSON output.

## Testing

Core tests will cover:

- First sample for a PID reports zero disk rates.
- Second sample computes expected read/s and write/s from elapsed time.
- Counter decrease reports zero for that sample.
- Stopped process frames have no metrics.
- Subagent metric propagation includes the new rate fields.

CLI/TUI tests will cover:

- JSON includes cumulative counters and rate fields.
- `--list` renders read/s and write/s.
- Refresh adapter preserves and caps per-session activity history.
- Activity samples use the larger of CPU percentage and normalized disk throughput.
- Session table and Process tab snapshot expectations are updated for the new fields and columns.
