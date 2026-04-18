//! OpenCode provider — `~/.local/share/opencode/`.
//!
//! **Storage format history:**
//! - v1.1.x and earlier: JSON files under `storage/session/<projectId>/ses_*.json`
//!   and `storage/message/<sessionId>/msg_*.json`.
//! - v1.4.x+: SQLite database at `opencode.db` with `session` and `message` tables.
//!   Message data is stored as JSON in the `data` column.
//!
//! This provider tries SQLite first (preferred), then falls back to the legacy
//! JSON layout so that old session history is still visible.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::OptionalExtension;

use crate::error::{Error, Result};
use crate::pricing::{self, Plan, PlanMode};
use crate::provider::Provider;
use crate::providers::util::dir_exists;
use crate::session::{
    PlanUsage, PlanWindow, ProviderKind, SessionAnalysis, SessionSummary, TokenTotals,
};

#[derive(Debug, Clone)]
pub struct OpenCodeProvider {
    pub storage_root: PathBuf,
}

impl Default for OpenCodeProvider {
    fn default() -> Self {
        // XDG data dir; fallback to ~/.local/share.
        let base = dirs::data_dir().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/"))
                .join(".local")
                .join("share")
        });
        Self {
            storage_root: base.join("opencode"),
        }
    }
}

impl Provider for OpenCodeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenCode
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let db_path = self.storage_root.join("opencode.db");
        let subscriptions = read_subscriptions(&self.storage_root);
        let mut out = Vec::new();

        // --- SQLite path (v1.4+) ---
        if db_path.exists() {
            match list_sessions_sqlite(&db_path, &subscriptions) {
                Ok(mut rows) => out.append(&mut rows),
                Err(e) => {
                    tracing::warn!(path = %db_path.display(), error = %e, "opencode sqlite list failed")
                }
            }
        }

        // --- Legacy JSON path (v1.1 and earlier) ---
        let session_root = self.storage_root.join("storage").join("session");
        if dir_exists(&session_root) {
            match list_sessions_json(&session_root, &self.storage_root, &subscriptions) {
                Ok(mut rows) => out.append(&mut rows),
                Err(e) => tracing::warn!(error = %e, "opencode json list failed"),
            }
        }

        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        let db_path = self.storage_root.join("opencode.db");

        // Try SQLite first (session IDs are the same format in both storage backends).
        if db_path.exists() {
            match analyze_session_sqlite(summary, plan, &db_path) {
                Ok(a) => return Ok(a),
                Err(Error::NoUsage(_)) => {} // no rows in DB → fall through to JSON
                Err(e) => {
                    tracing::debug!(error = %e, session = %summary.session_id, "sqlite analyze failed, trying json")
                }
            }
        }

        // Fallback: legacy JSON message files.
        analyze_opencode_session_json(summary, plan, &self.storage_root)
    }

    fn plan_usage(&self) -> Result<Vec<PlanUsage>> {
        Ok(collect_plan_usage(&self.storage_root))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ms_to_utc(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
}

fn state_from_opencode_message(v: &serde_json::Value) -> Option<(String, String)> {
    match v.get("finish").and_then(|x| x.as_str()) {
        Some("tool-calls") => Some(("waiting".to_string(), "finish=tool-calls".to_string())),
        Some("stop") => Some(("stopped".to_string(), "finish=stop".to_string())),
        _ => None,
    }
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

// ---------------------------------------------------------------------------
// Plan usage
// ---------------------------------------------------------------------------

fn collect_plan_usage(storage_root: &Path) -> Vec<PlanUsage> {
    let subscriptions = read_subscriptions(storage_root);
    let auth_kinds = read_auth_kinds(storage_root);
    let db_path = storage_root.join("opencode.db");

    let mut oauth_providers: Vec<String> = auth_kinds
        .iter()
        .filter_map(|(provider_id, kind)| {
            if *kind == AuthKind::Oauth {
                Some(provider_id.clone())
            } else {
                None
            }
        })
        .collect();
    oauth_providers.sort();

    if oauth_providers.is_empty() {
        return Vec::new();
    }

    let anthropic_oauth = auth_kinds.get("anthropic") == Some(&AuthKind::Oauth);

    let mut anthropic_windows: Vec<PlanWindow> = Vec::new();
    let mut anthropic_last_limit_hit: Option<DateTime<Utc>> = None;
    let mut anthropic_note: Option<String> = None;

    if anthropic_oauth && db_path.exists() {
        match read_latest_anthropic_snapshot(&db_path) {
            Ok(Some(snapshot)) => {
                anthropic_last_limit_hit = ms_to_utc(snapshot.time_created_ms);

                let bind_5h = snapshot.representative_claim.as_deref() == Some("five_hour");
                let bind_7d = snapshot.representative_claim.as_deref() == Some("weekly");

                if snapshot.util_5h.is_some() || snapshot.reset_5h.is_some() {
                    anthropic_windows.push(PlanWindow {
                        label: "5h".to_string(),
                        utilization: snapshot.util_5h,
                        reset_at: snapshot
                            .reset_5h
                            .and_then(|secs| Utc.timestamp_opt(secs, 0).single()),
                        reset_hint: None,
                        binding: bind_5h,
                    });
                }

                if snapshot.util_7d.is_some() || snapshot.reset_7d.is_some() {
                    anthropic_windows.push(PlanWindow {
                        label: "7d".to_string(),
                        utilization: snapshot.util_7d,
                        reset_at: snapshot
                            .reset_7d
                            .and_then(|secs| Utc.timestamp_opt(secs, 0).single()),
                        reset_hint: None,
                        binding: bind_7d,
                    });
                }
            }
            Ok(None) => anthropic_note = Some("no recent rate-limit snapshot".to_string()),
            Err(e) => {
                tracing::debug!(path = %db_path.display(), error = %e, "opencode plan_usage query failed");
                anthropic_note = Some("no recent rate-limit snapshot".to_string());
            }
        }
    } else if anthropic_oauth {
        anthropic_note = Some("no recent rate-limit snapshot".to_string());
    }

    let mut out = Vec::new();
    for provider_id in oauth_providers {
        if provider_id == "anthropic" {
            out.push(PlanUsage {
                provider: ProviderKind::OpenCode,
                label: "OpenCode · Max 5x".to_string(),
                plan_name: Some("Max 5x".to_string()),
                windows: anthropic_windows.clone(),
                last_limit_hit: anthropic_last_limit_hit,
                note: anthropic_note.clone(),
            });
            continue;
        }

        let plan_name = subscriptions
            .get(&provider_id)
            .cloned()
            .unwrap_or_else(|| format!("{} (OAuth)", title_case_words(&provider_id)));
        out.push(PlanUsage {
            provider: ProviderKind::OpenCode,
            label: format!("OpenCode · {plan_name}"),
            plan_name: Some(plan_name),
            windows: Vec::new(),
            last_limit_hit: None,
            note: Some("usage windows unavailable in OpenCode telemetry".to_string()),
        });
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthKind {
    Oauth,
    Api,
}

fn read_auth_kinds(storage_root: &Path) -> HashMap<String, AuthKind> {
    let auth_path = storage_root.join("auth.json");
    let raw = match fs::read_to_string(&auth_path) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let Some(obj) = parsed.as_object() else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    for (provider_id, entry) in obj {
        let kind = match entry.get("type").and_then(|x| x.as_str()) {
            Some("oauth") => AuthKind::Oauth,
            Some("api") => AuthKind::Api,
            _ => continue,
        };
        out.insert(provider_id.to_string(), kind);
    }
    out
}

fn read_subscriptions(storage_root: &Path) -> HashMap<String, String> {
    let auth_path = storage_root.join("auth.json");
    let raw = match fs::read_to_string(&auth_path) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let Some(obj) = parsed.as_object() else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    for (provider_id, entry) in obj {
        let Some(label) = subscription_label_for_provider(provider_id, entry) else {
            continue;
        };
        out.insert(provider_id.to_string(), label);
    }
    out
}

fn subscription_label_for_provider(
    provider_id: &str,
    auth_entry: &serde_json::Value,
) -> Option<String> {
    let auth_kind = match auth_entry.get("type").and_then(|x| x.as_str()) {
        Some("oauth") => AuthKind::Oauth,
        Some("api") => AuthKind::Api,
        _ => return None,
    };

    match (provider_id, auth_kind) {
        ("anthropic", AuthKind::Oauth) => Some("Max 5x".to_string()),
        ("anthropic", AuthKind::Api) => Some("Anthropic API key".to_string()),
        ("openai", AuthKind::Oauth) => {
            read_openai_plan_name(auth_entry).or_else(|| Some("ChatGPT (OAuth)".to_string()))
        }
        ("openai", AuthKind::Api) => Some("OpenAI API key".to_string()),
        ("github-copilot", AuthKind::Oauth) => Some("GitHub Copilot".to_string()),
        ("github-copilot", AuthKind::Api) => Some("GitHub Copilot API key".to_string()),
        ("amazon-bedrock", AuthKind::Oauth) => Some("Amazon Bedrock (OAuth)".to_string()),
        ("amazon-bedrock", AuthKind::Api) => Some("Amazon Bedrock API key".to_string()),
        (_, AuthKind::Oauth) => Some(format!("{} (OAuth)", title_case_words(provider_id))),
        (_, AuthKind::Api) => Some(format!("{} API key", title_case_words(provider_id))),
    }
}

fn read_openai_plan_name(auth_entry: &serde_json::Value) -> Option<String> {
    let token = auth_entry
        .get("id_token")
        .and_then(|x| x.as_str())
        .or_else(|| auth_entry.get("access").and_then(|x| x.as_str()))?;
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64url_decode(payload_b64)?;
    let payload: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let auth = payload.get("https://api.openai.com/auth")?;
    let plan_type = auth.get("chatgpt_plan_type").and_then(|x| x.as_str())?;
    Some(map_openai_plan_type(plan_type))
}

fn map_openai_plan_type(raw: &str) -> String {
    match raw {
        "plus" => "ChatGPT Plus".to_string(),
        "pro" => "ChatGPT Pro".to_string(),
        "business" => "ChatGPT Business".to_string(),
        "enterprise" => "ChatGPT Enterprise".to_string(),
        other => title_case_words(other),
    }
}

fn title_case_words(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut make_upper = true;
    for c in raw.chars() {
        if c == '-' || c == '_' || c == ' ' {
            out.push(' ');
            make_upper = true;
            continue;
        }
        if make_upper {
            for up in c.to_uppercase() {
                out.push(up);
            }
            make_upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn resolve_subscription(
    subscriptions: &HashMap<String, String>,
    provider_id: Option<&str>,
    model: Option<&str>,
) -> Option<String> {
    if let Some(provider) = provider_id {
        if let Some(name) = subscriptions.get(provider) {
            return Some(name.clone());
        }
    }

    let inferred_provider = model.and_then(infer_provider_from_model);
    inferred_provider.and_then(|provider| subscriptions.get(provider).cloned())
}

fn infer_provider_from_model(model: &str) -> Option<&'static str> {
    let lower = model.to_ascii_lowercase();
    for provider in [
        "anthropic",
        "openai",
        "github-copilot",
        "amazon-bedrock",
        "opencode",
    ] {
        let slash = format!("{provider}/");
        let dot = format!("{provider}.");
        if lower.starts_with(&slash) || lower.starts_with(&dot) {
            return Some(provider);
        }
    }
    None
}

/// Decode a base64url-encoded string (RFC 4648 §5) — standard alphabet
/// with `-`/`_` and optional padding. Returns `None` on any character or
/// length error.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut s: String = input
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    while s.len() % 4 != 0 {
        s.push('=');
    }

    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 4 {
            return None;
        }
        let pad = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            let v = if b == b'=' { 0 } else { val(b)? };
            n |= (v as u32) << (18 - 6 * i);
        }
        out.push(((n >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((n & 0xFF) as u8);
        }
    }
    Some(out)
}

#[derive(Debug, Clone, Default)]
struct AnthropicRateLimitSnapshot {
    util_5h: Option<f64>,
    reset_5h: Option<i64>,
    util_7d: Option<f64>,
    reset_7d: Option<i64>,
    representative_claim: Option<String>,
    time_created_ms: i64,
}

fn read_latest_anthropic_snapshot(db_path: &Path) -> Result<Option<AnthropicRateLimitSnapshot>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT data, time_created FROM message \
         WHERE data LIKE ?1 \
         ORDER BY time_created DESC LIMIT 1",
    )?;

    let like = "%anthropic-ratelimit-unified-5h-utilization%";
    let row = stmt
        .query_row(rusqlite::params![like], |row| {
            let data: String = row.get(0)?;
            let time_created: i64 = row.get(1)?;
            Ok((data, time_created))
        })
        .optional()?;

    let Some((data, time_created_ms)) = row else {
        return Ok(None);
    };

    let parsed: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let headers = parsed
        .get("error")
        .and_then(|x| x.get("data"))
        .and_then(|x| x.get("responseHeaders"));
    let Some(h) = headers else {
        return Ok(None);
    };

    let get_str = |k: &str| h.get(k).and_then(|x| x.as_str());
    let util_5h =
        get_str("anthropic-ratelimit-unified-5h-utilization").and_then(|s| s.parse::<f64>().ok());
    let reset_5h = get_str("anthropic-ratelimit-unified-5h-reset").and_then(|s| s.parse().ok());
    let util_7d =
        get_str("anthropic-ratelimit-unified-7d-utilization").and_then(|s| s.parse::<f64>().ok());
    let reset_7d = get_str("anthropic-ratelimit-unified-7d-reset").and_then(|s| s.parse().ok());
    let representative_claim =
        get_str("anthropic-ratelimit-unified-representative-claim").map(|s| s.to_string());

    Ok(Some(AnthropicRateLimitSnapshot {
        util_5h,
        reset_5h,
        util_7d,
        reset_7d,
        representative_claim,
        time_created_ms,
    }))
}

// ---------------------------------------------------------------------------
// SQLite backend (v1.4+)
// ---------------------------------------------------------------------------

fn open_db(db_path: &Path) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

fn list_sessions_sqlite(
    db_path: &Path,
    subscriptions: &HashMap<String, String>,
) -> Result<Vec<SessionSummary>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT id, directory, time_created, time_updated FROM session \
         WHERE time_archived IS NULL OR time_archived = 0 \
         ORDER BY time_updated DESC",
    )?;

    let rows: Vec<SessionSummary> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let cwd: Option<String> = row.get(1)?;
            let created_ms: Option<i64> = row.get(2)?;
            let updated_ms: Option<i64> = row.get(3)?;
            Ok((id, cwd, created_ms, updated_ms))
        })?
        .filter_map(|r| r.ok())
        .map(|(id, cwd, created_ms, updated_ms)| {
            let started_at = created_ms.and_then(ms_to_utc);
            let last_active = updated_ms.and_then(ms_to_utc).or(started_at);
            let (model, provider_id) = first_message_identity_sqlite(&conn, &id);
            let (state, state_detail) = latest_message_state_sqlite(&conn, &id);
            let subscription =
                resolve_subscription(subscriptions, provider_id.as_deref(), model.as_deref());
            SessionSummary {
                provider: ProviderKind::OpenCode,
                subscription,
                session_id: id.clone(),
                started_at,
                last_active,
                model,
                cwd,
                state,
                state_detail,
                model_effort: None,
                model_effort_detail: None,
                data_path: db_path.to_path_buf(),
            }
        })
        .collect();

    Ok(rows)
}

