# Meta-Plan: Self-Dispatching Pipeline for the TUI Redesign

> **This document is the source of truth for the autonomous agent pipeline.**
>
> Four agents run sequentially in a worktree. Each agent executes one plan in full, commits its work, ticks the corresponding checkbox in this file, then dispatches the next agent and exits. The pipeline halts on any failure.
>
> **Read this file top-to-bottom before starting. Always.**

---

## Pipeline status

> Single source of truth. Each agent updates exactly one row before exiting.
> Status legend: `[ ]` pending · `[🚧]` in progress · `[x]` done · `[❌]` failed (see notes).

- [x] **Phase 0 — Worktree bootstrap** — verifies/creates worktree, dispatches Agent 1
- [x] **Phase 1 — Foundation** (`docs/superpowers/plans/2026-04-26-foundation.md`) — 22 tasks, ~core normalization + theme + widgets
- [x] **Phase 2 — Dashboard redesign** (`docs/superpowers/plans/2026-04-26-dashboard-redesign.md`) — 22 tasks, ~header + sessions + quota + info drawer
- [🚧] **Phase 3 — Aggregation view** (`docs/superpowers/plans/2026-04-26-aggregation-view.md`) — 8 tasks, ~group-by analytics
- [ ] **Phase 4 — Config view** (`docs/superpowers/plans/2026-04-26-config-view.md`) — 14 tasks, ~settings page
- [ ] **Phase 5 — Final acceptance** — runs spec §8 acceptance checklist, writes summary

When all 6 boxes above are `[x]`, the pipeline is complete.

---

## Pipeline rules (every agent reads these)

### R1. Worktree isolation
- **Every command runs inside the worktree.** No agent ever touches the original repo's working tree.
- The worktree path is recorded in **§Worktree info** below. If the file claims it doesn't exist, halt and report.
- Branch name: `feat/tui-redesign`.

### R2. Read this file first
- Open this meta-plan.
- Find the first `[ ]` Phase row. **That is your phase.**
- If a row is `[🚧]`, halt — a previous agent didn't finish cleanly. Report.
- If your phase is already `[x]`, halt — pipeline thinks you've already run. Report.

### R3. Mark phase in-progress
- Before doing any work, change your phase row from `[ ]` to `[🚧]`, append your start timestamp + agent id to the **§Run log**, commit with message `chore(pipeline): start phase N`, push to the worktree branch.

### R4. Execute the phase plan
- Open the linked plan file (e.g. `docs/superpowers/plans/2026-04-26-foundation.md`).
- Use the **superpowers:executing-plans** skill (or, if instructed, **superpowers:subagent-driven-development**) to execute every task.
- Each task in the plan has its own commits — keep them. The pipeline-level commit comes only at phase boundaries (start / finish / failure).

### R5. Verification gate (mandatory, no shortcuts)
After completing every task in the plan, run **all** of these inside the worktree:
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
**All three must succeed.** Output goes to a per-phase log under `.pipeline-logs/phase-<N>.log` (relative to the worktree root).

### R6. Failure handling (halt pipeline)
If verification fails, OR a plan task cannot be completed:
- Change your phase row from `[🚧]` to `[❌]`.
- Append a failure note to **§Run log** with: timestamp, the failing command, the last 50 lines of its output (or a truncation note pointing at the log file).
- Commit with message `chore(pipeline): phase N FAILED — <one-line summary>`, push.
- **Do NOT dispatch the next agent. Exit.**

### R7. Success → tick + dispatch + exit
On a clean verification:
- Change your phase row from `[🚧]` to `[x]`.
- Append a success note to **§Run log** (timestamp, total commits this phase, headline of last commit).
- Commit with message `chore(pipeline): phase N complete`, push.
- Dispatch the next agent (see **§Dispatch commands** for the exact line).
- Exit. **You are done. Do not wait for the next agent.**

### R8. No cross-phase work
- Agent N executes Phase N's plan only. Never edit later phases' plans or this meta-plan's later phase descriptions.
- The meta-plan **Pipeline status** checkboxes are the only structural fields you may edit, and only your own row.

