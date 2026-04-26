//! Gemini CLI client — `~/.gemini/tmp/<slug>/chats/session-*.(json|jsonl)`.
//!
//! Older Gemini CLI builds wrote JSONL session headers; current builds store
//! the full session as a single JSON document.
//! Local telemetry in `~/.gemini/settings.json` provides the best token and
//! runtime metrics. When telemetry is missing, the client falls back to
//! token fields embedded in the session file.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::client::Client;
use crate::clients::util::{dir_exists, for_each_jsonl, mtime, parse_ts, DiscoverCache};
use crate::error::Result;
use crate::pricing::{self, Plan, PlanMode};
use crate::session::{ClientKind, CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals};

#[derive(Debug)]
pub struct GeminiCliClient {
    pub gemini_dir: PathBuf,
    pub discover_cache: Mutex<DiscoverCache>,
}

impl Default for GeminiCliClient {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            gemini_dir: home.join(".gemini"),
            discover_cache: Mutex::default(),
        }
    }
}

impl Client for GeminiCliClient {
    fn kind(&self) -> ClientKind {
        ClientKind::GeminiCli
    }

    fn display_name(&self) -> &'static str {
        "Gemini CLI"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !dir_exists(&self.gemini_dir) {
            return Ok(vec![]);
        }

        // Load the slug → absolute path mapping from projects.json.
        let slug_to_path = read_projects_map(&self.gemini_dir);

        // Load the global model from settings.json.
        let global_model = read_global_model(&self.gemini_dir);

        // Determine subscription from presence of oauth_creds.json.
        let subscription = if self.gemini_dir.join("oauth_creds.json").exists() {
            Some("Google · Gemini".to_string())
        } else {
            Some("Gemini API key".to_string())
        };

        let tmp_dir = self.gemini_dir.join("tmp");
        if !dir_exists(&tmp_dir) {
            return Ok(vec![]);
        }

        let mut out = Vec::new();

        let slug_dirs = match fs::read_dir(&tmp_dir) {
            Ok(d) => d,
            Err(_) => return Ok(out),
        };

        for slug_entry in slug_dirs.flatten() {
            let slug_path = slug_entry.path();
            if !slug_path.is_dir() {
                continue;
            }
            let slug = match slug_path.file_name().and_then(|n| n.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let cwd = slug_to_path.get(&slug).cloned();

            let chats_dir = slug_path.join("chats");
            if !chats_dir.is_dir() {
                continue;
            }

            let chat_files = match fs::read_dir(&chats_dir) {
                Ok(d) => d,
                Err(_) => continue,
            };

            for chat_entry in chat_files.flatten() {
                let path = chat_entry.path();
                let ext = path.extension().and_then(|e| e.to_str());
                if !matches!(ext, Some("json") | Some("jsonl")) {
                    continue;
                }
                let cwd2 = cwd.clone();
                let gm = global_model.clone();
                let sub = subscription.clone();
                let cached = {
                    let mut guard = self.discover_cache.lock().unwrap();
                    guard.get_or_insert_with(&path, || parse_gemini_session(&path, cwd2, gm, sub))
                };
                match cached {
                    Ok(s) => out.push(s),
                    Err(e) => {
                        tracing::debug!(path = %path.display(), error = %e, "skip gemini session");
                    }
                }
            }
        }

        // Collapse duplicate session_ids. Gemini CLI writes a NEW
        // `session-<datetime>-<shortid>.jsonl` every time you `--resume <uuid>`,
        // but each file's first record carries the SAME `sessionId`. Other
        // clients (Claude renames per resume; Codex appends; SQLite-backed
        // clients enforce PK uniqueness) never produce duplicates, and the
        // correlator's PID-binding map is keyed on `session_id` — so duplicates
        // here cause one entry to silently lose its PID match.
        //
        // Tradeoff: the older transcript files become invisible to the session
        // list. See `docs/gemini-cli.md` ("Resume semantics") for rationale.
        out = collapse_duplicate_session_ids(out);

        {
            use std::collections::HashSet;
            let live_paths: HashSet<&std::path::Path> =
                out.iter().map(|s| s.data_path.as_path()).collect();
            self.discover_cache
                .lock()
                .unwrap()
                .retain_paths(&live_paths);
        }

        Ok(out)
    }