fn latest_message_state_sqlite(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT data FROM message WHERE session_id = ?1 ORDER BY time_created DESC LIMIT 1",
        rusqlite::params![session_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|data| serde_json::from_str::<serde_json::Value>(&data).ok())
    .and_then(|value| state_from_opencode_message(&value))
    .map_or((None, None), |(state, detail)| (Some(state), Some(detail)))
}

fn first_message_identity_sqlite(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT json_extract(data, '$.modelID'), json_extract(data, '$.providerID') FROM message \
         WHERE session_id = ?1 \
           AND json_extract(data, '$.role') = 'assistant' \
         ORDER BY time_created ASC \
         LIMIT 1",
        rusqlite::params![session_id],
        |row| {
            let model: Option<String> = row.get(0)?;
            let provider_id: Option<String> = row.get(1)?;
            Ok((model, provider_id))
        },
    )
    .unwrap_or((None, None))
}

// ---------------------------------------------------------------------------
// Turn accumulator — shared by SQLite and JSON analysis paths
// ---------------------------------------------------------------------------

/// Accumulates token totals, cost, and context-window metrics across the
/// assistant-role messages of a single OpenCode session.  Both the SQLite
/// path and the legacy JSON path iterate over `serde_json::Value` objects
/// that have the same schema; only the *source* of those values differs.
struct TurnAccumulator {
    totals: TokenTotals,
    model: Option<String>,
    cost_reported: f64,
    saw: bool,
    tool_call_count: u64,
    context_used_pct: Option<f64>,
    context_used_tokens: Option<u64>,
    context_window: Option<u64>,
}

