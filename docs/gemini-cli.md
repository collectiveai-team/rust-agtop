# Gemini CLI integration

This document describes how agtop integrates with Gemini CLI, which metrics it
surfaces through the common session model, and what remains version-sensitive.

## Overview

Gemini CLI is an AI coding agent with three useful local integration surfaces:

- session history files for passive discovery and titles;
- OpenTelemetry output for request, token, latency, tool, and agent metrics;
- headless JSON/JSONL output for runs that agtop or another wrapper launches.

For agtop, the preferred model is passive local monitoring. Session files provide
the list of conversations and lightweight metadata, while OpenTelemetry provides
the most reliable token and request accounting when users have enabled local
telemetry.

Quota information is weaker. Gemini CLI and Gemini Code Assist agent mode share
official request limits, but Gemini CLI does not expose a stable external quota
API. Treat quota as a best-effort estimate unless the value comes from a
version-tested provider integration.

## Current agtop support

`GeminiCliClient` scans Gemini CLI session files under:

```text
~/.gemini/tmp/<project_hash>/chats/*.json
~/.gemini/tmp/<project_hash>/chats/*.jsonl
```

Current Gemini CLI builds store full sessions as JSON documents. Older builds
used JSONL session records, so agtop supports both formats.

Discovery currently reads:

- `~/.gemini/projects.json` to map Gemini project hashes back to absolute
  project paths;
- `~/.gemini/settings.json` to derive the configured global model;
- `~/.gemini/oauth_creds.json` presence to label sessions as OAuth-backed, with
  API key as the fallback label;
- each session file for session id, timestamps, model, title, and coarse state.

Analysis currently prefers local telemetry:

```text
~/.gemini/telemetry.log
```

agtop aggregates `gemini_cli.api_response` events that fall inside the session
timestamp window. It supports flattened local JSONL events and records where
fields are nested under an `attributes` object. It currently reads these token
attributes:

- `input_token_count`
- `output_token_count`
- `cached_content_token_count`
- `thoughts_token_count`
- `tool_token_count`, for context-size accounting only
- `total_token_count`, for context-size accounting when present

Telemetry also contributes:

- effective model from `model`;
- agent turn count from in-window `gemini_cli.api_response` events;
- tool-call count from in-window `gemini_cli.tool_call` events;
- peak context usage from `total_token_count` plus the model context window.

When telemetry is missing or produces zero tokens, agtop falls back to token
fields embedded in the session file. This fallback is useful for old or partial
session artifacts, but OpenTelemetry remains the primary source for runtime
usage. Session files also fill gaps in telemetry-derived analyses, such as user
turns and tool calls.

Google quota support lives in the Google quota provider, not in
`GeminiCliClient`. It follows the Gemini Code Assist-style account flow:

- call `:loadCodeAssist` to discover tier and project metadata;
- for paid tiers, call `:retrieveUserQuota` to fetch per-model quota buckets;
- for free tiers, report account metadata without per-model buckets because
  Gemini CLI itself does not fetch those buckets for free-tier accounts.

This is useful operationally, but it should not be described as a stable Gemini
CLI quota API.

## Gemini CLI data sources

### Session files

Session files are the best source for passive discovery:

- session id;
- created and last-updated timestamps;
- first user prompt or generated title;
- project hash and project path;
- model when recorded;
- tool executions and token usage when present.

The location is project-scoped:

```text
~/.gemini/tmp/<project_hash>/chats/
```

### OpenTelemetry

Gemini CLI has built-in OpenTelemetry support for logs, metrics, and traces.
For local development, file output is the simplest source:

```json
{
  "telemetry": {
    "enabled": true,
    "target": "local",
    "outfile": ".gemini/telemetry.log",
    "logPrompts": false
  }
}
```

Equivalent environment variables include:

```sh
export GEMINI_TELEMETRY_ENABLED=true
export GEMINI_TELEMETRY_TARGET=local
export GEMINI_TELEMETRY_OUTFILE=.gemini/telemetry.log
export GEMINI_TELEMETRY_LOG_PROMPTS=false
```

Important log events and metrics:

- `gemini_cli.api_response`
- `gemini_cli.api.request.count`
- `gemini_cli.api.request.latency`
- `gemini_cli.token.usage`
- `gemini_cli.tool.call.count`
- `gemini_cli.tool.call.latency`
- `gemini_cli.agent.run.count`
- `gemini_cli.agent.duration`
- `gemini_cli.agent.turns`
- `gemini_cli.memory.usage`
- `gemini_cli.cpu.usage`

The current `gemini_cli.api_response` shape includes useful attributes such as:

- `model`
- `status_code`
- `duration_ms`
- `input_token_count`
- `output_token_count`
- `cached_content_token_count`
- `thoughts_token_count`
- `tool_token_count`
- `total_token_count`
- `prompt_id`
- `auth_type`
- `finish_reasons`

Some versions may also include `response_text`; agtop should not require it.

The `gemini_cli.token.usage` metric is tagged by `model` and token `type`,
where `type` can be `input`, `output`, `thought`, `cache`, or `tool`.

### Headless JSON and JSONL

For runs launched by another process, Gemini CLI supports structured headless
output:

```sh
gemini -p "..." --output-format json
gemini -p "..." --output-format stream-json
```

The single JSON output includes `response`, `stats`, and optional `error`.
The streaming format emits JSONL events such as `init`, `message`, `tool_use`,
`tool_result`, `error`, and `result`.

This is a good integration point for wrappers that launch Gemini CLI directly.
It should stay separate from agtop's passive local session discovery because
agtop usually observes already-running or historical CLI sessions.

### `/stats model`

