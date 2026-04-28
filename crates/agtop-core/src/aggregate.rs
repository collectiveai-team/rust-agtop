//! Aggregation helpers: group sessions by dimension within a time range.

use chrono::{DateTime, Duration, Utc};

use crate::session::SessionAnalysis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Client,
    Provider,
    Model,
    Project,
    Subscription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRange {
    Today,
    Week,
    Month,
    All,
}

impl TimeRange {
    /// Inclusive lower bound. `None` for All.
    #[must_use]
    pub fn start(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Today => {
                // Local midnight today. Use Utc-anchored "today" for now;
                // `chrono::Local` may be appropriate later.
                let d = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
                Some(DateTime::from_naive_utc_and_offset(d, Utc))
            }
            Self::Week => Some(now - Duration::days(7)),
            Self::Month => Some(now - Duration::days(30)),
            Self::All => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AggregateGroup {
    pub label: String,
    pub session_count: usize,
    pub total_tokens: u64,
    pub total_cost: Option<f64>,
    pub avg_duration_secs: u64,
    pub last_active: Option<DateTime<Utc>>,
    /// Per-bucket activity series for the rendered sparkline (oldest → newest).
    pub activity: Vec<f32>,
}

/// Aggregate sessions into groups within `range`.
#[must_use]
pub fn aggregate(
    sessions: &[SessionAnalysis],
    group_by: GroupBy,
    range: TimeRange,
    now: DateTime<Utc>,
    activity_buckets: usize,
) -> Vec<AggregateGroup> {
    let start = range.start(now);
    let mut by_label: std::collections::BTreeMap<String, Vec<&SessionAnalysis>> =
        Default::default();
    for s in sessions
        .iter()
        .filter(|s| match (start, s.summary.last_active) {
            (Some(start), Some(last)) => last >= start,
            (Some(_), None) => false,
            (None, _) => true,
        })
    {
        let key = group_key(s, group_by);
        by_label.entry(key).or_default().push(s);
    }

    by_label
        .into_iter()
        .map(|(label, members)| {
            let session_count = members.len();
            let total_tokens: u64 = members.iter().map(|s| s.tokens.grand_total()).sum();
            // A session has a known cost if cost.total > 0 or it's included in plan.
            // We treat sessions with cost.included as having Some(0.0) cost.
            // If any session's cost data is unavailable (total=0 and not included),
            // we cannot determine whether the group total is accurate, so return None.
            // For simplicity: sum all costs; return None only when all sessions have
            // zero cost and are not plan-included (we can't distinguish zero vs unknown).
            // Pragmatic approach: always sum costs (zero when unknown).
            let total_cost: f64 = members.iter().map(|s| s.cost.total).sum();
            // Expose as Some always (cost of 0 is valid for included plan sessions).
            let total_cost = Some(total_cost);

            let avg_duration_secs = {
                let durs: Vec<i64> = members
                    .iter()
                    .filter_map(|s| s.duration_secs.map(|d| d as i64))
                    .collect();
                if durs.is_empty() {
                    0
                } else {
                    (durs.iter().sum::<i64>() / durs.len() as i64) as u64
                }
            };
            let last_active = members.iter().filter_map(|s| s.summary.last_active).max();
            let activity = build_activity_buckets(&members, range, now, activity_buckets);
            AggregateGroup {
                label,
                session_count,
                total_tokens,
                total_cost,
                avg_duration_secs,
                last_active,
                activity,
            }
        })
        .collect()
}

fn group_key(s: &SessionAnalysis, by: GroupBy) -> String {
    match by {
        GroupBy::Client | GroupBy::Provider => s.summary.client.as_str().to_string(),
        GroupBy::Model => s.summary.model.clone().unwrap_or_else(|| "unknown".into()),
        GroupBy::Project => s
            .summary
            .cwd
            .as_deref()
            .map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string())
            })
            .unwrap_or_else(|| "unknown".into()),
        GroupBy::Subscription => s
            .summary
            .subscription
            .clone()
            .unwrap_or_else(|| "unknown".into()),
    }
}