impl TurnAccumulator {
    fn new(initial_model: Option<String>) -> Self {
        Self {
            totals: TokenTotals::default(),
            model: initial_model,
            cost_reported: 0.0,
            saw: false,
            tool_call_count: 0,
            context_used_pct: None,
            context_used_tokens: None,
            context_window: None,
        }
    }

    /// Ingest one assistant-role message value.
    fn process_turn(&mut self, v: &serde_json::Value) {
        if self.model.is_none() {
            if let Some(m) = v.get("modelID").and_then(|x| x.as_str()) {
                self.model = Some(m.to_string());
            }
        }
        if v.get("finish").and_then(|x| x.as_str()) == Some("tool-calls") {
            self.tool_call_count += 1;
        }
        if let Some(m) = v.get("modelID").and_then(|x| x.as_str()) {
            if let Some(window) =
                pricing::context_window(ProviderKind::OpenCode, m).filter(|w| *w > 0)
            {
                let turn_total = v
                    .get("tokens")
                    .map(|t| {
                        t.get("input").and_then(|x| x.as_u64()).unwrap_or(0)
                            + t.get("output").and_then(|x| x.as_u64()).unwrap_or(0)
                            + t.get("reasoning").and_then(|x| x.as_u64()).unwrap_or(0)
                            + t.get("cache")
                                .and_then(|c| c.get("read"))
                                .and_then(|x| x.as_u64())
                                .unwrap_or(0)
                            + t.get("cache")
                                .and_then(|c| c.get("write"))
                                .and_then(|x| x.as_u64())
                                .unwrap_or(0)
                    })
                    .unwrap_or(0);
                let pct = (turn_total as f64 / window as f64) * 100.0;
                let is_new_peak = self.context_used_pct.is_none_or(|cur| pct > cur);
                self.context_used_pct = Some(match self.context_used_pct {
                    Some(cur) if cur >= pct => cur,
                    _ => pct,
                });
                if is_new_peak {
                    self.context_used_tokens = Some(turn_total);
                    self.context_window = Some(window);
                }
            }
        }
        if let Some(c) = v.get("cost").and_then(|x| x.as_f64()) {
            self.cost_reported += c;
        }
        if let Some(t) = v.get("tokens") {
            self.saw = true;
            let g = |k: &str| t.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            self.totals.input += g("input");
            self.totals.output += g("output");
            self.totals.reasoning_output += g("reasoning");
            if let Some(cache) = t.get("cache") {
                self.totals.cache_read += cache.get("read").and_then(|x| x.as_u64()).unwrap_or(0);
                self.totals.cache_write_5m +=
                    cache.get("write").and_then(|x| x.as_u64()).unwrap_or(0);
            }
        }
    }

