# Per-client PID matching reference

agtop attempts to bind each session it lists to the OS process that's
currently writing it. When the binding succeeds, the session table's
`PID` column shows the PID and the info panel shows liveness (`live` /
`stopped`) and match confidence.

This document explains how the matching works and what to expect for
each supported client.

If you only need a quick "does my client get a PID?" answer, jump to
the [Status matrix](#status-matrix).

---

## How matching works

For each session the correlator runs three tiers in priority order; the
first tier that produces a unique candidate wins.

| Tier | Signal | Confidence |
|------|--------|------------|
| A | A session id literal appears in the candidate's argv (e.g. `claude --resume <uuid>`, `opencode run -s <id>`). | High |
| B | The candidate has the session's transcript file open (procfs / `lsof`). | High |
| C | Candidate scores ≥ 2 / 3 against `(binary, cwd, start_time-in-window)`. | Medium |

A 1-PID-per-session invariant is enforced post-tier so a single daemon
can't be stamped onto every session it has hosted.

### Tier C scoring

```
score = binary(0|1) + cwd_match(0|1) + time_in_window(0|1)
```

with these gates and tie-breakers:

* **cwd hard gate.** If both candidate and session report a cwd and they
  disagree, score is forced to 0. Otherwise `binary + time` could
  launder a daemon in `/tmp/foo` onto a session in `/home/user/proj`.
* **Threshold ≥ 2** to be considered.
* **Descendant wins.** When two candidates tie, the one whose
  `parent_pid` is itself a candidate (the leaf in a wrapper chain like
  `npm-loader → node` or `bun → opencode`) is preferred.
* **Closer start_time wins.** When still tied, the candidate whose
  `start_time` is closer to `session.started_at` wins, with a 30s
  margin to avoid clock-jitter noise.
* **Recency dedup.** When multiple sessions in the same `(cwd, client)`
  group all match the same PID (typical for a long-lived daemon hosting
  many historical sessions), only the session with the most-recent
  `last_active` keeps the binding.

### Scanner-level filters

* **Thread leaders only.** Threads (TIDs under `/proc/<pid>/task`)
  inherit the parent's `comm` and would create phantom-tie candidates;
  agtop only enumerates thread group leaders.
* **Daemon exclusion.** `mcp-server`, `mcp-stdio`, `daemon`,
  `--input-format stream-json`, `*.app/` (macOS bundles).
  `serve` / `server` / `app-server` are NOT excluded — those are
  user-facing on Linux+VSCode setups.
* **Node main-thread comm normalization.** Recent Node.js renames the
  main thread comm via `prctl(PR_SET_NAME)`. Two forms are seen in the
  wild and both are accepted:
  * Node v25.x emits `node-MainThread`
  * Node v24.x emits bare `MainThread`
  
  Both normalize back to `"node"` so the rest of the pipeline stays
  ignorant of kernel/comm version quirks. Without this normalization,
  Gemini CLI under nvm-managed Node 24 would never produce candidates.

### Subagents

Subagents (Claude `Task` tool, Codex `thread_spawn`, Gemini parent-keyed
sessions) execute IN-PROCESS within their parent CLI — the same OS
process, just async tasks on the event loop. The correlator binds
sessions to PIDs and a separate pass propagates the parent's
`(pid, liveness, confidence)` onto every entry in `session.children`,
so the TUI shows the correct PID for any selected subagent.

---

## Status matrix

Status legend:

- **Live** — battle-tested on a real running process.
- **Should work** — code path implemented and unit-tested but not
  exercised live.
- **Limited** — partially supported; see notes.
- **Not supported** — explicitly out of scope or no signal available.

| Client      | Tier A (argv)                               | Tier B (fd)                | Tier C (cwd+score)  | Notes |
|-------------|---------------------------------------------|----------------------------|---------------------|-------|
| Claude      | Live (`-r` / `--resume` / `--session-id`)   | Live (`*.jsonl`)           | Live                | Subagents share parent PID (in-process Task tool). |
| Codex       | Should work (`resume <uuid>`, `fork <uuid>`) | Should work (`*.jsonl`)   | Live (`app-server`) | `app-server` is the user-facing Codex IDE backend on Linux. |
| Gemini CLI  | Live (`-r` / `--resume`)                    | Should work (`*.jsonl`)    | Live                | Multi-file `--resume` and Node v24/v25 hosts both supported; see [Gemini CLI](#gemini-cli). |
| OpenCode    | Live (`-s ses_…`)                           | Disabled (shared SQLite)   | Live                | argv-tier uses an OpenCode-shaped id validator (`ses_` + 26 base62 chars). |
| Copilot     | Not supported (no public resume flag)       | Should work (`*.jsonl`)    | Limited             | CLI-mode sessions at `~/.copilot/session-state/<uuid>/events.jsonl` are not yet surfaced by the parser. |
| Cursor      | Not supported                               | Should work (`*.jsonl`)    | Should work         | Standard cwd + fd tiers; not exercised live. |
| Antigravity | Not supported                               | Disabled (shared SQLite)   | Should work         | Same shared-DB caveat as OpenCode. |

---

## Per-client details

### Claude

| Aspect                | Value                                              |
|-----------------------|----------------------------------------------------|
| Binary names accepted | `claude`                                           |
| Daemon-exclusion args | `mcp-server`, `--input-format stream-json`         |
| argv flags (Tier A)   | `-r`, `--resume`, `--session-id` (UUID-shaped)     |
| fd path (Tier B)      | `<projects>/<slug>/<session_id>.jsonl`             |
| Subagents             | In-process; PID propagated from parent             |

The `claude --output-format stream-json --input-format stream-json …`
helper (the IDE bridge) is excluded from candidates because it isn't
an interactive session and would tie with the real `claude`.

### Codex

| Aspect                | Value                                                           |
|-----------------------|-----------------------------------------------------------------|
| Binary names accepted | `codex`                                                         |
| Daemon-exclusion args | `mcp-server`, `mcp-stdio`                                       |
| argv subcommands (A)  | `resume <uuid>`, `fork <uuid>`                                  |
| fd path (Tier B)      | `<rollouts>/<slug>/<session_id>.jsonl`                          |
| Subagents             | Tracked via `parent_thread_id`; PID propagated from parent      |

`codex app-server` is the user-facing IDE backend on Linux+VSCode and
is matched via cwd-tier. `codex exec` and standalone `codex resume`
are also supported via argv-tier.

### Gemini CLI

| Aspect                | Value                                                                  |
|-----------------------|------------------------------------------------------------------------|
| Binary names accepted | `gemini`, `node`, `node-MainThread`, `MainThread` (last three normalized to `node`) |
| argv flags (Tier A)   | `-r`, `--resume` (UUID-shaped)                                         |
| fd path (Tier B)      | `~/.gemini/tmp/<slug>/chats/session-<datetime>-<shortid>.jsonl`        |
| Subagents             | Stored under `<parent_session_id>/<subagent_id>.jsonl`; PID propagated |

Two Gemini-specific behaviors are handled in the parser/scanner:

* **Node host detection.** Gemini CLI runs under Node and the kernel
  exposes the main thread comm via prctl-rename. Different Node majors
  use different rename shapes (`node-MainThread` on v25, bare
  `MainThread` on v24). Both are accepted and normalized to `"node"`.
* **`--resume` writes a NEW transcript file each invocation** but
  every file's first record carries the SAME `sessionId`. The Gemini
  parser collapses these into one logical session (earliest
  `started_at`, latest `last_active`, newest file's `data_path`). See
  `docs/gemini-cli.md` for the full discussion of this tradeoff.

When `--resume` argv passes the session UUID, Tier A matches at HIGH
confidence. Otherwise Tier C with the descendant tie-breaker picks
the leaf node process out of the npm-loader → real-gemini wrapper
chain.

### OpenCode

| Aspect                | Value                                                                  |
|-----------------------|------------------------------------------------------------------------|
| Binary names accepted | `opencode`                                                             |
| argv flags (Tier A)   | `-s`, `--session` with OpenCode-id shape (`ses_` + 26 base62 chars)    |
| fd (Tier B)           | Disabled — shared SQLite DB at `~/.local/share/opencode/opencode.db`   |
| Subagents             | In-process (model-orchestrated `task` tool); PID propagated            |

OpenCode session IDs are NOT UUIDs; argv-tier uses an OpenCode-shaped
validator. fd-tier is disabled because every session shares the same
SQLite DB — holding the DB open identifies the daemon, not a session.
Multiple `opencode serve` daemons in different cwds and `opencode run`
clients are all disambiguated via Tier C with cwd hard gating and the
closest-start-time tie-breaker.

### Copilot

| Aspect                | Value                                                |
|-----------------------|------------------------------------------------------|
| Binary names accepted | `copilot`, `gh-copilot`                              |
| argv flags (Tier A)   | None (no public resume flag)                         |
| fd path (Tier B)      | `<workspaceStorage>/.../chatSessions/<uuid>.jsonl`   |

Copilot CLI (`@github/copilot`) writes session events to
`~/.copilot/session-state/<uuid>/events.jsonl`. The current copilot
parser only reads VSCode-side `chatSessions`, so CLI-mode sessions
don't appear in agtop and the correlator has nothing to match against
for them. The native ELF binary reports `comm == "copilot"` (no Node
renaming) so once parser support lands, fd-tier should work without
further correlator changes.

### Cursor

| Aspect                | Value                                       |
|-----------------------|---------------------------------------------|
| Binary names accepted | `cursor`, `cursor-agent`                    |
| argv flags (Tier A)   | None implemented                            |
| fd path (Tier B)      | `<cursor data>/sessions/<uuid>.jsonl`       |

No special handling; standard cwd + fd tier coverage.

### Antigravity

| Aspect                | Value                                                                |
|-----------------------|----------------------------------------------------------------------|
| Binary names accepted | `antigravity`                                                        |
| Daemon-exclusion args | `*.app/` paths (macOS desktop bundle)                                |
| fd (Tier B)           | Disabled — shared SQLite DB (same reasoning as OpenCode)             |

Antigravity is a VSCode-fork IDE and shares the SQLite-fan-out problem.
Tier C is the only path.

---

## Known gaps

* **Copilot CLI parser.** Add support for
  `~/.copilot/session-state/<uuid>/events.jsonl` so CLI-mode Copilot
  sessions appear in agtop. The correlator already knows how to match
  them via fd-tier once they're surfaced.
* **JSON `children` exposure.** `--json` output today only includes
  top-level sessions; subagent PIDs are propagated internally but not
  serialized.
* **Cursor / Antigravity live verification.** Implementations exist;
  not yet exercised on a live process.
