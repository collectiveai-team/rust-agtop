use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// SessionState — canonical 6-variant domain state
// ---------------------------------------------------------------------------

/// Canonical session state. Owned by `agtop-core`; no display-layer mapping.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum SessionState {
    /// Agent actively producing output or executing a tool call.
    Running,
    /// Agent paused waiting for user response.
    Waiting(WaitReason),
    /// Live but anomalous — stalled past threshold or other warning condition.
    Warning(WarningReason),
    /// Ended with an explicit error.
    Error(ErrorReason),
    /// Live, ready for input, not currently working.
    Idle,
    /// No live process; historical/archival.
    Closed,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    Input,
    Permission,
    Other(String),
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningReason {
    Stalled { since: chrono::DateTime<chrono::Utc> },
    Other(String),
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorReason {
    ExitCode(i32),
    Crash,
    ParserDetected(String),
}

/// Coarse state inferred by a per-client parser from session log content.
///
/// This is the *parser's* opinion of what the agent is doing based on the
/// session file alone (e.g. "the last assistant turn ended"); it is fed
/// into `state_resolution::resolve_state` along with OS liveness data to
/// produce the canonical [`SessionState`].
///
/// Parsers MUST return a typed `ParserState` value. Callers MUST NOT
/// inspect parser state via string matching — use this enum.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum ParserState {
    /// Last assistant turn ended cleanly; the agent is awaiting user input.
    /// Maps to `SessionState::Idle` when the process is live.
    Idle,
    /// Agent is mid-turn — actively generating output or running a tool.
    /// Maps to `SessionState::Running` when the process is live.
    Running,
    /// Agent is paused waiting for a specific kind of user response.
    /// Maps to `SessionState::Waiting(_)` when the process is live.
    Waiting(WaitReason),
    /// Parser detected an explicit error in the session log
    /// (e.g. tool execution failure, crash trace).
    Error(ErrorReason),
    /// Parser had no opinion. Resolution falls back to recency + liveness.
    #[default]
    Unknown,
}

impl SessionState {
    /// Coarse string label (matches the outer tag in serde).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Running    => "running",
            Self::Waiting(_) => "waiting",
            Self::Warning(_) => "warning",
            Self::Error(_)   => "error",
            Self::Idle       => "idle",
            Self::Closed     => "closed",
        }
    }

    /// Short label for narrow UI columns.
    #[must_use]
    pub const fn compact_label(&self) -> &'static str {
        // Same as as_str for now — distinct only if a column needs <7 chars.
        self.as_str()
    }

    /// True if the session is doing or could resume work without external input.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::Idle | Self::Warning(_))
    }

    /// True if the session is blocked on user response.
    #[must_use]
    pub const fn needs_user(&self) -> bool {
        matches!(self, Self::Waiting(_))
    }
}

// ---------------------------------------------------------------------------
// SessionAnalysis extension: session_state field
// ---------------------------------------------------------------------------

/// Which agent produced a session.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Claude,
    Codex,
    OpenCode,
    Copilot,
    #[serde(rename = "gemini-cli")]
    GeminiCli,
    Cursor,
    Antigravity,
}

impl ClientKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Copilot => "copilot",
            Self::GeminiCli => "gemini-cli",
            Self::Cursor => "cursor",
            Self::Antigravity => "antigravity",
        }
    }

    /// Every `ClientKind` variant, in a stable display order.
    /// Keep this in sync with the enum definition — a missing variant
    /// here silently excludes that client from default-enabled sets.
    #[must_use]
    pub const fn all() -> &'static [ClientKind] {
        &[
            Self::Claude,
            Self::Codex,
            Self::OpenCode,
            Self::Copilot,
            Self::GeminiCli,
            Self::Cursor,
            Self::Antigravity,
        ]
    }
}