    /// Finalise and build a `SessionAnalysis`.  Returns `Err(NoUsage)` when
    /// no token-bearing turns were seen.
    fn finish(mut self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        if !self.saw {
            return Err(Error::NoUsage(summary.session_id.clone()));
        }
        self.totals.cached_input = self.totals.cache_read;

        let included = matches!(plan.mode_for(ProviderKind::OpenCode), PlanMode::Included);
        let cost = match self
            .model
            .as_deref()
            .and_then(|m| pricing::lookup(ProviderKind::OpenCode, m))
        {
            Some(rates) => pricing::compute_cost(&self.totals, &rates, included),
            None => {
                if included {
                    Default::default()
                } else {
                    crate::session::CostBreakdown {
                        total: self.cost_reported,
                        output: self.cost_reported,
                        ..Default::default()
                    }
                }
            }
        };

        Ok(SessionAnalysis {
            summary: summary.clone(),
            tokens: self.totals,
            cost,
            effective_model: self.model,
            subagent_file_count: 0,
            tool_call_count: Some(self.tool_call_count),
            duration_secs: summary
                .started_at
                .zip(summary.last_active)
                .and_then(|(start, end)| {
                    if end >= start {
                        Some((end - start).num_seconds() as u64)
                    } else {
                        None
                    }
                }),
            context_used_pct: self.context_used_pct,
            context_used_tokens: self.context_used_tokens,
            context_window: self.context_window,
        })
    }
}

