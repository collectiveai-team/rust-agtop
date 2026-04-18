//! GitHub Copilot provider — VS Code chat session discovery + quota API.
//!
//! Session transcripts are stored in VS Code's workspace storage:
//!   `~/.config/Code/User/workspaceStorage/<hash>/chatSessions/<uuid>.json`
//!
//! Token counts are NOT persisted by Copilot. Sessions are returned with
//! zero token totals. Plan quota is fetched live from the GitHub API using
//! an OAuth token found in `~/.config/gh/hosts.yml` or
//! `~/.config/github-copilot/hosts.json`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::pricing::Plan;
use crate::provider::Provider;
use crate::providers::util::mtime;
use crate::session::{
    CostBreakdown, PlanUsage, PlanWindow, ProviderKind, SessionAnalysis, SessionSummary,
    TokenTotals,
};

/// Quota API cache TTL.
const QUOTA_CACHE_SECS: u64 = 300;

/// Data extracted from a single Copilot session JSON that is needed by
/// both `list_sessions` and `analyze`. Stored in a provider-level cache
/// so the file is parsed exactly once per refresh cycle.
#[derive(Debug, Clone)]
struct CopilotParsed {
    tool_call_count: Option<u64>,
    duration_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CopilotProvider {
    pub workspace_storage_root: PathBuf,
    pub gh_hosts_path: PathBuf,
    pub vim_hosts_path: PathBuf,
    /// Parsed data from the last `list_sessions` call, keyed by session
    /// file path. Shared via `Arc<RwLock<…>>` so `Clone` shares the cache
    /// (correct — provider instances are shared across threads via `Arc`).
    cache: Arc<RwLock<HashMap<PathBuf, CopilotParsed>>>,
}

impl Default for CopilotProvider {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        // VS Code on Linux uses ~/.config/Code; on macOS ~/Library/Application Support/Code.
        #[cfg(target_os = "macos")]
        let code_user = home
            .join("Library")
            .join("Application Support")
            .join("Code")
            .join("User");
        #[cfg(not(target_os = "macos"))]
        let code_user = home.join(".config").join("Code").join("User");

        Self {
            workspace_storage_root: code_user.join("workspaceStorage"),
            gh_hosts_path: home.join(".config").join("gh").join("hosts.yml"),
            vim_hosts_path: home
                .join(".config")
                .join("github-copilot")
                .join("hosts.json"),
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Provider for CopilotProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Copilot
    }

    fn display_name(&self) -> &'static str {
        "GitHub Copilot"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !self.workspace_storage_root.exists() {
            return Ok(vec![]);
        }

        let mut out = Vec::new();

        let workspace_dirs = match fs::read_dir(&self.workspace_storage_root) {
            Ok(d) => d,
            Err(_) => return Ok(out),
        };

        for ws_entry in workspace_dirs.flatten() {
            let ws_path = ws_entry.path();
            if !ws_path.is_dir() {
                continue;
            }
            let chat_sessions_dir = ws_path.join("chatSessions");
            if !chat_sessions_dir.is_dir() {
                continue;
            }
            let files = match fs::read_dir(&chat_sessions_dir) {
                Ok(d) => d,
                Err(_) => continue,
            };
            for file_entry in files.flatten() {
                let path = file_entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let stem = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                match parse_session_all(&path, stem) {
                    Ok((summary, parsed)) => {
                        if let Ok(mut guard) = self.cache.write() {
                            guard.insert(path.clone(), parsed);
                        }
                        out.push(summary);
                    }
                    Err(e) => {
                        tracing::debug!(path = %path.display(), error = %e, "skip copilot session");
                    }
                }
            }
        }

        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, _plan: Plan) -> Result<SessionAnalysis> {
        // Token counts are not available from Copilot local files.
        // Return zero-cost analysis with whatever metadata we have.
        let path = &summary.data_path;

        // Fast path: use data cached by the preceding `list_sessions` call.
        // One file read per session per refresh instead of two.
        let (tool_call_count, duration_secs) = self
            .cache
            .read()
            .ok()
            .and_then(|guard| guard.get(path).cloned())
            .map(|p| (p.tool_call_count, p.duration_secs))
            // Fallback: re-read the file (tests or list_sessions was skipped).
            .unwrap_or_else(|| parse_session_metadata(path));

        Ok(SessionAnalysis::new(
            summary.clone(),
            TokenTotals::default(),
            CostBreakdown::default(),
            summary.model.clone(),
            0,
            tool_call_count,
            duration_secs,
            None,
            None,
            None,
        ))
    }

