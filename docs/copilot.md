# Copilot integration

This document describes how agtop integrates with GitHub Copilot, the current
state of session discovery and metric surfacing, and what remains to be done.

## Overview

"GitHub Copilot" is not a single application. For agtop, there are two
separate clients that must be handled independently:

1. **Copilot in VS Code** — the VS Code extension. Sessions are stored in
   VS Code's workspace storage directory. Token data is not persisted locally.
2. **`gh copilot` CLI** — a separate command-line tool distributed via the
   GitHub CLI (`gh`) plugin system. Not yet studied for agtop integration.

These two clients have different session storage locations, different data
formats, and potentially different available metrics. They must be implemented
as separate agtop clients.

---

## Client 1: Copilot in VS Code

### Session discovery

`CopilotClient` scans VS Code's workspace storage for chat sessions:

```text
~/.config/Code/User/workspaceStorage/<hash>/chatSessions/<uuid>.json
~/.config/Code/User/workspaceStorage/<hash>/chatSessions/<uuid>.jsonl
```

On macOS, the base path is:

```text
~/Library/Application Support/Code/User/workspaceStorage/
```

Both `.json` (single document) and `.jsonl` (line-delimited records) formats
are supported. Each workspace gets a separate hash-named directory, and each
chat session is stored as a separate file.

### Available data

The session files produced by the VS Code Copilot extension contain:

- session id
- timestamps (start/last active)
- model name (when recorded)
- user and agent message turns
- tool calls (function call events)
- elapsed duration (derived from timestamps)

### What is NOT available

**Token counts are not persisted by VS Code Copilot.** The extension does not
write token usage to the local session files. This means:

- `input_tokens`, `output_tokens`, `cache_read_tokens` are always zero
- Billing cost cannot be computed
- Context window utilization cannot be measured

VS Code Copilot does not expose a local telemetry API or file comparable to
Gemini CLI's OpenTelemetry integration. There is no known local file or socket
that surfaces real-time token counts for passively observed sessions.

### Current agtop support

`CopilotClient` surfaces:

| Field | Status | Source |
| --- | --- | --- |
| Session discovery | Reliable | Workspace storage JSON/JSONL files |
| Session title | Best-effort | First user message or generated title |
| Session id | Reliable | Session file |
| Timestamps (start, last active) | Reliable | Session file |
| Model | Best-effort | Session file |
| User turns | Best-effort | Session file message count |
| Agent turns | Best-effort | Session file message count |
| Tool call count | Best-effort | Session file tool events |
| Session duration | Best-effort | Derived from timestamps |
| Token usage | **Not available** | Not persisted by VS Code Copilot |
| Cost | **Not available** | Requires token data |
| Context usage | **Not available** | Requires token data |

### Quota

Plan quota is fetched live from the GitHub API using an OAuth token discovered
in one of:

```text
~/.config/gh/hosts.yml          (gh CLI OAuth token)
~/.config/github-copilot/hosts.json  (Vim/Neovim Copilot plugin token)
```

The endpoint is:

```
GET https://api.github.com/copilot_internal/user
```

Required headers:

```
Authorization: token <oauth_token>
Accept: application/json
Editor-Version: vscode/1.96.2
X-Github-Api-Version: 2025-04-01
```

Note: the authorization scheme is `token`, not `Bearer`.

The response includes `quota_snapshots` with keys such as `chat`,
`completions`, and `premium_interactions`. agtop remaps
`premium_interactions` → `premium` and honors the `unlimited: true` flag.
A `copilot_addon` provider also exists that filters to the `premium` window
only, used when opencode bills add-on entitlements separately.

Quota responses are cached at `~/.cache/agtop/copilot_quota.json` with a
5-minute TTL.

### Remaining work

1. **Investigate live token data sources.**
   Determine whether any VS Code extension API, language server protocol
   message, or GitHub API endpoint exposes token usage for active or completed
   sessions. If such a source exists, implement it as a supplementary provider.

2. **Improve session-file coverage.**
   Add anonymized real-world session fixtures from different VS Code and
   Copilot extension versions. Validate that both `.json` and `.jsonl` formats
   are handled correctly across versions.

3. **Document limitations in the TUI.**
   Surface a clear signal in the session list (e.g., a dash or N/A marker)
   when token data is unavailable due to client limitations, rather than
   displaying zeros that may be mistaken for real values.

---

## Client 2: `gh copilot` CLI

### Status: not yet investigated

`gh copilot` is a plugin for the GitHub CLI (`gh`) that provides AI-assisted
command-line help (e.g., `gh copilot explain`, `gh copilot suggest`). It is a
completely separate application from the VS Code Copilot extension.

The following are **unknown** and need investigation before any implementation
plan can be written:

- Session storage location (if any sessions are persisted locally)
- Session file format
- Available metrics (token counts, model, timestamps)
- Whether a local telemetry mechanism exists
- Whether the same quota API is shared with VS Code Copilot

### Next step

Inspect a live `gh copilot` installation to determine:

1. Whether it creates any local session files or logs
2. What format those files use
3. What metrics are available
4. Whether the existing `CopilotClient` quota provider is reusable

Only after this investigation can an implementation plan be written.

---

## Feature matrix (VS Code Copilot)

| Feature | Status | Source | Notes |
| --- | --- | --- | --- |
| Session discovery | Reliable | Workspace storage | JSON and JSONL formats supported |
| Session title | Best-effort | Session file | Derived from first user message |
| Timestamps | Reliable | Session file | |
| Model | Best-effort | Session file | May be missing in older sessions |
| User turns | Best-effort | Session file | |
| Agent turns | Best-effort | Session file | |
| Tool calls | Best-effort | Session file | |
| Session duration | Best-effort | Derived from timestamps | |
| Token usage | Not available | N/A | Not persisted by VS Code Copilot |
| Cost | Not available | N/A | Requires token data |
| Context usage | Not available | N/A | Requires token data |
| Quota remaining | Reliable | GitHub API | Cached with 5-min TTL |
| Plan metadata | Reliable | GitHub API | login, sku, reset date |

---

## Test strategy

Run the package tests after Copilot integration changes:

```sh
cargo test -p agtop-core
cargo test -p agtop-cli
```

Current coverage includes:

- Quota provider: configured check, token scheme, editor headers, success/meta, 401, not-configured
- Copilot add-on provider: configured check, same endpoint, premium-only filter, meta matching

Missing coverage:

- Session file parsing with real-world fixtures from multiple VS Code versions
- Both `.json` and `.jsonl` session formats with realistic content
- Session file parsing when token fields are absent

---

## References

- [GitHub Copilot VS Code extension](https://marketplace.visualstudio.com/items?itemName=GitHub.copilot)
- [gh copilot CLI plugin](https://docs.github.com/en/copilot/github-copilot-in-the-cli/about-github-copilot-in-the-cli)
- [GitHub Copilot quota API](https://docs.github.com/en/rest/copilot)
- [gh CLI](https://cli.github.com/)