`/stats model` is useful for human inspection and some versions expose
limit-related information. Do not build core accounting around scraping it.
Quota display has changed across Gemini CLI versions, and current docs should
be treated as version-sensitive.

### Google quota docs

Official Google docs define request limits by account or license type. Gemini
CLI and Gemini Code Assist agent mode share these quotas, and one prompt can
produce multiple model requests.

Use these limits as plan metadata, not as proof of provider-side remaining
quota. agtop can combine official limits with observed request counts to show an
estimated remaining quota.

## Feature matrix

| Feature | Status | Source | Notes |
| --- | --- | --- | --- |
| Session discovery | Reliable | Session files | JSON and legacy JSONL are supported. |
| Session title | Best-effort | Session files | Derived from first user prompt when available. |
| Project path | Best-effort | `projects.json` | Missing project mappings should degrade gracefully. |
| Model | Best-effort | Session file, settings | Sessions can use more than one model. |
| Token usage | Reliable when telemetry is enabled | OpenTelemetry | Session-file tokens are fallback only. |
| Tool calls | Best-effort | Telemetry or session files | Populates common `tool_call_count`. |
| Agent turns | Best-effort | Telemetry or session files | API responses or assistant/model messages. |
| User turns | Best-effort | Session files | Not expected from telemetry alone. |
| Context usage | Best-effort | Telemetry or session files | Requires model context-window lookup. |
| Request count | Best-effort | OpenTelemetry | Used as agent-turn/request signal. |
| Quota remaining | Version-sensitive | Google provider, `/stats model` | Not a stable Gemini CLI observability API. |
| Billing cost | Estimate | Public pricing | Subscription plans may not map to API prices. |
| Headless run stats | Reliable for launched runs | `--output-format json` or `stream-json` | Separate from passive discovery. |
| Provider billing truth | Unsupported | N/A | Use Google billing/quota systems outside agtop. |

## Implementation status and next work

Implemented:

- Gemini sessions populate the same common `SessionAnalysis` fields used by
  Claude Code, Codex, and OpenCode: tokens, cost, effective model, duration,
  tool-call count, agent turns, user turns, and peak context usage when enough
  data is available.
- Telemetry parsing accepts flattened local records and nested `attributes`
  records.
- Telemetry correlation uses `session.id` when present and falls back to the
  session timestamp window otherwise.
- Token fallback from session files remains in place when telemetry is disabled
  or empty.
- JSON and legacy JSONL session formats are covered by tests.
- Telemetry tests cover nested attributes, tool calls, model extraction, context
  usage, and timestamp-window filtering.

Remaining work:

1. Improve prompt-level telemetry correlation.
   - Use `prompt_id` if it becomes useful for per-turn joins in local telemetry
     output.
   - Preserve the current `session.id` plus timestamp-window fallback.

2. Expand real-world fixtures.
   - Add anonymized current Gemini CLI JSON sessions from multiple CLI versions.
   - Add telemetry fixtures containing multiple sessions and multiple models.
   - Cover missing or disabled telemetry with explicit integration fixtures.

3. Track estimated quota usage.
   - Count observed model requests from `gemini_cli.api_response` or
     `gemini_cli.api.request.count`.
   - Combine observed request counts with official plan limits when known.
   - Mark remaining quota as estimated unless it comes from a tested provider
     endpoint.

4. Keep `/stats` optional.
   - Do not scrape the TUI in the core path.
   - If `/stats model` support is added, implement it as an optional adapter
     gated by observed CLI version and output shape.

5. Keep headless integration separate.
   - Use `stream-json` only for workflows where agtop or another wrapper
     launches Gemini CLI.
   - Store `init.session_id` and final `result.stats` directly for those runs.

Historical implementation notes:

- Harden telemetry parsing.
   - Parse both flattened local JSONL records and OpenTelemetry-shaped records
     where attributes are nested.
   - Support `tool_token_count`, `total_token_count`, `auth_type`, and
     `finish_reasons` without requiring them.
   - Keep unknown or missing fields non-fatal.
- Make telemetry requirements explicit.
   - Document the local telemetry settings users need for token accounting.
   - Keep `logPrompts` off in examples to avoid collecting prompt text by
     default.

## Quota strategy

agtop should report quota in three tiers of confidence:

- **Known plan limit**: official limits from Google documentation or account
  tier metadata.
- **Observed usage**: request counts and token usage from local telemetry.
- **Estimated remaining**: plan limit minus observed requests in the reset
  window, clearly marked as estimated.

Do not present `/stats model` output as authoritative unless the implementation
is tied to a specific Gemini CLI version and verified output format.

## Test strategy

Run the package tests after Gemini integration changes:

```sh
cargo test -p agtop-core
cargo test -p agtop-cli
```

Current coverage includes:

- current Gemini CLI JSON session format;
- legacy Gemini CLI JSONL session format;
- local telemetry fields in flattened and nested-attributes shapes;
- telemetry timestamp filtering;
- `session.id` telemetry filtering with timestamp fallback;
- common metric extraction for tokens, tool calls, turns, duration, model, and
  context usage;
- session-file token fallback;
- free-tier Google quota response with no per-model buckets;
- paid-tier Google quota response with per-model buckets;

Future coverage should add anonymized real-world fixtures with multiple
sessions and models in the same telemetry file.

## References

- [Gemini CLI telemetry](https://geminicli.com/docs/cli/telemetry/)
- [Gemini CLI session management](https://geminicli.com/docs/cli/session-management/)
- [Gemini CLI headless mode](https://geminicli.com/docs/cli/headless/)
- [Gemini CLI command reference](https://geminicli.com/docs/reference/commands/)
- [Gemini Code Assist quotas](https://developers.google.com/gemini-code-assist/resources/quotas)
