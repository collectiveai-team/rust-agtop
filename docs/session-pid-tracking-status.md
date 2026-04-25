# Session PID Tracking — Per-Client Integration Status

Companion to [`docs/specs/2026-04-24-session-pid-tracking-design.md`](specs/2026-04-24-session-pid-tracking-design.md).
That spec describes the original two-tier design. This document captures
the **post-spec evolution** discovered during live battle-testing on PR #28
(branch `feature/session-pid-tracking`): which decisions changed, why, and
what each client kind currently supports.

If you only need a quick "does my client get a PID?" answer, jump to the
[Status matrix](#status-matrix).

> **Coordination note.** A separate session is currently fixing parser
> bugs in `docs/gemini-cli.md` / the gemini-cli client. This document
> covers ONLY the PID-correlation pipeline. Per-client transcript
> parsing, quota providers, and TUI rendering details belong in the
> per-client docs and are owned elsewhere.

---

## Architecture recap

The correlator (`crates/agtop-core/src/process/`) is invoked once per TUI
refresh and once per `--json` one-shot. It:

1. Scans candidate processes (`scanner::SysinfoScanner`).
2. Optionally inspects each candidate's open file descriptors
   (`fd::LinuxFdScanner` on Linux; `default_fd_scanner()` on macOS).
3. Walks all session summaries and, for each, runs three tiers in
   priority order; the first that succeeds wins.
4. Returns a `HashMap<session_id, ProcessInfo>` containing pid, liveness,
   and `Confidence`.
5. The CLI calls `process::attach_process_info(map, &mut analyses)` to
   write PID/liveness/confidence onto each parent **and propagate to
   subagent children** (subagents share their parent's OS process).

### The three tiers

| Tier | Name              | Confidence | Signal                                                                         |
|------|-------------------|------------|--------------------------------------------------------------------------------|
| A    | argv UUID         | High       | A session id literal appears in the candidate's argv (e.g. `--resume <uuid>`). |
| B    | fd UUID-in-path   | High       | The candidate has the session's transcript file open (procfs / lsof).          |
| C    | cwd + binary + start-time score | Medium | Score 2/3 against `(binary, cwd, start_time-in-window)`.   |

A 1-PID-per-session invariant is enforced post-tier (via `enforce_unique_pid`)
to prevent the same daemon from being stamped onto every session.

### Tier C scoring (current rules)

```
score = binary(0|1) + cwd_match(0|1) + time_in_window(0|1)
```

with these gates and tie-breakers (added during battle-testing):

* **cwd hard gate.** If both candidate and session report a cwd and they
  disagree, score is forced to 0. Without this, `binary + time` could
  launder a daemon in `/tmp/foo` onto a session in `/home/user/proj`.
* **Threshold ≥ 2** to be considered.
* **Tie-breaker 1: descendant wins.** When two candidates tie, the one
  whose `parent_pid` is itself a candidate (the leaf in a wrapper chain
  like `npm-loader → node` or `bun → opencode`) is preferred.
* **Tie-breaker 2: closer start_time.** When still tied, the candidate
  whose `start_time` is closer to `session.started_at` wins, with a 30s
  margin to avoid clock-jitter noise.
* **Recency dedup.** When multiple sessions in the same `(cwd, client)`
  group all match the same PID (typical for a long-lived daemon hosting
  many historical sessions), the session with the most-recent
  `last_active` wins.

### Scanner-level filters

* **Thread leaders only.** `proc.thread_kind().is_some()` skips threads
  (sysinfo enumerates each TID as if it were a process; threads inherit
  parent comm, which would create phantom-tie candidates).
* **Daemon exclusion** (narrowed list — see history): `mcp-server`,
  `mcp-stdio`, `daemon`, `--input-format stream-json`, `*.app/`. We
  deliberately do NOT exclude `serve`, `server`, or `app-server` (those
  are user-facing in the OpenCode/Codex IDE setups).
* **Comm normalization.** Recent Node.js renames the main thread comm
  to `node-MainThread`; we normalize that back to `"node"` so
  `expected_binaries(GeminiCli)` matches.

---

## Status matrix

Status legend:
- **Live-verified** — battle-tested on a running process during PR #28.
- **Should work** — code path implemented and unit-tested but not
  reproduced live (usually because no live process was available on the
  test machine).
- **Limited** — partially implemented; see notes.
- **Not supported** — explicitly out of scope or no signal available.

| Client      | Tier A (argv)               | Tier B (fd)               | Tier C (cwd) | Notes                                                       |
|-------------|-----------------------------|---------------------------|--------------|-------------------------------------------------------------|
| Claude      | live (`-r`/`--resume`/`--session-id`) | live (`*.jsonl`) | live | Subagents share parent PID (in-process Task tool).          |
| Codex       | should work (`resume`/`fork` UUID) | should work (`*.jsonl`) | live (app-server) | Tested only against IDE `app-server`; no live `codex exec`. |
| Gemini CLI  | live (`-r`/`--resume` after `c7264ee`) | should work (`*.jsonl`) | live | Parser-side duplicate-id collapse landed in `c7264ee` (separate session); correlator side battle-tested in `6f075cc`. See [Gemini CLI](#gemini-cli) for the parser/correlator split. |
| OpenCode    | live (`-s ses_…`)           | disabled (shared SQLite)   | live | argv-tier requires the new opencode-id validator; live-verified with `opencode run -s`. |
| Copilot     | n/a (no resume flag)        | should work (`*.jsonl`)   | n/a          | Live CLI sessions stored at `~/.copilot/session-state/<uuid>/events.jsonl` are NOT yet picked up by the parser; correlator can't match what the parser doesn't surface. |
| Cursor      | n/a                         | should work (`*.jsonl`)   | should work  | No live cursor-agent process available on the test machine. |
| Antigravity | n/a                         | disabled (shared SQLite)   | should work  | Same SQLite caveat as OpenCode.                             |

---

## Per-client integration details

### Claude

**Live status:** ✅ Live-verified on PR #28.

| Aspect                | Value                                              |
|-----------------------|----------------------------------------------------|
| Binary names accepted | `claude`                                           |
| Daemon-exclusion args | `mcp-server`, `--input-format stream-json`         |
| argv flags (Tier A)   | `-r`, `--resume`, `--session-id` (all UUID-shaped) |
| fd path (Tier B)      | `<projects>/<slug>/<session_id>.jsonl`             |
| Subagent location     | `<projects>/<slug>/<session_id>/subagents/agent-*.jsonl` |

**Key decisions:**

* `--session-id <uuid>` was added to argv-tier in commit `6f075cc`. The
  prior implementation only matched `-r/--resume`; freshly-spawned
  Claude sessions with `--session-id` could not be matched at High
  confidence.
* `claude --output-format stream-json --input-format stream-json …`
  (the IDE bridge helper) is excluded from candidates: it's not an
  interactive session and would tie with the real bare `claude`.
* `claude mcp-server` is excluded: MCP transport child, not a session.
* **Subagents.** Claude `Task` tool subagents run in-process within the
  parent `claude` process (verified empirically: `pgrep -x claude` shows
  one PID even when 3 subagents are running). `attach_process_info`
  propagates the parent's `(pid, liveness, confidence)` onto each
  `parent.children` entry so the TUI shows the correct PID for any
  selected subagent.

**Live test (PR #28):**
```
$ claude --session-id 0b8dbe7a-aa98-4ad9-86de-2ac02508a58d \
         --model haiku -p 'sleep 12 && echo done'
→ matched within 3s at HIGH confidence (argv tier)
```

---

### Codex

**Live status:** ⚠️ Partial — `codex app-server` (IDE backend) live-verified;
`codex exec` could not be tested due to ChatGPT-account model restrictions
(`gpt-5.5` requires a newer Codex than the installed version).

| Aspect                | Value                                                            |
|-----------------------|------------------------------------------------------------------|
| Binary names accepted | `codex`                                                          |
| Daemon-exclusion args | `mcp-server`, `mcp-stdio`                                        |
| argv subcommands (A)  | `resume <uuid>`, `fork <uuid>`                                   |
| fd path (Tier B)      | `<rollouts>/<slug>/<session_id>.jsonl`                           |
| Subagent semantics    | `parent_thread_id` in session_meta — also in-process; propagation applies |

**Key decisions:**

* `app-server` was REMOVED from the daemon-exclusion list in commit
  `6f075cc`. On Linux+VSCode, `codex app-server --analytics-default-enabled`
  IS the user-facing Codex IDE backend; banning it prevented every
  Codex session from ever matching. The `app-server` daemon is matched
  via cwd-tier; recency dedup picks the active session.
* Live verified: `codex app-server` PID 13220 (cwd `/home/rbarriga`)
  matched session `019dbceb-1cc5-7fa0-ba59-aeef89db4163` (same cwd).
* `codex resume <uuid>` and `codex fork <uuid>` argv-tier extraction
  exists in `argv_uuid::find_subcommand_uuid`. Should work; not
  battle-tested live because of the model-version block on this machine.

---

### Gemini CLI

**Live status:** ✅ Live-verified on PR #28 (cwd-tier with descendant tie-breaker).

> **Coordination boundary.** Per the user, a separate session is
> actively fixing **gemini-cli parser/transcript** issues. This section
> documents only the PID-correlation side. Do not modify gemini-cli's
> client parser (`crates/agtop-core/src/clients/gemini_cli.rs`),
> session discovery, or `docs/gemini-cli.md` from the correlator
> branch.

| Aspect                | Value                                                       |
|-----------------------|-------------------------------------------------------------|
| Binary names accepted | `gemini`, `node` (or `node-MainThread`, normalized to `node`) |
| argv flags (Tier A)   | `-r`, `--resume` (UUID-shaped)                              |
| fd path (Tier B)      | `<gemini_data>/.../<session-name>.jsonl`                    |
| Subagent location     | `<parent_session_id>/<subagent_id>.jsonl`                   |

**Key decisions:**

* `node-MainThread` (Linux Node.js main-thread comm rename) is now an
  accepted binary form for gemini detection and is normalized back to
  `"node"` in the candidate so downstream code stays kernel-comm
  agnostic. Without this, every Gemini CLI candidate was rejected at
  the scanner level.
* The descendant tie-breaker (Tier C) handles the npm-loader → real
  gemini wrapper chain that produces two near-identical `node-MainThread`
  candidates with the same cwd, argv, and start_time. The leaf wins.
* Live test (PR #28): `gemini -p 'shell sleep 18'` matched at PID
  1222329 with Medium confidence; the npm-loader pre-fork (parent of
  1222329) was correctly rejected by the descendant rule.

**What the correlator-side commits (`6f075cc`, `60c1424`, `b448e06`) do
NOT change:**

* Gemini transcript parsing (the `clients::gemini_cli` module).
* Subagent file discovery in `<parent>/<child>.jsonl` subdirectories.
* Quota / pricing / model handling.

**Parser-side dedup (separate commit, `c7264ee`).** A concurrent
session landed `fix(gemini-cli): collapse duplicate-id sessions from
--resume`, which addresses a Gemini-only quirk: each
`gemini --resume <uuid>` writes a NEW `session-<datetime>-<shortid>.jsonl`
file but every file's first record carries the SAME `sessionId`. Without
collapsing, two `SessionSummary` entries share an id; the correlator's
`HashMap<session_id, ProcessInfo>` can only hold one binding, so one
row appears orphaned and the other PID can leak to an unrelated session
in the same workspace. With the parser-side collapse in place, the
correlator now sees one summary per logical session and the argv-tier
(`-r/--resume <uuid>`) matches at High confidence cleanly. See that
commit's message and `docs/gemini-cli.md` for the details.

If a gemini session is correctly listed by `agtop --json` but no PID is
attached, that's a correlator-side issue (this doc's territory). If the
session itself is missing/wrong/duplicated, that's a parser-side issue
(see `docs/gemini-cli.md`).

---

### OpenCode

**Live status:** ✅ Live-verified on PR #28 — including the multi-process
`opencode serve` + `opencode run` scenarios.

| Aspect                | Value                                                |
|-----------------------|------------------------------------------------------|
| Binary names accepted | `opencode`                                           |
| Daemon-exclusion args | none specific (was `serve` in spec; removed)         |
| argv flags (Tier A)   | `-s`, `--session` with **OpenCode id shape** (`ses_` + 26 base62 chars), NOT a UUID |
| fd (Tier B)           | **disabled** — shared SQLite DB                      |
| Subagent semantics    | model-orchestrated `task` tool; same OS process; propagation applies |

**Key decisions (chronological):**

1. **Original spec** assumed Tier B (fd) would identify per-session.
   This is wrong for OpenCode: every session shares the same SQLite
   DB (`~/.local/share/opencode/opencode.db`). Holding it open
   identifies the daemon, not a session — so fd-tier was disabled
   (`paths_for` returns `vec![]` for OpenCode/Antigravity).
2. **Original aggressive daemon exclusion** banned `serve`. On Linux+
   VSCode this banned the user-facing OpenCode entirely (the webview
   talks HTTP to `opencode serve`). Fixed in `6f075cc`: removed
   `serve`/`server`/`app-server` from the exclusion list. Cwd+recency
   handles attribution.
3. **OpenCode session IDs are not UUIDs.** They look like
   `ses_23c93ae9fffeyoWWO2jksO2OxI` (length 30 = `ses_` + 26 base62
   chars). The argv-tier UUID validator rejected them, so
   `opencode run -s <id>` could never match at High confidence. Fixed
   in `b448e06`: added `is_valid_opencode_id` and a per-client
   validator selector (`id_validator(client)`).
4. **Multi-process disambiguation.** Live testing with 4 simultaneous
   opencode processes in two cwds (2 `serve` daemons + 2 `run` clients)
   exposed two ambiguities, both fixed in `b448e06`:
   * cwd-mismatch laundering: a fresh daemon scored 2 against an
     unrelated session via binary+time. Fix: cwd hard gate.
   * Sibling tie (daemon vs run-client in same cwd): added
     closest-start-time tie-breaker.

**Live test results (PR #28):**

| Process                                     | PID     | Session                            | Confidence | Tier        |
|---------------------------------------------|---------|------------------------------------|------------|-------------|
| `opencode serve` (interactive, rust-agtop)  | 987184  | `ses_23c93ae9fffeyoWWO2jksO2OxI`   | medium     | cwd+recency |
| `opencode serve` (test, /tmp/agtop-bt-oc)   | 1242026 | `ses_23c49eed2ffenKzfGl0qT9LQ1Y`   | medium     | cwd+start-time |
| `opencode run -s <id>` (resume client)      | 1264984 | `ses_23cf9cbc0ffe8FG6hs4acdn4Wt`   | **high**   | argv        |
| `opencode run` (subagent client)            | 1265027 | `ses_23c4158d3ffeu6vHu9Dxbh5qeZ`   | medium     | cwd+start-time |

---

### Copilot

**Live status:** ⚠️ Limited — correlator support exists, but the parser
does not yet surface CLI-mode sessions (only IDE-side `chatSessions`).

| Aspect                | Value                                                |
|-----------------------|------------------------------------------------------|
| Binary names accepted | `copilot`, `gh-copilot`                              |
| Daemon-exclusion args | none                                                 |
| argv flags (Tier A)   | none implemented (no --resume flag publicly known)   |
| fd path (Tier B)      | `<workspaceStorage>/.../chatSessions/<uuid>.jsonl`   |

**Key decisions:**

* Copilot CLI (`@github/copilot`) writes session events to
  `~/.copilot/session-state/<uuid>/events.jsonl`. The current copilot
  parser only reads VSCode-side `~/.config/Code/User/workspaceStorage/.../chatSessions/<uuid>.jsonl`.
  CLI sessions therefore don't appear in agtop, and the correlator has
  nothing to match against.
* This is a parser-side gap, not a correlator-side one. It's noted
  here for traceability; fixing it is out of scope for this branch.
* The native ELF binary at
  `/usr/lib/node_modules/@github/copilot/node_modules/@github/copilot-linux-x64/copilot`
  reports `comm == "copilot"` (no Node renaming), so when the parser
  is fixed, fd-tier should work without further correlator changes.

---

### Cursor

**Live status:** Should work — no live cursor-agent process on the test
machine to verify.

| Aspect                | Value                                       |
|-----------------------|---------------------------------------------|
| Binary names accepted | `cursor`, `cursor-agent`                    |
| Daemon-exclusion args | none                                        |
| argv flags (Tier A)   | none implemented                            |
| fd path (Tier B)      | `<cursor data>/sessions/<uuid>.jsonl`       |

No special handling. Standard cwd+fd tier coverage. Should match a live
`cursor-agent` process if one exists; not exercised in PR #28.

---

### Antigravity

**Live status:** Should work — no live antigravity process on the test
machine to verify.

| Aspect                | Value                                                                |
|-----------------------|----------------------------------------------------------------------|
| Binary names accepted | `antigravity`                                                        |
| Daemon-exclusion args | `*.app/` paths (macOS desktop bundle)                                |
| argv flags (Tier A)   | none implemented                                                     |
| fd (Tier B)           | **disabled** — shared SQLite DB (same reasoning as OpenCode)         |

Antigravity is a VSCode-fork IDE ("Jetski" agent) and shares the
SQLite-fan-out problem. Cwd-tier is the only path; should work but is
unverified live.

---

## History (PR #28 commits)

This branch evolved through six commits relevant to PID correlation.
Each addresses distinct bugs surfaced by progressively-deeper testing.
The first five are correlator-side (this doc's territory); the sixth
is parser-side (owned by a parallel session) and listed for reader
context because it removes a duplicate-id condition the correlator
otherwise can't disambiguate.

| Commit  | Theme                                               | Impact                                  |
|---------|-----------------------------------------------------|-----------------------------------------|
| `933424b` | Disable fd-tier for SQLite clients; add argv-UUID extractor | Fixed OpenCode "all sessions same PID". |
| `7f7df83` | Three-tier algorithm (argv → fd → cwd+recency)      | Established the current tier structure. |
| `6f075cc` | Real Linux/VSCode setups: thread filter, daemon-exclusion narrowed, `--session-id` for Claude, `node-MainThread` normalization, descendant tie-breaker | Made it work on the user's actual machine; lifted matched-session count from 0 to 3. |
| `60c1424` | Subagent PID propagation                            | Children inherit parent's PID/liveness/confidence via `attach_process_info`. |
| `b448e06` | Multi-process OpenCode: cwd hard gate, OpenCode-id argv validator, closest-start-time tie-breaker | Made `opencode run -s` get HIGH confidence; fixed false matches and sibling ties. |
| `c7264ee` | (parser-side, parallel session) Gemini-cli `--resume` duplicate-id collapse | Removes the duplicate `(session_id)` rows that would otherwise force the correlator to drop one of two valid PID matches. |

Each commit message includes a "Bug → root cause → fix" trace; read
those for the empirical data behind each decision.

---

## Open issues / future work

* **Copilot CLI parser.** Add support for
  `~/.copilot/session-state/<uuid>/events.jsonl` so CLI Copilot
  sessions appear in agtop. Once they do, the correlator already
  knows how to match them via fd-tier (binary `copilot`, native ELF).
* **JSON children exposure.** `--json` output today only includes
  top-level sessions. Subagent PID propagation works (in `attach_process_info`)
  but subagents are never serialized to JSON because `JsonSession`
  has no `children` field and `analyze_all` doesn't call
  `client.children()`. Adding both is straightforward but is a
  schema change to `--json` output.
* **`codex exec` live verification.** The installed Codex CLI on the
  test machine targets `gpt-5.5`, which the user's ChatGPT account
  doesn't authorize. With a working model the standard `codex exec`
  argv-tier path (`codex resume <uuid>`) should match at High; until
  then, only the IDE `app-server` cwd-tier path is live-verified.
* **Cursor / Antigravity live verification.** Same as above —
  implementations exist, no live process available on the test
  machine to confirm.
