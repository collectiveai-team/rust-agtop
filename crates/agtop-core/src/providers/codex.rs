//! Codex provider — `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
//!
//! Relevant record types inside each file:
//! - `session_meta` (line 1): carries session id, timestamp, cwd.
//! - `turn_context`: carries `payload.model`.
//! - `event_msg` with `payload.type == "token_count"`: carries
//!   `payload.info.last_token_usage` — a per-turn delta. We sum every
//!   delta across the file to reconstruct the session total.
//!
//! This matches the accounting logic in the original `extractCodexSessionData`
//! in index.js (the `total_token_usage` field is a running total provided by
//! the server, but summing `last_token_usage` is equivalent and robust to
//! missing/partial entries).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, TimeZone, Utc};
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::pricing::{self, Plan, PlanMode};
use crate::provider::Provider;
use crate::providers::util::{dir_exists, for_each_jsonl, mtime, parse_ts, DiscoverCache};
use crate::session::{
    PlanUsage, PlanWindow, ProviderKind, SessionAnalysis, SessionSummary, TokenTotals,
};

/// Number of most-recently-modified rollout files to scan when looking
/// for the latest `rate_limits` snapshot. Backends only emit this field
/// intermittently; 5 is a pragmatic bound that keeps the scan cheap while
/// still catching recent activity across multiple machines/days.
const RATE_LIMIT_SCAN_MAX_FILES: usize = 5;

#[derive(Debug)]
pub struct CodexProvider {
    pub sessions_root: PathBuf,
    /// Path to `auth.json`. Separated from `sessions_root` so tests (and
    /// unusual Codex layouts) can point at an arbitrary file.
    pub auth_path: PathBuf,
    pub discover_cache: Mutex<DiscoverCache>,
}

impl Default for CodexProvider {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let codex_dir = home.join(".codex");
        Self {
            sessions_root: codex_dir.join("sessions"),
            auth_path: codex_dir.join("auth.json"),
            discover_cache: Mutex::default(),
        }
    }
}

impl Provider for CodexProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Codex
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !dir_exists(&self.sessions_root) {
            return Ok(vec![]);
        }
        let subscription = read_plan_name(&self.auth_path);
        let mut out = Vec::new();
        for entry in WalkDir::new(&self.sessions_root)
            .into_iter()
            .filter_map(|r| r.ok())
        {
            let p = entry.path().to_path_buf();
            if !entry.file_type().is_file() {
                continue;
            }
            if p.extension().map(|e| e != "jsonl").unwrap_or(true) {
                continue;
            }
            let cached = {
                let mut guard = self.discover_cache.lock().unwrap();
                guard.get_or_insert_with(&p, || summarize_codex_file(&p))
            };
            match cached {
                Ok(mut s) => {
                    s.subscription = subscription.clone();
                    out.push(s)
                }
                Err(e) => {
                    tracing::debug!(path = %p.display(), error = %e, "skip codex file");
                    continue;
                }
            }
        }
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

    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        analyze_codex_file(summary, plan)
    }

    fn plan_usage(&self) -> Result<Vec<PlanUsage>> {
        Ok(collect_plan_usage(&self.auth_path, &self.sessions_root))
    }
}

// ---------------------------------------------------------------------------
// Plan usage
// ---------------------------------------------------------------------------

/// Result of scanning the most-recent rollout files for a `rate_limits`
/// payload. `rollout_seen` is true if *any* rollout files existed (even
/// if none contained a populated rate_limits), so callers can distinguish
/// "no data at all" from "data present but backend hasn't populated".
struct RateLimitScan {
    rollout_seen: bool,
    windows: Vec<PlanWindow>,
}

