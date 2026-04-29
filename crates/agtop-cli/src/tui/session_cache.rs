//! Persistent SQLite cache for `SessionAnalysis` values.
//!
//! Schema:
//!   session_cache(client TEXT, session_id TEXT, last_active TEXT,
//!                 cache_version INTEGER, data TEXT,
//!                 PRIMARY KEY (client, session_id))
//!
//! A cache hit requires client + session_id + last_active + cache_version all match.
//! On schema change, bump CACHE_VERSION to force full re-analysis on next launch.

use agtop_core::session::SessionAnalysis;
use agtop_core::ClientKind;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags};

/// Bump this when `SessionAnalysis` or nested types change in a breaking way.
pub const CACHE_VERSION: u32 = 3;

/// Identifies a specific version of a session for cache lookup.
#[derive(Debug, Clone)]
pub struct CacheKey {
    pub client: ClientKind,
    pub session_id: String,
    pub last_active: Option<DateTime<Utc>>,
}

pub struct SessionCache {
    conn: Connection,
    no_cache: bool,
}

impl SessionCache {
    /// Open the production cache at `~/.cache/agtop/session-cache.db`.
    pub fn open(no_cache: bool) -> anyhow::Result<Self> {
        let path = cache_db_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let cache = Self { conn, no_cache };
        cache.create_schema()?;
        Ok(cache)
    }

    /// Open an in-memory database (for tests).
    #[cfg(test)]
    pub fn open_in_memory(no_cache: bool) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let cache = Self { conn, no_cache };
        cache.create_schema()?;
        Ok(cache)
    }

    fn create_schema(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_cache (
                client        TEXT    NOT NULL,
                session_id    TEXT    NOT NULL,
                last_active   TEXT    NOT NULL,
                cache_version INTEGER NOT NULL,
                data          TEXT    NOT NULL,
                PRIMARY KEY (client, session_id)
            );",
        )?;
        Ok(())
    }

    /// Look up a cached `SessionAnalysis`. Returns `None` on any miss
    /// (unknown session, stale `last_active`, version mismatch, or `no_cache` mode).
    pub fn lookup(&self, key: &CacheKey) -> Option<SessionAnalysis> {
        if self.no_cache {
            return None;
        }
        let last_active_str = key.last_active.map(|dt| dt.to_rfc3339())?;
        let result: rusqlite::Result<String> = self.conn.query_row(
            "SELECT data FROM session_cache
             WHERE client = ?1 AND session_id = ?2
               AND last_active = ?3 AND cache_version = ?4",
            params![
                key.client.as_str(),
                key.session_id,
                last_active_str,
                CACHE_VERSION,
            ],
            |row| row.get(0),
        );
        match result {
            Ok(json) => serde_json::from_str(&json).ok(),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => {
                tracing::warn!("session_cache lookup error: {e}");
                None
            }
        }
    }

    /// Persist a `SessionAnalysis`. Uses INSERT OR REPLACE so stale rows are
    /// atomically overwritten.
    pub fn store(&self, analysis: &SessionAnalysis) -> anyhow::Result<()> {
        let last_active_str = analysis
            .summary
            .last_active
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        let json = serde_json::to_string(analysis)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO session_cache
             (client, session_id, last_active, cache_version, data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                analysis.summary.client.as_str(),
                analysis.summary.session_id,
                last_active_str,
                CACHE_VERSION,
                json,
            ],
        )?;
        Ok(())
    }

    /// Remove rows for session IDs no longer in the live set.
    #[allow(dead_code)]
    pub fn prune_stale(
        &self,
        live_ids: &std::collections::HashSet<String>,
    ) -> anyhow::Result<usize> {
        let mut deleted = 0usize;
        let ids: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT session_id FROM session_cache")?;
            let collected: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };
        for id in ids {
            if !live_ids.contains(&id) {
                self.conn.execute(
                    "DELETE FROM session_cache WHERE session_id = ?1",
                    params![id],
                )?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }
}