    fn children(&self, parent: &SessionSummary) -> Result<Vec<SessionSummary>> {
        // Gemini CLI stores subagent sessions in a subdirectory named after
        // the parent session ID, alongside the parent's own chat file:
        //   ~/.gemini/tmp/<slug>/chats/<parent_session_id>/<subagent_id>.jsonl
        let chats_dir = match parent.data_path.parent() {
            Some(p) => p.to_path_buf(),
            None => return Ok(vec![]),
        };
        let subagent_dir = chats_dir.join(&parent.session_id);
        if !subagent_dir.is_dir() {
            return Ok(vec![]);
        }

        let cwd = parent.cwd.clone();
        let global_model = read_global_model(&self.gemini_dir);
        let subscription = parent.subscription.clone();
        let mut out = Vec::new();

        let entries = match fs::read_dir(&subagent_dir) {
            Ok(d) => d,
            Err(_) => return Ok(out),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !matches!(ext, Some("json") | Some("jsonl")) {
                continue;
            }
            let cwd2 = cwd.clone();
            let gm = global_model.clone();
            let sub = subscription.clone();
            let cached = {
                let mut guard = self.discover_cache.lock().unwrap();
                guard.get_or_insert_with(&path, || parse_gemini_session(&path, cwd2, gm, sub))
            };
            match cached {
                Ok(s) => out.push(s),
                Err(e) => {
                    tracing::debug!(path = %path.display(), error = %e, "skip gemini subagent session");
                }
            }
        }
        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        let telemetry = extract_analysis_from_telemetry(
            &self.gemini_dir,
            summary.started_at,
            summary.last_active,
            &summary.session_id,
        );
        let session = extract_analysis_from_session_file(&summary.data_path, summary.model.clone());
        let analysis = if telemetry.tokens.grand_total() == 0 {
            session
        } else {
            telemetry.with_session_fallback(session)
        };
        let tokens = analysis.tokens;

        let effective_model = analysis.effective_model.or_else(|| summary.model.clone());
        let included = matches!(plan.mode_for(ClientKind::GeminiCli), PlanMode::Included);

        let cost = match &effective_model {
            Some(m) if tokens.grand_total() > 0 => {
                match pricing::lookup(ClientKind::GeminiCli, m) {
                    Some(rates) => pricing::compute_cost(&tokens, &rates, included),
                    None => CostBreakdown::default(),
                }
            }
            _ => CostBreakdown::default(),
        };

        let duration_secs = summary
            .started_at
            .zip(summary.last_active)
            .and_then(|(start, end)| {
                if end >= start {
                    Some((end - start).num_seconds() as u64)
                } else {
                    None
                }
            });

        Ok(SessionAnalysis {
            summary: summary.clone(),
            tokens,
            cost,
            effective_model,
            subagent_file_count: 0,
            tool_call_count: analysis.tool_call_count,
            duration_secs,
            context_used_pct: analysis.context_used_pct,
            context_used_tokens: analysis.context_used_tokens,
            context_window: analysis.context_window,
            children: Vec::new(),
            agent_turns: analysis.agent_turns,
            user_turns: analysis.user_turns,
            project_name: None,
            pid: None,
            liveness: None,
            match_confidence: None,
            process_metrics: None,
        })
    }
}

/// Collapse `SessionSummary` entries that share the same `session_id`.
///
/// For each duplicate group, keep one summary whose:
///   * `started_at` = earliest `started_at` across the group (or any non-`None`
///     value if the most-recent entry's `started_at` was `None`);
///   * `last_active` = latest `last_active` across the group;
///   * remaining fields (incl. `data_path`, `session_title`, `model`, `state`)
///     come from the entry with the latest `last_active` — that's the most
///     recent invocation, which has the freshest title and current state.
///
/// Stable on session_id ordering: groups iterate in original insertion order,
/// preserving the rest of the list's order.
fn collapse_duplicate_session_ids(sessions: Vec<SessionSummary>) -> Vec<SessionSummary> {
    use std::collections::HashMap;

    // First pass: bucket indices by session_id, preserving insertion order
    // for ids encountered exactly once.
    let mut order: Vec<String> = Vec::with_capacity(sessions.len());
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, s) in sessions.iter().enumerate() {
        let entry = by_id.entry(s.session_id.clone()).or_default();
        if entry.is_empty() {
            order.push(s.session_id.clone());
        }
        entry.push(idx);
    }

    // Second pass: for each id, pick the representative (latest last_active)
    // and merge in the earliest started_at from the rest of the group.
    let mut out: Vec<SessionSummary> = Vec::with_capacity(order.len());
    for id in order {
        let indices = by_id.remove(&id).unwrap_or_default();
        if indices.len() == 1 {
            out.push(sessions[indices[0]].clone());
            continue;
        }

        // Find the index with the latest last_active. `None` sorts before
        // any `Some(_)`, so a missing timestamp loses to a present one.
        let rep_idx = *indices
            .iter()
            .max_by_key(|&&i| sessions[i].last_active)
            .expect("indices non-empty by construction");
        let mut rep = sessions[rep_idx].clone();

        // Earliest started_at across the group. Treat `None` as "later than
        // any Some" so a present timestamp always wins over a missing one.
        let earliest_started = indices.iter().filter_map(|&i| sessions[i].started_at).min();
        if let Some(ts) = earliest_started {
            rep.started_at = Some(ts);
        }

        // Latest last_active is already on `rep` because rep_idx maximised
        // it — but defensively recompute in case the representative had
        // `None` and another duplicate had `Some(_)`.
        let latest_last_active = indices
            .iter()
            .filter_map(|&i| sessions[i].last_active)
            .max();
        if rep.last_active.is_none() {
            rep.last_active = latest_last_active;
        }

        out.push(rep);
    }
    out
}

/// Read `~/.gemini/projects.json` and build a slug → path map.
/// The file format is `{ "/absolute/path": "slug" }` — we invert it.
fn read_projects_map(gemini_dir: &std::path::Path) -> HashMap<String, String> {
    let path = gemini_dir.join("projects.json");
    let Ok(bytes) = fs::read(&path) else {
        return HashMap::new();
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return HashMap::new();
    };
    let obj = v
        .get("projects")
        .and_then(|projects| projects.as_object())
        .or_else(|| v.as_object());
    let Some(obj) = obj else {
        return HashMap::new();
    };
    // Invert: value (slug) → key (absolute path).
    obj.iter()
        .filter_map(|(abs_path, slug_val)| {
            slug_val
                .as_str()
                .map(|slug| (slug.to_string(), abs_path.clone()))
        })
        .collect()
}

