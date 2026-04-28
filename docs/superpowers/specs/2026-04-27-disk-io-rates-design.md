# Disk I/O Rates Design

## Goal

Track live disk read and write throughput for matched agent processes, alongside the existing CPU, memory, and cumulative disk I/O metrics. The rates should be visible everywhere process metrics are exposed: TUI process details, configurable/default session table columns, `--list`, and JSON output.

## Current State

`agtop-core::process::SysinfoScanner` already samples cumulative disk counters from `sysinfo::Process::disk_usage()` and stores them in `ProcessMetrics` as `disk_read_bytes` and `disk_written_bytes`. Those cumulative counters are attached to `SessionAnalysis` by `ProcessCorrelator` and rendered in optional TUI columns plus the Process tab.

The missing behavior is per-second throughput. A rate cannot be derived from a single process sample; it needs a prior sample and elapsed time between refreshes.

## Approach

Compute disk I/O rates in `ProcessCorrelator`, not in individual UI layers.

`ProcessMetrics` will keep the existing cumulative counters and gain two explicit rate fields:

- `disk_read_bytes_per_sec: f64`
- `disk_written_bytes_per_sec: f64`

The scanner remains responsible only for collecting instantaneous OS data. The correlator already persists state across refreshes, so it is the right boundary for deriving deltas from cumulative counters.

## Data Flow

1. `SysinfoScanner::refresh()` samples CPU, memory, virtual memory, and cumulative disk counters for candidate agent processes.
2. `ProcessCorrelator::snapshot()` records the current sample time and compares each live candidate's cumulative disk counters with the previous sample for the same PID.
3. If a prior sample exists, elapsed time is positive, and counters did not decrease, the correlator computes bytes per second from the counter delta.
4. If this is the first observation for a PID, elapsed time is zero, counters decreased, or the process is stopped, the rate fields are set to `0.0`.
5. `attach_process_info()` propagates the enriched metrics to parent sessions and in-process subagent rows exactly as it does today.

## Presentation

The new rates are displayed as compact byte-per-second values, for example `0/s`, `512/s`, `1.2K/s`, or `4.8M/s`.

- The Process tab shows both cumulative totals (`disk_read`, `disk_written`) and rates (`disk_read/s`, `disk_written/s`).
- The session table adds read/s and write/s column identifiers near CPU and memory. They are visible by default because the requested scope is "everywhere."
- The Config tab exposes the new columns like other table columns.
- `--list` prints read/s and write/s next to PID, CPU, and MEM.
- `--json` includes the new fields inside `process_metrics`, preserving the existing cumulative fields.

## Error Handling

Disk I/O rates are best-effort process observability. The feature must not cause refresh failures or panics.

- Missing metrics remain `None` at the `ProcessInfo` level as they do today.
- First samples and invalid deltas produce `0.0` rates rather than `None`, because the process metrics object is present but there is not enough history yet.
- Counter decreases are treated as process replacement, OS reset, or unsupported counter behavior and produce `0.0` for that sample.
- Stopped processes keep `metrics: None`, so they do not show stale rates.

## Compatibility

Existing JSON fields remain unchanged. Adding fields to `ProcessMetrics` is an additive JSON change. Existing column configuration files continue to deserialize because new columns are opt-in through the default/all column definitions, not required in persisted configs.

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
- Session table and Process tab snapshot expectations are updated for the new fields and columns.