impl std::fmt::Display for ClientKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lightweight session metadata derived from file headers/names.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    #[serde(alias = "provider")]
    pub client: ClientKind,
    /// Billing/auth bucket for this session when known, e.g. "Claude Max 5x",
    /// "ChatGPT Plus", or "API key".
    #[serde(default)]
    pub subscription: Option<String>,
    pub session_id: String,
    pub started_at: Option<DateTime<Utc>>,
    pub last_active: Option<DateTime<Utc>>,
    pub model: Option<String>,
    /// Working directory (best-effort). Used for display labels.
    pub cwd: Option<String>,
    /// Coarse workflow state such as `waiting` or `stopped`.
    #[serde(default)]
    pub state: Option<String>,
    /// Client-specific explanation of the derived state.
    #[serde(default)]
    pub state_detail: Option<String>,
    /// Explicit reasoning/model effort when the client exposes it.
    #[serde(default)]
    pub model_effort: Option<String>,
    /// Client-specific explanation of where the effort came from.
    #[serde(default)]
    pub model_effort_detail: Option<String>,
    /// Human-readable session title when the client stores one (e.g. the
    /// first user message summary in OpenCode).
    #[serde(default)]
    pub session_title: Option<String>,
    /// Primary data file / directory for this session.
    pub data_path: PathBuf,
}

/// Aggregated token counts across all turns in a session.
///
/// Fields map to the vocabulary each client exposes; not every client
/// populates every field.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenTotals {
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
    pub reasoning_output: u64,
    /// Claude-style 5-minute ephemeral cache writes.
    pub cache_write_5m: u64,
    /// Claude-style 1-hour ephemeral cache writes.
    pub cache_write_1h: u64,
    /// Claude-style cache reads (cheap).
    pub cache_read: u64,
}

impl SessionSummary {
    /// Construct a [`SessionSummary`] with all fields explicitly specified.
    ///
    /// Prefer this over struct-literal syntax so callers remain compatible
    /// when new (non-exhaustive) fields are added.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: ClientKind,
        subscription: Option<String>,
        session_id: String,
        started_at: Option<DateTime<Utc>>,
        last_active: Option<DateTime<Utc>>,
        model: Option<String>,
        cwd: Option<String>,
        data_path: std::path::PathBuf,
        state: Option<String>,
        state_detail: Option<String>,
        model_effort: Option<String>,
        model_effort_detail: Option<String>,
    ) -> Self {
        Self {
            client,
            subscription,
            session_id,
            started_at,
            last_active,
            model,
            cwd,
            state,
            state_detail,
            model_effort,
            model_effort_detail,
            session_title: None,
            data_path,
        }
    }
}

impl TokenTotals {
    /// Grand total of every distinct bucket (input + output + cache activity).
    #[must_use]
    pub fn grand_total(&self) -> u64 {
        self.input
            + self.cached_input
            + self.output
            + self.cache_write_5m
            + self.cache_write_1h
            + self.cache_read
    }
}

/// Dollar-denominated cost breakdown (USD).
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub total: f64,
    /// True when the active billing plan marks this session as included
    /// (e.g. Claude Max, ChatGPT Plus) and the dollar figures are zero.
    pub included: bool,
}