fn analyze_session_sqlite(
    summary: &SessionSummary,
    plan: Plan,
    db_path: &Path,
) -> Result<SessionAnalysis> {
    let conn = open_db(db_path)?;

    let mut stmt = conn.prepare(
        "SELECT data FROM message \
         WHERE session_id = ?1 \
           AND json_extract(data, '$.role') = 'assistant'",
    )?;

    let mut acc = TurnAccumulator::new(summary.model.clone());

    let rows = stmt.query_map(rusqlite::params![&summary.session_id], |row| {
        row.get::<_, String>(0)
    })?;

    for row in rows {
        let data_str = match row {
            Ok(s) => s,
            Err(_) => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&data_str) {
            Ok(v) => v,
            Err(_) => continue,
        };
        acc.process_turn(&v);
    }

    acc.finish(summary, plan)
}

// ---------------------------------------------------------------------------
// Legacy JSON backend (v1.1 and earlier)
// ---------------------------------------------------------------------------

fn list_sessions_json(
    session_root: &Path,
    storage_root: &Path,
    subscriptions: &HashMap<String, String>,
) -> Result<Vec<SessionSummary>> {
    let mut out = Vec::new();
    let project_dirs = match fs::read_dir(session_root) {
        Ok(r) => r,
        Err(_) => return Ok(out),
    };
    for proj in project_dirs.flatten() {
        if !proj.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let files = match fs::read_dir(proj.path()) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().map(|e| e != "json").unwrap_or(true) {
                continue;
            }
            match summarize_opencode_session_json(&p, storage_root, subscriptions) {
                Ok(s) => out.push(s),
                Err(e) => {
                    tracing::debug!(path = %p.display(), error = %e, "skip opencode json session");
                    continue;
                }
            }
        }
    }
    Ok(out)
}