fn collect_plan_usage(auth_path: &Path, sessions_root: &Path) -> Vec<PlanUsage> {
    // Run both sources independently so one failing doesn't hide the other.
    let plan_name = read_plan_name(auth_path);
    let scan = scan_recent_rate_limits(sessions_root);

    // If neither source produced anything, return an empty vec.
    if plan_name.is_none() && !scan.rollout_seen {
        return Vec::new();
    }

    let label = match &plan_name {
        Some(n) => format!("Codex · {n}"),
        None => "Codex".to_string(),
    };

    let note = if scan.windows.is_empty() && plan_name.is_some() {
        Some("waiting for backend to populate rate_limits".to_string())
    } else {
        None
    };

    vec![PlanUsage {
        provider: ProviderKind::Codex,
        label,
        plan_name,
        windows: scan.windows,
        last_limit_hit: None,
        note,
    }]
}

/// Read `~/.codex/auth.json`, pull `tokens.id_token`, and decode the JWT
/// payload's `https://api.openai.com/auth.chatgpt_plan_type` into a
/// display-ready plan name. Returns `None` on any error — callers treat
/// this as "plan unknown", not fatal.
fn read_plan_name(auth_path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(auth_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let id_token = v
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(|x| x.as_str())?;
    let payload_b64 = id_token.split('.').nth(1)?;
    let bytes = base64url_decode(payload_b64)?;
    let payload: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let claim = payload.get("https://api.openai.com/auth")?;
    let plan_type = claim.get("chatgpt_plan_type").and_then(|x| x.as_str())?;
    Some(map_plan_type(plan_type))
}

fn map_plan_type(raw: &str) -> String {
    match raw {
        "plus" => "ChatGPT Plus".to_string(),
        "pro" => "ChatGPT Pro".to_string(),
        "business" => "ChatGPT Business".to_string(),
        "enterprise" => "ChatGPT Enterprise".to_string(),
        other => title_case(other),
    }
}

fn title_case(s: &str) -> String {
    // Simple Unicode-aware title-case of the first character.
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn effort_from_turn_context(payload: &serde_json::Value) -> Option<(String, String)> {
    payload
        .get("effort")
        .and_then(|x| x.as_str())
        .map(|effort| (effort.to_string(), "turn_context.effort".to_string()))
        .or_else(|| {
            payload
                .get("collaboration_mode")
                .and_then(|x| x.get("settings"))
                .and_then(|x| x.get("reasoning_effort"))
                .and_then(|x| x.as_str())
                .map(|effort| {
                    (
                        effort.to_string(),
                        "turn_context.collaboration_mode.settings.reasoning_effort".to_string(),
                    )
                })
        })
}

fn state_from_response_item(payload: &serde_json::Value) -> Option<(String, String)> {
    match payload.get("type").and_then(|x| x.as_str()) {
        Some("function_call") => Some((
            "waiting".to_string(),
            "response_item:function_call".to_string(),
        )),
        _ => None,
    }
}

/// Scan the newest rollout jsonl files for the latest `rate_limits`
/// snapshot. Returns whichever windows we find (`primary` / `secondary`)
/// and whether any rollout files were present.
fn scan_recent_rate_limits(sessions_root: &Path) -> RateLimitScan {
    if !dir_exists(sessions_root) {
        return RateLimitScan {
            rollout_seen: false,
            windows: Vec::new(),
        };
    }

    // Collect (mtime, path) for every rollout jsonl under sessions_root.
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in WalkDir::new(sessions_root)
        .into_iter()
        .filter_map(|r| r.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().map(|e| e != "jsonl").unwrap_or(true) {
            continue;
        }
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !fname.starts_with("rollout-") {
            continue;
        }
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        files.push((modified, path.to_path_buf()));
    }

    if files.is_empty() {
        return RateLimitScan {
            rollout_seen: false,
            windows: Vec::new(),
        };
    }

    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.truncate(RATE_LIMIT_SCAN_MAX_FILES);

    let mut windows = Vec::new();
    for (_, path) in &files {
        if let Some(w) = extract_latest_rate_limits(path) {
            windows = w;
            break;
        }
    }

    RateLimitScan {
        rollout_seen: true,
        windows,
    }
}

/// Walk a single rollout file and return the `primary`/`secondary`
/// windows from the *last* `event_msg` / `token_count` record that has a
/// non-null `rate_limits`. Returns `None` when no such record exists.
fn extract_latest_rate_limits(path: &Path) -> Option<Vec<PlanWindow>> {
    let mut latest: Option<Vec<PlanWindow>> = None;
    let _ = for_each_jsonl(path, |v| {
        if v.get("type").and_then(|x| x.as_str()) != Some("event_msg") {
            return;
        }
        let Some(payload) = v.get("payload") else {
            return;
        };
        if payload.get("type").and_then(|x| x.as_str()) != Some("token_count") {
            return;
        }
        let Some(rl) = payload.get("rate_limits") else {
            return;
        };
        if rl.is_null() {
            return;
        }
        let mut windows = Vec::new();
        if let Some(w) = window_from_json("primary", rl.get("primary")) {
            windows.push(w);
        }
        if let Some(w) = window_from_json("secondary", rl.get("secondary")) {
            windows.push(w);
        }
        if !windows.is_empty() {
            latest = Some(windows);
        }
    });
    latest
}

fn window_from_json(label: &str, v: Option<&serde_json::Value>) -> Option<PlanWindow> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    let utilization = v
        .get("used_percent")
        .and_then(|x| x.as_f64())
        .map(|p| p / 100.0);
    let reset_at = v
        .get("resets_at")
        .and_then(|x| x.as_i64())
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single());
    Some(PlanWindow {
        label: label.to_string(),
        utilization,
        reset_at,
        reset_hint: None,
        binding: false,
    })
}

