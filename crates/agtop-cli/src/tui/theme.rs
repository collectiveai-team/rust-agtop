//! Centralised style constants for the TUI.
//!
//! Every `Style` used in the widget layer should be defined here so that
//! colour choices are easy to audit and change in one place. All items
//! are `pub const` — `Style::new()` and its builder methods are `const fn`
//! in ratatui 0.26+, so there is zero run-time cost.

use ratatui::style::{Color, Modifier, Style};

// ── Table / general chrome ────────────────────────────────────────────────

/// Header row in tables (session table, cost table, config table).
pub const HEADER: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

/// Highlighted / selected row in the session table and config tab.
pub const SELECTED: Style = Style::new()
    .bg(Color::Blue)
    .fg(Color::White)
    .add_modifier(Modifier::BOLD);

/// Status bar at the top of the screen.
pub const STATUS_BAR: Style = Style::new()
    .bg(Color::DarkGray)
    .fg(Color::White)
    .add_modifier(Modifier::BOLD);

/// Active tab title in the tab bar.
pub const TAB_ACTIVE: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

// ── Empty / placeholder states ────────────────────────────────────────────

/// Dimmed text used when nothing is selected or the list is empty.
pub const EMPTY_HINT: Style = Style::new().add_modifier(Modifier::DIM);

// ── Client colours ───────────────────────────────────────────────────────

pub const CLIENT_CLAUDE: Style = Style::new().fg(Color::Magenta);
pub const CLIENT_CODEX: Style = Style::new().fg(Color::Cyan);
pub const CLIENT_OPENCODE: Style = Style::new().fg(Color::Green);
pub const CLIENT_COPILOT: Style = Style::new().fg(Color::Blue);
pub const CLIENT_GEMINI_CLI: Style = Style::new().fg(Color::Yellow);

// ── Cost cell colours ─────────────────────────────────────────────────────

/// Session included in plan — no out-of-pocket cost.
pub const COST_INCLUDED: Style = Style::new().fg(Color::Green).add_modifier(Modifier::DIM);

/// Session costing ≥ $5 — worth a visual nudge.
pub const COST_HIGH: Style = Style::new().fg(Color::Yellow);

// ── Session state colours ──────────────────────────────────────────────────

pub const STATE_RUNNING: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const STATE_BLOCKED: Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
pub const STATE_IDLE: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::BOLD);
pub const STATE_CLOSED: Style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::BOLD);

// ── Cost tab ─────────────────────────────────────────────────────────────

/// "Total" row in the cost breakdown table.
pub const COST_TOTAL: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

/// Bucket label cells in the cost table.
pub const COST_BUCKET_LABEL: Style = Style::new().fg(Color::Cyan);

/// "incl" indicator for plan-covered sessions.
pub const COST_INCL: Style = Style::new().fg(Color::Green);

// ── Info tab ─────────────────────────────────────────────────────────────

/// Key labels in the key:value info panel.
pub const INFO_KEY: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

// ── Config tab ────────────────────────────────────────────────────────────

/// `[x]` checkbox for a visible column.
pub const CONFIG_CHECKED: Style = Style::new().fg(Color::Green);

/// `[ ]` checkbox for a hidden column.
pub const CONFIG_UNCHECKED: Style = Style::new().fg(Color::DarkGray);

// ── Dashboard ─────────────────────────────────────────────────────────────

/// Chart line for Claude token history.
pub const CHART_CLAUDE: Style = Style::new().fg(Color::Blue);

/// Chart line for Codex token history.
pub const CHART_CODEX: Style = Style::new().fg(Color::Green);

/// Chart line for OpenCode token history.
pub const CHART_OPENCODE: Style = Style::new().fg(Color::Magenta);

/// Summary line beneath the chart.
pub const CHART_SUMMARY: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::DIM);

/// Plan client label in the plan-usage panel.
pub const PLAN_LABEL: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

/// Dimmed note / hint text in the plan panel.
pub const PLAN_NOTE: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::DIM);

/// Dimmed text used when no plan data is available.
#[allow(dead_code)]
pub const PLAN_EMPTY: Style = Style::new().add_modifier(Modifier::DIM);

/// Filled portion of a usage bar when utilization < 30 % (safe).
pub const PLAN_BAR_GREEN: Style = Style::new().fg(Color::Green);

/// Filled portion of a usage bar when utilization is 30–80 % (caution).
pub const PLAN_BAR_YELLOW: Style = Style::new().fg(Color::Yellow);

/// Filled portion of a usage bar when utilization ≥ 80 % (critical).
pub const PLAN_BAR_RED: Style = Style::new().fg(Color::Red);

/// Highlighted / selected row in the subscription list.
#[allow(dead_code)]
pub const PLAN_SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

// ── Footer ────────────────────────────────────────────────────────────────

/// Footer text when filter mode is active.
pub const FOOTER_FILTER: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

/// Footer help text in normal mode.
pub const FOOTER_NORMAL: Style = Style::new().fg(Color::Gray);

// ── Subagent hierarchy ────────────────────────────────────────────────────

/// Dimmed style for child (subagent) rows in an expanded group.
pub const SUBAGENT_CHILD: Style = Style::new().fg(Color::DarkGray);

// ── Quota pane ────────────────────────────────────────────────────────────

/// Bar fill when used_percent < 30 % (safe).
pub const QUOTA_BAR_OK: Style = Style::new().fg(Color::Green);

/// Bar fill when used_percent is in [30, 80) (caution).
pub const QUOTA_BAR_WARN: Style = Style::new().fg(Color::Yellow);

/// Bar fill when used_percent >= 80 % (critical).
pub const QUOTA_BAR_CRIT: Style = Style::new().fg(Color::Red);

/// Card rendered dim when the last fetch failed but a prior good result exists.
pub const QUOTA_BAR_STALE: Style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM);

/// Empty (unfilled) bar cells — dim so they read as "unoccupied" without disappearing.
pub const QUOTA_BAR_EMPTY_CELL: Style =
    Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM);

/// Style used when `used_percent` is None and no value_label exists.
pub const QUOTA_EMPTY: Style = Style::new();

/// Highlighted provider row in the Dashboard list.
#[allow(dead_code)]
pub const QUOTA_SELECTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

/// Title for centered placeholder messages (Idle / Loading / Error).
pub const QUOTA_TITLE: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::BOLD);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_thresholds_exist() {
        let _ = QUOTA_BAR_OK;
        let _ = QUOTA_BAR_WARN;
        let _ = QUOTA_BAR_CRIT;
        let _ = QUOTA_BAR_STALE;
        let _ = QUOTA_EMPTY;
        let _ = QUOTA_SELECTED;
        let _ = QUOTA_TITLE;
    }
}
