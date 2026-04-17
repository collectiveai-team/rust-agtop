//! Claude Code provider — `~/.claude/projects/<slug>/<uuid>.jsonl`.
//!
//! Each line is a JSON record. For token accounting we care about:
//!  - records where `type == "assistant"` and `message.usage` is present
//!  - Claude streams the same request multiple times writing the same
//!    `requestId`; the last write wins for that turn (same policy as the
//!    original `extractClaudeSessionData`).
//!
//! Subagent sidechains (`<slug>/<uuid>/subagents/*.jsonl`) are folded into
//! the parent session's totals so the reported cost reflects the full
//! agent tree. Each sidechain file is a small Claude transcript in its
//! own right — we reuse the same per-request dedup logic to sum it,
//! then add the result to the parent.

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
    let mut effective_model = summary.model.clone();
    let mut totals = TokenTotals::default();

    // Main transcript.
    let main_file_totals = sum_jsonl_usage(path, &mut effective_model)?;
    add_file_totals(&mut totals, &main_file_totals);

    // Subagent sidechain transcripts (if any). They live in a directory
    // sibling to the main `<uuid>.jsonl`, named `<uuid>/subagents/*.jsonl`.
    // Each sidechain is its own small Claude transcript, so we sum them
    // exactly the same way. Subagents may run a different model than the
    // main session (Haiku for "search" subagents, etc.); we still attribute
    // their tokens to the main session's model for costing purposes —
    // that's a small fidelity loss in exchange for keeping the cost math
    // single-rate per session, matching the original JS agtop.
    let subagent_files = list_subagent_files(path, &summary.session_id);
    let subagent_file_count = subagent_files.len();
    for sub in &subagent_files {
        // `sum_jsonl_usage` updates `effective_model` if a subagent reports
        // one and the main transcript didn't — fine. We intentionally pass
        // the shared `effective_model` so a subagent-only session (rare
        // but possible on an abandoned main transcript) still resolves.
        match sum_jsonl_usage(sub, &mut effective_model) {
            Ok(sub_totals) => add_file_totals(&mut totals, &sub_totals),
            Err(e) => tracing::debug!(path = %sub.display(), error = %e, "skip subagent file"),
        }
    }

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
        subagent_file_count,
    })
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
            if m != "<synthetic>" && effective_model.is_none() {
                *effective_model = Some(m.to_string());
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
    Ok(ft)
}

fn add_file_totals(totals: &mut TokenTotals, ft: &FileTotals) {
    totals.input += ft.input;
    totals.output += ft.output;
    totals.cache_read += ft.cache_read;
    totals.cache_write_5m += ft.cache_write_5m;
    totals.cache_write_1h += ft.cache_write_1h;
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
