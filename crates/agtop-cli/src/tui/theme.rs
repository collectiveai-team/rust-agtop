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

// ── Provider colours ──────────────────────────────────────────────────────

pub const PROVIDER_CLAUDE: Style = Style::new().fg(Color::Magenta);
pub const PROVIDER_CODEX: Style = Style::new().fg(Color::Cyan);
pub const PROVIDER_OPENCODE: Style = Style::new().fg(Color::Green);

// ── Cost cell colours ─────────────────────────────────────────────────────

/// Session included in plan — no out-of-pocket cost.
pub const COST_INCLUDED: Style = Style::new().fg(Color::Green).add_modifier(Modifier::DIM);

/// Session costing ≥ $5 — worth a visual nudge.
pub const COST_HIGH: Style = Style::new().fg(Color::Yellow);

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

/// Plan provider label in the plan-usage panel.
pub const PLAN_LABEL: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

/// Dimmed note / hint text in the plan panel.
pub const PLAN_NOTE: Style = Style::new().fg(Color::Gray).add_modifier(Modifier::DIM);

/// Dimmed text used when no plan data is available.
pub const PLAN_EMPTY: Style = Style::new().add_modifier(Modifier::DIM);

/// Filled portion of a usage bar when utilization < 30 % (safe).
pub const PLAN_BAR_GREEN: Style = Style::new().fg(Color::Green);

/// Filled portion of a usage bar when utilization is 30–80 % (caution).
pub const PLAN_BAR_YELLOW: Style = Style::new().fg(Color::Yellow);

/// Filled portion of a usage bar when utilization ≥ 80 % (critical).
pub const PLAN_BAR_RED: Style = Style::new().fg(Color::Red);

/// Highlighted / selected row in the subscription list.
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