/// Read `model` from `~/.gemini/settings.json`.
fn read_global_model(gemini_dir: &std::path::Path) -> Option<String> {
    let path = gemini_dir.join("settings.json");
    let bytes = fs::read(&path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("model")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .or_else(|| {
            // Some versions nest it as `model.name`.
            v.pointer("/model/name")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
}

/// Parse a Gemini session file.
fn parse_gemini_session(
    path: &std::path::Path,
    cwd: Option<String>,
    global_model: Option<String>,
    subscription: Option<String>,
) -> Result<SessionSummary> {
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        return parse_gemini_session_json(path, cwd, global_model, subscription);
    }

    let mut session_id: Option<String> = None;
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut last_updated: Option<DateTime<Utc>> = None;
    let mut model = global_model;
    let mut session_title: Option<String> = None;
    let mut state: Option<String> = None;
    let mut state_detail: Option<String> = None;
    let mut seen = 0usize;

    for_each_jsonl(path, |v| {
        seen += 1;

        if let Some(id) = v.get("sessionId").and_then(|x| x.as_str()) {
            session_id = Some(id.to_string());
        }
        if let Some(ts) = v
            .get("startTime")
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
        {
            started_at = Some(ts);
        }
        if let Some(ts) = v
            .get("lastUpdated")
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
        {
            last_updated = Some(ts);
        }
        if let Some(ts) = v
            .get("$set")
            .and_then(|set| set.get("lastUpdated"))
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
        {
            last_updated = Some(ts);
        }
        if session_title.is_none() && v.get("type").and_then(|x| x.as_str()) == Some("user") {
            session_title = v
                .get("content")
                .and_then(gemini_content_text)
                .map(|text| summarize_title(&text));
        }
        if let Some(m) = v.get("model").and_then(|x| x.as_str()) {
            model = Some(m.to_string());
        }
        if v.get("type").and_then(|x| x.as_str()) == Some("gemini") {
            update_state_from_gemini_message(v, &mut state, &mut state_detail);
        }
    })?;

    let session_id = session_id.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    let last_active = last_updated.or_else(|| mtime(path));

    let mut summary = SessionSummary::new(
        ClientKind::GeminiCli,
        subscription,
        session_id,
        started_at,
        last_active,
        model,
        cwd,
        path.to_path_buf(),
        state,
        state_detail,
        None,
        None,
    );
    summary.session_title = session_title;
    Ok(summary)
}

fn parse_gemini_session_json(
    path: &std::path::Path,
    cwd: Option<String>,
    global_model: Option<String>,
    subscription: Option<String>,
) -> Result<SessionSummary> {
    let bytes = fs::read(path)?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)?;

    let session_id = v
        .get("sessionId")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    let started_at = v
        .get("startTime")
        .and_then(|x| x.as_str())
        .and_then(parse_ts);
    let last_active = v
        .get("lastUpdated")
        .and_then(|x| x.as_str())
        .and_then(parse_ts)
        .or_else(|| mtime(path));

    let model = v
        .get("messages")
        .and_then(|messages| messages.as_array())
        .and_then(|messages| {
            messages.iter().rev().find_map(|message| {
                message
                    .get("model")
                    .and_then(|model| model.as_str())
                    .map(str::to_string)
            })
        })
        .or(global_model);

    let mut state: Option<String> = None;
    let mut state_detail: Option<String> = None;
    if let Some(messages) = v.get("messages").and_then(|messages| messages.as_array()) {
        for message in messages {
            if message.get("type").and_then(|x| x.as_str()) == Some("gemini") {
                update_state_from_gemini_message(message, &mut state, &mut state_detail);
            }
        }
    }

    let mut summary = SessionSummary::new(
        ClientKind::GeminiCli,
        subscription,
        session_id,
        started_at,
        last_active,
        model,
        cwd,
        path.to_path_buf(),
        state,
        state_detail,
        None,
        None,
    );
    summary.session_title = v
        .get("messages")
        .and_then(|messages| messages.as_array())
        .and_then(|messages| {
            messages.iter().find_map(|message| {
                if message.get("type").and_then(|x| x.as_str()) == Some("user") {
                    message
                        .get("content")
                        .and_then(gemini_content_text)
                        .map(|text| summarize_title(&text))
                } else {
                    None
                }
            })
        });
    Ok(summary)
}

fn update_state_from_gemini_message(
    message: &serde_json::Value,
    state: &mut Option<String>,
    state_detail: &mut Option<String>,
) {
    if let Some(tool_calls) = gemini_tool_calls(message) {
        if tool_calls
            .iter()
            .any(|call| call.get("status").and_then(|x| x.as_str()) != Some("success"))
        {
            *state = Some("waiting".to_string());
            *state_detail = Some("gemini.toolCalls.pending_or_error".to_string());
        } else if !tool_calls.is_empty() {
            *state = Some("stopped".to_string());
            *state_detail = Some("gemini.toolCalls.success".to_string());
        }
    } else {
        *state = Some("stopped".to_string());
        *state_detail = Some("gemini.message".to_string());
    }
}

fn gemini_content_text(content: &serde_json::Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let arr = content.as_array()?;
    let mut out = String::new();
    for part in arr {
        let text = part
            .get("text")
            .or_else(|| part.get("content"))
            .and_then(|x| x.as_str());
        if let Some(text) = text {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(text);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn summarize_title(text: &str) -> String {
    const MAX_TITLE_CHARS: usize = 80;
    let mut title = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > MAX_TITLE_CHARS {
        title = title.chars().take(MAX_TITLE_CHARS - 1).collect::<String>() + "...";
    }
    title
}

#[derive(Debug, Clone, Default)]
struct GeminiAnalysis {
    tokens: TokenTotals,
    effective_model: Option<String>,
    tool_call_count: Option<u64>,
    agent_turns: Option<u64>,
    user_turns: Option<u64>,
    context_used_pct: Option<f64>,
    context_used_tokens: Option<u64>,
    context_window: Option<u64>,
}

impl GeminiAnalysis {
    fn with_session_fallback(mut self, session: GeminiAnalysis) -> Self {
        if self.effective_model.is_none() {
            self.effective_model = session.effective_model;
        }
        if self.tool_call_count.is_none() {
            self.tool_call_count = session.tool_call_count;
        }
        if self.agent_turns.is_none() {
            self.agent_turns = session.agent_turns;
        }
        if self.user_turns.is_none() {
            self.user_turns = session.user_turns;
        }
        if self.context_used_pct.is_none() {
            self.context_used_pct = session.context_used_pct;
            self.context_used_tokens = session.context_used_tokens;
            self.context_window = session.context_window;
        }
        self
    }

    fn add_context_observation(&mut self, model: Option<&str>, total_tokens: Option<u64>) {
        let Some(total_tokens) = total_tokens.filter(|total| *total > 0) else {
            return;
        };
        let Some(model) = model else {
            return;
        };
        let Some(window) = pricing::context_window(ClientKind::GeminiCli, model).filter(|w| *w > 0)
        else {
            return;
        };

        let pct = (total_tokens as f64 / window as f64) * 100.0;
        if self.context_used_pct.is_none_or(|cur| pct > cur) {
            self.context_used_pct = Some(pct);
            self.context_used_tokens = Some(total_tokens);
            self.context_window = Some(window);
        }
    }
}

/// Parse `~/.gemini/telemetry.log` (JSONL) for Gemini events whose timestamps
/// fall within the session's time range.
fn extract_analysis_from_telemetry(
    gemini_dir: &std::path::Path,
    started_at: Option<DateTime<Utc>>,
    last_active: Option<DateTime<Utc>>,
    session_id: &str,
) -> GeminiAnalysis {
    let telemetry_path = gemini_dir.join("telemetry.log");
    if !telemetry_path.exists() {
        return GeminiAnalysis::default();
    }

    let (start, end) = match (started_at, last_active) {
        (Some(s), Some(e)) => (s, e),
        _ => return GeminiAnalysis::default(),
    };

    // Add a small buffer for clock skew.
    let start_buf = start - chrono::Duration::seconds(5);
    let end_buf = end + chrono::Duration::seconds(5);

    let mut analysis = GeminiAnalysis::default();
    let mut tool_call_count = 0u64;
    let mut saw_tool_call = false;
    let mut api_response_count = 0u64;

    let _ = for_each_jsonl(&telemetry_path, |v| {
        // Check timestamp falls in session window.
        let event_ts = telemetry_timestamp(v);
        let Some(ts) = event_ts else { return };
        if ts < start_buf || ts > end_buf {
            return;
        }
        if let Some(event_session_id) = telemetry_str(v, "session.id") {
            if event_session_id != session_id {
                return;
            }
        }

        let Some(event_name) = telemetry_event_name(v) else {
            return;
        };
        match event_name {
            "gemini_cli.api_response" => {
                api_response_count += 1;
                let model = telemetry_str(v, "model");
                if let Some(model) = model {
                    analysis.effective_model = Some(model.to_string());
                }
                let input = telemetry_u64(v, "input_token_count");
                let output = telemetry_u64(v, "output_token_count");
                let cache = telemetry_u64(v, "cached_content_token_count");
                let thoughts = telemetry_u64(v, "thoughts_token_count");
                analysis.tokens.input += input;
                analysis.tokens.output += output;
                analysis.tokens.cache_read += cache;
                analysis.tokens.reasoning_output += thoughts;

                let explicit_total = telemetry_value(v, "total_token_count").and_then(value_as_u64);
                let derived_total =
                    input + output + cache + thoughts + telemetry_u64(v, "tool_token_count");
                let derived_total = if derived_total > 0 {
                    Some(derived_total)
                } else {
                    None
                };
                analysis.add_context_observation(model, explicit_total.or(derived_total));
            }
            "gemini_cli.tool_call" => {
                saw_tool_call = true;
                tool_call_count += 1;
            }
            _ => {}
        }
    });

    // Post-accumulation: cached_input mirrors cache_read.
    analysis.tokens.cached_input = analysis.tokens.cache_read;
    if saw_tool_call {
        analysis.tool_call_count = Some(tool_call_count);
    }
    if api_response_count > 0 {
        analysis.agent_turns = Some(api_response_count);
    }

    analysis
}

fn extract_analysis_from_session_file(
    path: &std::path::Path,
    initial_model: Option<String>,
) -> GeminiAnalysis {
    let mut analysis = GeminiAnalysis {
        effective_model: initial_model,
        ..Default::default()
    };
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        let Ok(bytes) = fs::read(path) else {
            return analysis;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return analysis;
        };
        if let Some(messages) = v.get("messages").and_then(|x| x.as_array()) {
            for message in messages {
                analysis.add_session_message(message);
            }
        }
    } else {
        let _ = for_each_jsonl(path, |v| {
            analysis.add_session_message(v);
        });
    }
    analysis.tokens.cached_input = analysis.tokens.cache_read;
    analysis
}

impl GeminiAnalysis {
    fn add_session_message(&mut self, message: &serde_json::Value) {
        match message.get("type").and_then(|x| x.as_str()) {
            Some("user") => increment_opt(&mut self.user_turns),
            Some("gemini" | "assistant" | "model") => {
                increment_opt(&mut self.agent_turns);
                let call_count = gemini_tool_calls(message).map_or(0, |calls| calls.len() as u64);
                if call_count > 0 {
                    add_opt(&mut self.tool_call_count, call_count);
                }
            }
            _ => {}
        }

        if let Some(m) = message.get("model").and_then(|x| x.as_str()) {
            self.effective_model = Some(m.to_string());
        }

        let before = self.tokens.clone();
        add_gemini_tokens(&mut self.tokens, message.get("tokens"));
        let added_total = self.tokens.input.saturating_sub(before.input)
            + self.tokens.output.saturating_sub(before.output)
            + self.tokens.cache_read.saturating_sub(before.cache_read)
            + self
                .tokens
                .reasoning_output
                .saturating_sub(before.reasoning_output);

        let model = self.effective_model.clone();
        self.add_context_observation(model.as_deref(), Some(added_total));
    }
}

fn add_gemini_tokens(totals: &mut TokenTotals, raw: Option<&serde_json::Value>) {
    let Some(tokens) = raw else {
        return;
    };
    totals.input += token_u64(
        tokens,
        &["input", "input_tokens", "prompt", "prompt_tokens"],
    );
    totals.output += token_u64(
        tokens,
        &["output", "output_tokens", "response", "response_tokens"],
    );
    totals.cache_read += token_u64(
        tokens,
        &[
            "cached",
            "cached_tokens",
            "cached_content_token_count",
            "cache_read",
            "cache_read_input_tokens",
        ],
    );
    if let Some(cache) = tokens.get("cache") {
        totals.cache_read += token_u64(cache, &["read", "input", "cached"]);
        totals.cache_write_5m += token_u64(cache, &["write"]);
    }
    totals.reasoning_output += token_u64(
        tokens,
        &[
            "thoughts",
            "thought",
            "thoughts_token_count",
            "reasoning",
            "reasoning_output_tokens",
        ],
    );
}

fn token_u64(tokens: &serde_json::Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| tokens.get(*key).and_then(value_as_u64))
        .unwrap_or(0)
}

fn gemini_tool_calls(message: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    message
        .get("toolCalls")
        .or_else(|| message.get("tool_calls"))
        .or_else(|| message.get("functionCalls"))
        .or_else(|| message.get("function_calls"))
        .and_then(|x| x.as_array())
}

fn increment_opt(value: &mut Option<u64>) {
    add_opt(value, 1);
}

fn add_opt(value: &mut Option<u64>, amount: u64) {
    *value = Some(value.unwrap_or(0) + amount);
}

fn telemetry_event_name(v: &serde_json::Value) -> Option<&str> {
    v.get("name")
        .and_then(|x| x.as_str())
        .or_else(|| telemetry_str(v, "event.name"))
}

fn telemetry_timestamp(v: &serde_json::Value) -> Option<DateTime<Utc>> {
    v.get("timestamp")
        .or_else(|| v.get("time"))
        .or_else(|| v.get("time_unix_nano"))
        .and_then(|x| {
            x.as_str()
                .and_then(parse_ts)
                .or_else(|| x.as_u64().and_then(timestamp_nanos_to_datetime))
        })
}

fn timestamp_nanos_to_datetime(nanos: u64) -> Option<DateTime<Utc>> {
    let secs = (nanos / 1_000_000_000) as i64;
    let sub_nanos = (nanos % 1_000_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, sub_nanos)
}

fn telemetry_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    telemetry_value(v, key).and_then(|x| x.as_str())
}

fn telemetry_u64(v: &serde_json::Value, key: &str) -> u64 {
    telemetry_value(v, key).and_then(value_as_u64).unwrap_or(0)
}

fn telemetry_value<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    v.get(key).or_else(|| {
        let attrs = v.get("attributes")?;
        attrs.get(key).and_then(otel_attribute_value).or_else(|| {
            attrs.as_array().and_then(|items| {
                items.iter().find_map(|item| {
                    if item.get("key").and_then(|x| x.as_str()) == Some(key) {
                        item.get("value").and_then(otel_attribute_value)
                    } else {
                        None
                    }
                })
            })
        })
    })
}