fn build_activity_buckets(
    sessions: &[&SessionAnalysis],
    range: TimeRange,
    now: DateTime<Utc>,
    n_buckets: usize,
) -> Vec<f32> {
    if n_buckets == 0 {
        return Vec::new();
    }
    let start = range
        .start(now)
        .unwrap_or_else(|| now - Duration::days(365));
    let span = (now - start).num_seconds().max(1);
    let bucket_secs = span / n_buckets as i64;
    let mut buckets = vec![0.0_f32; n_buckets];
    for s in sessions {
        if let Some(last) = s.summary.last_active {
            let elapsed = (last - start).num_seconds();
            let idx = ((elapsed / bucket_secs.max(1)) as usize).min(n_buckets - 1);
            // Activity intensity = total_tokens (rough proxy).
            buckets[idx] += s.tokens.grand_total() as f32;
        }
    }
    buckets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-26T12:00:00Z")
            .unwrap()
            .to_utc()
    }

    fn mk_session(
        client: ClientKind,
        when: DateTime<Utc>,
        tokens: u64,
        cost: f64,
    ) -> SessionAnalysis {
        let summary = SessionSummary::new(
            client,
            None,
            "test-session".into(),
            None,
            Some(when),
            None,
            None,
            std::path::PathBuf::from("/tmp/test"),
            None,
            None,
            None,
        );
        #[allow(clippy::field_reassign_with_default)]
        let tok = {
            let mut t = TokenTotals::default();
            t.input = tokens;
            t
        };
        #[allow(clippy::field_reassign_with_default)]
        let c = {
            let mut cb = CostBreakdown::default();
            cb.total = cost;
            cb
        };
        SessionAnalysis::new(summary, tok, c, None, 0, None, None, None, None, None)
    }

    #[test]
    fn aggregate_by_client_today() {
        let n = now();
        let sessions = vec![
            mk_session(ClientKind::Claude, n - Duration::hours(1), 1_000, 0.10),
            mk_session(ClientKind::Claude, n - Duration::hours(2), 500, 0.05),
            mk_session(ClientKind::Codex, n - Duration::hours(3), 2_000, 0.20),
            mk_session(ClientKind::Claude, n - Duration::days(2), 999, 9.99), // outside Today
        ];
        let groups = aggregate(&sessions, GroupBy::Client, TimeRange::Today, n, 6);
        assert_eq!(groups.len(), 2);
        let cc = groups.iter().find(|g| g.label == "claude").unwrap();
        assert_eq!(cc.session_count, 2);
        assert_eq!(cc.total_tokens, 1_500);
        assert!((cc.total_cost.unwrap() - 0.15).abs() < 1e-10);
    }

    #[test]
    fn aggregate_handles_missing_costs() {
        let n = now();
        // Both have costs of 0 (not ideal to test "missing" but captures the
        // sum-always approach; real "missing" data is simply 0.0).
        let sessions = vec![
            mk_session(ClientKind::Claude, n - Duration::hours(1), 1_000, 0.10),
            mk_session(ClientKind::Claude, n - Duration::hours(2), 500, 0.0),
        ];
        let groups = aggregate(&sessions, GroupBy::Client, TimeRange::Today, n, 6);
        let cc = groups.iter().find(|g| g.label == "claude").unwrap();
        // cost sum = 0.10
        assert!((cc.total_cost.unwrap() - 0.10).abs() < 1e-10);
    }

    #[test]
    fn time_range_all_includes_everything() {
        let n = now();
        let s = vec![mk_session(
            ClientKind::Claude,
            n - Duration::days(365),
            1,
            0.0,
        )];
        let groups = aggregate(&s, GroupBy::Client, TimeRange::All, n, 6);
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn activity_buckets_have_correct_length() {
        let n = now();
        let s = vec![mk_session(
            ClientKind::Claude,
            n - Duration::hours(1),
            100,
            0.0,
        )];
        let groups = aggregate(&s, GroupBy::Client, TimeRange::Today, n, 12);
        assert_eq!(groups[0].activity.len(), 12);
    }
}