fn summarize_opencode_session_json(
    session_file: &Path,
    storage_root: &Path,
    subscriptions: &HashMap<String, String>,
) -> Result<SessionSummary> {
    let v = read_json(session_file)?;
    let session_id = v
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| Error::Other("opencode session missing id".into()))?
        .to_string();
    let created = v
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(|x| x.as_i64())
        .and_then(ms_to_utc);
    let updated = v
        .get("time")
        .and_then(|t| t.get("updated"))
        .and_then(|x| x.as_i64())
        .and_then(ms_to_utc);
    let cwd = v
        .get("directory")
        .and_then(|x| x.as_str())
        .map(str::to_string);

    // Peek at one message file to learn the model, if available.
    let msg_dir = storage_root
        .join("storage")
        .join("message")
        .join(&session_id);
    let (model, provider_id) = first_message_identity_json(&msg_dir);
    let (state, state_detail) = latest_message_state_json(&msg_dir);
    let subscription =
        resolve_subscription(subscriptions, provider_id.as_deref(), model.as_deref());

    Ok(SessionSummary {
        provider: ProviderKind::OpenCode,
        subscription,
        session_id,
        started_at: created,
        last_active: updated.or(created),
        model,
        cwd,
        state,
        state_detail,
        model_effort: None,
        model_effort_detail: None,
        data_path: session_file.to_path_buf(),
    })
}

fn latest_message_state_json(msg_dir: &Path) -> (Option<String>, Option<String>) {
    if !dir_exists(msg_dir) {
        return (None, None);
    }

    let mut latest: Option<(std::time::SystemTime, serde_json::Value)> = None;
    let entries = match fs::read_dir(msg_dir) {
        Ok(entries) => entries,
        Err(_) => return (None, None),
    };
    for f in entries.flatten() {
        let p = f.path();
        if p.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        let Ok(meta) = fs::metadata(&p) else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(value) = read_json(&p) else {
            continue;
        };
        match &latest {
            Some((cur_modified, _)) if *cur_modified >= modified => {}
            _ => latest = Some((modified, value)),
        }
    }

    latest
        .and_then(|(_, value)| state_from_opencode_message(&value))
        .map_or((None, None), |(state, detail)| (Some(state), Some(detail)))
}

fn first_message_identity_json(msg_dir: &Path) -> (Option<String>, Option<String>) {
    if !dir_exists(msg_dir) {
        return (None, None);
    }
    let entries = match fs::read_dir(msg_dir) {
        Ok(entries) => entries,
        Err(_) => return (None, None),
    };
    for f in entries.flatten() {
        let p = f.path();
        if p.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        if let Ok(v) = read_json(&p) {
            if let Some(m) = v.get("modelID").and_then(|x| x.as_str()) {
                let provider_id = v
                    .get("providerID")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                return (Some(m.to_string()), provider_id);
            }
        }
    }
    (None, None)
}