### R9. Log files
- Each phase writes to `.pipeline-logs/phase-<N>.log` in the worktree.
- The dispatch command (see §Dispatch commands) routes the next agent's stdout/stderr to its own log file.
- Logs are gitignored (`.pipeline-logs/` is in `.gitignore` of the worktree, not the main repo — add it if missing).

### R10. Tooling
- You are running as `--agent build` with `--dangerously-skip-permissions` and full filesystem access in the worktree.
- Use `cargo`, `git`, `rg`/`grep`, the standard editing tools, and the superpowers skills.
- **Do not** run anything that reaches outside the worktree (no `cd ..`, no `--workspace-root` overrides outside the worktree).

---

## Worktree info

> Filled in by the dispatcher (§Phase 0 below). All agents read these values and use them as-is.

```yaml
worktree_path:    /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign
branch_name:      feat/tui-redesign
base_branch:      main
start_commit:     7cb7a3abe0af83e45c077db03fc39b05065d272f
created_at:       2026-04-27T07:11:55Z
```

If `worktree_path` is still `.worktrees/tui-redesign` and `start_commit` is `<sha>` (placeholder), Phase 0 has not run. Phase 0 is the only phase that creates the worktree.

---

## Dispatch commands

The first dispatch is manual (you fire Phase 0). Every subsequent dispatch is performed by the agent finishing its phase.

### IMPORTANT: opencode requires a server to attach to

`opencode run` will fail with `Error: Session not found` if there's no server running on the same database, because v1.14.x bootstraps via a server-proxy that expects an existing instance. **Solution:** the pipeline runs a dedicated headless `opencode serve` on a fixed port; every `opencode run` dispatch attaches to it via `--attach`.

The dedicated server's lifecycle:
- Started by the **manual kickoff** before Phase 0 dispatch (you, the human, do this).
- Persists for the entire pipeline.
- Stopped by Phase 5 (the final agent) right before it exits.

Server port: **37766** (one off from a typical TUI server on 37765 to avoid collision). If port 37766 is taken, the kickoff script must pick a different free port and substitute it everywhere.

### Manual kickoff (one-time, you run this)

Run this **once**, from the original repo root (NOT inside any worktree):

```bash
# 1. Start a dedicated headless opencode server for the pipeline.
mkdir -p /tmp/agtop-pipeline
nohup setsid opencode serve --hostname 127.0.0.1 --port 37766 \
  > /tmp/agtop-pipeline/server.log 2>&1 < /dev/null &
disown
SERVER_PID=$!
echo "$SERVER_PID" > /tmp/agtop-pipeline/server.pid
sleep 3   # let the server bind the port

# 2. Verify the server is responding.
curl -sf http://127.0.0.1:37766/api/health > /dev/null && echo "server up" || { echo "server failed to start; see /tmp/agtop-pipeline/server.log"; exit 1; }

# 3. Dispatch the Phase 0 agent (attached to the new server).
nohup setsid opencode run \
  "Read docs/superpowers/plans/2026-04-26-meta-pipeline.md and execute Phase 0 (Worktree bootstrap). Follow the rules in that file exactly. The pipeline server is at http://127.0.0.1:37766 and you should pass it via --attach to every dispatch you make. After verifying the worktree, dispatch Agent 1 and exit." \
  --attach "http://127.0.0.1:37766" \
  --title "agtop-pipeline-phase-0-bootstrap" \
  --model "anthropic/claude-sonnet-4-6" \
  --agent build \
  --dangerously-skip-permissions \
  > /tmp/agtop-pipeline/phase-0.log 2>&1 < /dev/null &
disown
echo "phase-0 dispatched, pid=$!"
```

Then watch `/tmp/agtop-pipeline/phase-0.log` for progress and the worktree path.

### Per-phase dispatch (used by agents 0→1, 1→2, 2→3, 3→4, 4→5)

The exact line each agent runs at the end of its phase. **Agent N runs the dispatch for Phase N+1.**

