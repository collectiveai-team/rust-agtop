//! Matching algorithm: sessions -> running OS processes.
//!
//! Three-tier strategy, run in order. Each tier only operates on
//! sessions and PIDs not already claimed by an earlier tier.
//!
//! 1. **Tier A — argv UUID** (High confidence). If a candidate's argv
//!    contains a recognized "resume this session" invocation
//!    (`claude --resume <uuid>`, `codex resume <uuid>`,
//!    `opencode -s <uuid>`, etc.) and that UUID equals an actual
//!    session's `session_id`, bind them. This works regardless of cwd,
//!    fd permissions, or the client's storage layout.
//!
//! 2. **Tier B — fd UUID-in-path** (High confidence). For sessions with
//!    UUID-shaped ids whose transcripts live in named files, scan each
//!    candidate's open file paths for that UUID as a substring. Replaces
//!    the previous "exact-path-in-fd-list" logic, which was fooled by
//!    SQLite-backed clients that share a single DB file across all
//!    sessions and stamped the same PID onto every session.
//!
//! 3. **Tier C — cwd + binary + recency** (Medium confidence). Score
//!    remaining candidates against remaining sessions on
//!    binary + cwd + start-time-overlap. Threshold 2/3. When two
//!    sessions in the same `(cwd, client)` would both win the same PID
//!    (e.g. two OpenCode windows in the same workdir), only the
//!    most-recently-active one keeps the match.
//!
//! After all three tiers run, a final safety pass enforces the hard
//! invariant that one PID maps to at most one session.

use std::collections::{HashMap, HashSet};

use crate::process::argv_uuid::{extract_session_uuid, is_valid_uuid};
use crate::process::fd::FdScanner;
use crate::process::scanner::{Candidate, Scanner};
use crate::process::transcript_paths::expected_binaries;
use crate::process::{Confidence, Liveness, ProcessInfo, ProcessMetrics};
use crate::session::{ClientKind, SessionSummary};

