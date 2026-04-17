//! Claude Code provider — `~/.claude/projects/<slug>/<uuid>.jsonl`.
//!
//! Each line is a JSON record. For token accounting we care about:
//!  - records where `type == "assistant"` and `message.usage` is present
//!  - Claude streams the same request multiple times writing the same
//!    `requestId`; the last write wins for that turn (same policy as the
//!    original `extractClaudeSessionData`).
//!
//! We intentionally skip subagent sidechain files (`<uuid>/subagents/*.jsonl`)
//! for the MVP — they inflate a session's effective cost but most users reason
//! about the main transcript first. Wiring them in is a later feature.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::error::{Error, Result};
use crate::pricing::{self, Plan, PlanMode};
use crate::provider::Provider;
use crate::providers::util::{dir_exists, for_each_jsonl, mtime, parse_ts};
use crate::session::{ProviderKind, SessionAnalysis, SessionSummary, TokenTotals};

#[derive(Debug, Clone)]
pub struct ClaudeProvider {
    pub projects_root: PathBuf,
}

impl Default for ClaudeProvider {
    fn default() -> Self {
        // Honor $CLAUDE_CONFIG_DIR like the original.
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let base = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"));
        Self {
            projects_root: base.join("projects"),
        }
    }
}

impl Provider for ClaudeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Claude
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        if !dir_exists(&self.projects_root) {
            return Ok(vec![]);
        }
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
                match summarize_claude_file(&p) {
                    Ok(s) if s.model.is_some() => out.push(s),
                    Ok(_) => continue, // skip empty/abandoned sessions
                    Err(e) => {
                        tracing::debug!(path = %p.display(), error = %e, "skip claude file");
                        continue;
                    }
                }
            }
        }
        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        analyze_claude_file(summary, plan)
    }
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
    })?;

    let last_active = mtime(path).or(earliest);

    Ok(SessionSummary {
        provider: ProviderKind::Claude,
        session_id,
        started_at: earliest,
        last_active,
        model,
        cwd,
        data_path: path.to_path_buf(),
    })
}

fn analyze_claude_file(summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
    let path = &summary.data_path;
    // Per-request-id snapshot: streaming rewrites the same requestId as it
    // progresses; only the final write has correct totals. Keep the last.
    let mut last_snapshot: HashMap<String, Snapshot> = HashMap::new();
    // Entries with no requestId/messageId accumulate directly.
    let mut keyless = Snapshot::default();
    let mut effective_model = summary.model.clone();

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
        let m = message.get("model").and_then(|x| x.as_str());
        if let Some(m) = m {
            if m != "<synthetic>" {
                effective_model = Some(m.to_string());
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
            None => keyless.add(&snap),
        }
    })?;

    let mut totals = TokenTotals::default();
    for snap in last_snapshot.values() {
        totals.input += snap.input;
        totals.output += snap.output;
        totals.cache_read += snap.cache_read;
        totals.cache_write_5m += snap.cache_write_5m;
        totals.cache_write_1h += snap.cache_write_1h;
    }
    totals.input += keyless.input;
    totals.output += keyless.output;
    totals.cache_read += keyless.cache_read;
    totals.cache_write_5m += keyless.cache_write_5m;
    totals.cache_write_1h += keyless.cache_write_1h;
    // Claude's "cached_input" bucket for our cost math is cache_read.
    totals.cached_input = totals.cache_read;

    if totals.grand_total() == 0 {
        return Err(Error::NoUsage(summary.session_id.clone()));
    }

    let model = effective_model
        .clone()
        .ok_or_else(|| Error::NoUsage(summary.session_id.clone()))?;
    let rates =
        pricing::lookup(ProviderKind::Claude, &model).ok_or_else(|| Error::UnknownPricing {
            provider: "claude".into(),
            model: model.clone(),
        })?;
    let included = matches!(plan.mode_for(ProviderKind::Claude), PlanMode::Included);
    let cost = pricing::compute_cost(&totals, &rates, included);

    Ok(SessionAnalysis {
        summary: summary.clone(),
        tokens: totals,
        cost,
        effective_model,
    })
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
}