```bash
# Inside the worktree, after committing the phase-complete update:
nohup setsid opencode run \
  "Read $(pwd)/docs/superpowers/plans/2026-04-26-meta-pipeline.md and execute Phase <NEXT_N>. Worktree path is $(pwd). Pipeline server is at http://127.0.0.1:37766 (always pass --attach with this URL when dispatching). Follow the rules in the meta-plan exactly. After phase complete, dispatch the next agent and exit." \
  --attach "http://127.0.0.1:37766" \
  --title "agtop-pipeline-phase-<NEXT_N>" \
  --model "anthropic/claude-sonnet-4-6" \
  --agent build \
  --dangerously-skip-permissions \
  > "$(pwd)/.pipeline-logs/phase-<NEXT_N>.log" 2>&1 < /dev/null &
disown
```

**Substitution rules:**
- `<NEXT_N>` is the phase number this agent is about to dispatch (the next one, NOT the one this agent ran).
- `$(pwd)` is evaluated at dispatch time to inject the worktree path.
- `--attach "http://127.0.0.1:37766"` is **required**. Without it, the new agent's `opencode run` will fail with `Session not found`.
- The trailing `< /dev/null` is required to fully detach stdin so the parent process can exit.
- `disown` removes the process from the shell's job table so it survives the parent's exit.
- `setsid` runs the new process in its own session — independent of any TTY.

If `setsid` is unavailable on the host (rare on Linux, sometimes missing on macOS without coreutils), fall back to:
```bash
nohup opencode run "..." --attach "http://127.0.0.1:37766" > log 2>&1 < /dev/null &
disown
```

### Server cleanup (Phase 5 only)

The Phase 5 agent, after marking the pipeline complete, kills the dedicated server before exiting:

```bash
if [ -f /tmp/agtop-pipeline/server.pid ]; then
  kill "$(cat /tmp/agtop-pipeline/server.pid)" 2>/dev/null || true
  rm /tmp/agtop-pipeline/server.pid
fi
```

### Pipeline complete (Phase 5 → no dispatch)

Phase 5 is the terminal phase. Its agent ticks `[x]`, writes the final summary to **§Run log**, commits with `chore(pipeline): pipeline complete`, and exits without dispatching anything.

---

## Run log

> Append-only. Newest entries at the bottom. Each entry: `<phase> <status> <iso8601> <agent-id> <note>`.

```
phase-0 done 2026-04-27T07:11:55Z agent-phase-0 Worktree created at /home/rbarriga/collective.ai/projects/rust-agtop/.worktrees/tui-redesign, branch feat/tui-redesign forked from main@3dc85ff5, cargo build succeeded, dispatching Agent 1.
phase-1 start 2026-04-27T08:00:00Z agent-phase-1 Starting Phase 1 Foundation: 22 tasks for core normalization + theme + widgets.
phase-1 done  2026-04-27T07:55:34Z agent-phase-1 Phase 1 complete. 22 tasks done. cargo build/test/clippy all green. ~21 commits this phase. Head: 59229f5 fix(tui): add allow(dead_code) to foundation modules.
phase-2 start 2026-04-27T10:00:00Z agent-phase-2 Starting Phase 2 Dashboard redesign: 22 tasks for header + sessions + quota + info drawer.
phase-2 done  2026-04-27T12:30:00Z agent-phase-2 Phase 2 complete. 22 tasks done. cargo build/test/clippy all green. ~22 commits this phase. Head: fix(tui): add allow(dead_code) to legacy modules; fix clippy warnings.
phase-3 start 2026-04-27T14:00:00Z agent-phase-3 Starting Phase 3 Aggregation view: 8 tasks for group-by analytics.
```

---

## Phase 0 — Worktree bootstrap

**Acting agent's task:** create the worktree, populate **§Worktree info**, dispatch Agent 1, exit.

**Steps (the executing agent runs these):**

1. **Verify location.** You start in the original repo (e.g. `/home/rbarriga/collective.ai/projects/rust-agtop`). If `git rev-parse --is-inside-work-tree` returns `false`, halt with an error.

