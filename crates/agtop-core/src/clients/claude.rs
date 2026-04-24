//! Claude Code client — `~/.claude/projects/<slug>/<uuid>.jsonl`.
//!
//! Each line is a JSON record. For token accounting we care about:
//!  - records where `type == "assistant"` and `message.usage` is present
//!  - Claude streams the same request multiple times writing the same
//!    `requestId`; the last write wins for that turn (same policy as the
//!    original `extractClaudeSessionData`).
//!
//! Subagent sidechains (`<slug>/<uuid>/subagents/*.jsonl`) are exposed as
//! child sessions. Parent analysis only reflects direct transcript usage.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::client::Client;
use crate::clients::util::{dir_exists, for_each_jsonl, mtime, parse_ts, DiscoverCache};
use crate::error::{Error, Result};
use crate::pricing::{self, Plan, PlanMode};
use crate::session::{
    ClientKind, PlanUsage, PlanWindow, SessionAnalysis, SessionSummary, TokenTotals,
};

/// Upper bound on how many recent transcripts we scan for synthetic
/// `rate_limit` error events in `plan_usage()`. Users can accumulate
/// thousands of session files, and walking every one on each refresh is
/// wasteful: limit-hits are rare and, once logged, persist, so the newest
/// hit is overwhelmingly likely to live in a recently-modified file. We
/// sort by mtime descending and inspect at most this many files.
const PLAN_USAGE_RECENT_FILE_SCAN_LIMIT: usize = 50;

#[derive(Debug)]
pub struct ClaudeClient {
    pub projects_root: PathBuf,
    pub discover_cache: Mutex<DiscoverCache>,
}

impl Default for ClaudeClient {
    fn default() -> Self {
        // Honor $CLAUDE_CONFIG_DIR like the original.
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let base = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"));
        Self {
            projects_root: base.join("projects"),
            discover_cache: Mutex::default(),
        }
    }
}

impl Client for ClaudeClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Claude
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !dir_exists(&self.projects_root) {
            return Ok(vec![]);
        }
        let claude_dir = self
            .projects_root
            .parent()
            .unwrap_or(self.projects_root.as_path());
        let subscription = read_credentials_plan(claude_dir).1;
        let history_titles = read_history_titles(claude_dir);
        let mut out = Vec::new();
        let projects = match fs::read_dir(&self.projects_root) {
            Ok(r) => r,
            Err(_) => return Ok(out),
        };
        for entry in projects.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let proj_dir = entry.path();
            let files = match fs::read_dir(&proj_dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for f in files.flatten() {
                let p = f.path();
                if p.extension().map(|e| e != "jsonl").unwrap_or(true) {
                    continue;
                }
                let stem = match p.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s,
                    None => continue,
                };
                if !is_full_uuid(stem) {
                    continue;
                }
                let cached = {
                    let mut guard = self.discover_cache.lock().unwrap();
                    guard.get_or_insert_with(&p, || summarize_claude_file(&p))
                };
                match cached {
                    Ok(mut s) if s.model.is_some() => {
                        s.subscription = subscription.clone();
                        if s.session_title.is_none() {
                            s.session_title = history_titles.get(&s.session_id).cloned();
                        }
                        out.push(s)
                    }
                    Ok(_) => continue, // skip empty/abandoned sessions
                    Err(e) => {
                        tracing::debug!(path = %p.display(), error = %e, "skip claude file");
                        continue;
                    }
                }
            }
        }
        {
            use std::collections::HashSet;
            let live_paths: HashSet<&Path> = out.iter().map(|s| s.data_path.as_path()).collect();
            self.discover_cache
                .lock()
                .unwrap()
                .retain_paths(&live_paths);
        }
        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        analyze_claude_file(summary, plan)
    }

    fn children(&self, parent: &SessionSummary) -> Result<Vec<SessionSummary>> {
        let mut out = Vec::new();
        for path in list_subagent_files(&parent.data_path, &parent.session_id) {
            let cached = {
                let mut guard = self.discover_cache.lock().unwrap();
                guard.get_or_insert_with(&path, || summarize_claude_file(&path))
            };
            match cached {
                Ok(mut summary) if summary.model.is_some() => {
                    summary.subscription = parent.subscription.clone();
                    out.push(summary);
                }
                Ok(_) => continue,
                Err(e) => tracing::debug!(path = %path.display(), error = %e, "skip claude child"),
            }
        }
        Ok(out)
    }

    fn plan_usage(&self) -> Result<Vec<PlanUsage>> {
        plan_usage_for(&self.projects_root)
    }
}