/// Decode a base64url-encoded string (RFC 4648 §5) — standard alphabet
/// with `-`/`_` and optional padding. Returns `None` on any character or
/// length error.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    // Normalise alphabet and re-pad to a multiple of 4.
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

    // Standard base64 alphabet lookup.
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
        // Count trailing '=' padding.
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

fn summarize_codex_file(path: &Path) -> Result<SessionSummary> {
    let mut session_id: Option<String> = None;
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut model: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model_effort: Option<String> = None;
    let mut model_effort_detail: Option<String> = None;
    let mut state: Option<String> = None;
    let mut state_detail: Option<String> = None;
    let mut seen = 0usize;

    // We only need the first ~50 records for metadata.
    for_each_jsonl(path, |v| {
        if seen > 50 {
            return;
        }
        seen += 1;
        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let payload = v.get("payload");
        match ty {
            "session_meta" => {
                if let Some(p) = payload {
                    if session_id.is_none() {
                        session_id = p.get("id").and_then(|x| x.as_str()).map(str::to_string);
                    }
                    if started_at.is_none() {
                        started_at = p
                            .get("timestamp")
                            .and_then(|x| x.as_str())
                            .and_then(parse_ts)
                            .or_else(|| {
                                v.get("timestamp")
                                    .and_then(|x| x.as_str())
                                    .and_then(parse_ts)
                            });
                    }
                    if cwd.is_none() {
                        cwd = p.get("cwd").and_then(|x| x.as_str()).map(str::to_string);
                    }
                }
            }
            "turn_context" => {
                if let Some(p) = payload {
                    if model.is_none() {
                        model = p.get("model").and_then(|x| x.as_str()).map(str::to_string);
                    }
                    if let Some((effort, detail)) = effort_from_turn_context(p) {
                        model_effort = Some(effort);
                        model_effort_detail = Some(detail);
                    }
                }
            }
            "response_item" => {
                if let Some(p) = payload {
                    if let Some((next_state, detail)) = state_from_response_item(p) {
                        state = Some(next_state);
                        state_detail = Some(detail);
                    }
                }
            }
            _ => {}
        }
    })?;

    // Fallback: extract UUID from filename if session_meta missing.
    if session_id.is_none() {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            // rollout-YYYY-...-<uuid>
            if let Some(uuid) = stem.rsplit('-').next() {
                if uuid.len() == 12 {
                    // last piece of a UUID
                    session_id = Some(stem.to_string());
                }
            }
        }
    }

    let last_active = mtime(path).or(started_at);

    Ok(SessionSummary {
        provider: ProviderKind::Codex,
        subscription: None,
        session_id: session_id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        }),
        started_at,
        last_active,
        model,
        cwd,
        state,
        state_detail,
        model_effort,
        model_effort_detail,
        data_path: path.to_path_buf(),
    })
}