2. **Verify `opencode run` flag set.** Run `opencode run --help` and confirm the flags `--title`, `--model`, `--agent`, `--dangerously-skip-permissions` all exist. If any is missing or renamed, halt and report — the dispatch commands in this file will not work as written and need the user's correction before continuing.

3. **Verify clean state of `main`.** Run `git fetch && git rev-parse origin/main` — if there are uncommitted changes that would conflict with creating a worktree off `main`, halt and report. (The current repo state at pipeline-write time HAS uncommitted changes; the user must be informed and decide whether to stash, commit, or abort.)

4. **Compute worktree path.** Use `.worktrees/tui-redesign` if it does not exist. If it exists and is clean, reuse it. If it exists and is dirty, halt and report.

5. **Create the worktree.**
   ```bash
   git worktree add .worktrees/tui-redesign -b feat/tui-redesign main
   ```
   (Replace `main` with the actual base branch if different — read it via `git symbolic-ref refs/remotes/origin/HEAD`.)

6. **Bootstrap the worktree.**
   ```bash
   cd .worktrees/tui-redesign
   mkdir -p .pipeline-logs
   echo ".pipeline-logs/" >> .gitignore   # only if not already present
   git add .gitignore
   git commit -m "chore(pipeline): bootstrap worktree" || true
   ```

7. **Verify the toolchain.** Run `cargo build --workspace`. If it fails, halt — the worktree is unusable.

8. **Update §Worktree info in this file** with the real path, branch, base, current commit sha, ISO8601 timestamp. Commit:
   ```bash
   git add docs/superpowers/plans/2026-04-26-meta-pipeline.md
   git commit -m "chore(pipeline): bootstrap complete"
   ```

9. **Tick Phase 0** in **§Pipeline status** (`[ ]` → `[x]`), append a success entry to **§Run log**, commit with `chore(pipeline): phase 0 complete`, push to the worktree branch (`git push -u origin feat/tui-redesign`).

10. **Dispatch Agent 1** using the per-phase dispatch command (substitute `<NEXT_N>` = `1`). Example:
    ```bash
    nohup setsid opencode run \
      "Read $(pwd)/docs/superpowers/plans/2026-04-26-meta-pipeline.md and execute Phase 1. Worktree path is $(pwd). Follow the rules in that file exactly. After phase complete, dispatch the next agent and exit." \
      --title "agtop-pipeline-phase-1-foundation" \
      --model "anthropic/claude-sonnet-4-6" \
      --agent build \
      --dangerously-skip-permissions \
      > "$(pwd)/.pipeline-logs/phase-1.log" 2>&1 < /dev/null &
    disown
    ```

11. **Exit.** Do not wait. Do not check on the dispatched process. The meta-plan + log files are sufficient state for monitoring.

---

## Phase 1 — Foundation

**Plan:** `docs/superpowers/plans/2026-04-26-foundation.md`
**Tasks in plan:** 22 (Tasks 1–22, including the 2.5 parser sweep).
**Expected duration:** several hours of agent work.
**Critical:** Plan 1 includes the `SessionState` normalization in `agtop-core` (Task 2 + 2.5). This is the breaking change. Do not skip Task 2.5 (parser updates) — the workspace will not build without it.

**Steps (the Phase 1 agent runs these):**

1. Read this meta-plan top to bottom.
2. Confirm worktree info matches your `pwd`. Halt if mismatch.
3. Mark Phase 1 `[🚧]` in **§Pipeline status**, append a Run-log entry, commit (`chore(pipeline): start phase 1`).
4. Open `docs/superpowers/plans/2026-04-26-foundation.md` and execute **all 22 tasks** in order using the **superpowers:executing-plans** skill (or **subagent-driven-development** if you can dispatch sub-subagents).
5. Each plan task has its own checkbox steps and commits — do not skip them. The plan tells you exactly what to type and what to test.
6. After the last task, run the **R5 verification gate** (`cargo build / test / clippy`).
7. On success: mark Phase 1 `[x]`, log it, commit (`chore(pipeline): phase 1 complete`), push.
8. **Dispatch Agent 2** with `<NEXT_N>=2`, `--title "agtop-pipeline-phase-2-dashboard"`.
9. Exit.