/// Full analysis of a single session.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAnalysis {
    pub summary: SessionSummary,
    pub tokens: TokenTotals,
    pub cost: CostBreakdown,
    /// Model actually observed during token accounting (may differ from the
    /// summary model if a session uses multiple).
    pub effective_model: Option<String>,
    /// Number of Claude subagent sidechain transcripts folded into
    /// `tokens` / `cost` (0 when the session has none, or for non-Claude
    /// clients). Defaults to 0 on deserialization so older JSON
    /// consumers remain compatible.
    #[serde(default)]
    pub subagent_file_count: usize,
    /// Number of tool invocations observed in the transcript (client
    /// specific best-effort). `None` when unavailable.
    #[serde(default)]
    pub tool_call_count: Option<u64>,
    /// Session wall-clock duration in seconds. `None` when we cannot
    /// infer both start and end timestamps.
    #[serde(default)]
    pub duration_secs: Option<u64>,
    /// Peak per-turn context usage as a percentage of the model context
    /// window (0..=100+), when the client exposes both values.
    #[serde(default)]
    pub context_used_pct: Option<f64>,
    /// Raw token count at the peak-utilization turn (numerator of
    /// `context_used_pct`). `None` when `context_used_pct` is `None`.
    #[serde(default)]
    pub context_used_tokens: Option<u64>,
    /// Model context window size in tokens used to compute
    /// `context_used_pct`. `None` when `context_used_pct` is `None`.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Child subagent sessions, if this client exposes a parent/child
    /// relationship. Empty by default; populated by the refresh layer.
    #[serde(default)]
    pub children: Vec<SessionAnalysis>,
    /// Number of agent/assistant turns observed in the transcript.
    /// `None` when the client does not expose this information.
    #[serde(default)]
    pub agent_turns: Option<u64>,
    /// Number of user turns observed in the transcript.
    /// `None` when the client does not expose this information.
    #[serde(default)]
    pub user_turns: Option<u64>,
    /// Inferred project name (e.g. from `git remote get-url origin`).
    /// `None` when unavailable.
    #[serde(default)]
    pub project_name: Option<String>,
    /// OS PID of the agent CLI process currently running this session.
    /// `None` when no match was established.
    #[serde(default)]
    pub pid: Option<u32>,
    /// Whether the matched process is currently live or has just exited.
    /// `None` when no match was established.
    #[serde(default)]
    pub liveness: Option<crate::process::Liveness>,
    /// How we matched the PID. `None` when no match.
    #[serde(default)]
    pub match_confidence: Option<crate::process::Confidence>,
    /// Live OS resource metrics for the matched process. `None` when no
    /// process is matched or the process has stopped.
    #[serde(default)]
    pub process_metrics: Option<crate::process::ProcessMetrics>,
    /// Canonical domain state for this session, derived from transcript events
    /// and liveness information. `None` when parsers haven't computed it yet
    /// (legacy path) or during initial discovery (string-based state only).
    #[serde(default)]
    pub session_state: Option<SessionState>,
    /// Latest in-flight tool call or response status, derived from session log.
    /// `None` when the session is idle, closed, or the parser doesn't extract this.
    #[serde(default)]
    pub current_action: Option<String>,
}

impl SessionAnalysis {
    /// Construct a [`SessionAnalysis`] with all fields explicitly specified.
    ///
    /// Prefer this over struct-literal syntax so callers remain compatible
    /// when new (non-exhaustive) fields are added.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        summary: SessionSummary,
        tokens: TokenTotals,
        cost: CostBreakdown,
        effective_model: Option<String>,
        subagent_file_count: usize,
        tool_call_count: Option<u64>,
        duration_secs: Option<u64>,
        context_used_pct: Option<f64>,
        context_used_tokens: Option<u64>,
        context_window: Option<u64>,
    ) -> Self {
        Self {
            summary,
            tokens,
            cost,
            effective_model,
            subagent_file_count,
            tool_call_count,
            duration_secs,
            context_used_pct,
            context_used_tokens,
            context_window,
            children: Vec::new(),
            agent_turns: None,
            user_turns: None,
            project_name: None,
            pid: None,
            liveness: None,
            match_confidence: None,
            process_metrics: None,
            session_state: None,
            current_action: None,
        }
    }
}

impl PlanWindow {
    /// Construct a [`PlanWindow`] with all fields explicitly specified.
    pub fn new(
        label: String,
        utilization: Option<f64>,
        reset_at: Option<DateTime<Utc>>,
        reset_hint: Option<String>,
        binding: bool,
    ) -> Self {
        Self {
            label,
            utilization,
            reset_at,
            reset_hint,
            binding,
        }
    }
}

