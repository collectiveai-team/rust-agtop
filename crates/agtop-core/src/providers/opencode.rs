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
        let mut out = Vec::new();

        // --- SQLite path (v1.4+) ---
        if db_path.exists() {
            match list_sessions_sqlite(&db_path) {
                Ok(mut rows) => out.append(&mut rows),
                Err(e) => {
                    tracing::warn!(path = %db_path.display(), error = %e, "opencode sqlite list failed")
                }
            }
        }

        // --- Legacy JSON path (v1.1 and earlier) ---
        let session_root = self.storage_root.join("storage").join("session");
        if dir_exists(&session_root) {
            match list_sessions_json(&session_root, &self.storage_root) {
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ms_to_utc(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

// ---------------------------------------------------------------------------
// SQLite backend (v1.4+)
// ---------------------------------------------------------------------------

fn open_db(db_path: &Path) -> Result<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| Error::Other(format!("sqlite open {}: {e}", db_path.display())))
}

fn list_sessions_sqlite(db_path: &Path) -> Result<Vec<SessionSummary>> {
    let conn = open_db(db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, directory, time_created, time_updated FROM session \
             WHERE time_archived IS NULL OR time_archived = 0 \
             ORDER BY time_updated DESC",
        )
        .map_err(|e| Error::Other(format!("sqlite prepare: {e}")))?;

    let rows: Vec<SessionSummary> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let cwd: Option<String> = row.get(1)?;
            let created_ms: Option<i64> = row.get(2)?;
            let updated_ms: Option<i64> = row.get(3)?;
            Ok((id, cwd, created_ms, updated_ms))
        })
        .map_err(|e| Error::Other(format!("sqlite query: {e}")))?
        .filter_map(|r| r.ok())
        .map(|(id, cwd, created_ms, updated_ms)| {
            let started_at = created_ms.and_then(ms_to_utc);
            let last_active = updated_ms.and_then(ms_to_utc).or(started_at);
            // model is filled lazily from the first assistant message during analysis;
            // we set it to None here and let analyze() resolve it.
            let model = first_message_model_sqlite(&conn, &id);
            SessionSummary {
                provider: ProviderKind::OpenCode,
                session_id: id.clone(),
                started_at,
                last_active,
                model,
                cwd,
                data_path: db_path.to_path_buf(),
            }
        })
        .collect();

    Ok(rows)
}

fn first_message_model_sqlite(conn: &rusqlite::Connection, session_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT json_extract(data, '$.modelID') FROM message \
         WHERE session_id = ?1 \
           AND json_extract(data, '$.role') = 'assistant' \
         ORDER BY time_created ASC \
         LIMIT 1",
        rusqlite::params![session_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

fn analyze_session_sqlite(
    summary: &SessionSummary,
    plan: Plan,
    db_path: &Path,
) -> Result<SessionAnalysis> {
    let conn = open_db(db_path)?;

    let mut stmt = conn
        .prepare(
            "SELECT data FROM message \
             WHERE session_id = ?1 \
               AND json_extract(data, '$.role') = 'assistant'",
        )
        .map_err(|e| Error::Other(format!("sqlite prepare analyze: {e}")))?;

    let mut totals = TokenTotals::default();
    let mut model: Option<String> = summary.model.clone();
    let mut cost_reported: f64 = 0.0;
    let mut saw = false;

    let rows = stmt
        .query_map(rusqlite::params![&summary.session_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| Error::Other(format!("sqlite query analyze: {e}")))?;

    for row in rows {
        let data_str = match row {
            Ok(s) => s,
            Err(_) => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&data_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

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
                    output: cost_reported,
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
        subagent_file_count: 0,
    })
}

// ---------------------------------------------------------------------------
// Legacy JSON backend (v1.1 and earlier)
// ---------------------------------------------------------------------------

fn list_sessions_json(session_root: &Path, storage_root: &Path) -> Result<Vec<SessionSummary>> {
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
            match summarize_opencode_session_json(&p, storage_root) {
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
    let model = first_message_model_json(&msg_dir);

    Ok(SessionSummary {
        provider: ProviderKind::OpenCode,
        session_id,
        started_at: created,
        last_active: updated.or(created),
        model,
        cwd,
        data_path: session_file.to_path_buf(),
    })
}

fn first_message_model_json(msg_dir: &Path) -> Option<String> {
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
                    output: cost_reported,
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
        subagent_file_count: 0,
    })
}