fn analyze_codex_file(summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
    let path = &summary.data_path;
    let mut totals = TokenTotals::default();
    let mut saw_usage = false;
    let mut effective_model = summary.model.clone();
    let mut tool_call_count: u64 = 0;
    let mut agent_turns: u64 = 0;
    let mut user_turns: u64 = 0;
    let mut first_ts = summary.started_at;
    let mut last_ts = summary.last_active;
    let mut context_used_pct: Option<f64> = None;
    let mut context_used_tokens: Option<u64> = None;
    let mut context_window: Option<u64> = None;

    for_each_jsonl(path, |v| {
        if let Some(ts) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_ts)
        {
            first_ts = Some(match first_ts {
                Some(cur) if cur <= ts => cur,
                _ => ts,
            });
            last_ts = Some(match last_ts {
                Some(cur) if cur >= ts => cur,
                _ => ts,
            });
        }

        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let payload = v.get("payload");

        if ty == "response_item" {
            if let Some(kind) = payload.and_then(|p| p.get("type")).and_then(|x| x.as_str()) {
                if matches!(kind, "function_call" | "custom_tool_call") {
                    tool_call_count += 1;
                }
            }
        }

        if ty == "turn_context" {
            user_turns += 1;
            if let Some(p) = payload {
                if let Some(m) = p.get("model").and_then(|x| x.as_str()) {
                    effective_model = Some(m.to_string());
                }
            }
        }

        if ty == "event_msg" {
            let p = match payload {
                Some(p) => p,
                None => return,
            };
            if p.get("type").and_then(|x| x.as_str()) != Some("token_count") {
                return;
            }
            agent_turns += 1;
            let info = match p.get("info") {
                Some(i) if !i.is_null() => i,
                _ => return,
            };

            let model_context_window = info
                .get("model_context_window")
                .and_then(|x| x.as_u64())
                .filter(|w| *w > 0);

            let turn_total = info
                .get("last_token_usage")
                .and_then(|l| l.get("total_tokens"))
                .and_then(|x| x.as_u64());
            if let (Some(total), Some(window)) = (turn_total, model_context_window) {
                let pct = (total as f64 / window as f64) * 100.0;
                let is_new_peak = context_used_pct.is_none_or(|cur| pct > cur);
                context_used_pct = Some(match context_used_pct {
                    Some(cur) if cur >= pct => cur,
                    _ => pct,
                });
                if is_new_peak {
                    context_used_tokens = Some(total);
                    context_window = Some(window);
                }
            }

            let last = match info.get("last_token_usage") {
                Some(l) if !l.is_null() => l,
                _ => return,
            };
            saw_usage = true;
            let g = |k: &str| last.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            totals.input += g("input_tokens");
            totals.cached_input += g("cached_input_tokens");
            totals.output += g("output_tokens");
            totals.reasoning_output += g("reasoning_output_tokens");
        }
    })?;

    if !saw_usage {
        return Err(Error::NoUsage(summary.session_id.clone()));
    }

    let model = effective_model
        .clone()
        .ok_or_else(|| Error::NoUsage(summary.session_id.clone()))?;
    let rates =
        pricing::lookup(ProviderKind::Codex, &model).ok_or_else(|| Error::UnknownPricing {
            provider: "codex".into(),
            model: model.clone(),
        })?;
    let included = matches!(plan.mode_for(ProviderKind::Codex), PlanMode::Included);
    let cost = pricing::compute_cost(&totals, &rates, included);
    let duration_secs = match (first_ts, last_ts) {
        (Some(start), Some(end)) if end >= start => Some((end - start).num_seconds() as u64),
        _ => None,
    };

    Ok(SessionAnalysis {
        summary: summary.clone(),
        tokens: totals,
        cost,
        effective_model,
        subagent_file_count: 0,
        tool_call_count: Some(tool_call_count),
        duration_secs,
        context_used_pct,
        context_used_tokens,
        context_window,
        agent_turns: if agent_turns > 0 {
            Some(agent_turns)
        } else {
            None
        },
        user_turns: if user_turns > 0 {
            Some(user_turns)
        } else {
            None
        },
        project_name: None,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Minimal throwaway temp-dir helper. Avoids adding `tempfile` as a
    /// dev-dep; the tests take responsibility for cleanup in Drop.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "agtop-codex-test-{tag}-{}-{nanos}-{seq}",
                std::process::id(),
            ));
            fs::create_dir_all(&path).expect("create tmp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Encode bytes to base64url (no padding) — helper for building JWTs.
    fn base64url_encode(input: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0];
            let b1 = chunk.get(1).copied().unwrap_or(0);
            let b2 = chunk.get(2).copied().unwrap_or(0);
            let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(n & 0x3F) as usize] as char);
            }
        }
        out
    }

    fn make_jwt(plan_type: &str) -> String {
        let header = base64url_encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload_json =
            format!(r#"{{"https://api.openai.com/auth":{{"chatgpt_plan_type":"{plan_type}"}}}}"#);
        let payload = base64url_encode(payload_json.as_bytes());
        // Signature segment can be any opaque bytes.
        let sig = base64url_encode(b"sig");
        format!("{header}.{payload}.{sig}")
    }

    fn write_auth_json(path: &Path, id_token: &str) {
        let body = format!(
            r#"{{"tokens":{{"id_token":"{id_token}","access_token":"a","refresh_token":"r"}}}}"#
        );
        fs::write(path, body).expect("write auth.json");
    }

    // --- base64url_decode unit tests ---

    #[test]
    fn base64url_decode_known_vectors() {
        // Standard RFC 4648 vectors encoded as base64url without padding.
        assert_eq!(base64url_decode("").unwrap(), b"");
        assert_eq!(base64url_decode("Zg").unwrap(), b"f");
        assert_eq!(base64url_decode("Zm8").unwrap(), b"fo");
        assert_eq!(base64url_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64url_decode("Zm9vYg").unwrap(), b"foob");
        assert_eq!(base64url_decode("Zm9vYmE").unwrap(), b"fooba");
        assert_eq!(base64url_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64url_decode_url_alphabet() {
        // `-` and `_` correspond to `+` and `/` in standard base64.
        // Byte 0xFB 0xFF -> standard "+/8=", base64url "-_8"
        assert_eq!(base64url_decode("-_8").unwrap(), vec![0xFB, 0xFF]);
        // With explicit padding should also work.
        assert_eq!(base64url_decode("-_8=").unwrap(), vec![0xFB, 0xFF]);
    }

    #[test]
    fn base64url_decode_rejects_garbage() {
        assert!(base64url_decode("!!!").is_none());
    }

    // --- plan_usage end-to-end tests ---

    #[test]
    fn plan_usage_jwt_plus_decodes_to_chatgpt_plus() {
        let tmp = TmpDir::new("jwt-plus");
        let auth_path = tmp.path().join("auth.json");
        let sessions_root = tmp.path().join("sessions");
        write_auth_json(&auth_path, &make_jwt("plus"));

        let provider = CodexProvider {
            sessions_root,
            auth_path,
            discover_cache: Mutex::default(),
        };
        let usages = provider.plan_usage().expect("plan_usage");
        assert_eq!(usages.len(), 1);
        let u = &usages[0];
        assert_eq!(u.plan_name.as_deref(), Some("ChatGPT Plus"));
        assert_eq!(u.provider, ProviderKind::Codex);
        assert_eq!(u.label, "Codex · ChatGPT Plus");
    }

    #[test]
    fn plan_usage_auth_only_no_rollouts_yields_waiting_note() {
        let tmp = TmpDir::new("auth-only");
        let auth_path = tmp.path().join("auth.json");
        let sessions_root = tmp.path().join("sessions");
        write_auth_json(&auth_path, &make_jwt("pro"));
        // sessions dir intentionally absent — scan_recent_rate_limits
        // should short-circuit on the missing directory.

        let provider = CodexProvider {
            sessions_root,
            auth_path,
            discover_cache: Mutex::default(),
        };
        let usages = provider.plan_usage().expect("plan_usage");
        assert_eq!(usages.len(), 1);
        let u = &usages[0];
        assert_eq!(u.plan_name.as_deref(), Some("ChatGPT Pro"));
        assert!(u.windows.is_empty());
        assert!(
            u.note
                .as_deref()
                .unwrap_or("")
                .contains("waiting for backend"),
            "expected waiting note, got {:?}",
            u.note
        );
    }

    #[test]
    fn plan_usage_populated_rate_limits_yields_two_windows() {
        let tmp = TmpDir::new("populated");
        let auth_path = tmp.path().join("auth.json");
        let sessions_root = tmp
            .path()
            .join("sessions")
            .join("2025")
            .join("09")
            .join("14");
        fs::create_dir_all(&sessions_root).unwrap();
        write_auth_json(&auth_path, &make_jwt("plus"));

        // Build a rollout file with a single event_msg carrying populated
        // rate_limits. resets_at = 1774299600 (2026-04-01T12:20:00Z).
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {},
                "rate_limits": {
                    "limit_id": "abc",
                    "limit_name": "chatgpt",
                    "primary":   {"used_percent": 45.2, "window_minutes": 300, "resets_at": 1_774_299_600i64},
                    "secondary": {"used_percent": 12.0, "window_minutes": 10080, "resets_at": 1_774_886_400i64},
                    "credits": null,
                    "plan_type": "plus"
                }
            }
        });
        let rollout_path = sessions_root.join("rollout-2025-09-14T00-00-00-abcdef.jsonl");
        fs::write(&rollout_path, format!("{line}\n")).unwrap();

        let provider = CodexProvider {
            sessions_root: tmp.path().join("sessions"),
            auth_path,
            discover_cache: Mutex::default(),
        };
        let usages = provider.plan_usage().expect("plan_usage");
        assert_eq!(usages.len(), 1);
        let u = &usages[0];
        assert_eq!(u.plan_name.as_deref(), Some("ChatGPT Plus"));
        assert!(
            u.note.is_none(),
            "no waiting note when windows populated: {:?}",
            u.note
        );
        assert_eq!(u.windows.len(), 2);

        let primary = &u.windows[0];
        assert_eq!(primary.label, "primary");
        let util = primary.utilization.expect("primary util");
        assert!((util - 0.452).abs() < 1e-9, "got {util}");
        assert_eq!(primary.reset_at.map(|d| d.timestamp()), Some(1_774_299_600));

        let secondary = &u.windows[1];
        assert_eq!(secondary.label, "secondary");
        let util = secondary.utilization.expect("secondary util");
        assert!((util - 0.12).abs() < 1e-9, "got {util}");
        assert_eq!(
            secondary.reset_at.map(|d| d.timestamp()),
            Some(1_774_886_400)
        );
    }

    #[test]
    fn plan_usage_nothing_present_returns_empty() {
        let tmp = TmpDir::new("empty");
        let provider = CodexProvider {
            sessions_root: tmp.path().join("sessions"),
            auth_path: tmp.path().join("auth.json"),
            discover_cache: Mutex::default(),
        };
        let usages = provider.plan_usage().expect("plan_usage");
        assert!(usages.is_empty(), "expected empty, got {usages:?}");
    }
}