impl PlanUsage {
    /// Construct a [`PlanUsage`] with all fields explicitly specified.
    pub fn new(
        client: ClientKind,
        label: String,
        plan_name: Option<String>,
        windows: Vec<PlanWindow>,
        last_limit_hit: Option<DateTime<Utc>>,
        note: Option<String>,
    ) -> Self {
        Self {
            client,
            label,
            plan_name,
            windows,
            last_limit_hit,
            note,
        }
    }
}

// ---------------------------------------------------------------------------
// Plan usage (for the --dashboard pane)
// ---------------------------------------------------------------------------

/// One rate-limit window (e.g. Anthropic's 5-hour rolling cap).
///
/// `utilization` is a fraction in 0.0..=1.0 where available; `reset_at` is
/// the UTC time the window resets, also when available. Clients may
/// populate only a subset of these fields; renderers must treat every
/// field as optional.
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanWindow {
    /// Short label shown next to the gauge, e.g. "5h" or "7d".
    pub label: String,
    /// Utilization as a fraction [0.0, 1.0]. `None` when the client
    /// does not expose a gauge (e.g. Claude Code).
    pub utilization: Option<f64>,
    /// When the window resets, in UTC. `None` when unknown.
    pub reset_at: Option<DateTime<Utc>>,
    /// Free-form human text from the client (e.g. Claude's "resets 3pm
    /// (America/Buenos_Aires)"). Used when we have no structured reset_at.
    pub reset_hint: Option<String>,
    /// True when this window is the representative/binding one for the plan
    /// ("representative-claim" in Anthropic's unified rate-limit protocol).
    #[serde(default)]
    pub binding: bool,
}