/// Map a Claude `rateLimitTier` string to a human-readable plan name.
/// Returns `None` for unknown/empty tiers so the caller can fall back to
/// `subscriptionType`.
pub fn rate_limit_tier_to_plan_name(tier: &str) -> Option<String> {
    match tier {
        "default_claude_pro" => Some("Pro".to_string()),
        "default_claude_max_5x" => Some("Claude Max 5x".to_string()),
        "default_claude_max_20x" => Some("Claude Max 20x".to_string()),
        _ => None,
    }
}

/// Title-case a short ASCII identifier like `"max"` → `"Max"`. Used only
/// as a last-ditch fallback when `rateLimitTier` is unrecognised.
fn title_case_ascii(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Derive plan_name from a parsed `.credentials.json` value.
fn plan_name_from_credentials(v: &serde_json::Value) -> Option<String> {
    let oauth = v.get("claudeAiOauth")?;
    let tier = oauth.get("rateLimitTier").and_then(|x| x.as_str());
    if let Some(t) = tier {
        if let Some(name) = rate_limit_tier_to_plan_name(t) {
            return Some(name);
        }
    }
    oauth
        .get("subscriptionType")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(title_case_ascii)
}

/// Read `<claude_dir>/.credentials.json` and derive plan_name.
/// Returns `(credentials_exist, plan_name)`.
fn read_credentials_plan(claude_dir: &Path) -> (bool, Option<String>) {
    let path = claude_dir.join(".credentials.json");
    let Ok(bytes) = fs::read(&path) else {
        return (false, None);
    };
    let parsed: std::result::Result<serde_json::Value, _> = serde_json::from_slice(&bytes);
    match parsed {
        Ok(v) => (true, plan_name_from_credentials(&v)),
        Err(_) => (true, None),
    }
}

fn read_history_titles(claude_dir: &Path) -> HashMap<String, String> {
    let path = claude_dir.join("history.jsonl");
    let mut out = HashMap::new();
    let _ = for_each_jsonl(&path, |v| {
        let Some(session_id) = v.get("sessionId").and_then(|x| x.as_str()) else {
            return;
        };
        let Some(display) = v.get("display").and_then(|x| x.as_str()) else {
            return;
        };
        if !display.trim().is_empty() {
            out.insert(session_id.to_string(), display.trim().to_string());
        }
    });
    out
}

/// Extract a flat text body from a Claude assistant message record.
/// Returns `None` if no text parts are present.
fn extract_message_text(v: &serde_json::Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    let arr = content.as_array()?;
    let mut out = String::new();
    for part in arr {
        if part.get("type").and_then(|x| x.as_str()) == Some("text") {
            if let Some(t) = part.get("text").and_then(|x| x.as_str()) {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn state_from_claude_record(v: &serde_json::Value) -> Option<(String, String)> {
    match v
        .get("message")
        .and_then(|m| m.get("stop_reason"))
        .and_then(|x| x.as_str())
    {
        Some("tool_use") => Some((
            "waiting".to_string(),
            "assistant.stop_reason=tool_use".to_string(),
        )),
        Some("end_turn") => Some((
            "stopped".to_string(),
            "assistant.stop_reason=end_turn".to_string(),
        )),
        _ => None,
    }
}

/// Enumerate `<projects_root>/<slug>/<uuid>.jsonl` files and return them
/// sorted by mtime descending.
fn list_transcript_files_by_mtime_desc(projects_root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(PathBuf, DateTime<Utc>)> = Vec::new();
    let Ok(projects) = fs::read_dir(projects_root) else {
        return Vec::new();
    };
    for entry in projects.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let proj_dir = entry.path();
        let Ok(inner) = fs::read_dir(&proj_dir) else {
            continue;
        };
        for f in inner.flatten() {
            let p = f.path();
            if p.extension().map(|e| e != "jsonl").unwrap_or(true) {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_full_uuid(stem) {
                continue;
            }
            let ts = mtime(&p).unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
            files.push((p, ts));
        }
    }
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.into_iter().map(|(p, _)| p).collect()
}

/// Scan `projects_root` for the most-recent `rate_limit` error event and
/// return `(timestamp, optional_text_body)`. Bounded to the
/// `PLAN_USAGE_RECENT_FILE_SCAN_LIMIT` most-recently-modified transcripts.
fn find_latest_limit_hit(projects_root: &Path) -> Option<(DateTime<Utc>, Option<String>)> {
    let files = list_transcript_files_by_mtime_desc(projects_root);
    let mut latest: Option<(DateTime<Utc>, Option<String>)> = None;
    for path in files.iter().take(PLAN_USAGE_RECENT_FILE_SCAN_LIMIT) {
        let _ = for_each_jsonl(path, |v| {
            if v.get("error").and_then(|x| x.as_str()) != Some("rate_limit") {
                return;
            }
            let ts = match v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .and_then(parse_ts)
            {
                Some(t) => t,
                None => return,
            };
            let text = extract_message_text(v);
            match &latest {
                Some((cur_ts, _)) if *cur_ts >= ts => {}
                _ => latest = Some((ts, text)),
            }
        });
    }
    latest
}

/// Build the single `PlanUsage` entry (or none) for the Claude client.
/// Exposed as a free function for ease of testing with a mocked
/// `projects_root`.
fn plan_usage_for(projects_root: &Path) -> Result<Vec<PlanUsage>> {
    let claude_dir = match projects_root.parent() {
        Some(p) => p,
        // Root path (e.g. "/"): treat as absent.
        None => return Ok(vec![]),
    };

    let (creds_exist, plan_name) = read_credentials_plan(claude_dir);
    let limit_hit = find_latest_limit_hit(projects_root);

    if !creds_exist && limit_hit.is_none() {
        return Ok(vec![]);
    }

    let label = match &plan_name {
        Some(p) => format!("Claude Code · {}", p),
        None => "Claude Code".to_string(),
    };

    let mut windows: Vec<PlanWindow> = Vec::new();
    let last_limit_hit = limit_hit.as_ref().map(|(ts, _)| *ts);
    if let Some((_, hint)) = &limit_hit {
        windows.push(PlanWindow {
            label: "last limit-hit".to_string(),
            utilization: None,
            reset_at: None,
            reset_hint: hint.clone(),
            binding: true,
        });
    }

    let note = if plan_name.is_some() && last_limit_hit.is_none() {
        Some("no utilization data available from transcripts".to_string())
    } else {
        None
    };

    Ok(vec![PlanUsage {
        client: ClientKind::Claude,
        label,
        plan_name,
        windows,
        last_limit_hit,
        note,
    }])
}

fn is_full_uuid(s: &str) -> bool {
    // 8-4-4-4-12 hex
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected_lens = [8, 4, 4, 4, 12];
    for (p, &want) in parts.iter().zip(expected_lens.iter()) {
        if p.len() != want {
            return false;
        }
        if !p.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    true
}

fn summarize_claude_file(path: &Path) -> Result<SessionSummary> {
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut earliest: Option<DateTime<Utc>> = None;
    let mut model: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut state: Option<String> = None;
    let mut state_detail: Option<String> = None;
    let mut session_title: Option<String> = None;
    let mut seen = 0usize;

    for_each_jsonl(path, |v| {
        if seen > 30 {
            return;
        }
        seen += 1;
        if let Some(ts) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
        {
            earliest = Some(match earliest {
                Some(cur) if cur < ts => cur,
                _ => ts,
            });
        }
        if cwd.is_none() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                cwd = Some(c.to_string());
            }
        }
        if model.is_none() && v.get("type").and_then(|x| x.as_str()) == Some("assistant") {
            let m = v
                .get("message")
                .and_then(|m| m.get("model"))
                .and_then(|x| x.as_str());
            if let Some(m) = m {
                if m != "<synthetic>" {
                    model = Some(m.to_string());
                }
            }
        }
        if let Some((next_state, detail)) = state_from_claude_record(v) {
            state = Some(next_state);
            state_detail = Some(detail);
        }
        // Claude Code writes an "ai-title" record with an AI-generated session summary.
        if session_title.is_none() && v.get("type").and_then(|x| x.as_str()) == Some("ai-title") {
            if let Some(t) = v.get("aiTitle").and_then(|x| x.as_str()) {
                session_title = Some(t.to_string());
            }
        }
    })?;

    let last_active = mtime(path).or(earliest);

    Ok(SessionSummary {
        client: ClientKind::Claude,
        subscription: None,
        session_id,
        started_at: earliest,
        last_active,
        model,
        cwd,
        state,
        state_detail,
        model_effort: None,
        model_effort_detail: None,
        session_title,
        data_path: path.to_path_buf(),
    })
}

fn analyze_claude_file(summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
    let path = &summary.data_path;
    let mut effective_model = summary.model.clone();
    let mut totals = TokenTotals::default();
    let mut tool_call_count: u64 = 0;
    let mut agent_turns: u64 = 0;
    let mut context_used_pct: Option<f64> = None;
    let mut context_used_tokens: Option<u64> = None;
    let mut context_window: Option<u64> = None;

    // Helper to merge a FileTotals' context peak into our running max.
    let mut merge_context = |ft: &FileTotals| {
        if let (Some(pct), Some(toks), Some(win)) = (
            ft.context_used_pct,
            ft.context_used_tokens,
            ft.context_window,
        ) {
            if context_used_pct.is_none_or(|cur| pct > cur) {
                context_used_pct = Some(pct);
                context_used_tokens = Some(toks);
                context_window = Some(win);
            }
        } else {
            context_used_pct = max_pct(context_used_pct, ft.context_used_pct);
        }
    };

    // Main transcript.
    let main_file_totals = sum_jsonl_usage(path, &mut effective_model)?;
    add_file_totals(&mut totals, &main_file_totals);
    tool_call_count += main_file_totals.tool_call_count;
    add_agent_turns(&mut agent_turns, &main_file_totals);
    merge_context(&main_file_totals);

    // Claude's "cached_input" bucket for our cost math is cache_read.
    totals.cached_input = totals.cache_read;

    if totals.grand_total() == 0 {
        return Err(Error::NoUsage(summary.session_id.clone()));
    }

    let model = effective_model
        .clone()
        .ok_or_else(|| Error::NoUsage(summary.session_id.clone()))?;
    let rates =
        pricing::lookup(ClientKind::Claude, &model).ok_or_else(|| Error::UnknownPricing {
            client: "claude".into(),
            model: model.clone(),
        })?;
    let included = matches!(plan.mode_for(ClientKind::Claude), PlanMode::Included);
    let cost = pricing::compute_cost(&totals, &rates, included);

    Ok(SessionAnalysis {
        summary: summary.clone(),
        tokens: totals,
        cost,
        effective_model,
        subagent_file_count: 0,
        tool_call_count: Some(tool_call_count),
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
        context_used_pct,
        context_used_tokens,
        context_window,
        children: Vec::new(),
        agent_turns: if agent_turns > 0 {
            Some(agent_turns)
        } else {
            None
        },
        user_turns: None,
        project_name: None,
        pid: None,
        liveness: None,
        match_confidence: None,
    })
}

fn max_pct(cur: Option<f64>, next: Option<f64>) -> Option<f64> {
    match (cur, next) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Scan a Claude-format JSONL (main transcript OR subagent sidechain)
/// and return its per-file request-deduplicated usage totals. Updates
/// `effective_model` as a side-effect when the file observes a model.
///
/// Subagent files are structurally identical to main transcripts at the
/// record level: every assistant message carries its own requestId, so
/// the same "last snapshot wins" dedup strategy is correct. Keyed
/// snapshots are file-local (NOT shared across files) because we must
/// not let a subagent's requestId accidentally collide with the parent's.
fn sum_jsonl_usage(path: &Path, effective_model: &mut Option<String>) -> Result<FileTotals> {
    // Per-request-id snapshot: streaming rewrites the same requestId as it
    // progresses; only the final write has correct totals. Keep the last.
    let mut last_snapshot: HashMap<String, Snapshot> = HashMap::new();
    let mut keyless = Snapshot::default();
    let mut keyless_turns: u64 = 0; // keyless assistant records each count as one turn
    let mut tool_call_count: u64 = 0;
    let mut context_used_pct: Option<f64> = None;
    let mut context_used_tokens: Option<u64> = None;
    let mut context_window_size: Option<u64> = None;

    for_each_jsonl(path, |v| {
        if v.get("type").and_then(|x| x.as_str()) != Some("assistant") {
            return;
        }
        let message = match v.get("message") {
            Some(m) => m,
            None => return,
        };
        let usage = match message.get("usage") {
            Some(u) if !u.is_null() => u,
            _ => return,
        };

        if let Some(arr) = message.get("content").and_then(|x| x.as_array()) {
            for part in arr {
                if part.get("type").and_then(|x| x.as_str()) == Some("tool_use") {
                    tool_call_count += 1;
                }
            }
        }

        let m = message.get("model").and_then(|x| x.as_str());
        if let Some(m) = m {
            if m != "<synthetic>" && effective_model.is_none() {
                *effective_model = Some(m.to_string());
            }

            if let Some(window) = pricing::context_window(ClientKind::Claude, m).filter(|w| *w > 0)
            {
                let g = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
                let cache_creation = usage
                    .get("cache_creation")
                    .and_then(|cc| cc.get("ephemeral_5m_input_tokens"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0)
                    + usage
                        .get("cache_creation")
                        .and_then(|cc| cc.get("ephemeral_1h_input_tokens"))
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0)
                    + usage
                        .get("cache_creation_input_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                let total = g("input_tokens")
                    + g("cache_read_input_tokens")
                    + cache_creation
                    + g("output_tokens");
                let pct = (total as f64 / window as f64) * 100.0;
                // Track raw tokens and window size at the peak-utilization turn.
                let is_new_peak = context_used_pct.is_none_or(|cur| pct > cur);
                context_used_pct = max_pct(context_used_pct, Some(pct));
                if is_new_peak {
                    context_used_tokens = Some(total);
                    context_window_size = Some(window);
                }
            }
        }
        let snap = snapshot_from_usage(usage);
        let key = v
            .get("requestId")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .or_else(|| {
                message
                    .get("id")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            });
        match key {
            Some(k) => {
                last_snapshot.insert(k, snap);
            }
            None => {
                keyless.add(&snap);
                keyless_turns += 1;
            }
        }
    })?;

    let mut ft = FileTotals::default();
    for snap in last_snapshot.values() {
        ft.input += snap.input;
        ft.output += snap.output;
        ft.cache_read += snap.cache_read;
        ft.cache_write_5m += snap.cache_write_5m;
        ft.cache_write_1h += snap.cache_write_1h;
    }
    ft.input += keyless.input;
    ft.output += keyless.output;
    ft.cache_read += keyless.cache_read;
    ft.cache_write_5m += keyless.cache_write_5m;
    ft.cache_write_1h += keyless.cache_write_1h;
    ft.tool_call_count = tool_call_count;
    // Each unique requestId is one agent turn; keyless assistant records each count as one.
    ft.agent_turns = last_snapshot.len() as u64 + keyless_turns;
    ft.context_used_pct = context_used_pct;
    ft.context_used_tokens = context_used_tokens;
    ft.context_window = context_window_size;
    Ok(ft)
}

fn add_file_totals(totals: &mut TokenTotals, ft: &FileTotals) {
    totals.input += ft.input;
    totals.output += ft.output;
    totals.cache_read += ft.cache_read;
    totals.cache_write_5m += ft.cache_write_5m;
    totals.cache_write_1h += ft.cache_write_1h;
}

fn add_agent_turns(agent_turns: &mut u64, ft: &FileTotals) {
    *agent_turns += ft.agent_turns;
}

/// Return sorted list of subagent JSONL files for a main transcript.
/// The layout is `<main_transcript_dir>/<session_id>/subagents/*.jsonl`.
/// Missing directory → empty vec. Never panics on I/O errors.
fn list_subagent_files(main_path: &Path, session_id: &str) -> Vec<PathBuf> {
    let Some(parent) = main_path.parent() else {
        return vec![];
    };
    let sub_dir = parent.join(session_id).join("subagents");
    let mut out = Vec::new();
    let entries = match fs::read_dir(&sub_dir) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().map(|e| e == "jsonl").unwrap_or(false) {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Per-file accumulator. Kept separate from `TokenTotals` because
/// `TokenTotals` also carries `cached_input` / `reasoning_output` which
/// are derived at the session level, not per-file.
#[derive(Debug, Default, Clone)]
struct FileTotals {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    tool_call_count: u64,
    /// Number of distinct agent turns (unique requestIds).
    agent_turns: u64,
    context_used_pct: Option<f64>,
    /// Raw token count at the peak-utilization turn.
    context_used_tokens: Option<u64>,
    /// Context window size (denominator) at the peak-utilization turn.
    context_window: Option<u64>,
}

#[derive(Debug, Default, Clone)]
struct Snapshot {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
}

impl Snapshot {
    fn add(&mut self, other: &Snapshot) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
    }
}

fn snapshot_from_usage(usage: &serde_json::Value) -> Snapshot {
    let g = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let cache_creation = usage.get("cache_creation");
    let (c5, c1h) = match cache_creation {
        Some(cc) if cc.is_object() => {
            let c5 = cc
                .get("ephemeral_5m_input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let c1h = cc
                .get("ephemeral_1h_input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            (c5, c1h)
        }
        _ => (0, 0),
    };
    // Older transcripts report a flat `cache_creation_input_tokens` — attribute
    // it to 5m by convention (same as the original).
    let flat_cc = g("cache_creation_input_tokens");
    let c5 = c5 + if cache_creation.is_some() { 0 } else { flat_cc };

    Snapshot {
        input: g("input_tokens"),
        output: g("output_tokens"),
        cache_read: g("cache_read_input_tokens"),
        cache_write_5m: c5,
        cache_write_1h: c1h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_filename_detection() {
        assert!(is_full_uuid("02742fb3-d98e-4fa2-8184-2fddd7ee544d"));
        assert!(!is_full_uuid("not-a-uuid"));
        assert!(!is_full_uuid("02742fb3d98e4fa281842fddd7ee544d"));
    }

    #[test]
    fn snapshot_reads_nested_cache_creation() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3,
                "cache_creation":{"ephemeral_5m_input_tokens":7,"ephemeral_1h_input_tokens":1}}"#,
        )
        .unwrap();
        let s = snapshot_from_usage(&v);
        assert_eq!(s.input, 10);
        assert_eq!(s.output, 5);
        assert_eq!(s.cache_read, 3);
        assert_eq!(s.cache_write_5m, 7);
        assert_eq!(s.cache_write_1h, 1);
    }

    #[test]
    fn snapshot_falls_back_to_flat_cache_creation() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"input_tokens":1,"cache_creation_input_tokens":9}"#).unwrap();
        let s = snapshot_from_usage(&v);
        assert_eq!(s.cache_write_5m, 9);
    }

    #[test]
    fn assistant_stop_reason_tool_use_maps_to_waiting() {
        let v = serde_json::json!({
            "message": { "stop_reason": "tool_use" }
        });
        assert_eq!(
            state_from_claude_record(&v),
            Some((
                "waiting".to_string(),
                "assistant.stop_reason=tool_use".to_string(),
            ))
        );
    }

    // ---------------- plan_usage tests ----------------

    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Minimal tempdir substitute: creates a unique directory under
    /// `std::env::temp_dir()` and cleans it up on drop. We avoid pulling
    /// in the `tempfile` crate as a dev-dep (see Cargo.toml constraints).
    struct TestDir {
        path: PathBuf,
    }
    impl TestDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("agtop-claude-test-{pid}-{ts}-{n}"));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Build a temp `~/.claude/` dir for a test. Returns (guard, projects_root).
    fn make_fake_claude_home() -> (TestDir, PathBuf) {
        let td = TestDir::new();
        let claude_dir = td.path().join(".claude");
        let projects_root = claude_dir.join("projects");
        fs::create_dir_all(&projects_root).unwrap();
        (td, projects_root)
    }

    fn write_credentials(claude_dir: &Path, tier: &str, subscription: &str) {
        let body = format!(
            r#"{{"claudeAiOauth":{{"subscriptionType":"{}","rateLimitTier":"{}","accessToken":"x","refreshToken":"y","expiresAt":1774299600000}}}}"#,
            subscription, tier
        );
        let mut f = fs::File::create(claude_dir.join(".credentials.json")).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    fn write_jsonl(path: &Path, lines: &[&str]) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        for l in lines {
            f.write_all(l.as_bytes()).unwrap();
            f.write_all(b"\n").unwrap();
        }
    }

    #[test]
    fn plan_name_mapping_covers_known_tiers() {
        assert_eq!(
            rate_limit_tier_to_plan_name("default_claude_pro"),
            Some("Pro".to_string())
        );
        assert_eq!(
            rate_limit_tier_to_plan_name("default_claude_max_5x"),
            Some("Claude Max 5x".to_string())
        );
        assert_eq!(
            rate_limit_tier_to_plan_name("default_claude_max_20x"),
            Some("Claude Max 20x".to_string())
        );
        assert_eq!(rate_limit_tier_to_plan_name("something_else"), None);
        assert_eq!(rate_limit_tier_to_plan_name(""), None);
    }

    #[test]
    fn plan_usage_with_credentials_and_recent_limit_hit() {
        let (_td, projects_root) = make_fake_claude_home();
        let claude_dir = projects_root.parent().unwrap();
        write_credentials(claude_dir, "default_claude_max_5x", "max");

        let transcript = projects_root
            .join("proj_x")
            .join("02742fb3-d98e-4fa2-8184-2fddd7ee544d.jsonl");
        let expected_ts_str = "2026-03-12T15:30:00.000Z";
        let normal = r#"{"type":"assistant","message":{"model":"claude-sonnet-4","content":[{"type":"text","text":"hi"}]},"timestamp":"2026-03-12T15:00:00.000Z"}"#;
        let limit = r#"{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"You've hit your limit · resets 3pm (America/Buenos_Aires)"}]},"error":"rate_limit","timestamp":"2026-03-12T15:30:00.000Z"}"#;
        write_jsonl(&transcript, &[normal, limit]);

        let out = plan_usage_for(&projects_root).unwrap();
        assert_eq!(out.len(), 1);
        let pu = &out[0];
        assert_eq!(pu.client, ClientKind::Claude);
        assert_eq!(pu.plan_name.as_deref(), Some("Claude Max 5x"));
        assert_eq!(pu.label, "Claude Code · Claude Max 5x");
        let expected_ts = parse_ts(expected_ts_str).unwrap();
        assert_eq!(pu.last_limit_hit, Some(expected_ts));
        assert_eq!(pu.windows.len(), 1);
        let w = &pu.windows[0];
        assert_eq!(w.label, "last limit-hit");
        assert!(w.utilization.is_none());
        assert!(w.reset_at.is_none());
        assert!(w.binding);
        let hint = w.reset_hint.as_deref().unwrap();
        assert!(hint.contains("resets 3pm"), "hint was {:?}", hint);
        // note should be None because we DO have a last_limit_hit.
        assert!(pu.note.is_none());
    }

    #[test]
    fn plan_usage_credentials_only_no_limit_hit() {
        let (_td, projects_root) = make_fake_claude_home();
        let claude_dir = projects_root.parent().unwrap();
        write_credentials(claude_dir, "default_claude_pro", "pro");

        // A transcript with no rate_limit errors.
        let transcript = projects_root
            .join("proj_y")
            .join("12742fb3-d98e-4fa2-8184-2fddd7ee544d.jsonl");
        let normal = r#"{"type":"assistant","message":{"model":"claude-sonnet-4","content":[{"type":"text","text":"hello"}]},"timestamp":"2026-03-12T15:00:00.000Z"}"#;
        write_jsonl(&transcript, &[normal]);

        let out = plan_usage_for(&projects_root).unwrap();
        assert_eq!(out.len(), 1);
        let pu = &out[0];
        assert_eq!(pu.plan_name.as_deref(), Some("Pro"));
        assert!(pu.windows.is_empty());
        assert!(pu.last_limit_hit.is_none());
        assert_eq!(
            pu.note.as_deref(),
            Some("no utilization data available from transcripts")
        );
    }

    #[test]
    fn plan_usage_no_credentials_no_transcripts() {
        let (_td, projects_root) = make_fake_claude_home();
        // No .credentials.json, no transcripts written.
        let out = plan_usage_for(&projects_root).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn plan_usage_unknown_tier_falls_back_to_subscription_type() {
        let (_td, projects_root) = make_fake_claude_home();
        let claude_dir = projects_root.parent().unwrap();
        write_credentials(claude_dir, "brand_new_tier", "max");

        let out = plan_usage_for(&projects_root).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].plan_name.as_deref(), Some("Max"));
    }

    #[test]
    fn second_list_sessions_uses_cache() {
        let td = TestDir::new();
        let projects = td.path().join("projects");
        fs::create_dir_all(projects.join("proj")).unwrap();
        // Write a minimal JSONL with a uuid stem
        let jsonl = projects
            .join("proj")
            .join("deadbeef-aaaa-bbbb-cccc-012345678901.jsonl");
        write_jsonl(
            &jsonl,
            &[
                r#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"model":"claude-opus-4","content":[]}}"#,
            ],
        );

        let client = ClaudeClient {
            projects_root: projects.clone(),
            discover_cache: std::sync::Mutex::default(),
        };

        // First call
        let r1 = client.list_sessions();
        // Second call - should use cache and return same results
        let r2 = client.list_sessions();

        match (r1, r2) {
            (Ok(s1), Ok(s2)) => {
                assert_eq!(s1.len(), s2.len(), "session count changed between calls");
            }
            _ => panic!("list_sessions failed"),
        }
    }

    #[test]
    fn children_returns_empty_when_subagent_dir_is_missing() {
        let td = TestDir::new();
        let projects = td.path().join("projects");
        let client = ClaudeClient {
            projects_root: projects,
            discover_cache: std::sync::Mutex::default(),
        };
        let parent_path = td
            .path()
            .join("proj")
            .join("02742fb3-d98e-4fa2-8184-2fddd7ee544d.jsonl");
        write_jsonl(
            &parent_path,
            &[
                r#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"model":"claude-sonnet-4","content":[]}}"#,
            ],
        );
        let parent = SessionSummary::new(
            ClientKind::Claude,
            Some("Claude Max 5x".to_string()),
            "02742fb3-d98e-4fa2-8184-2fddd7ee544d".to_string(),
            None,
            None,
            Some("claude-sonnet-4".to_string()),
            None,
            parent_path,
            None,
            None,
            None,
            None,
        );

        let children = crate::client::Client::children(&client, &parent).unwrap();

        assert!(children.is_empty());
    }

    #[test]
    fn children_returns_subagent_summary_and_inherits_subscription() {
        let td = TestDir::new();
        let projects = td.path().join("projects");
        let client = ClaudeClient {
            projects_root: projects,
            discover_cache: std::sync::Mutex::default(),
        };
        let session_id = "02742fb3-d98e-4fa2-8184-2fddd7ee544d";
        let parent_path = td.path().join("proj").join(format!("{session_id}.jsonl"));
        write_jsonl(
            &parent_path,
            &[
                r#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"model":"claude-sonnet-4","content":[]}}"#,
            ],
        );
        let child_path = td
            .path()
            .join("proj")
            .join(session_id)
            .join("subagents")
            .join("subagent-child.jsonl");
        write_jsonl(
            &child_path,
            &[
                r#"{"type":"assistant","timestamp":"2026-01-01T00:01:00Z","cwd":"/tmp/subagent","message":{"model":"claude-3-5-haiku-20241022","content":[],"stop_reason":"end_turn"}}"#,
            ],
        );
        let parent = SessionSummary::new(
            ClientKind::Claude,
            Some("Claude Max 5x".to_string()),
            session_id.to_string(),
            None,
            None,
            Some("claude-sonnet-4".to_string()),
            None,
            parent_path,
            None,
            None,
            None,
            None,
        );

        let children = crate::client::Client::children(&client, &parent).unwrap();

        assert_eq!(children.len(), 1);
        let child = &children[0];
        assert_eq!(child.subscription.as_deref(), Some("Claude Max 5x"));
        assert_eq!(child.session_id, "subagent-child");
        assert_eq!(child.model.as_deref(), Some("claude-3-5-haiku-20241022"));
        assert_eq!(child.cwd.as_deref(), Some("/tmp/subagent"));
        assert_eq!(child.state.as_deref(), Some("stopped"));
        assert_eq!(
            child.state_detail.as_deref(),
            Some("assistant.stop_reason=end_turn")
        );
        assert_eq!(child.data_path, child_path);
    }
}
