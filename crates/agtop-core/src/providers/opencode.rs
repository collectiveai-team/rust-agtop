//! OpenCode provider — `~/.local/share/opencode/storage/`.
//!
//! On-disk layout (observed on-system):
//! - `storage/session/<projectId>/ses_*.json` — one file per session with
//!   metadata `{id, slug, version, projectID, directory, title, time}`.
//! - `storage/message/ses_*/msg_*.json` — one file per assistant/user message
//!   with embedded `tokens: {input, output, reasoning, cache: {read, write}}`
//!   and `modelID`/`providerID`.
//!
//! This is best-effort for v1: the format is undocumented and will likely
//! change. We read everything conservatively and degrade gracefully.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};

use crate::error::{Error, Result};
use crate::pricing::{self, Plan, PlanMode};
use crate::provider::Provider;
use crate::providers::util::dir_exists;
use crate::session::{ProviderKind, SessionAnalysis, SessionSummary, TokenTotals};

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
            storage_root: base.join("opencode").join("storage"),
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
        let session_root = self.storage_root.join("session");
        if !dir_exists(&session_root) {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        let project_dirs = match fs::read_dir(&session_root) {
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
                match summarize_opencode_session(&p, &self.storage_root) {
                    Ok(s) => out.push(s),
                    Err(e) => {
                        tracing::debug!(path = %p.display(), error = %e, "skip opencode session");
                        continue;
                    }
                }
            }
        }
        Ok(out)
    }

    fn analyze(&self, summary: &SessionSummary, plan: Plan) -> Result<SessionAnalysis> {
        analyze_opencode_session(summary, plan, &self.storage_root)
    }
}

fn ms_to_utc(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn summarize_opencode_session(session_file: &Path, storage_root: &Path) -> Result<SessionSummary> {
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
    let msg_dir = storage_root.join("message").join(&session_id);
    let model = first_message_model(&msg_dir);

    Ok(SessionSummary {
        provider: ProviderKind::OpenCode,
        session_id,
        started_at: created,
        last_active: updated.or(created),
        model,
        cwd,
        // Store the session metadata file as primary path; message dir is
        // derived from the id during analysis.
        data_path: session_file.to_path_buf(),
    })
}

fn first_message_model(msg_dir: &Path) -> Option<String> {
    if !dir_exists(msg_dir) {
        return None;
    }
    let entries = fs::read_dir(msg_dir).ok()?;
    for f in entries.flatten() {
        let p = f.path();
        if p.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        if let Ok(v) = read_json(&p) {
            if let Some(m) = v.get("modelID").and_then(|x| x.as_str()) {
                return Some(m.to_string());
            }
        }
    }
    None
}

fn analyze_opencode_session(
    summary: &SessionSummary,
    plan: Plan,
    storage_root: &Path,
) -> Result<SessionAnalysis> {
    let msg_dir = storage_root.join("message").join(&summary.session_id);
    if !dir_exists(&msg_dir) {
        return Err(Error::NoUsage(summary.session_id.clone()));
    }

    let mut totals = TokenTotals::default();
    let mut model: Option<String> = summary.model.clone();
    let mut cost_reported: f64 = 0.0;
    let mut saw = false;

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
        if model.is_none() {
            if let Some(m) = v.get("modelID").and_then(|x| x.as_str()) {
                model = Some(m.to_string());
            }
        }
        if let Some(c) = v.get("cost").and_then(|x| x.as_f64()) {
            cost_reported += c;
        }
        if let Some(t) = v.get("tokens") {
            saw = true;
            let g = |k: &str| t.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            totals.input += g("input");
            totals.output += g("output");
            totals.reasoning_output += g("reasoning");
            if let Some(cache) = t.get("cache") {
                totals.cache_read += cache.get("read").and_then(|x| x.as_u64()).unwrap_or(0);
                totals.cache_write_5m += cache.get("write").and_then(|x| x.as_u64()).unwrap_or(0);
            }
        }
    }

    if !saw {
        return Err(Error::NoUsage(summary.session_id.clone()));
    }
    totals.cached_input = totals.cache_read;

    let included = matches!(plan.mode_for(ProviderKind::OpenCode), PlanMode::Included);
    // Prefer our calculation via pricing lookup, but fall back to the cost
    // that OpenCode wrote into the file if we don't know the model.
    let cost = match model
        .as_deref()
        .and_then(|m| pricing::lookup(ProviderKind::OpenCode, m))
    {
        Some(rates) => pricing::compute_cost(&totals, &rates, included),
        None => {
            if included {
                Default::default()
            } else {
                crate::session::CostBreakdown {
                    total: cost_reported,
                    output: cost_reported, // unknown breakdown; stash in output
                    ..Default::default()
                }
            }
        }
    };

    Ok(SessionAnalysis {
        summary: summary.clone(),
        tokens: totals,
        cost,
        effective_model: model,
    })
}