    fn plan_usage(&self) -> Result<Vec<PlanUsage>> {
        let token = match read_gh_token(&self.gh_hosts_path, &self.vim_hosts_path) {
            Some(t) => t,
            None => return Ok(vec![]),
        };

        // Check cache file to avoid hammering the API.
        let cache_path = quota_cache_path();
        if let Some(cached) = read_quota_cache(&cache_path) {
            return Ok(cached);
        }

        let result = fetch_copilot_quota(&token);
        if let Ok(ref usages) = result {
            write_quota_cache(&cache_path, usages);
        }
        result
    }
}

/// Read a Copilot session JSON file exactly once and return both the
/// lightweight `SessionSummary` fields and the `CopilotParsed` extras
/// needed by `analyze`. Keeps file I/O to a single read per session.
fn parse_session_all(
    path: &std::path::Path,
    session_id: String,
) -> Result<(SessionSummary, CopilotParsed)> {
    let bytes = fs::read(path)?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);

    let mut model: Option<String> = None;
    let mut tool_calls: u64 = 0;
    let mut total_elapsed_ms: u64 = 0;

    if let Some(requests) = v.get("requests").and_then(|r| r.as_array()) {
        for req in requests {
            if model.is_none() {
                model = req
                    .get("modelId")
                    .and_then(|m| m.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }
            if let Some(rounds) = req
                .pointer("/result/metadata/toolCallRounds")
                .and_then(|r| r.as_array())
            {
                tool_calls += rounds.len() as u64;
            }
            if let Some(elapsed) = req
                .pointer("/result/timings/totalElapsed")
                .and_then(|x| x.as_u64())
            {
                total_elapsed_ms += elapsed;
            }
        }
    }

    let last_active = mtime(path);
    let started_at = match (last_active, total_elapsed_ms > 0) {
        (Some(end), true) => Some(end - chrono::Duration::milliseconds(total_elapsed_ms as i64)),
        _ => last_active,
    };

    let summary = SessionSummary::new(
        ProviderKind::Copilot,
        None, // subscription set by list_sessions caller if available
        session_id,
        started_at,
        last_active,
        model,
        None, // cwd — workspace hash not reversible
        path.to_path_buf(),
        None,
        None,
        None,
        None,
    );
    let parsed = CopilotParsed {
        tool_call_count: if tool_calls > 0 {
            Some(tool_calls)
        } else {
            None
        },
        duration_secs: if total_elapsed_ms > 0 {
            Some(total_elapsed_ms / 1000)
        } else {
            None
        },
    };

    Ok((summary, parsed))
}

fn parse_session_metadata(path: &std::path::Path) -> (Option<u64>, Option<u64>) {
    let Ok(bytes) = fs::read(path) else {
        return (None, None);
    };
    let v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let mut tool_calls: u64 = 0;
    let mut total_elapsed_ms: u64 = 0;

    if let Some(requests) = v.get("requests").and_then(|r| r.as_array()) {
        for req in requests {
            if let Some(rounds) = req
                .pointer("/result/metadata/toolCallRounds")
                .and_then(|r| r.as_array())
            {
                tool_calls += rounds.len() as u64;
            }
            if let Some(elapsed) = req
                .pointer("/result/timings/totalElapsed")
                .and_then(|x| x.as_u64())
            {
                total_elapsed_ms += elapsed;
            }
        }
    }

    let tool_call_count = if tool_calls > 0 {
        Some(tool_calls)
    } else {
        None
    };
    let duration_secs = if total_elapsed_ms > 0 {
        Some(total_elapsed_ms / 1000)
    } else {
        None
    };
    (tool_call_count, duration_secs)
}