fn cache_db_path() -> anyhow::Result<std::path::PathBuf> {
    let base =
        dirs::cache_dir().ok_or_else(|| anyhow::anyhow!("cannot determine cache directory"))?;
    Ok(base.join("agtop").join("session-cache.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::session::{CostBreakdown, SessionAnalysis, SessionSummary, TokenTotals};
    use agtop_core::ClientKind;
    use chrono::Utc;
    use std::path::PathBuf;

    /// Build a minimal SessionAnalysis using the real (non-exhaustive) constructors.
    fn minimal_analysis(session_id: &str) -> SessionAnalysis {
        let last_active = Some(Utc::now());
        let summary = SessionSummary::new(
            ClientKind::Claude,               // client
            None,                             // subscription
            session_id.to_string(),           // session_id
            last_active,                      // started_at
            last_active,                      // last_active
            None,                             // model
            None,                             // cwd
            PathBuf::from("/tmp/fake.jsonl"), // data_path
            None,                             // state_detail
            None,                             // model_effort
            None,                             // model_effort_detail
        );
        SessionAnalysis::new(
            summary,
            TokenTotals::default(),
            CostBreakdown::default(),
            None, // effective_model
            0,    // subagent_file_count
            None, // tool_call_count
            None, // duration_secs
            None, // context_used_pct
            None, // context_used_tokens
            None, // context_window
        )
    }

    #[test]
    fn roundtrip_hit() {
        let cache = SessionCache::open_in_memory(false).unwrap();
        let a = minimal_analysis("ses_abc123");
        cache.store(&a).unwrap();
        let key = CacheKey {
            client: ClientKind::Claude,
            session_id: "ses_abc123".to_string(),
            last_active: a.summary.last_active,
        };
        let hit = cache.lookup(&key);
        assert!(hit.is_some(), "expected cache hit immediately after store");
        assert_eq!(hit.unwrap().summary.session_id, "ses_abc123");
    }

    #[test]
    fn miss_on_stale_last_active() {
        let cache = SessionCache::open_in_memory(false).unwrap();
        let a = minimal_analysis("ses_xyz");
        cache.store(&a).unwrap();
        let key = CacheKey {
            client: ClientKind::Claude,
            session_id: "ses_xyz".to_string(),
            last_active: Some(Utc::now() + chrono::Duration::seconds(3600)),
        };
        assert!(cache.lookup(&key).is_none());
    }

    #[test]
    fn no_cache_flag_skips_read() {
        let cache = SessionCache::open_in_memory(true).unwrap();
        let a = minimal_analysis("ses_skip");
        cache.store(&a).unwrap();
        let key = CacheKey {
            client: ClientKind::Claude,
            session_id: "ses_skip".to_string(),
            last_active: a.summary.last_active,
        };
        assert!(cache.lookup(&key).is_none());
    }

    #[test]
    fn lookup_ignores_legacy_rows_without_recent_messages() {
        let cache = SessionCache::open_in_memory(false).unwrap();
        let mut a = minimal_analysis("ses_legacy_opencode");
        a.summary.client = ClientKind::OpenCode;

        let last_active_str = a.summary.last_active.unwrap().to_rfc3339();
        let mut json = serde_json::to_value(&a).unwrap();
        json.as_object_mut().unwrap().remove("recent_messages");
        cache
            .conn
            .execute(
                "INSERT OR REPLACE INTO session_cache
                 (client, session_id, last_active, cache_version, data)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    ClientKind::OpenCode.as_str(),
                    a.summary.session_id,
                    last_active_str,
                    2,
                    serde_json::to_string(&json).unwrap(),
                ],
            )
            .unwrap();

        let key = CacheKey {
            client: ClientKind::OpenCode,
            session_id: "ses_legacy_opencode".to_string(),
            last_active: a.summary.last_active,
        };
        assert!(cache.lookup(&key).is_none());
    }
}