fn analyze_opencode_session_json(
    summary: &SessionSummary,
    plan: Plan,
    storage_root: &Path,
) -> Result<SessionAnalysis> {
    let msg_dir = storage_root
        .join("storage")
        .join("message")
        .join(&summary.session_id);
    if !dir_exists(&msg_dir) {
        return Err(Error::NoUsage(summary.session_id.clone()));
    }

    let mut acc = TurnAccumulator::new(summary.model.clone());

    let entries = fs::read_dir(&msg_dir)?;
    for f in entries.flatten() {
        let p = f.path();
        if p.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        let v = match read_json(&p) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("role").and_then(|x| x.as_str()) != Some("assistant") {
            continue;
        }
        acc.process_turn(&v);
    }

    acc.finish(summary, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp test dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_auth_json(root: &Path, auth_kind: &str) {
        let auth = serde_json::json!({
            "anthropic": {
                "type": auth_kind,
                "access": "token"
            }
        });
        fs::write(
            root.join("auth.json"),
            serde_json::to_vec(&auth).expect("auth json"),
        )
        .expect("write auth.json");
    }

    fn write_auth_json_value(root: &Path, v: serde_json::Value) {
        fs::write(
            root.join("auth.json"),
            serde_json::to_vec(&v).expect("auth json"),
        )
        .expect("write auth.json");
    }

    fn init_db(root: &Path) {
        let conn = rusqlite::Connection::open(root.join("opencode.db")).expect("open sqlite");
        conn.execute(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT)",
            [],
        )
        .expect("create message table");
    }

    fn insert_message(root: &Path, data: serde_json::Value, time_created_ms: i64) {
        let conn = rusqlite::Connection::open(root.join("opencode.db")).expect("open sqlite");
        conn.execute(
            "INSERT INTO message(id, session_id, time_created, data) VALUES(?1, ?2, ?3, ?4)",
            rusqlite::params![
                "msg_1",
                "ses_1",
                time_created_ms,
                serde_json::to_string(&data).expect("msg json")
            ],
        )
        .expect("insert message row");
    }

    #[test]
    fn plan_usage_happy_path_with_all_headers() {
        let tmp = TestDir::new("agtop-opencode-plan-usage");
        write_auth_json(&tmp.path, "oauth");
        init_db(&tmp.path);

        let data = serde_json::json!({
            "role": "assistant",
            "error": {
                "name": "APIError",
                "data": {
                    "responseHeaders": {
                        "anthropic-ratelimit-unified-5h-utilization": "0.64",
                        "anthropic-ratelimit-unified-5h-reset": "1774299600",
                        "anthropic-ratelimit-unified-7d-utilization": "0.05",
                        "anthropic-ratelimit-unified-7d-reset": "1774886400",
                        "anthropic-ratelimit-unified-representative-claim": "five_hour"
                    }
                }
            }
        });
        insert_message(&tmp.path, data, 1_774_290_000_000);

        let provider = OpenCodeProvider {
            storage_root: tmp.path.clone(),
        };
        let out = provider.plan_usage().expect("plan_usage");
        assert_eq!(out.len(), 1);
        let pu = &out[0];
        assert_eq!(pu.plan_name.as_deref(), Some("Max 5x"));
        assert_eq!(pu.windows.len(), 2);

        let w5 = pu.windows.iter().find(|w| w.label == "5h").expect("5h");
        let w7 = pu.windows.iter().find(|w| w.label == "7d").expect("7d");
        assert_eq!(w5.utilization, Some(0.64));
        assert_eq!(w7.utilization, Some(0.05));
        assert!(w5.binding);
        assert!(!w7.binding);
        assert!(pu.last_limit_hit.is_some());
    }

    #[test]
    fn plan_usage_api_auth_returns_empty() {
        // Implementation choice: skip API-key auth entirely because plan
        // usage panes are only meaningful for OAuth/subscription auth.
        let tmp = TestDir::new("agtop-opencode-plan-api");
        write_auth_json(&tmp.path, "api");
        init_db(&tmp.path);

        let provider = OpenCodeProvider {
            storage_root: tmp.path.clone(),
        };
        let out = provider.plan_usage().expect("plan_usage");
        assert!(out.is_empty());
    }

    #[test]
    fn plan_usage_missing_db_returns_empty_without_auth() {
        let tmp = TestDir::new("agtop-opencode-plan-missing-db");
        let provider = OpenCodeProvider {
            storage_root: tmp.path.clone(),
        };
        let out = provider.plan_usage().expect("plan_usage");
        assert!(out.is_empty());
    }

    #[test]
    fn plan_usage_no_matching_rows_emits_note_when_oauth_exists() {
        let tmp = TestDir::new("agtop-opencode-plan-no-rows");
        write_auth_json(&tmp.path, "oauth");
        init_db(&tmp.path);

        let provider = OpenCodeProvider {
            storage_root: tmp.path.clone(),
        };
        let out = provider.plan_usage().expect("plan_usage");
        assert_eq!(out.len(), 1);
        let pu = &out[0];
        assert_eq!(pu.note.as_deref(), Some("no recent rate-limit snapshot"));
        assert!(pu.windows.is_empty());
    }

    #[test]
    fn plan_usage_emits_cards_for_multiple_oauth_providers() {
        let tmp = TestDir::new("agtop-opencode-plan-multi-oauth");
        let auth = serde_json::json!({
            "anthropic": {"type": "oauth", "access": "x"},
            "openai": {"type": "oauth", "access": "not-a-jwt"},
            "github-copilot": {"type": "oauth", "access": "x"}
        });
        write_auth_json_value(&tmp.path, auth);
        init_db(&tmp.path);

        let provider = OpenCodeProvider {
            storage_root: tmp.path.clone(),
        };
        let out = provider.plan_usage().expect("plan_usage");

        assert_eq!(out.len(), 3);
        let anthropic = out
            .iter()
            .find(|pu| pu.label == "OpenCode · Max 5x")
            .expect("anthropic card");
        assert_eq!(anthropic.plan_name.as_deref(), Some("Max 5x"));
        assert_eq!(
            anthropic.note.as_deref(),
            Some("no recent rate-limit snapshot")
        );

        let openai = out
            .iter()
            .find(|pu| pu.label == "OpenCode · ChatGPT (OAuth)")
            .expect("openai card");
        assert_eq!(openai.windows.len(), 0);
        assert_eq!(
            openai.note.as_deref(),
            Some("usage windows unavailable in OpenCode telemetry")
        );

        let copilot = out
            .iter()
            .find(|pu| pu.label == "OpenCode · GitHub Copilot")
            .expect("copilot card");
        assert_eq!(copilot.plan_name.as_deref(), Some("GitHub Copilot"));
    }

    #[test]
    fn finish_tool_calls_maps_to_waiting() {
        let v = serde_json::json!({ "finish": "tool-calls" });
        assert_eq!(
            state_from_opencode_message(&v),
            Some(("waiting".to_string(), "finish=tool-calls".to_string()))
        );
    }

    #[test]
    fn read_subscriptions_maps_multiple_providers() {
        let tmp = TestDir::new("agtop-opencode-subscriptions");
        let auth = serde_json::json!({
            "anthropic": {"type": "oauth", "access": "x"},
            "openai": {"type": "oauth", "access": "not-a-jwt"},
            "github-copilot": {"type": "oauth", "access": "x"},
            "amazon-bedrock": {"type": "api", "key": "x"}
        });
        write_auth_json_value(&tmp.path, auth);

        let subscriptions = read_subscriptions(&tmp.path);
        assert_eq!(
            subscriptions.get("anthropic").map(String::as_str),
            Some("Max 5x")
        );
        assert_eq!(
            subscriptions.get("openai").map(String::as_str),
            Some("ChatGPT (OAuth)")
        );
        assert_eq!(
            subscriptions.get("github-copilot").map(String::as_str),
            Some("GitHub Copilot")
        );
        assert_eq!(
            subscriptions.get("amazon-bedrock").map(String::as_str),
            Some("Amazon Bedrock API key")
        );
    }

    #[test]
    fn resolve_subscription_prefers_provider_id_over_model_name() {
        let mut subscriptions = HashMap::new();
        subscriptions.insert("anthropic".to_string(), "Max 5x".to_string());
        subscriptions.insert("openai".to_string(), "ChatGPT Plus".to_string());

        let got = resolve_subscription(
            &subscriptions,
            Some("openai"),
            Some("anthropic/claude-opus-4-6"),
        );
        assert_eq!(got.as_deref(), Some("ChatGPT Plus"));
    }

    #[test]
    fn resolve_subscription_falls_back_to_model_prefix() {
        let mut subscriptions = HashMap::new();
        subscriptions.insert("openai".to_string(), "ChatGPT Plus".to_string());

        let got = resolve_subscription(&subscriptions, None, Some("openai/gpt-5.4"));
        assert_eq!(got.as_deref(), Some("ChatGPT Plus"));
    }
}
