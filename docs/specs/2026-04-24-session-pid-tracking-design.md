# Session PID Tracking — Design

Spec for issue [#23](https://github.com/collectiveai-team/rust-agtop/issues/23).
Feature branch: `feature/session-pid-tracking`.

## Goal

For every session rust-agtop displays, determine whether the agent CLI that
produced that session is still running on the host. When it is, surface the
OS process ID (PID) and liveness state. Work identically on Linux and macOS.

The issue lists three potential wins: liveness detection, CPU/memory
correlation, and better session-state detection. This spec covers **liveness
+ PID display** only. CPU/mem is out of scope but the data model leaves room
to add it without schema changes.

## Problem

None of the seven supported client transcript formats (Claude Code, Codex,
Gemini CLI, OpenCode, Copilot, Cursor, Antigravity) write the agent CLI's
OS PID into the transcript. The "pid" identifiers already in the codebase
are either agtop's own `std::process::id()` (used for test temp-file names)
or UUIDs named `parent_thread_id`. Getting a PID therefore requires
**discovering** the running CLI process and correlating it to a session.

Simple correlation by current working directory is not sufficient: automated
workflows frequently run two or more agent CLIs concurrently in the same
project directory. cwd-matching alone produces ambiguous results in that
case, and ambiguous data shown confidently is worse than no data at all.

## Approach

A new core module `agtop-core::process` encapsulates all OS-process
concerns. The CLI refresh layer calls it once per refresh and attaches the
result to each `SessionAnalysis` before rendering. The core crate stays
TUI-free (existing project convention).

Correlation uses a two-tier strategy:

1. **Open-file match (High confidence)**: enumerate candidate CLI processes
   via `sysinfo`; for each, list open file descriptors and check whether any
   open path corresponds to a session's transcript path. This is the only
   signal that disambiguates N concurrent agents in the same cwd. Requires
   `procfs` on Linux and `libproc` on macOS — both abstract behind an
   internal `FdScanner` trait so the correlator has one code path.

2. **cwd + binary + start-time scoring (Medium confidence)**: when fd-match
   produces no result for a session, fall back to scoring by process `cwd`
   matching session `cwd`, process binary name matching the expected CLI for
   the session's client, and process start-time falling inside
   `[session.started_at, session.last_active]`. Match only when score ≥ 2
   *and* the match is unique among candidates.

Sessions that match neither tier get no `ProcessInfo`; the UI renders `—`.
Showing nothing is preferable to guessing.

## Architecture

```
agtop-core
└── process/
    ├── mod.rs              — public API: ProcessCorrelator, ProcessInfo, Liveness
    ├── scanner.rs          — Scanner trait + sysinfo-backed impl
    ├── fd.rs               — FdScanner trait + cfg-gated impls
    ├── correlator.rs       — matching algorithm (fd-first, then scoring)
    └── transcript_paths.rs — per-client: which files a matching process should hold open
```

### Public API

```rust
// agtop-core/src/process/mod.rs

pub struct ProcessCorrelator { /* owns Scanner + FdScanner + prior-snapshot state */ }

impl ProcessCorrelator {
    pub fn new() -> Self;

    /// Refresh OS-process state and match against the given sessions.
    /// Returns a map from `session_id` to `ProcessInfo`. Sessions with no
    /// match are absent from the map.
    pub fn snapshot(&mut self, sessions: &[SessionSummary]) -> HashMap<String, ProcessInfo>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub liveness: Liveness,
    pub match_confidence: Confidence,
    pub parent_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Liveness { Live, Stopped }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence { High, Medium }
```

### Internal traits

```rust
// agtop-core/src/process/scanner.rs

pub(crate) trait Scanner {
    /// Re-read the OS process table; return candidate processes whose binary
    /// name is in the known-CLIs set.
    fn refresh(&mut self);
    fn candidates(&self) -> &[Candidate];
}

pub(crate) struct Candidate {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub binary: String,      // from sysinfo::Process::name()
    pub argv: Vec<String>,   // from sysinfo::Process::cmd()
    pub cwd: Option<PathBuf>,
    pub start_time: u64,     // unix epoch seconds, from sysinfo::Process::start_time()
}
```

```rust
// agtop-core/src/process/fd.rs

pub(crate) trait FdScanner {
    /// Return the set of open file paths for `pid`. Returns empty Vec on
    /// permission denied or process-gone, never errors.
    fn open_paths(&self, pid: u32) -> Vec<PathBuf>;
}

#[cfg(target_os = "linux")]
pub(crate) struct LinuxFdScanner;  // uses procfs::process::Process::new(pid)?.fd()?

#[cfg(target_os = "macos")]
pub(crate) struct MacosFdScanner;  // uses libproc::proc_pid::pidfdinfo

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) struct NoopFdScanner;   // always returns Vec::new()
```

### Known-CLIs set

Binary-name filter applied during process enumeration. Keeping this small
keeps scan cost linear in running-agent count rather than total-process
count.

| ClientKind | Candidate binary names |
|------------|------------------------|
| Claude     | `claude` |
| Codex      | `codex` |
| GeminiCli  | `gemini`, `node` (disambiguated by argv containing `gemini`) |
| OpenCode   | `opencode` |
| Copilot    | `copilot`, `gh-copilot` |
| Cursor     | `cursor`, `cursor-agent` |
| Antigravity| `antigravity` |

Maintained in `transcript_paths.rs` alongside the path-expectation logic so
both live with their client-specific knowledge.

### Correlation algorithm

```
snapshot(sessions):
  1. scanner.refresh()
  2. Build reverse index `open_path -> pid` by calling
     fd_scanner.open_paths(c.pid) for each candidate c. Only retain paths
     that appear in any session's transcript-path set (computed by
     transcript_paths::paths_for).
  3. For each session:
       - If any path in paths_for(session) is in the reverse index:
           record ProcessInfo { pid, Live, High, parent_pid }
           continue
       - Otherwise, score every candidate:
           +1 if candidate.binary matches expected binary for session.client
           +1 if candidate.cwd == session.cwd
           +1 if session.started_at <= candidate.start_time <= session.last_active
         Keep candidates with score >= 2. If exactly one remains,
         record ProcessInfo { pid, Live, Medium, parent_pid }.
  4. Stopped-state emission:
       - For every session that had a ProcessInfo in the previous snapshot
         with a pid that is no longer present in scanner.candidates(),
         emit ProcessInfo { pid, Stopped, <prior confidence>, parent_pid }
         for this snapshot only. Drop on the following snapshot.
```

The "one refresh of Stopped then drop" rule lets the user see a brief
confirmation that their session ended, without accumulating stale entries.

### transcript_paths

Per-client mapping from a `SessionSummary` to the set of files a running
agent process is expected to hold open.

| ClientKind | Expected open paths |
|------------|---------------------|
| Claude     | `summary.data_path` (the `.jsonl` file) |
| Codex      | `summary.data_path` (the `rollout-*.jsonl` file) |
| GeminiCli  | `summary.data_path` (`chats/*.json` or `*.jsonl`) |
| OpenCode   | SQLite DB file + `-wal` + `-shm` siblings (under `~/.local/share/opencode/storage/`) |
| Copilot    | `summary.data_path` |
| Cursor     | `summary.data_path` |
| Antigravity| SQLite state DB + `-wal` + `-shm` |

SQLite clients need the WAL/SHM siblings because the DB file itself may or
may not be listed as open depending on journaling mode; WAL always is while
the process is writing.

## Data flow

```
CLI refresh tick
    |
    v
existing session collection (per-client parsers produce SessionSummary + SessionAnalysis)
    |
    v
ProcessCorrelator::snapshot(&[SessionSummary])
    |             sysinfo refresh -> Candidates
    |             FdScanner::open_paths per candidate -> reverse index
    |             Correlation -> HashMap<session_id, ProcessInfo>
    v
CLI refresh layer attaches ProcessInfo to each SessionAnalysis
    |
    v
TUI renders info panel rows + PID column
JSON output serializes pid / liveness fields
```

## UI changes

### Info panel (when a session is selected)

Two new rows below the existing session-id row:

```
PID:     12345 (live)      # or "(stopped)" or "—" when no match
Match:   fd                # or "cwd+argv", only shown when pid is known
```

The match annotation is there because users should be able to tell whether
a displayed PID is a definitive fd-match or a fallback heuristic match.

`(stopped)` is transient: it appears for one refresh immediately after the
correlator notices a previously-matched process has exited, then the PID
row reverts to `—` on the next refresh. This gives the user a brief visual
confirmation of the transition without accumulating stale entries.

### Sessions table

A new right-aligned `PID` column, 6 characters wide, placed immediately
after the existing `state` column. Shows `—` for unmatched sessions. Total
minimum terminal width stays under 80 columns.

### `--list`

No change. The one-shot table is already tight and its primary audience is
scripts that parse fixed columns; adding a column would break them.

### `--json`

`SessionAnalysis` gains two optional fields:

```rust
#[serde(default)]
pub pid: Option<u32>,
#[serde(default)]
pub liveness: Option<Liveness>,
```

`#[serde(default)]` keeps existing JSON consumers working.

## Error handling and degradation

| Scenario | Behavior |
|----------|----------|
| `sysinfo` returns empty process list | Correlator returns empty map. UI shows `—` everywhere. No user-visible error. |
| `FdScanner::open_paths(pid)` fails (permission / race / process gone) | Silently return `Vec::new()`. Correlator falls through to scoring. Log at debug level only. |
| Process user differs from agtop user | `sysinfo` omits it; correlator never sees it. Expected — agent CLIs run as the same user as agtop. |
| Windows | `NoopFdScanner` returns empty. sysinfo still gives cwd+binary+time, so scoring-based matches still work with Medium confidence. |
| Scan cost spikes under high process count | Known-CLIs filter caps cost at O(running agents) × O(fds per agent). Expected < 10ms on typical workstations. |

No code path in `process/` is allowed to panic or return `Err` to the
correlator's public `snapshot` method. This is a best-effort observability
feature; it must never degrade the core session display.

## Testing

### Unit (no real processes)

- `Scanner` and `FdScanner` are behind traits. Tests inject fakes that
  return canned candidate lists and canned fd-path lists.
- Correlator tests:
  - One session, one candidate process holding the transcript open →
    `ProcessInfo { pid, Live, High }`.
  - Two candidates in the same cwd, one holding the transcript open → the
    one with the open fd wins with High; the other does not match.
  - No fd match, one candidate matches cwd + binary + start-time window →
    `Medium` match.
  - No fd match, two candidates both match all three criteria → no match
    (ambiguous).
  - Previous snapshot had a Live match; current snapshot has no candidate
    with that pid → emit `Stopped` once, drop on the next snapshot.
- `transcript_paths::paths_for` — one parameterized test per `ClientKind`
  asserting the expected paths for a representative `SessionSummary`.

### Integration

- A test helper binary opens a temp file and prints its pid. The test
  constructs a `SessionSummary` whose `data_path` points at that temp file,
  runs the correlator, and asserts the match is `{pid, Live, High}`. No
  real agent CLI involvement.

### Platform coverage

Unit tests run on all platforms (they use fakes). Integration test runs on
Linux and macOS via CI; Windows runs unit tests only and confirms
`NoopFdScanner` compiles.

## Dependencies

| Crate | Scope | Justification |
|-------|-------|---------------|
| `sysinfo` | all platforms | Cross-platform process enum + name + cmd + cwd + start_time + parent. |
| `procfs` | `cfg(target_os = "linux")` | Typed `/proc` access; reliable fd enumeration. |
| `libproc` | `cfg(target_os = "macos")` | Rust binding for the OS API `lsof` uses; gives fd listing without shelling out. |

All three are well-maintained crates widely used by system tools
(`bottom`, `procs`, `bandwhich`). `sysinfo` is configured with
`default-features = false, features = ["system"]` to skip disk/network
scanners we don't need.

## Out of scope

- CPU and memory per session. `ProcessInfo` is designed to accept
  `cpu_pct: Option<f32>` and `rss_bytes: Option<u64>` additions without
  schema changes.
- Process-tree visualization. `parent_pid` is captured for future use.
- Windows support beyond a stub that keeps the crate compiling.
- Killing or signaling processes from the TUI.
- Historical/archived session PIDs (we only correlate against currently
  running processes).

## Implementation order

The feature ships as a single branch / single PR. Implementation proceeds
in this order so each step is independently testable before the next is
written:

1. `agtop-core::process` module with internal traits and fake-backed unit
   tests. No production consumers yet.
2. sysinfo-backed `Scanner` + Linux/macOS `FdScanner` implementations,
   integration-tested via the temp-file helper.
3. JSON fields on `SessionAnalysis` and correlator wiring in the CLI
   refresh layer. `--json` now reports PIDs.
4. TUI info-panel rows.
5. TUI sessions-table PID column.
6. README update covering the feature and the platform matrix.