/// Run one correlation pass.
///
/// Does not track prior snapshots; `Liveness` is always `Live` here.
/// Stopped-state emission lives on `ProcessCorrelator` (task 9).
#[allow(dead_code)]
pub(crate) fn correlate(
    scanner: &dyn Scanner,
    fd_scanner: &dyn FdScanner,
    sessions: &[SessionSummary],
) -> HashMap<String, ProcessInfo> {
    let candidates = scanner.candidates();
    let mut out: HashMap<String, ProcessInfo> = HashMap::new();

    // PIDs already bound in an earlier (higher-confidence) tier; later
    // tiers won't reconsider these candidates.
    let mut used_pids: HashSet<u32> = HashSet::new();

    // Quick lookup: session_id -> session.
    let by_id: HashMap<&str, &SessionSummary> = sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s))
        .collect();

    // ── Tier A: argv UUID match ────────────────────────────────────────
    //
    // For each candidate, try every ClientKind. The first kind whose
    // extractor returns a UUID that matches a real, still-unmatched
    // session wins. This caps work at O(candidates * |ClientKind|).
    for c in candidates {
        if used_pids.contains(&c.pid) {
            continue;
        }
        for &kind in ClientKind::all() {
            if let Some(uuid) = extract_session_uuid(kind, &c.argv) {
                if let Some(session) = by_id.get(uuid.as_str()) {
                    if out.contains_key(&session.session_id) {
                        continue;
                    }
                    out.insert(
                        session.session_id.clone(),
                        ProcessInfo {
                            pid: c.pid,
                            liveness: Liveness::Live,
                            match_confidence: Confidence::High,
                            parent_pid: c.parent_pid,
                            metrics: c.metrics.clone(),
                        },
                    );
                    used_pids.insert(c.pid);
                    break;
                }
            }
        }
    }

    // ── Tier B: fd UUID-in-path match ─────────────────────────────────
    //
    // Only sessions with UUID-shaped ids participate. We never call
    // open_paths for already-bound PIDs.
    //
    // Build wanted_uuids from sessions still unmatched after Tier A.
    let wanted_uuids: HashMap<&str, &SessionSummary> = sessions
        .iter()
        .filter(|s| !out.contains_key(&s.session_id))
        .filter(|s| is_valid_uuid(&s.session_id))
        .map(|s| (s.session_id.as_str(), s))
        .collect();

    if !wanted_uuids.is_empty() {
        // Reverse index: uuid -> set of pids whose open paths contain
        // that uuid. If >1, treat as ambiguous (don't match in Tier B).
        let mut uuid_to_pids: HashMap<String, HashSet<u32>> = HashMap::new();
        for c in candidates {
            if used_pids.contains(&c.pid) {
                continue;
            }
            let paths = fd_scanner.open_paths(c.pid);
            for p in paths {
                let s = p.to_string_lossy();
                for uuid in find_uuids_in(&s) {
                    if wanted_uuids.contains_key(uuid.as_str()) {
                        uuid_to_pids.entry(uuid).or_default().insert(c.pid);
                    }
                }
            }
        }

        // pid -> Candidate for parent_pid lookup on bound matches.
        let cand_by_pid: HashMap<u32, &Candidate> = candidates.iter().map(|c| (c.pid, c)).collect();

        for (uuid, pids) in uuid_to_pids {
            // Module contract forbids panics (this is best-effort
            // observability). Drain the set instead of indexing + expect
            // so the compiler — not a reviewer — proves the "exactly
            // one PID" invariant.
            let mut iter = pids.into_iter();
            let pid = match (iter.next(), iter.next()) {
                (Some(pid), None) => pid, // exactly one
                _ => continue,            // zero or ambiguous
            };
            // Don't double-bind a pid that another uuid in this map
            // also pointed at (pathological: one process's open paths
            // mention two of our session UUIDs uniquely). Keep first
            // assignment, skip subsequent.
            if used_pids.contains(&pid) {
                continue;
            }
            let session = match wanted_uuids.get(uuid.as_str()) {
                Some(s) => *s,
                None => continue,
            };
            if out.contains_key(&session.session_id) {
                continue;
            }
            let c = match cand_by_pid.get(&pid) {
                Some(c) => *c,
                None => continue,
            };
            out.insert(
                session.session_id.clone(),
                ProcessInfo {
                    pid: c.pid,
                    liveness: Liveness::Live,
                    match_confidence: Confidence::High,
                    parent_pid: c.parent_pid,
                    metrics: c.metrics.clone(),
                },
            );
            used_pids.insert(c.pid);
        }
    }

    // ── Tier C: cwd + binary + start-time score ───────────────────────
    //
    // Iterate unmatched sessions, scoring against unmatched candidates.
    // Threshold 2/3 (unchanged). We collect intermediate matches first
    // so we can apply the (cwd, client) recency dedup before committing.
    //
    // Tie-breaker: when two candidates score equally for the same
    // session, prefer the descendant. Common pattern: an npm-loader
    // wrapper spawns the real CLI; both inherit the same comm/cwd/argv
    // and start a few ms apart, but only the child runs the session. By
    // preferring the candidate whose parent_pid is itself a candidate,
    // we collapse the wrapper chain to the leaf without needing argv
    // shape inspection.
    struct ScoreMatch<'a> {
        session: &'a SessionSummary,
        pid: u32,
        parent_pid: Option<u32>,
        metrics: Option<ProcessMetrics>,
    }
    let mut score_matches: Vec<ScoreMatch<'_>> = Vec::new();

    let candidate_pids: HashSet<u32> = candidates.iter().map(|c| c.pid).collect();
    let is_descendant_of_candidate =
        |c: &Candidate| matches!(c.parent_pid, Some(p) if candidate_pids.contains(&p));

    for session in sessions {
        if out.contains_key(&session.session_id) {
            continue;
        }
        let mut best: Option<(u32, &Candidate)> = None; // (score, cand)
        let mut tie = false;
        for c in candidates {
            if used_pids.contains(&c.pid) {
                continue;
            }
            let score = score_candidate(c, session);
            if score < 2 {
                continue;
            }
            match best {
                None => {
                    best = Some((score, c));
                    tie = false;
                }
                Some((s, _)) if score > s => {
                    best = Some((score, c));
                    tie = false;
                }
                Some((s, prev_c)) if score == s => {
                    // Tie-breaker 1: descendant wins (npm-loader → real CLI).
                    let cur_desc = is_descendant_of_candidate(c);
                    let prev_desc = is_descendant_of_candidate(prev_c);
                    match (cur_desc, prev_desc) {
                        (true, false) => {
                            best = Some((score, c));
                            tie = false;
                            continue;
                        }
                        (false, true) => {
                            tie = false;
                            continue;
                        }
                        _ => {}
                    }
                    // Tie-breaker 2: closer start_time to session.started_at.
                    // For per-session CLIs (`opencode run`, `claude`, etc.)
                    // the process is launched right when the session is
                    // created, so the time gap is typically <2s. The
                    // long-lived daemon's gap is minutes-to-hours.
                    if let Some(started) = session.started_at {
                        let s_t = started.timestamp();
                        let cur_gap = (c.start_time as i64 - s_t).abs();
                        let prev_gap = (prev_c.start_time as i64 - s_t).abs();
                        // Require a meaningful margin (>= 30s) so we
                        // don't crown a winner from clock-jitter noise.
                        const MARGIN: i64 = 30;
                        if cur_gap + MARGIN < prev_gap {
                            best = Some((score, c));
                            tie = false;
                            continue;
                        }
                        if prev_gap + MARGIN < cur_gap {
                            tie = false;
                            continue;
                        }
                    }
                    tie = true;
                }
                _ => {}
            }
        }
        if let (Some((_, c)), false) = (best, tie) {
            score_matches.push(ScoreMatch {
                session,
                pid: c.pid,
                parent_pid: c.parent_pid,
                metrics: c.metrics.clone(),
            });
        }
    }

    // Same-(cwd, client) recency dedup: when two sessions in the same
    // cwd+client both score-match the same PID (typical for two
    // OpenCode windows of the same project), keep only the
    // most-recently-active one.
    //
    // We group by (cwd, client, pid). Within each group, keep the
    // session with the latest `last_active` (`None` < any timestamp).
    // Sessions with `None` cwd participate per session_id (no group
    // collisions).
    let mut keep: HashSet<usize> = HashSet::new();
    {
        type GroupKey<'a> = (Option<&'a str>, ClientKind, u32);
        let mut best_in_group: HashMap<GroupKey<'_>, usize> = HashMap::new();
        for (idx, m) in score_matches.iter().enumerate() {
            let key: GroupKey<'_> = (m.session.cwd.as_deref(), m.session.client, m.pid);
            // Sessions without a cwd get a unique-per-session key so
            // they never collide with each other; using the session_id
            // pointer would be overkill — instead, only dedup when cwd
            // is Some.
            if key.0.is_none() {
                keep.insert(idx);
                continue;
            }
            match best_in_group.get(&key).copied() {
                None => {
                    best_in_group.insert(key, idx);
                }
                Some(prev) => {
                    let prev_t = score_matches[prev].session.last_active;
                    let cur_t = m.session.last_active;
                    // Newer wins; ties: keep the earlier-found (prev).
                    if cur_t > prev_t {
                        best_in_group.insert(key, idx);
                    }
                }
            }
        }
        for &idx in best_in_group.values() {
            keep.insert(idx);
        }
    }

    for (idx, m) in score_matches.into_iter().enumerate() {
        if !keep.contains(&idx) {
            continue;
        }
        if used_pids.contains(&m.pid) {
            continue;
        }
        out.insert(
            m.session.session_id.clone(),
            ProcessInfo {
                pid: m.pid,
                liveness: Liveness::Live,
                match_confidence: Confidence::Medium,
                parent_pid: m.parent_pid,
                metrics: m.metrics.clone(),
            },
        );
        used_pids.insert(m.pid);
    }

    // ── Defense in depth: one PID -> one session ──────────────────────
    //
    // Hard invariant: a single OS process runs at most one interactive
    // session. If somehow two session_ids landed on the same pid (e.g.
    // a pathological argv-tier clash with duplicate UUIDs across
    // clients), keep only the most-recently-active session.
    enforce_unique_pid(&mut out, sessions);

    out
}

