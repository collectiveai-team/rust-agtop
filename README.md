# rust-agtop

An htop-style monitor for AI coding agent sessions written in Rust.

Reads session transcripts from your local agent data directories, extracts
token usage, and estimates cost using built-in pricing tables.

## Supported agents

| Provider     | Data source                                                  | Status       |
|--------------|--------------------------------------------------------------|--------------|
| Claude Code  | `~/.claude/projects/<slug>/<uuid>.jsonl`                     | Stable       |
| Codex        | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`               | Stable       |
| OpenCode     | `~/.local/share/opencode/storage/{session,message}/…`        | Best-effort  |

Adding a new provider is a single `impl Provider` in `agtop-core::providers`.

## Status

**MVP scope** — v0.1.0 ships session discovery, token extraction, and cost
estimation with two output modes:

- `--list` — human-readable table (default when invoked with no args).
- `--json` — full analysis as JSON (includes per-bucket token + cost breakdown).

The original JS agtop's interactive htop-style TUI (tabs, filters, sort,
mouse support, delete, etc.) is planned for a follow-up release. The `ratatui`
crate is already selected; the provider-abstraction layer is TUI-ready.

## Build

```sh
cargo build --release
# binary at target/release/agtop
```

Rust toolchain: 1.75+ (works with 1.93 as of this commit).

## Usage

```sh
# Table of all sessions across all providers
agtop --list

# Only Claude Code sessions under the "Max" plan (Claude sessions marked included)
agtop --list --plan max --provider claude

# Dump everything as JSON (good for piping to jq)
agtop --json

# Multiple provider filters
agtop --list --provider claude --provider codex

# Refresh the table every 5 seconds (Ctrl-C to exit). Useful for CI/ops-
# style monitoring until the interactive TUI lands. Not valid with --json.
agtop --list --watch --delay 5
```

### Plans

- `retail` — standard API pricing for all providers (default).
- `max` — treats Claude sessions as included (Claude Max / Pro), retail elsewhere.
- `included` — every session marked as included (enterprise / bundled).

## Architecture

```
rust-agtop/
├── crates/
│   ├── agtop-core/         # Provider trait + parsing + pricing (library)
│   │   └── src/providers/  # claude.rs, codex.rs, opencode.rs, util.rs
│   └── agtop-cli/          # `agtop` binary (clap + table/json output)
└── Cargo.toml              # workspace root
```

Core types:

- `Provider` trait: `list_sessions()` + `analyze(summary, plan)`.
- `SessionSummary`: metadata discovered without re-reading the full transcript.
- `SessionAnalysis`: summary + `TokenTotals` + `CostBreakdown`.

The provider layer is `Send + Sync`, so the upcoming TUI can drive it from
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
- OpenCode's on-disk format is undocumented and may change. The provider is
  conservative and degrades gracefully when fields are missing.
- Claude Code subagent sidechain transcripts (`<uuid>/subagents/*.jsonl`)
  **are** folded into the parent session's totals. The table marks such
  sessions as `<id>+N` where N is the number of sidechain files merged.
  The JSON output surfaces the same count as `subagent_file_count`.

## License

GPL-2.0-only, matching the upstream project.