On failure: R6 path — `[❌]`, log, commit `chore(pipeline): phase 1 FAILED`, push, exit without dispatching.

---

## Phase 2 — Dashboard redesign

**Plan:** `docs/superpowers/plans/2026-04-26-dashboard-redesign.md`
**Tasks in plan:** 22.
**Critical:** the plan contains a **Translation note** at the top about the post-state-normalization rewrite. Read it before touching any code that mentions `DisplayState`. The `state_style` module from Phase 1 is what you call instead.

**Steps (the Phase 2 agent runs these):**

1. Read this meta-plan top to bottom.
2. Confirm worktree info matches your `pwd`. Halt if mismatch.
3. Verify Phase 1 is `[x]` in **§Pipeline status**. If not, halt — you should not have been dispatched.
4. Mark Phase 2 `[🚧]`, log, commit (`chore(pipeline): start phase 2`).
5. Open `docs/superpowers/plans/2026-04-26-dashboard-redesign.md` and read the Translation note + every task. Execute all 22 tasks in order.
6. After the last task, run the **R5 verification gate**.
7. On success: mark Phase 2 `[x]`, log, commit (`chore(pipeline): phase 2 complete`), push.
8. **Dispatch Agent 3** with `<NEXT_N>=3`, `--title "agtop-pipeline-phase-3-aggregation"`.
9. Exit.

---

## Phase 3 — Aggregation view

**Plan:** `docs/superpowers/plans/2026-04-26-aggregation-view.md`
**Tasks in plan:** 8.
**Notes:** This plan has the smallest task count. Adds a new core module (`agtop_core::aggregate`) and the `screens/aggregation/*` files. Reuses `SessionsTable` from Phase 2 in the drill-down overlay.

**Steps:**

1. Read this meta-plan.
2. Confirm worktree, confirm Phase 2 `[x]`.
3. Mark Phase 3 `[🚧]`, log, commit (`chore(pipeline): start phase 3`).
4. Execute all 8 tasks in `docs/superpowers/plans/2026-04-26-aggregation-view.md` in order.
5. R5 verification gate.
6. Mark Phase 3 `[x]`, log, commit (`chore(pipeline): phase 3 complete`), push.
7. **Dispatch Agent 4** with `<NEXT_N>=4`, `--title "agtop-pipeline-phase-4-config"`.
8. Exit.

---

## Phase 4 — Config view

**Plan:** `docs/superpowers/plans/2026-04-26-config-view.md`
**Tasks in plan:** 14.
**Notes:** Replaces the placeholder Config screen with the full sidebar+detail VS Code-style settings page. Migrates the column editor from any remaining old config tab. Wires up immediate persistence for all setting changes.

**Steps:**

1. Read this meta-plan.
2. Confirm worktree, confirm Phase 3 `[x]`.
3. Mark Phase 4 `[🚧]`, log, commit (`chore(pipeline): start phase 4`).
4. Execute all 14 tasks in `docs/superpowers/plans/2026-04-26-config-view.md` in order.
5. R5 verification gate.
6. Mark Phase 4 `[x]`, log, commit (`chore(pipeline): phase 4 complete`), push.
7. **Dispatch Agent 5** with `<NEXT_N>=5`, `--title "agtop-pipeline-phase-5-acceptance"`.
8. Exit.

---

## Phase 5 — Final acceptance

**Goal:** validate the entire redesign against spec §8 acceptance criteria, write a summary, then exit. **No further dispatch.**

**Spec reference:** `docs/superpowers/specs/2026-04-26-tui-btop-redesign-design.md` §8

**Steps (the Phase 5 agent runs these):**

