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

use chrono::{DateTime, Utc};
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::pricing::{self, Plan, PlanMode};
use crate::provider::Provider;
use crate::providers::util::{dir_exists, for_each_jsonl, mtime, parse_ts};
use crate::session::{ProviderKind, SessionAnalysis, SessionSummary, TokenTotals};

#[derive(Debug, Clone)]
pub struct CodexProvider {
    pub sessions_root: PathBuf,
}

impl Default for CodexProvider {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            sessions_root: home.join(".codex").join("sessions"),
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
        let mut out = Vec::new();
        for entry in WalkDir::new(&self.sessions_root)
            .into_iter()
            .filter_map(|r| r.ok())
        {
            let p = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }
            if p.extension().map(|e| e != "jsonl").unwrap_or(true) {
                continue;
            }
            match summarize_codex_file(p) {
                Ok(s) => out.push(s),
                Err(e) => {
                    tracing::debug!(path = %p.display(), error = %e, "skip codex file");
                    continue;
                }
            }
        }
        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        analyze_codex_file(summary, plan)
    }
}

fn summarize_codex_file(path: &Path) -> Result<SessionSummary> {
    let mut session_id: Option<String> = None;
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut model: Option<String> = None;
    let mut cwd: Option<String> = None;
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
        data_path: path.to_path_buf(),
    })
}

fn analyze_codex_file(summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
    let path = &summary.data_path;
    let mut totals = TokenTotals::default();
    let mut saw_usage = false;
    let mut effective_model = summary.model.clone();

    for_each_jsonl(path, |v| {
        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let payload = v.get("payload");

        if ty == "turn_context" {
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
            let info = match p.get("info") {
                Some(i) if !i.is_null() => i,
                _ => return,
            };
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

    Ok(SessionAnalysis {
        summary: summary.clone(),
        tokens: totals,
        cost,
        effective_model,
        subagent_file_count: 0,
    })
}
