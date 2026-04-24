//! Gemini CLI client — `~/.gemini/tmp/<slug>/chats/session-*.(json|jsonl)`.
//!
//! Older Gemini CLI builds wrote JSONL session headers; current builds store
//! the full session as a single JSON document.
//! Token counts are only available when the user has enabled local
//! telemetry in `~/.gemini/settings.json`; they are read from
//! `~/.gemini/telemetry.log` and matched to sessions by timestamp range.

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
            Some("OAuth".to_string())
        } else {
            Some("API key".to_string())
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
        let tokens = extract_tokens_from_telemetry(
            &self.gemini_dir,
            summary.started_at,
            summary.last_active,
        );
        let tokens = if tokens.grand_total() == 0 {
            extract_tokens_from_session_file(&summary.data_path)
        } else {
            tokens
        };

        let effective_model = summary.model.clone();
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

        Ok(SessionAnalysis::new(
            summary.clone(),
            tokens,
            cost,
            effective_model,
            0,
            None,
            duration_secs,
            None,
            None,
            None,
        ))
    }
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
            if let Some(tool_calls) = v.get("toolCalls").and_then(|x| x.as_array()) {
                if tool_calls
                    .iter()
                    .any(|call| call.get("status").and_then(|x| x.as_str()) != Some("success"))
                {
                    state = Some("waiting".to_string());
                    state_detail = Some("gemini.toolCalls.pending_or_error".to_string());
                } else if !tool_calls.is_empty() {
                    state = Some("stopped".to_string());
                    state_detail = Some("gemini.toolCalls.success".to_string());
                }
            } else {
                state = Some("stopped".to_string());
                state_detail = Some("gemini.message".to_string());
            }
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

    let mut summary = SessionSummary::new(
        ClientKind::GeminiCli,
        subscription,
        session_id,
        started_at,
        last_active,
        model,
        cwd,
        path.to_path_buf(),
        None,
        None,
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

/// Parse `~/.gemini/telemetry.log` (JSONL) for `gemini_cli.api_response`
/// events whose timestamps fall within the session's time range.
/// Returns aggregated `TokenTotals`.
fn extract_tokens_from_telemetry(
    gemini_dir: &std::path::Path,
    started_at: Option<DateTime<Utc>>,
    last_active: Option<DateTime<Utc>>,
) -> TokenTotals {
    let telemetry_path = gemini_dir.join("telemetry.log");
    if !telemetry_path.exists() {
        return TokenTotals::default();
    }

    let (start, end) = match (started_at, last_active) {
        (Some(s), Some(e)) => (s, e),
        _ => return TokenTotals::default(),
    };

    // Add a small buffer for clock skew.
    let start_buf = start - chrono::Duration::seconds(5);
    let end_buf = end + chrono::Duration::seconds(5);

    let mut totals = TokenTotals::default();

    let _ = for_each_jsonl(&telemetry_path, |v| {
        // Match only api_response events.
        let is_api_response = v
            .get("name")
            .and_then(|n| n.as_str())
            .map(|n| n == "gemini_cli.api_response")
            .unwrap_or(false);
        if !is_api_response {
            return;
        }

        // Check timestamp falls in session window.
        let event_ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_ts);
        let Some(ts) = event_ts else { return };
        if ts < start_buf || ts > end_buf {
            return;
        }

        let g = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        totals.input += g("input_token_count");
        totals.output += g("output_token_count");
        totals.cache_read += g("cached_content_token_count");
        totals.reasoning_output += g("thoughts_token_count");
    });

    // Post-accumulation: cached_input mirrors cache_read.
    totals.cached_input = totals.cache_read;

    totals
}

fn extract_tokens_from_session_file(path: &std::path::Path) -> TokenTotals {
    let mut totals = TokenTotals::default();
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        let Ok(bytes) = fs::read(path) else {
            return totals;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return totals;
        };
        if let Some(messages) = v.get("messages").and_then(|x| x.as_array()) {
            for message in messages {
                add_gemini_tokens(&mut totals, message.get("tokens"));
            }
        }
    } else {
        let _ = for_each_jsonl(path, |v| {
            add_gemini_tokens(&mut totals, v.get("tokens"));
        });
    }
    totals.cached_input = totals.cache_read;
    totals
}

fn add_gemini_tokens(totals: &mut TokenTotals, raw: Option<&serde_json::Value>) {
    let Some(tokens) = raw else {
        return;
    };
    let g = |k: &str| tokens.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    totals.input += g("input");
    totals.output += g("output");
    totals.cache_read += g("cached");
    totals.reasoning_output += g("thoughts");
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
        let tokens = extract_tokens_from_telemetry(&td.path, Some(start), Some(end));

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
        let tokens = extract_tokens_from_telemetry(&td.path, Some(start), Some(end));

        assert_eq!(tokens.input, 0);
    }
}