1. Read this meta-plan.
2. Confirm worktree, confirm Phase 4 `[x]`.
3. Mark Phase 5 `[🚧]`, log, commit (`chore(pipeline): start phase 5`).
4. Run the full **R5 verification gate** one more time. Halt on any failure.
5. Open the spec at `docs/superpowers/specs/2026-04-26-tui-btop-redesign-design.md` §8 (Acceptance criteria). Walk through each `[ ]` checkbox and verify it against the implementation. For each:
   - If it passes: tick it.
   - If it fails: do NOT tick. Add a sub-bullet under the criterion describing what's missing.
6. Run a smoke test: `cargo run -p agtop-cli` with a 30-second timeout. Verify the binary starts, the dashboard renders, `d/a/c/q` keys all work. Capture the test output to `.pipeline-logs/phase-5-smoke.log`.
7. Write a final summary to a new file `docs/superpowers/plans/2026-04-26-pipeline-summary.md` with:
   - Pipeline duration (start of Phase 0 → end of Phase 5).
   - Per-phase commit count and head SHA.
   - Acceptance criteria results (X/Y passed).
   - Any open follow-ups from §8 that weren't met.
   - The branch name and instructions for merging (`git checkout main && git merge feat/tui-redesign`, or open a PR).
8. Mark Phase 5 `[x]`, append a final Run-log entry summarizing the pipeline, commit (`chore(pipeline): pipeline complete`), push.
9. **Exit. No dispatch.** The pipeline is done.

---

## Failure recovery (manual)

If any phase ends with `[❌]`:

1. Read **§Run log** for the failure note.
2. Read the relevant `.pipeline-logs/phase-<N>.log` for the agent's full output.
3. Decide:
   - **Fix-and-resume:** manually fix the issue inside the worktree, set the failed phase row back to `[ ]`, commit your fix, then re-run the dispatcher for that phase only.
   - **Revert:** `git reset --hard <start_commit>` (from §Worktree info), `git push -f`, edit this meta-plan to set all phases back to `[ ]`, restart from Phase 0 dispatch.
   - **Abort:** `git worktree remove .worktrees/tui-redesign`, delete the branch, accept defeat.

There is no automatic retry. The pipeline is intentionally simple.

---

## Pre-pipeline sanity checklist (you run these once before kickoff)

Before triggering the manual Phase 0 dispatch, confirm in the **original repo root**:

- [ ] `opencode --version` works.
- [ ] `git status` shows the working tree state you expect (uncommitted changes will NOT be carried into the worktree, but if main has unmerged conflicts you should resolve them first).
- [ ] You have the model `anthropic/claude-sonnet-4-6` configured and accessible.
- [ ] `cargo build --workspace` succeeds on the current `main` (the worktree forks from here).
- [ ] You have ~10 GB free disk space (worktree clone + build artifacts).
- [ ] You're prepared for this to run for several hours unattended.

When all six are checked, run the **Manual kickoff** command from §Dispatch commands.

---

## Anti-patterns the agents must NOT do

- ❌ Edit this meta-plan's structure (only the Pipeline status checkboxes, Worktree info, and Run log are mutable, and only by the appropriate agent).
- ❌ Dispatch a phase out of order, or skip a phase, or dispatch your own phase again.
- ❌ Run any command outside the worktree.
- ❌ Modify the spec or other plan files (`docs/superpowers/specs/*`, `docs/superpowers/plans/*-foundation.md`, etc.) except where the plan you're executing tells you to.
- ❌ Use `--no-verify` on git commits, or skip the R5 verification gate, or interpret partial test failure as success.
- ❌ Continue past a failure. R6 says halt; halt means halt.
- ❌ Trust your memory over this file. Re-read this meta-plan at the start of every phase.

---

## Why this works

The meta-plan is the only shared state. Agents are stateless processes that read it, do their phase, update it, and dispatch the next process. The worktree's git history is the secondary state (every agent's work is committed and pushed). Failure is observable at multiple layers: the meta-plan checkboxes (`[❌]`), the run log (failure note), the per-phase log file (full output), and the git history (only successful phases push commits). You can audit the entire pipeline post-hoc without having watched it run.
