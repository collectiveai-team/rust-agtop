# agtop Roadmap

> Last updated: 2026-04-24 (Phase 2 complete — #11 closed/deferred)

A prioritized roadmap derived from the project TODO file. Items are grouped into phases based on impact and implementation complexity. Each item links to its GitHub issue.

---

## Phase 1 — Bug Fixes

*Highest priority. These are visible regressions or broken functionality that degrade the core experience.*

| # | Issue | Area | Description |
|---|-------|------|-------------|
| ✅ | [#1](https://github.com/collectiveai-team/rust-agtop/issues/1) | TUI / Info Panel | Session ID always truncated in info panel |
| ✅ | [#2](https://github.com/collectiveai-team/rust-agtop/issues/2) | TUI / Info Panel | Info panel uses only 1 column — add multi-column layout with scrollbar |
| ✅ | [#3](https://github.com/collectiveai-team/rust-agtop/issues/3) | Client / Codex | Bad session titles display `<environment> ...` |
| ✅ | [#4](https://github.com/collectiveai-team/rust-agtop/issues/4) | Client / Codex | Subagents not grouped under parent sessions |
| ✅ | [#5](https://github.com/collectiveai-team/rust-agtop/issues/5) | Client / Gemini CLI | Subagents not displayed |
| 🔴 | [#6](https://github.com/collectiveai-team/rust-agtop/issues/6) | Client / Copilot | No metrics — VS Code Copilot and `gh copilot` CLI are separate problems (see issue for details) |

---

## Phase 2 — UX Enhancements

*Quality-of-life improvements. Well-scoped changes that improve polish and usability without structural refactoring.*

| # | Issue | Area | Description |
|---|-------|------|-------------|
| ✅ | [#7](https://github.com/collectiveai-team/rust-agtop/issues/7) | TUI / Sessions | Gemini subscription shows "oauth" instead of real subscription name |
| ✅ | [#8](https://github.com/collectiveai-team/rust-agtop/issues/8) | TUI / Labels | Rename "Max x5" → "Claude Max 5x" / "Claude Max 20x" |
| ✅ | [#9](https://github.com/collectiveai-team/rust-agtop/issues/9) | TUI / Sessions | Add per-client colors for Copilot and Gemini CLI in session pane |
| ✅ | [#10](https://github.com/collectiveai-team/rust-agtop/issues/10) | TUI / Layout | Increase client column width by 2 chars to display "gemini-cli" fully |
| 🚫 | [#11](https://github.com/collectiveai-team/rust-agtop/issues/11) | Info | Retrieve and display API/tool usage time — closed, no data available for Claude/Gemini |

---

## Phase 3 — TUI Overhaul & Documentation

*Larger structural changes and foundational documentation. These require more planning and may span multiple PRs.*

### TUI Layout Overhaul

| # | Issue | Description |
|---|-------|-------------|
| 🔵 | [#13](https://github.com/collectiveai-team/rust-agtop/issues/13) | Dashboard as default view |
| 🔵 | [#14](https://github.com/collectiveai-team/rust-agtop/issues/14) | Config as full-page view (VS Code-style settings) |
| 🔵 | [#15](https://github.com/collectiveai-team/rust-agtop/issues/15) | Toggleable panels (btop-style) |
| 🔵 | [#16](https://github.com/collectiveai-team/rust-agtop/issues/16) | Consistent theme — colors, fonts, visual style |
| 🔵 | [#17](https://github.com/collectiveai-team/rust-agtop/issues/17) | Highlight focused panel with colored border |
| 🔵 | [#18](https://github.com/collectiveai-team/rust-agtop/issues/18) | Merge info and cost panels into one |
| 🔵 | [#19](https://github.com/collectiveai-team/rust-agtop/issues/19) | Quota tab as horizontal panel |
| 🔵 | [#20](https://github.com/collectiveai-team/rust-agtop/issues/20) | Use ratatui-image for client icons/logos |

### Documentation

| # | Issue | Description |
|---|-------|-------------|
| 🔵 | [#21](https://github.com/collectiveai-team/rust-agtop/issues/21) | Document all session columns and client feature status matrix |
| 🔵 | [#22](https://github.com/collectiveai-team/rust-agtop/issues/22) | Create agent-targeted feature documentation with index |

---

## Backlog — Future Features

*Exploratory features with uncertain implementation paths. Not scheduled for a specific phase.*

| # | Issue | Description |
|---|-------|-------------|
| ⚪ | [#12](https://github.com/collectiveai-team/rust-agtop/issues/12) | Add support for "pi" client |
| ⚪ | [#23](https://github.com/collectiveai-team/rust-agtop/issues/23) | Track PIDs of sessions and relate to OS process tree |
| ⚪ | [#24](https://github.com/collectiveai-team/rust-agtop/issues/24) | Open session in client directly from TUI dashboard |

---

## Legend

| Symbol | Meaning |
|--------|---------|
| 🔴 | Phase 1 — Bug fix |
| 🟡 | Phase 2 — UX enhancement |
| 🔵 | Phase 3 — Structural / Docs |
| ⚪ | Backlog |
| 🚫 | Closed / deferred |

---

## Recently Completed

See the Archive section in [`TODO`](./TODO) for completed items. Recent notable completions:

- Phase 2 UX enhancements: Gemini subscription label (#7), Claude Max label rename (#8), per-client colors (#9), client column width (#10)
- Subagent hierarchy display
- Session deduplication
- Sessions waiting for permission detection
- Cost and plan usage panes
- Gemini CLI, Cursor, Antigravity client integrations (Copilot partial — see #6)
- Google model usage scrollable table in quota pane
- Version badge in TUI and CLI