/// Find every UUID-shaped substring in `s`.
///
/// Linear scan; only returns 36-char windows that pass
/// `is_valid_uuid`. Paths are short, so the per-candidate cost is
/// negligible.
fn find_uuids_in(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    if bytes.len() < 36 {
        return Vec::new();
    }
    let mut out = Vec::new();
    // Iterate every byte offset; UUIDs are pure ASCII so byte offsets
    // == char offsets within candidate windows.
    for i in 0..=bytes.len() - 36 {
        // Cheap pre-filter: hyphen positions must hold '-'.
        if bytes[i + 8] != b'-'
            || bytes[i + 13] != b'-'
            || bytes[i + 18] != b'-'
            || bytes[i + 23] != b'-'
        {
            continue;
        }
        let window = &s[i..i + 36];
        if is_valid_uuid(window) {
            out.push(window.to_string());
        }
    }
    out
}

/// Drop entries that share a PID with another entry, keeping the one
/// whose session has the most recent `last_active`. Ties are broken by
/// session_id ordering (deterministic).
fn enforce_unique_pid(out: &mut HashMap<String, ProcessInfo>, sessions: &[SessionSummary]) {
    if out.len() < 2 {
        return;
    }
    let last_active_by_id: HashMap<&str, Option<chrono::DateTime<chrono::Utc>>> = sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s.last_active))
        .collect();

    // Group session_ids by pid.
    let mut by_pid: HashMap<u32, Vec<String>> = HashMap::new();
    for (sid, info) in out.iter() {
        by_pid.entry(info.pid).or_default().push(sid.clone());
    }

    let mut to_drop: Vec<String> = Vec::new();
    for (_pid, sids) in by_pid {
        if sids.len() < 2 {
            continue;
        }
        // Pick the keeper: most recent last_active, ties by session_id.
        let mut sorted = sids;
        sorted.sort_by(|a, b| {
            let ta = last_active_by_id.get(a.as_str()).copied().flatten();
            let tb = last_active_by_id.get(b.as_str()).copied().flatten();
            // Reverse order: newest first.
            tb.cmp(&ta).then_with(|| a.cmp(b))
        });
        // Keep sorted[0]; drop the rest.
        for sid in sorted.into_iter().skip(1) {
            to_drop.push(sid);
        }
    }
    for sid in to_drop {
        out.remove(&sid);
    }
}