/// Read the GitHub OAuth token from gh CLI hosts.yml or copilot hosts.json.
fn read_gh_token(gh_hosts: &std::path::Path, vim_hosts: &std::path::Path) -> Option<String> {
    // Try gh CLI hosts.yml first (most common).
    if let Ok(bytes) = fs::read(gh_hosts) {
        // Simple line scan: look for `oauth_token: <token>` under `github.com:`.
        let text = String::from_utf8_lossy(&bytes);
        let mut in_github = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("github.com:") {
                in_github = true;
                continue;
            }
            if in_github {
                if trimmed.starts_with("oauth_token:") {
                    let token = trimmed
                        .trim_start_matches("oauth_token:")
                        .trim()
                        .trim_matches('"')
                        .to_string();
                    if !token.is_empty() {
                        return Some(token);
                    }
                }
                // End of github.com block (new top-level key).
                if !trimmed.starts_with(' ') && !trimmed.starts_with('\t') && !trimmed.is_empty() {
                    in_github = false;
                }
            }
        }
    }

    // Fallback: vim/neovim hosts.json.
    if let Ok(bytes) = fs::read(vim_hosts) {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(token) = v
                .get("github.com")
                .and_then(|h| h.get("oauth_token"))
                .and_then(|t| t.as_str())
            {
                return Some(token.to_string());
            }
        }
    }

    None
}

fn quota_cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("agtop")
        .join("copilot_quota.json")
}

fn read_quota_cache(path: &std::path::Path) -> Option<Vec<PlanUsage>> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).ok()?;
    if age > Duration::from_secs(QUOTA_CACHE_SECS) {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_quota_cache(path: &std::path::Path, usages: &[PlanUsage]) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(usages) {
        let _ = fs::write(path, bytes);
    }
}

