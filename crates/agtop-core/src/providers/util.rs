use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// True iff `p` exists and is a directory.
pub fn dir_exists(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

/// mtime of `p` as UTC, or None on error.
pub fn mtime(p: &Path) -> Option<DateTime<Utc>> {
    let meta = fs::metadata(p).ok()?;
    let st = meta.modified().ok()?;
    let dt: DateTime<Utc> = st.into();
    Some(dt)
}

/// Iterate JSON Lines in `path`, yielding successfully-parsed values to the
/// callback. Malformed lines, overlong lines (>512KB; usually base64 image
/// payloads), and trailing garbage are silently skipped — matching the
/// original agtop's behavior of tolerating truncated transcripts.
pub fn for_each_jsonl<F>(path: &Path, mut f: F) -> std::io::Result<()>
where
    F: FnMut(&serde_json::Value),
{
    use std::io::{BufRead, BufReader};
    let file = fs::File::open(path)?;
    // 1 MB buffer accommodates normal transcript lines cheaply.
    let reader = BufReader::with_capacity(1 << 20, file);
    for line_res in reader.lines() {
        let Ok(line) = line_res else { continue };
        if line.is_empty() {
            continue;
        }
        if line.len() > 512 * 1024 {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => f(&v),
            Err(_) => continue,
        }
    }
    Ok(())
}

/// Parse a timestamp string (ISO 8601 / RFC 3339) to UTC.
pub fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Per-provider cache keyed on file identity. A file is considered
/// unchanged when both `mtime` and `size` match the previous call.
#[derive(Debug, Default)]
pub struct DiscoverCache {
    entries: HashMap<PathBuf, (SystemTime, u64, crate::session::SessionSummary)>,
}

impl DiscoverCache {
    /// Return a cached summary if (mtime, size) match, else call `make`,
    /// store, and return the fresh result.
    pub fn get_or_insert_with<F>(
        &mut self,
        path: &Path,
        make: F,
    ) -> std::io::Result<crate::session::SessionSummary>
    where
        F: FnOnce() -> crate::Result<crate::session::SessionSummary>,
    {
        let md = fs::metadata(path)?;
        let mtime = md.modified()?;
        let size = md.len();
        if let Some((m, s, cached)) = self.entries.get(path) {
            if *m == mtime && *s == size {
                return Ok(cached.clone());
            }
        }
        let fresh = make().map_err(|e| std::io::Error::other(e.to_string()))?;
        self.entries
            .insert(path.to_path_buf(), (mtime, size, fresh.clone()));
        Ok(fresh)
    }

    /// Drop entries for files that no longer exist in the live set.
    pub fn retain_paths(&mut self, live: &HashSet<&Path>) {
        self.entries.retain(|p, _| live.contains(p.as_path()));
    }
}