/// Score a candidate process against a session. Each criterion adds 1.
/// Returns 0..=3.
///
/// **cwd mismatch is a deal-breaker.** When both candidate and session
/// have a known cwd, they must agree exactly; otherwise we return 0
/// (regardless of binary/time match). This stops the binary+time
/// combination from match-laundering across unrelated cwds — common
/// when a user has, say, a daemon in `/tmp/foo` and an old session in
/// `/home/user/proj` whose only signal is "an opencode is running and
/// the session was active recently". Empirically this was the cause of
/// stale OpenCode sessions stealing daemon PIDs from the active ones.
fn score_candidate(c: &Candidate, s: &SessionSummary) -> u32 {
    // Hard cwd gate: when both sides know their cwd, they must match.
    if let (Some(cc), Some(sc)) = (&c.cwd, &s.cwd) {
        if cc.as_os_str() != std::ffi::OsStr::new(sc) {
            return 0;
        }
    }

    let mut score = 0;

    // Binary matches an expected name for this client.
    if expected_binaries(s.client).iter().any(|&b| c.binary == b) {
        score += 1;
    }

    // cwd exact match (counted only when both sides know it).
    if let (Some(cc), Some(sc)) = (&c.cwd, &s.cwd) {
        if cc.as_os_str() == std::ffi::OsStr::new(sc) {
            score += 1;
        }
    }

    // Process start time falls inside the session's observed window.
    if let (Some(started), Some(last_active)) = (s.started_at, s.last_active) {
        let started = started.timestamp() as u64;
        let last_active = last_active.timestamp() as u64;
        if c.start_time >= started && c.start_time <= last_active {
            score += 1;
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::fd::tests::FakeFdScanner;
    use crate::process::scanner::tests::FakeScanner;
    use crate::session::ClientKind;
    use chrono::{Duration, Utc};
    use std::path::PathBuf;

    const UUID_A: &str = "11111111-1111-4111-8111-111111111111";
    const UUID_B: &str = "22222222-2222-4222-8222-222222222222";

    fn claude_session(id: &str, path: &str) -> SessionSummary {
        SessionSummary::new(
            ClientKind::Claude,
            None,
            id.into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from(path),
            None,
            None,
            None,
        )
    }

    fn candidate(pid: u32, binary: &str, cwd: &str) -> Candidate {
        Candidate {
            pid,
            parent_pid: Some(1),
            binary: binary.into(),
            argv: vec![binary.into()],
            cwd: Some(PathBuf::from(cwd)),
            start_time: 1700000000,
            metrics: None,
        }
    }

    // ── Existing tier-B / tier-C behaviors ────────────────────────────

    #[test]
    fn fd_match_produces_high_confidence() {
        // Tier B now matches by UUID-in-path. Use a UUID-shaped session id
        // so the new fd-tier signal applies.
        let scanner = FakeScanner {
            processes: vec![candidate(42, "claude", "/home/user/proj")],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(42u32, vec![PathBuf::from(format!("/tmp/{UUID_A}.jsonl"))]);
        let fd = FakeFdScanner { map: fd_map };

        let sessions = vec![claude_session(UUID_A, &format!("/tmp/{UUID_A}.jsonl"))];
        let result = correlate(&scanner, &fd, &sessions);

        let info = result.get(UUID_A).expect("session must be matched");
        assert_eq!(info.pid, 42);
        assert_eq!(info.liveness, Liveness::Live);
        assert_eq!(info.match_confidence, Confidence::High);
    }

    #[test]
    fn fd_match_disambiguates_two_claudes_in_same_cwd() {
        // Two claude processes in the same cwd. Only one holds the
        // UUID-named transcript path open per session.
        let scanner = FakeScanner {
            processes: vec![
                candidate(42, "claude", "/home/user/proj"),
                candidate(43, "claude", "/home/user/proj"),
            ],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(42u32, vec![PathBuf::from(format!("/tmp/{UUID_A}.jsonl"))]);
        fd_map.insert(43u32, vec![PathBuf::from(format!("/tmp/{UUID_B}.jsonl"))]);
        let fd = FakeFdScanner { map: fd_map };

        let sessions = vec![
            claude_session(UUID_A, &format!("/tmp/{UUID_A}.jsonl")),
            claude_session(UUID_B, &format!("/tmp/{UUID_B}.jsonl")),
        ];
        let result = correlate(&scanner, &fd, &sessions);

        assert_eq!(result.get(UUID_A).map(|i| i.pid), Some(42));
        assert_eq!(result.get(UUID_B).map(|i| i.pid), Some(43));
    }

    #[test]
    fn no_fd_match_falls_back_to_score_tier() {
        // No fd match; binary + cwd match => score 2 => Medium confidence.
        let scanner = FakeScanner {
            processes: vec![candidate(42, "claude", "/home/user/proj")],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        let sessions = vec![claude_session("s1", "/tmp/s1.jsonl")];
        let result = correlate(&scanner, &fd, &sessions);
        let info = result.get("s1").expect("s1 must be matched by score tier");
        assert_eq!(info.pid, 42);
        assert_eq!(info.match_confidence, Confidence::Medium);
    }

    #[test]
    fn score_tier_matches_medium_confidence_when_unique() {
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::Claude,
            None,
            "s1".into(),
            Some(now - Duration::minutes(10)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/tmp/s1.jsonl"),
            None,
            None,
            None,
        );
        let mut c = candidate(42, "claude", "/home/user/proj");
        c.start_time = (now - Duration::minutes(5)).timestamp() as u64;

        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };

        let result = correlate(&scanner, &fd, &[s]);
        let info = result.get("s1").expect("s1 must be matched");
        assert_eq!(info.pid, 42);
        assert_eq!(info.match_confidence, Confidence::Medium);
    }

    #[test]
    fn score_tier_breaks_tie_by_closer_start_time() {
        // Two opencode candidates in the same cwd, neither a descendant
        // of the other (sibling processes, e.g. an existing daemon and
        // a fresh `opencode run`). Score equally on (binary, cwd, time).
        // Tie-breaker: the candidate whose start_time is closer to the
        // session's started_at wins.
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "ses_targeted".into(),
            Some(now - Duration::seconds(5)), // session just started
            Some(now),
            None,
            Some("/tmp/proj".into()),
            PathBuf::from("/.local/share/opencode/opencode.db"),
            None,
            None,
            None,
        );
        // Daemon: started 30 minutes ago.
        let mut daemon = candidate(100, "opencode", "/tmp/proj");
        daemon.parent_pid = Some(1);
        daemon.start_time = (now - Duration::minutes(30)).timestamp() as u64;
        // Fresh run client: started ~3s before the session_started_at
        // (well within the session's window so the +1 time bonus also
        // fires for both, leaving us at score==2 either way).
        let mut fresh = candidate(101, "opencode", "/tmp/proj");
        fresh.parent_pid = Some(2);
        fresh.start_time = (now - Duration::seconds(8)).timestamp() as u64;

        let scanner = FakeScanner {
            processes: vec![daemon, fresh],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };

        let result = correlate(&scanner, &fd, &[s]);
        let info = result
            .get("ses_targeted")
            .expect("closer-start-time candidate must win the tie");
        assert_eq!(info.pid, 101, "fresh run client must beat the old daemon");
    }

    #[test]
    fn score_tier_breaks_tie_in_favor_of_descendant() {
        // Wrapper pattern: a parent process and its child both have the
        // same comm/cwd/argv (e.g. npm-loader → real gemini node). Both
        // score 2 against the session. The descendant must win.
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::Claude,
            None,
            "s1".into(),
            Some(now - Duration::minutes(10)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/tmp/s1.jsonl"),
            None,
            None,
            None,
        );
        // PID 100 is the parent (wrapper); PID 101 is the child whose
        // parent_pid points back at PID 100.
        let mut parent = candidate(100, "claude", "/home/user/proj");
        parent.parent_pid = Some(1);
        parent.start_time = (now - Duration::minutes(5)).timestamp() as u64;
        let mut child = candidate(101, "claude", "/home/user/proj");
        child.parent_pid = Some(100);
        child.start_time = (now - Duration::minutes(5)).timestamp() as u64;

        let scanner = FakeScanner {
            processes: vec![parent, child],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };

        let result = correlate(&scanner, &fd, &[s]);
        let info = result
            .get("s1")
            .expect("descendant must win the tie and produce a match");
        assert_eq!(info.pid, 101, "child PID must win, not parent wrapper");
        assert_eq!(info.match_confidence, Confidence::Medium);
    }

    #[test]
    fn score_tier_refuses_ambiguous_tie() {
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::Claude,
            None,
            "s1".into(),
            Some(now - Duration::minutes(10)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/tmp/s1.jsonl"),
            None,
            None,
            None,
        );
        // Both candidates within the time tie-breaker margin (30s) of
        // each other relative to session.started_at, so neither wins
        // and the match is correctly refused.
        let mut c1 = candidate(42, "claude", "/home/user/proj");
        let mut c2 = candidate(43, "claude", "/home/user/proj");
        c1.start_time = (now - Duration::minutes(5)).timestamp() as u64;
        c2.start_time = (now - Duration::minutes(5) - Duration::seconds(10)).timestamp() as u64;

        let scanner = FakeScanner {
            processes: vec![c1, c2],
        };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };

        let result = correlate(&scanner, &fd, &[s]);
        assert!(result.is_empty(), "ambiguous match should not be emitted");
    }

    #[test]
    fn score_tier_rejects_cwd_mismatch_even_when_binary_and_time_agree() {
        // Regression: a long-running daemon in cwd A must NOT match a
        // session in cwd B just because both have the same binary and
        // the daemon started during the session's active window.
        // Empirically this was the cause of a fresh `opencode serve`
        // in /tmp/* stealing matches from sessions in /home/user/proj.
        let now = Utc::now();
        let s = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "ses_in_proj".into(),
            Some(now - Duration::hours(2)),
            Some(now),
            None,
            Some("/home/user/proj".into()),
            PathBuf::from("/.local/share/opencode/opencode.db"),
            None,
            None,
            None,
        );
        // Daemon in a totally unrelated cwd, started during the session
        // window — would have scored 2 (binary + time) under the prior
        // additive scheme. Must now score 0.
        let mut c = candidate(900, "opencode", "/tmp/elsewhere");
        c.start_time = (now - Duration::minutes(5)).timestamp() as u64;

        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        let result = correlate(&scanner, &fd, &[s]);
        assert!(
            result.is_empty(),
            "cwd-mismatched daemon must not match: {result:?}"
        );
    }

    #[test]
    fn score_tier_rejects_low_score() {
        let s = claude_session("s1", "/tmp/s1.jsonl");
        let c = candidate(42, "claude", "/elsewhere");
        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        let result = correlate(&scanner, &fd, &[s]);
        assert!(result.is_empty());
    }

    // ── Tier A: argv UUID match ───────────────────────────────────────

    #[test]
    fn argv_tier_matches_high_confidence_with_no_fd_or_cwd() {
        // Candidate runs `claude --resume <uuid>` from /elsewhere with
        // no fds. Score tier alone could not match (cwd mismatch).
        let mut c = candidate(99, "claude", "/elsewhere");
        c.argv = vec!["claude".into(), "--resume".into(), UUID_A.into()];
        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };
        // Session with cwd /home/user/proj (mismatched on purpose).
        let s = claude_session(UUID_A, &format!("/tmp/{UUID_A}.jsonl"));
        let result = correlate(&scanner, &fd, &[s]);
        let info = result
            .get(UUID_A)
            .expect("argv-tier must match by session UUID");
        assert_eq!(info.pid, 99);
        assert_eq!(info.match_confidence, Confidence::High);
    }

    #[test]
    fn argv_tier_wins_when_fd_would_match_too() {
        // Candidate has both: argv resume UUID and fd contains the same
        // UUID. Either path is High; confirm exactly one match, no
        // panic, no double-bind.
        let mut c = candidate(7, "claude", "/home/user/proj");
        c.argv = vec!["claude".into(), "--resume".into(), UUID_A.into()];
        let scanner = FakeScanner { processes: vec![c] };
        let mut fd_map = HashMap::new();
        fd_map.insert(7u32, vec![PathBuf::from(format!("/tmp/{UUID_A}.jsonl"))]);
        let fd = FakeFdScanner { map: fd_map };
        let s = claude_session(UUID_A, &format!("/tmp/{UUID_A}.jsonl"));
        let result = correlate(&scanner, &fd, &[s]);
        assert_eq!(result.len(), 1);
        let info = result.get(UUID_A).expect("must match");
        assert_eq!(info.pid, 7);
        assert_eq!(info.match_confidence, Confidence::High);
    }

    // ── Tier B: fd UUID-in-any-path ───────────────────────────────────

    #[test]
    fn fd_tier_matches_uuid_in_arbitrary_path() {
        // Session's data_path is one place; the candidate has the
        // session UUID open under a TOTALLY DIFFERENT directory. The
        // fd-tier should still match because the UUID appears in some
        // open path.
        let scanner = FakeScanner {
            processes: vec![candidate(31, "claude", "/elsewhere")],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(
            31u32,
            vec![PathBuf::from(format!(
                "/some/totally/different/dir/{UUID_A}.jsonl"
            ))],
        );
        let fd = FakeFdScanner { map: fd_map };

        let sessions = vec![claude_session(
            UUID_A,
            &format!("/canonical/path/to/{UUID_A}.jsonl"),
        )];
        let result = correlate(&scanner, &fd, &sessions);

        let info = result
            .get(UUID_A)
            .expect("fd-tier must match by uuid substring, not exact path");
        assert_eq!(info.pid, 31);
        assert_eq!(info.match_confidence, Confidence::High);
    }

    // ── Tier C: cwd+client recency dedup ──────────────────────────────

    #[test]
    fn cwd_recency_dedup_keeps_only_most_recent_session() {
        // Two OpenCode sessions in the SAME cwd, different last_active.
        // One opencode candidate in that cwd. Score tier would match
        // both; recency dedup must keep only the newer.
        let now = Utc::now();
        let cwd = "/home/user/proj";
        let older = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "older".into(),
            Some(now - Duration::hours(2)),
            Some(now - Duration::hours(1)),
            None,
            Some(cwd.into()),
            PathBuf::from("/.local/share/opencode/opencode.db"),
            None,
            None,
            None,
        );
        let newer = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "newer".into(),
            Some(now - Duration::minutes(30)),
            Some(now - Duration::minutes(1)),
            None,
            Some(cwd.into()),
            PathBuf::from("/.local/share/opencode/opencode.db"),
            None,
            None,
            None,
        );

        let mut c = candidate(50, "opencode", cwd);
        // In the time window of `newer`.
        c.start_time = (now - Duration::minutes(15)).timestamp() as u64;

        let scanner = FakeScanner { processes: vec![c] };
        let fd = FakeFdScanner {
            map: HashMap::new(),
        };

        let result = correlate(&scanner, &fd, &[older, newer]);
        assert_eq!(
            result.get("newer").map(|i| i.pid),
            Some(50),
            "newer session must win the cwd/client group"
        );
        assert!(
            !result.contains_key("older"),
            "older session must not be matched: {result:?}"
        );
    }

    // ── Defense in depth: same pid never bound twice ──────────────────

    #[test]
    fn enforce_unique_pid_drops_older_when_pid_collides() {
        // Direct unit test on the helper: two entries pointing at the
        // same pid; the older session must be dropped.
        let now = Utc::now();
        let older = SessionSummary::new(
            ClientKind::Claude,
            None,
            "older".into(),
            Some(now - Duration::hours(1)),
            Some(now - Duration::hours(1)),
            None,
            None,
            PathBuf::from("/tmp/o.jsonl"),
            None,
            None,
            None,
        );
        let newer = SessionSummary::new(
            ClientKind::Claude,
            None,
            "newer".into(),
            Some(now - Duration::minutes(1)),
            Some(now),
            None,
            None,
            PathBuf::from("/tmp/n.jsonl"),
            None,
            None,
            None,
        );

        let mut out: HashMap<String, ProcessInfo> = HashMap::new();
        out.insert(
            "older".into(),
            ProcessInfo {
                pid: 100,
                liveness: Liveness::Live,
                match_confidence: Confidence::High,
                parent_pid: None,
                metrics: None,
            },
        );
        out.insert(
            "newer".into(),
            ProcessInfo {
                pid: 100,
                liveness: Liveness::Live,
                match_confidence: Confidence::High,
                parent_pid: None,
                metrics: None,
            },
        );

        enforce_unique_pid(&mut out, &[older, newer]);
        assert!(out.contains_key("newer"));
        assert!(!out.contains_key("older"));
    }

    // ── OpenCode SQLite sanity ────────────────────────────────────────

    #[test]
    fn opencode_sqlite_db_does_not_fan_out_via_fd_tier() {
        // Two OpenCode sessions sharing the same DB data_path, and an
        // opencode daemon that has the DB open. The fd tier MUST NOT
        // bind that one PID to BOTH sessions.
        //
        // (cwd-tier may still bind one of them depending on the cwd
        // setup; that's fine. Test specifically asserts the daemon
        // isn't fanned out across sessions.)
        let db = "/.local/share/opencode/opencode.db";
        let s1 = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "11111111-1111-4111-8111-aaaaaaaaaaaa".into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            Some("/home/user/projA".into()),
            PathBuf::from(db),
            None,
            None,
            None,
        );
        let s2 = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "22222222-2222-4222-8222-bbbbbbbbbbbb".into(),
            Some(Utc::now()),
            Some(Utc::now()),
            None,
            Some("/home/user/projB".into()),
            PathBuf::from(db),
            None,
            None,
            None,
        );

        // Single opencode process in /elsewhere; no cwd match for
        // either session, so cwd-tier is also out. Argv plain (no -s).
        let scanner = FakeScanner {
            processes: vec![candidate(77, "opencode", "/elsewhere")],
        };
        let mut fd_map = HashMap::new();
        fd_map.insert(77u32, vec![PathBuf::from(db)]);
        let fd = FakeFdScanner { map: fd_map };

        let result = correlate(&scanner, &fd, &[s1, s2]);
        // Neither session should be matched via fd-tier (paths_for
        // returns empty for OpenCode), and cwd-tier doesn't apply.
        assert!(
            !result.values().any(|i| i.match_confidence == Confidence::High),
            "no high-confidence (fd-tier) match should be emitted for OpenCode SQLite, got {result:?}"
        );
    }

    // ── Pure helper tests ─────────────────────────────────────────────

    #[test]
    fn find_uuids_in_extracts_canonical_uuids() {
        let s = format!("/foo/{UUID_A}.jsonl");
        let found = find_uuids_in(&s);
        assert_eq!(found, vec![UUID_A.to_string()]);
    }

    #[test]
    fn find_uuids_in_returns_empty_when_no_uuid() {
        assert!(find_uuids_in("/no/uuid/here.txt").is_empty());
        assert!(find_uuids_in("").is_empty());
        assert!(find_uuids_in("short").is_empty());
    }

    #[test]
    fn tier_a_copies_candidate_metrics_to_process_info() {
        let session = claude_session(UUID_A, &format!("/tmp/{UUID_A}.jsonl"));
        let metrics = ProcessMetrics {
            cpu_percent: 7.0,
            memory_bytes: 10,
            virtual_memory_bytes: 20,
            disk_read_bytes: 30,
            disk_written_bytes: 40,
            disk_read_bytes_per_sec: 0.0,
            disk_written_bytes_per_sec: 0.0,
        };
        let scanner = FakeScanner {
            processes: vec![Candidate {
                pid: 42,
                parent_pid: None,
                binary: "claude".to_string(),
                argv: vec![
                    "claude".to_string(),
                    "--resume".to_string(),
                    UUID_A.to_string(),
                ],
                cwd: Some(std::path::PathBuf::from("/home/user/proj")),
                start_time: Utc::now().timestamp() as u64,
                metrics: Some(metrics.clone()),
            }],
        };
        let fd_scanner = FakeFdScanner::default();
        let out = correlate(&scanner, &fd_scanner, &[session]);
        let info = out.get(UUID_A).expect("should have matched");
        assert_eq!(
            info.metrics.as_ref().map(|m| m.disk_written_bytes),
            Some(40)
        );
        assert_eq!(info.metrics.as_ref().map(|m| m.cpu_percent), Some(7.0));
    }

    #[test]
    fn tier_b_copies_candidate_metrics_to_process_info() {
        let session = claude_session(UUID_A, &format!("/tmp/{UUID_A}.jsonl"));
        let metrics = ProcessMetrics {
            cpu_percent: 5.0,
            memory_bytes: 100,
            virtual_memory_bytes: 200,
            disk_read_bytes: 300,
            disk_written_bytes: 400,
            disk_read_bytes_per_sec: 0.0,
            disk_written_bytes_per_sec: 0.0,
        };
        // Candidate has a different argv so Tier A won't match.
        let scanner = FakeScanner {
            processes: vec![Candidate {
                pid: 99,
                parent_pid: None,
                binary: "claude".to_string(),
                argv: vec!["claude".to_string()],
                cwd: Some(std::path::PathBuf::from("/home/user/proj")),
                start_time: Utc::now().timestamp() as u64,
                metrics: Some(metrics.clone()),
            }],
        };
        // fd_scanner returns the session transcript path for pid 99.
        let fd_scanner = FakeFdScanner {
            map: [(
                99u32,
                vec![std::path::PathBuf::from(format!(
                    "/home/.claude/{UUID_A}.jsonl"
                ))],
            )]
            .into(),
        };
        let out = correlate(&scanner, &fd_scanner, &[session]);
        let info = out.get(UUID_A).expect("should have matched via Tier B");
        assert_eq!(
            info.metrics.as_ref().map(|m| m.disk_written_bytes),
            Some(400)
        );
        assert_eq!(info.metrics.as_ref().map(|m| m.cpu_percent), Some(5.0));
    }

    #[test]
    fn tier_c_copies_candidate_metrics_to_process_info() {
        // Session needs a cwd so the cwd hard gate passes.
        let mut session = claude_session(UUID_A, &format!("/tmp/{UUID_A}.jsonl"));
        session.cwd = Some("/home/user/proj".into());
        // started_at must be set so the time window criterion can fire.
        let start = Utc::now() - Duration::seconds(10);
        session.started_at = Some(start);
        session.last_active = Some(Utc::now());

        let metrics = ProcessMetrics {
            cpu_percent: 9.0,
            memory_bytes: 111,
            virtual_memory_bytes: 222,
            disk_read_bytes: 333,
            disk_written_bytes: 444,
            disk_read_bytes_per_sec: 0.0,
            disk_written_bytes_per_sec: 0.0,
        };
        // Binary matches, cwd matches, start_time inside window => score 3.
        let scanner = FakeScanner {
            processes: vec![Candidate {
                pid: 77,
                parent_pid: None,
                binary: "claude".to_string(),
                argv: vec!["claude".to_string()],
                cwd: Some(std::path::PathBuf::from("/home/user/proj")),
                start_time: start.timestamp() as u64 + 1,
                metrics: Some(metrics.clone()),
            }],
        };
        let fd_scanner = FakeFdScanner::default();
        let out = correlate(&scanner, &fd_scanner, &[session]);
        let info = out.get(UUID_A).expect("should have matched via Tier C");
        assert_eq!(
            info.metrics.as_ref().map(|m| m.disk_written_bytes),
            Some(444)
        );
        assert_eq!(info.metrics.as_ref().map(|m| m.cpu_percent), Some(9.0));
    }
}