fn fetch_copilot_quota(gh_token: &str) -> Result<Vec<PlanUsage>> {
    let response = ureq::get("https://api.github.com/copilot_internal/v2/token")
        .header("Authorization", &format!("token {}", gh_token))
        .header("Editor-Version", "vscode/1.115.0")
        .header("Copilot-Integration-Id", "vscode-chat")
        .header("User-Agent", "agtop")
        .call();

    let resp = match response {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "copilot quota API failed");
            return Ok(vec![]);
        }
    };

    let body: serde_json::Value = match resp.into_body().read_json() {
        Ok(v) => v,
        Err(_) => return Ok(vec![]),
    };

    let plan_name = body
        .get("copilot_plan")
        .and_then(|p| p.as_str())
        .map(str::to_string);

    let label = match &plan_name {
        Some(p) => format!("GitHub Copilot · {}", p),
        None => "GitHub Copilot".to_string(),
    };

    let mut windows = Vec::new();
    let mut note: Option<String> = None;

    if let Some(premium) = body.pointer("/quota_snapshots/premium_interactions") {
        let unlimited = premium
            .get("unlimited")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if unlimited {
            note = Some("unlimited premium interactions".to_string());
        } else {
            let entitlement = premium
                .get("entitlement")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let remaining = premium
                .get("remaining")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let utilization = if entitlement > 0.0 {
                Some(((entitlement - remaining) / entitlement).clamp(0.0, 1.0))
            } else {
                None
            };

            let reset_hint = body
                .get("quota_reset_date")
                .and_then(|d| d.as_str())
                .map(|d| format!("resets {}", d));

            windows.push(PlanWindow::new(
                "monthly".to_string(),
                utilization,
                None,
                reset_hint,
                true,
            ));
        }
    }

    if windows.is_empty() && note.is_none() {
        note = Some("no utilization data available".to_string());
    }

    Ok(vec![PlanUsage::new(
        ProviderKind::Copilot,
        label,
        plan_name,
        windows,
        None,
        note,
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    struct TestDir {
        path: std::path::PathBuf,
    }
    impl TestDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("agtop-copilot-{}-{}", name, std::process::id()));
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
    fn missing_root_returns_empty() {
        let p = CopilotProvider {
            workspace_storage_root: std::path::PathBuf::from("/no/such/path"),
            gh_hosts_path: std::path::PathBuf::from("/no/such/hosts.yml"),
            vim_hosts_path: std::path::PathBuf::from("/no/such/hosts.json"),
            ..CopilotProvider::default()
        };
        assert!(p.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn parses_chat_session_json() {
        let td = TestDir::new("parse");
        let session_dir = td.path.join("abc123").join("chatSessions");
        fs::create_dir_all(&session_dir).unwrap();
        let session_file = session_dir.join("02742fb3-d98e-4fa2-8184-2fddd7ee544d.json");
        let content = r#"{
            "version": 3,
            "requests": [{
                "requestId": "req1",
                "modelId": "copilot/gpt-4.1",
                "result": {
                    "timings": {"firstProgress": 100, "totalElapsed": 5000},
                    "metadata": {"toolCallRounds": []}
                }
            }]
        }"#;
        fs::File::create(&session_file)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();

        let p = CopilotProvider {
            workspace_storage_root: td.path.clone(),
            gh_hosts_path: std::path::PathBuf::from("/no/such/hosts.yml"),
            vim_hosts_path: std::path::PathBuf::from("/no/such/hosts.json"),
            ..CopilotProvider::default()
        };
        let sessions = p.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].session_id,
            "02742fb3-d98e-4fa2-8184-2fddd7ee544d"
        );
        assert_eq!(sessions[0].model.as_deref(), Some("copilot/gpt-4.1"));
        assert_eq!(sessions[0].provider, ProviderKind::Copilot);
    }

    #[test]
    fn reads_gh_token_from_hosts_yml() {
        let td = TestDir::new("token");
        let hosts = td.path.join("hosts.yml");
        fs::write(
            &hosts,
            "github.com:\n    oauth_token: ghu_testtoken123\n    user: testuser\n",
        )
        .unwrap();
        let token = read_gh_token(&hosts, &std::path::PathBuf::from("/no/such"));
        assert_eq!(token.as_deref(), Some("ghu_testtoken123"));
    }

    #[test]
    fn reads_gh_token_from_vim_hosts_json() {
        let td = TestDir::new("vim");
        let hosts = td.path.join("hosts.json");
        fs::write(
            &hosts,
            r#"{"github.com": {"oauth_token": "ghu_vimtoken", "user": "u"}}"#,
        )
        .unwrap();
        let token = read_gh_token(&std::path::PathBuf::from("/no/such"), &hosts);
        assert_eq!(token.as_deref(), Some("ghu_vimtoken"));
    }

    #[test]
    fn analyze_uses_parse_cache_not_disk() {
        let td = TestDir::new("cache");
        let session_dir = td.path.join("ws1").join("chatSessions");
        fs::create_dir_all(&session_dir).unwrap();
        let session_file = session_dir.join("cached-session-id.json");
        let content = r#"{
            "version": 3,
            "requests": [{
                "requestId": "r1",
                "modelId": "copilot/gpt-4.1",
                "result": {
                    "timings": {"firstProgress": 10, "totalElapsed": 2000},
                    "metadata": {"toolCallRounds": [1, 2]}
                }
            }]
        }"#;
        fs::write(&session_file, content).unwrap();

        let p = CopilotProvider {
            workspace_storage_root: td.path.clone(),
            gh_hosts_path: std::path::PathBuf::from("/no"),
            vim_hosts_path: std::path::PathBuf::from("/no"),
            ..CopilotProvider::default()
        };

        // list_sessions fills the cache
        let sessions = p.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);

        // Delete the file — analyze must still work from cache
        fs::remove_file(&session_file).unwrap();
        let analysis = p.analyze(&sessions[0], Plan::Retail).unwrap();
        assert_eq!(analysis.tool_call_count, Some(2));
    }
}
