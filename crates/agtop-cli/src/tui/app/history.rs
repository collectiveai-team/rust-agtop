//! Rolling usage-history ring-buffer for the dashboard spark charts.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use agtop_core::session::ProviderKind;

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

/// Width of the rolling chart window, in minutes.
pub const CHART_WINDOW_MINS: i64 = 60;

/// How long to retain points — twice the chart window so we have some
/// headroom when the ring-buffer prunes old entries.
const RETENTION_SECS: i64 = CHART_WINDOW_MINS * 60 * 2;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single data point: one snapshot of per-provider cumulative token totals.
#[derive(Debug, Clone)]
pub struct UsagePoint {
    pub ts: DateTime<Utc>,
    pub tokens_by_provider: [u64; 3],
}

/// Bounded ring-buffer of [`UsagePoint`]s used to draw the spark / line charts
/// in the dashboard view.
#[derive(Debug, Default)]
pub struct UsageHistory {
    points: VecDeque<UsagePoint>,
}

impl UsageHistory {
    /// Append a new point and prune entries older than `RETENTION_SECS`.
    pub fn push(&mut self, point: UsagePoint) {
        let cutoff = point.ts - chrono::Duration::seconds(RETENTION_SECS);
        self.points.push_back(point);
        while self.points.front().is_some_and(|p| p.ts < cutoff) {
            self.points.pop_front();
        }
    }

    /// Borrow all retained points.
    pub fn points(&self) -> &VecDeque<UsagePoint> {
        &self.points
    }

    /// Aggregate the last `CHART_WINDOW_MINS` of data into `n_buckets` evenly
    /// spaced buckets for the given provider.  Each bucket holds the *maximum*
    /// token value seen in that interval (max rather than sum because individual
    /// points are already cumulative snapshots, not deltas).
    pub fn buckets_by_provider(
        &self,
        now: DateTime<Utc>,
        n_buckets: usize,
        provider: ProviderKind,
    ) -> Vec<u64> {
        self.buckets_by_provider_idx(now, n_buckets, provider_idx(provider))
    }

    fn buckets_by_provider_idx(
        &self,
        now: DateTime<Utc>,
        n_buckets: usize,
        idx: usize,
    ) -> Vec<u64> {
        if n_buckets == 0 {
            return Vec::new();
        }
        let window_secs = CHART_WINDOW_MINS * 60;
        let bucket_secs = (window_secs / n_buckets as i64).max(1);
        let window_start = now - chrono::Duration::seconds(window_secs);
        let mut out = vec![0u64; n_buckets];

        for p in &self.points {
            if p.ts < window_start {
                continue;
            }
            let age_secs = (now - p.ts).num_seconds().max(0);
            let bucket_from_end = (age_secs / bucket_secs) as usize;
            if bucket_from_end >= n_buckets {
                continue;
            }
            let bucket = n_buckets - 1 - bucket_from_end;
            let v = p.tokens_by_provider[idx];
            out[bucket] = out[bucket].max(v);
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a `ProviderKind` to the index used in `UsagePoint::tokens_by_provider`.
pub(super) fn provider_idx(kind: ProviderKind) -> usize {
    match kind {
        ProviderKind::Claude => 0,
        ProviderKind::Codex => 1,
        ProviderKind::OpenCode => 2,
        _ => usize::MAX,
    }
}
