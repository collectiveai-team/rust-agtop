# rust-agtop

An htop-style monitor for AI coding agent sessions written in Rust.

Reads session transcripts from your local agent data directories, extracts
token usage, and estimates cost using built-in pricing tables.

## Supported agents

| Client       | Data source                                                  | Status       |
|--------------|--------------------------------------------------------------|--------------|
| Claude Code  | `~/.claude/projects/<slug>/<uuid>.jsonl`                     | Stable       |
| Codex        | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`               | Stable       |
| Gemini CLI   | `~/.gemini/tmp/<project_hash>/chats/*.(json|jsonl)`          | Best-effort  |
| OpenCode     | `~/.local/share/opencode/storage/{session,message}/…`        | Best-effort  |

Adding a new client is a single `impl Client` in `agtop-core::clients`.
See [`docs/gemini-cli.md`](docs/gemini-cli.md) for Gemini CLI integration
details, data sources, and implementation notes.

## Status

**v0.2** adds an interactive TUI (ratatui + crossterm) on top of the
v0.1 analysis engine. Three output modes:

- *(no flag)* — interactive htop-style dashboard (default).
- `--list` — human-readable one-shot table.
- `--json` — full analysis as JSON (per-bucket tokens + cost).

The TUI refreshes in the background every `--delay` seconds (default 2)
via a dedicated tokio worker thread; the core analysis layer remains
TUI-free.

## Build

```sh
cargo build --release
# binary at target/release/agtop
```

Rust toolchain: 1.75+ (works with 1.93 as of this commit).

## Usage

```sh
# Interactive TUI (default)
agtop

# Interactive TUI refreshing every 5 seconds instead of the default 2
agtop --delay 5

# Start directly in the btop-style dashboard view
agtop --dashboard

# One-shot table (good for scripts / CI)
agtop --list

# Only Claude Code sessions under the "Max" plan (Claude sessions marked included)
agtop --list --plan max --agentic-client claude

# Dump everything as JSON (good for piping to jq)
agtop --json

# Multiple agentic client filters
agtop --list --agentic-client claude --agentic-client codex

# Non-interactive refresh loop (for headless monitoring). Not valid with --json.
agtop --list --watch --delay 5
```

### TUI keys

| Keys                 | Action                                   |
|----------------------|------------------------------------------|
| `j` / `k` or ↓ / ↑   | Move selection                           |
| `g` / `G` (or Home/End) | Jump to first / last row              |
| PgUp / PgDn          | Move ±10 rows                            |
| `/`                  | Enter filter mode (Esc clears, Enter confirms) |
| `F6` or `>`          | Cycle sort column                        |
| `i`                  | Flip sort direction                      |
| `d`                  | Toggle classic/dashboard layout          |
| Tab / Shift-Tab      | Cycle bottom-panel tabs (Info, Cost)     |
| `F5` or `r`          | Manual refresh                           |
| `q` / `F10` / Ctrl-C | Quit                                     |

### Plans

- `retail` — standard API pricing for all clients (default).
- `max` — treats Claude sessions as included (Claude Max / Pro), retail elsewhere.
- `included` — every session marked as included (enterprise / bundled).

## Architecture

```
rust-agtop/
├── crates/
│   ├── agtop-core/         # Client trait + parsing + pricing (library)
│   │   └── src/clients/    # claude.rs, codex.rs, opencode.rs, util.rs
│   └── agtop-cli/          # `agtop` binary (clap + table/json output)
└── Cargo.toml              # workspace root
```

Core types:

- `Client` trait: `list_sessions()` + `analyze(summary, plan)`.
- `SessionSummary`: metadata discovered without re-reading the full transcript.
- `SessionAnalysis`: summary + `TokenTotals` + `CostBreakdown`.
- `PlanUsage`: best-effort plan/limit snapshots for dashboard panes.

The client layer is `Send + Sync`, so the upcoming TUI can drive it from
a background refresh thread.

## Testing

```sh
cargo test           # unit tests (pricing lookup + LiteLLM cache + parsing)
cargo clippy -- -D warnings
```

## Pricing data

agtop uses two sources, consulted in order:

1. **LiteLLM cache** — on first run (or when the cache is stale) agtop
   downloads [`model_prices_and_context_window.json`][litellm] to
   `~/.cache/agtop/litellm-pricing.json` with a 24 h TTL. Covers almost
   every model you're likely to see.
2. **Built-in fallback tables** in `agtop-core/src/pricing.rs`. Used for
   any model LiteLLM doesn't know about, and whenever agtop can't reach
   the network.

Control flags:

```sh
agtop --list --refresh-pricing      # force an immediate fetch
agtop --list --no-pricing-refresh   # stay fully offline (built-in tables only)
```

[litellm]: https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json

## Caveats

- Cost figures are **estimates** based on public API prices. Many subscription
  plans (Claude Max/Pro, ChatGPT Plus/Pro, etc.) charge flat rates or bundle
  tokens differently. Treat `$` as a resource-consumption proxy, not a bill.
- OpenCode's on-disk format is undocumented and may change. The client is
  conservative and degrades gracefully when fields are missing.
- Claude Code subagent sidechain transcripts (`<uuid>/subagents/*.jsonl`)
  **are** folded into the parent session's totals. The table marks such
  sessions as `<id>+N` where N is the number of sidechain files merged.
  The JSON output surfaces the same count as `subagent_file_count`.

## License

GPL-2.0-only, matching the upstream project.