fn otel_attribute_value(v: &serde_json::Value) -> Option<&serde_json::Value> {
    v.get("value")
        .or_else(|| v.get("stringValue"))
        .or_else(|| v.get("intValue"))
        .or_else(|| v.get("doubleValue"))
        .or_else(|| v.get("boolValue"))
        .or(Some(v))
}

fn value_as_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::sync::Mutex;

    struct TestDir {
        path: std::path::PathBuf,
    }
    impl TestDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("agtop-gemini-{}-{}", name, std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn missing_dir_returns_empty() {
        let p = GeminiCliClient {
            gemini_dir: std::path::PathBuf::from("/no/such/path"),
            discover_cache: Mutex::default(),
        };
        assert!(p.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn parses_session_jsonl() {
        let td = TestDir::new("session");
        let chats_dir = td.path.join("tmp").join("myproject").join("chats");
        fs::create_dir_all(&chats_dir).unwrap();

        fs::write(
            td.path.join("projects.json"),
            r#"{"/home/user/myproject": "myproject"}"#,
        )
        .unwrap();

        let session_file = chats_dir.join("session-abc123.jsonl");
        let line = r#"{"sessionId":"sess-001","startTime":"2026-04-10T10:00:00Z","lastUpdated":"2026-04-10T11:00:00Z","messageCount":5}"#;
        fs::File::create(&session_file)
            .unwrap()
            .write_all(line.as_bytes())
            .unwrap();

        let p = GeminiCliClient {
            gemini_dir: td.path.clone(),
            discover_cache: Mutex::default(),
        };
        let sessions = p.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-001");
        assert_eq!(sessions[0].client, ClientKind::GeminiCli);
        assert_eq!(sessions[0].cwd.as_deref(), Some("/home/user/myproject"));
    }

    #[test]
    fn parses_current_projects_json_shape() {
        let td = TestDir::new("projects-v2");
        fs::write(
            td.path.join("projects.json"),
            r#"{"projects":{"/home/user/myproject":"myproject"}}"#,
        )
        .unwrap();

        let projects = read_projects_map(&td.path);
        assert_eq!(
            projects.get("myproject").map(String::as_str),
            Some("/home/user/myproject")
        );
    }

    #[test]
    fn parses_current_session_json_format() {
        let td = TestDir::new("session-json");
        let chats_dir = td.path.join("tmp").join("myproject").join("chats");
        fs::create_dir_all(&chats_dir).unwrap();

        fs::write(
            td.path.join("projects.json"),
            r#"{"projects":{"/home/user/myproject":"myproject"}}"#,
        )
        .unwrap();

        let session_file = chats_dir.join("session-abc123.json");
        fs::write(
            &session_file,
            r#"{
  "sessionId": "sess-002",
  "startTime": "2026-04-10T10:00:00Z",
  "lastUpdated": "2026-04-10T11:00:00Z",
  "messages": [
    {
      "type": "gemini",
      "model": "gemini-3-flash-preview",
      "tokens": {
        "input": 100,
        "output": 50,
        "cached": 20,
        "thoughts": 10,
        "tool": 0,
        "total": 180
      }
    }
  ]
}"#,
        )
        .unwrap();

        let p = GeminiCliClient {
            gemini_dir: td.path.clone(),
            discover_cache: Mutex::default(),
        };
        let sessions = p.list_sessions().unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-002");
        assert_eq!(sessions[0].client, ClientKind::GeminiCli);
        assert_eq!(sessions[0].cwd.as_deref(), Some("/home/user/myproject"));
        assert_eq!(sessions[0].model.as_deref(), Some("gemini-3-flash-preview"));
    }

    #[test]
    fn telemetry_tokens_matched_by_timestamp() {
        let td = TestDir::new("telemetry");
        let event = r#"{"name":"gemini_cli.api_response","timestamp":"2026-04-10T10:30:00Z","input_token_count":100,"output_token_count":50,"cached_content_token_count":20,"thoughts_token_count":10}"#;
        fs::write(td.path.join("telemetry.log"), event).unwrap();

        let start = parse_ts("2026-04-10T10:00:00Z").unwrap();
        let end = parse_ts("2026-04-10T11:00:00Z").unwrap();
        let analysis =
            extract_analysis_from_telemetry(&td.path, Some(start), Some(end), "sess-001");
        let tokens = analysis.tokens;

        assert_eq!(tokens.input, 100);
        assert_eq!(tokens.output, 50);
        assert_eq!(tokens.cache_read, 20);
        assert_eq!(tokens.cached_input, 20); // post-accumulation
        assert_eq!(tokens.reasoning_output, 10);
    }

    #[test]
    fn telemetry_outside_window_not_counted() {
        let td = TestDir::new("outside");
        let event = r#"{"name":"gemini_cli.api_response","timestamp":"2026-04-10T09:00:00Z","input_token_count":999}"#;
        fs::write(td.path.join("telemetry.log"), event).unwrap();

        let start = parse_ts("2026-04-10T10:00:00Z").unwrap();
        let end = parse_ts("2026-04-10T11:00:00Z").unwrap();
        let analysis =
            extract_analysis_from_telemetry(&td.path, Some(start), Some(end), "sess-001");
        let tokens = analysis.tokens;

        assert_eq!(tokens.input, 0);
    }

    #[test]
    fn analyze_current_session_json_surfaces_common_metrics() {
        let td = TestDir::new("analysis-json");
        let session_file = td.path.join("session.json");
        fs::write(
            &session_file,
            r#"{
  "sessionId": "sess-json",
  "startTime": "2026-04-10T10:00:00Z",
  "lastUpdated": "2026-04-10T10:01:00Z",
  "messages": [
    {"type": "user", "content": "Build it"},
    {
      "type": "gemini",
      "model": "gemini-2.5-pro",
      "toolCalls": [{"name": "read_file", "status": "success"}, {"name": "shell", "status": "success"}],
      "tokens": {
        "input": 1000,
        "output": 200,
        "cached": 300,
        "thoughts": 50
      }
    },
    {"type": "user", "content": "Continue"},
    {
      "type": "gemini",
      "model": "gemini-2.5-pro",
      "tokens": {
        "input": 500,
        "output": 100
      }
    }
  ]
}"#,
        )
        .unwrap();

        let summary = parse_gemini_session_json(
            &session_file,
            Some("/repo".to_string()),
            None,
            Some("Google · Gemini".to_string()),
        )
        .unwrap();
        let client = GeminiCliClient {
            gemini_dir: td.path.clone(),
            discover_cache: Mutex::default(),
        };

        let analysis = client.analyze(&summary, Plan::Retail).unwrap();

        assert_eq!(analysis.tokens.input, 1500);
        assert_eq!(analysis.tokens.output, 300);
        assert_eq!(analysis.tokens.cache_read, 300);
        assert_eq!(analysis.tokens.cached_input, 300);
        assert_eq!(analysis.tokens.reasoning_output, 50);
        assert_eq!(analysis.effective_model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(analysis.tool_call_count, Some(2));
        assert_eq!(analysis.agent_turns, Some(2));
        assert_eq!(analysis.user_turns, Some(2));
        assert_eq!(analysis.duration_secs, Some(60));
        assert_eq!(analysis.context_window, Some(1_048_576));
        assert_eq!(analysis.context_used_tokens, Some(1550));
        assert!(analysis.context_used_pct.is_some());
    }

    #[test]
    fn telemetry_attributes_shape_counts_tokens_tools_and_turns() {
        let td = TestDir::new("telemetry-attrs");
        let events = [
            r#"{"timestamp":"2026-04-10T10:30:00Z","attributes":{"event.name":"gemini_cli.api_response","model":"gemini-2.5-pro","input_token_count":100,"output_token_count":50,"cached_content_token_count":20,"thoughts_token_count":10,"tool_token_count":5,"total_token_count":185}}"#,
            r#"{"timestamp":"2026-04-10T10:30:01Z","attributes":{"event.name":"gemini_cli.tool_call","function_name":"read_file","success":true}}"#,
        ]
        .join("\n");
        fs::write(td.path.join("telemetry.log"), events).unwrap();

        let start = parse_ts("2026-04-10T10:00:00Z").unwrap();
        let end = parse_ts("2026-04-10T11:00:00Z").unwrap();
        let analysis =
            extract_analysis_from_telemetry(&td.path, Some(start), Some(end), "sess-001");

        assert_eq!(analysis.tokens.input, 100);
        assert_eq!(analysis.tokens.output, 50);
        assert_eq!(analysis.tokens.cache_read, 20);
        assert_eq!(analysis.tokens.cached_input, 20);
        assert_eq!(analysis.tokens.reasoning_output, 10);
        assert_eq!(analysis.effective_model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(analysis.tool_call_count, Some(1));
        assert_eq!(analysis.agent_turns, Some(1));
        assert_eq!(analysis.context_window, Some(1_048_576));
        assert_eq!(analysis.context_used_tokens, Some(185));
    }

    #[test]
    fn telemetry_session_id_takes_precedence_inside_time_window() {
        let td = TestDir::new("telemetry-session-id");
        let events = [
            r#"{"name":"gemini_cli.api_response","timestamp":"2026-04-10T10:30:00Z","session.id":"other-session","input_token_count":999}"#,
            r#"{"name":"gemini_cli.api_response","timestamp":"2026-04-10T10:30:01Z","session.id":"sess-001","input_token_count":100}"#,
            r#"{"name":"gemini_cli.api_response","timestamp":"2026-04-10T10:30:02Z","output_token_count":50}"#,
        ]
        .join("\n");
        fs::write(td.path.join("telemetry.log"), events).unwrap();

        let start = parse_ts("2026-04-10T10:00:00Z").unwrap();
        let end = parse_ts("2026-04-10T11:00:00Z").unwrap();
        let analysis =
            extract_analysis_from_telemetry(&td.path, Some(start), Some(end), "sess-001");

        assert_eq!(analysis.tokens.input, 100);
        assert_eq!(analysis.tokens.output, 50);
        assert_eq!(analysis.agent_turns, Some(2));
    }

    #[test]
    fn children_found_in_subagent_subdirectory() {
        let td = TestDir::new("subagents");
        let chats_dir = td.path.join("tmp").join("myproject").join("chats");
        let parent_session_id = "d535618c-bd13-4e0d-96bc-ff061b181c8c";
        let subagent_dir = chats_dir.join(parent_session_id);
        fs::create_dir_all(&subagent_dir).unwrap();

        fs::write(
            td.path.join("projects.json"),
            r#"{"projects":{"/home/user/myproject":"myproject"}}"#,
        )
        .unwrap();

        // Parent session file
        let parent_file = chats_dir.join(format!(
            "session-2026-04-24T02-38-{}.jsonl",
            &parent_session_id[..8]
        ));
        fs::write(
            &parent_file,
            format!(
                "{}\n",
                serde_json::json!({
                    "sessionId": parent_session_id,
                    "startTime": "2026-04-24T02:38:42Z",
                    "lastUpdated": "2026-04-24T02:39:00Z",
                    "kind": "main"
                })
            ),
        )
        .unwrap();

        // Child subagent session
        let child_session_id = "apwtx0";
        fs::write(
            subagent_dir.join(format!("{child_session_id}.jsonl")),
            format!(
                "{}\n",
                serde_json::json!({
                    "sessionId": child_session_id,
                    "startTime": "2026-04-24T02:39:13Z",
                    "lastUpdated": "2026-04-24T02:39:48Z",
                    "kind": "subagent"
                })
            ),
        )
        .unwrap();

        let client = GeminiCliClient {
            gemini_dir: td.path.clone(),
            discover_cache: Mutex::default(),
        };

        let sessions = client.list_sessions().unwrap();
        // Only the parent should appear at the top level (not the subagent in the subdirectory)
        assert_eq!(
            sessions.len(),
            1,
            "expected only parent at top level, got {:?}",
            sessions.iter().map(|s| &s.session_id).collect::<Vec<_>>()
        );
        assert_eq!(sessions[0].session_id, parent_session_id);

        let children = client.children(&sessions[0]).unwrap();
        assert_eq!(children.len(), 1, "expected one child subagent");
        assert_eq!(children[0].session_id, child_session_id);
    }

    /// `gemini --resume <uuid>` writes a NEW jsonl file each invocation but
    /// reuses the same `sessionId` inside. Without dedup, the parser emits
    /// two `SessionSummary` records sharing the same `session_id`, which
    /// violates the correlator's implicit `(client, session_id)` uniqueness
    /// invariant — only one entry can be PID-matched, the other looks
    /// orphaned in the UI.
    ///
    /// `list_sessions` MUST collapse duplicates to a single summary that
    /// preserves the union of timestamps:
    ///   * `started_at` = earliest across the duplicates (when the session
    ///     was first created);
    ///   * `last_active` = latest across the duplicates (most recent activity);
    ///   * `data_path` from the most-recently-active file (so any future
    ///     "open transcript" action lands on current data).
    #[test]
    fn duplicate_session_ids_collapse_to_one_summary() {
        let td = TestDir::new("dup-resume");
        let chats_dir = td.path.join("tmp").join("myproject").join("chats");
        fs::create_dir_all(&chats_dir).unwrap();

        fs::write(
            td.path.join("projects.json"),
            r#"{"projects":{"/home/user/myproject":"myproject"}}"#,
        )
        .unwrap();

        let session_id = "81b96307-1e14-40fa-9a24-184a072591fa";

        // Older invocation: started 09:00, last activity 09:30.
        let older = chats_dir.join("session-2026-04-25T09-00-81b96307.jsonl");
        fs::write(
            &older,
            format!(
                "{{\"sessionId\":\"{session_id}\",\"startTime\":\"2026-04-25T09:00:00Z\",\"lastUpdated\":\"2026-04-25T09:30:00Z\",\"kind\":\"main\"}}\n"
            ),
        )
        .unwrap();

        // Newer invocation: started 10:00, last activity 10:15.
        let newer = chats_dir.join("session-2026-04-25T10-00-81b96307.jsonl");
        fs::write(
            &newer,
            format!(
                "{{\"sessionId\":\"{session_id}\",\"startTime\":\"2026-04-25T10:00:00Z\",\"lastUpdated\":\"2026-04-25T10:15:00Z\",\"kind\":\"main\"}}\n"
            ),
        )
        .unwrap();

        let client = GeminiCliClient {
            gemini_dir: td.path.clone(),
            discover_cache: Mutex::default(),
        };

        let sessions = client.list_sessions().unwrap();
        let collapsed: Vec<_> = sessions
            .iter()
            .filter(|s| s.session_id == session_id)
            .collect();

        assert_eq!(
            collapsed.len(),
            1,
            "two on-disk files with the same sessionId must collapse to ONE summary, got {}: {:?}",
            collapsed.len(),
            collapsed
                .iter()
                .map(|s| s.data_path.display().to_string())
                .collect::<Vec<_>>()
        );

        let s = collapsed[0];
        assert_eq!(
            s.started_at,
            parse_ts("2026-04-25T09:00:00Z"),
            "started_at must be the EARLIEST across duplicates",
        );
        assert_eq!(
            s.last_active,
            parse_ts("2026-04-25T10:15:00Z"),
            "last_active must be the LATEST across duplicates",
        );
        assert_eq!(
            s.data_path, newer,
            "data_path must point at the most-recently-active file",
        );
    }
}