/// Plan + usage snapshot for a (client, auth) pair.
///
/// A single client can contribute multiple entries when the user has
/// more than one auth (e.g. Claude Code on Anthropic Max AND OpenCode on
/// the same Anthropic Max — each produces its own `PlanUsage` because the
/// data sources are different).
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUsage {
    #[serde(alias = "provider")]
    pub client: ClientKind,
    /// Client-qualified label for the card header, e.g. "Claude Code ·
    /// Claude Max 5x" or "OpenCode · anthropic (Max)". Free-form; renderers
    /// display verbatim.
    pub label: String,
    /// Plan name when known (e.g. "max", "max_5x", "plus", "pro").
    pub plan_name: Option<String>,
    /// Zero or more usage windows. Typically "5h" + "7d" for Anthropic,
    /// "primary" + "secondary" for OpenAI/Codex when populated. Empty
    /// when no utilization data is available (e.g. Claude Code).
    #[serde(default)]
    pub windows: Vec<PlanWindow>,
    /// Most recent moment the user was observed to hit a rate limit.
    /// Used for clients that don't expose gauges but do record
    /// limit-hit events (Claude Code's synthetic error messages).
    pub last_limit_hit: Option<DateTime<Utc>>,
    /// Free-form note rendered below the gauges, e.g. "waiting for
    /// backend data" when the schema slot exists but is null.
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn client_kind_all_lists_every_variant() {
        let all = ClientKind::all();
        assert_eq!(all.len(), 7, "expected all 7 clients: {all:?}");
        // Spot-check a couple of variants to guard against silent drift.
        assert!(all.contains(&ClientKind::Claude));
        assert!(all.contains(&ClientKind::Antigravity));
    }

    #[test]
    fn session_summary_deserializes_legacy_provider_field() {
        let raw = json!({
            "provider": "claude",
            "subscription": "Claude Max 5x",
            "session_id": "abc",
            "started_at": null,
            "last_active": null,
            "model": null,
            "cwd": null,
            "state": null,
            "state_detail": null,
            "model_effort": null,
            "model_effort_detail": null,
            "session_title": null,
            "data_path": "/tmp/demo"
        });

        let summary: SessionSummary =
            serde_json::from_value(raw).expect("deserialize legacy summary");
        assert_eq!(summary.client, ClientKind::Claude);
        assert_eq!(summary.subscription.as_deref(), Some("Claude Max 5x"));
    }

    #[test]
    fn parser_state_default_is_unknown() {
        assert_eq!(ParserState::default(), ParserState::Unknown);
    }

    #[test]
    fn parser_state_serde_round_trip() {
        let cases = [
            ParserState::Idle,
            ParserState::Running,
            ParserState::Waiting(WaitReason::Input),
            ParserState::Waiting(WaitReason::Permission),
            ParserState::Error(ErrorReason::ParserDetected("boom".into())),
            ParserState::Unknown,
        ];
        for c in cases {
            let s = serde_json::to_string(&c).unwrap();
            let back: ParserState = serde_json::from_str(&s).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn plan_usage_deserializes_legacy_provider_field() {
        let raw = json!({
            "provider": "codex",
            "label": "Codex · Plus",
            "plan_name": "plus",
            "windows": [],
            "last_limit_hit": null,
            "note": null
        });

        let usage: PlanUsage = serde_json::from_value(raw).expect("deserialize legacy plan usage");
        assert_eq!(usage.client, ClientKind::Codex);
        assert_eq!(usage.plan_name.as_deref(), Some("plus"));
    }

    #[cfg(test)]
    mod state_tests {
        use super::*;

        #[test]
        fn as_str_returns_outer_kind() {
            assert_eq!(SessionState::Running.as_str(), "running");
            assert_eq!(SessionState::Waiting(WaitReason::Input).as_str(), "waiting");
            assert_eq!(SessionState::Waiting(WaitReason::Permission).as_str(), "waiting");
            assert_eq!(SessionState::Idle.as_str(), "idle");
            assert_eq!(SessionState::Closed.as_str(), "closed");
        }

        #[test]
        fn serialization_is_tagged() {
            let v = SessionState::Waiting(WaitReason::Permission);
            let json = serde_json::to_value(&v).unwrap();
            assert_eq!(
                json,
                serde_json::json!({"kind": "waiting", "reason": "permission"})
            );
        }

        #[test]
        fn warning_stalled_carries_since() {
            let t = chrono::DateTime::parse_from_rfc3339("2026-04-26T10:00:00Z").unwrap().to_utc();
            let v = SessionState::Warning(WarningReason::Stalled { since: t });
            let json = serde_json::to_value(&v).unwrap();
            assert_eq!(json["kind"], "warning");
            assert!(json["reason"]["stalled"]["since"].is_string());
        }

        #[test]
        fn error_exit_code_serializes() {
            let v = SessionState::Error(ErrorReason::ExitCode(127));
            let json = serde_json::to_value(&v).unwrap();
            assert_eq!(json["kind"], "error");
            assert_eq!(json["reason"]["exit_code"], 127);
        }

        #[test]
        fn deserialize_round_trips() {
            let original = SessionState::Waiting(WaitReason::Permission);
            let s = serde_json::to_string(&original).unwrap();
            let back: SessionState = serde_json::from_str(&s).unwrap();
            assert_eq!(back, original);
        }

        #[test]
        fn is_active_matches_expected_variants() {
            assert!(SessionState::Running.is_active());
            assert!(SessionState::Idle.is_active());
            assert!(!SessionState::Waiting(WaitReason::Input).is_active());
            assert!(!SessionState::Closed.is_active());
        }

        #[test]
        fn session_analysis_has_current_action_field() {
            let mut a = SessionAnalysis::new(
                SessionSummary::new(
                    ClientKind::Claude,
                    None,
                    "x".to_string(),
                    None,
                    None,
                    None,
                    None,
                    std::path::PathBuf::from("/tmp"),
                    None,
                    None,
                    None,
                    None,
                ),
                TokenTotals::default(),
                CostBreakdown::default(),
                None,
                0,
                None,
                None,
                None,
                None,
                None,
            );
            a.current_action = Some("bash: cargo test".to_string());
            assert_eq!(a.current_action.as_deref(), Some("bash: cargo test"));

            a.current_action = None;
            assert!(a.current_action.is_none());
        }
    }
}
