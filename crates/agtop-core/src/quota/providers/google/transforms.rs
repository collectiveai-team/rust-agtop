//! Transforms that convert Google response shapes into UsageWindows.
//!
//! Two response shapes are transformed:
//! - `:retrieveUserQuota` bucket → model-scoped daily window.
//! - `:fetchAvailableModels` per-model quotaInfo → window with the source's
//!   picker (Gemini = always daily; Antigravity = daily if >10h to reset,
//!   else 5h).
//!
//! Refresh tokens have an openchamber-specific encoding:
//!   `"<token>|<projectId>|<managedProjectId>"`
//! with optional trailing segments.

use super::auth::SourceId;
use crate::quota::time::{clamp_percent, iso_to_epoch_ms};
use crate::quota::types::UsageWindow;
use indexmap::IndexMap;
use serde::Deserialize;

const DAILY_SECONDS: u64 = 86_400;
const FIVE_HOUR_SECONDS: u64 = 5 * 3_600;
const TEN_HOURS_MS: i64 = 10 * 3_600 * 1000;

#[derive(Debug, Clone, Default)]
pub struct RefreshTokenParts {
    pub refresh_token: Option<String>,
    pub project_id: Option<String>,
    pub managed_project_id: Option<String>,
}

pub fn parse_refresh_token(raw: Option<&str>) -> RefreshTokenParts {
    let raw = match raw {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return RefreshTokenParts::default(),
    };
    let mut parts = raw.split('|');
    let refresh = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());
    let project = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());
    let managed = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());
    RefreshTokenParts {
        refresh_token: refresh.map(str::to_string),
        project_id: project.map(str::to_string),
        managed_project_id: managed.map(str::to_string),
    }
}

/// Returns `(label, seconds)` for the window emitted by this source.
/// `reset_at_ms` is the reset timestamp for this bucket (ms since epoch).
/// `now_ms` is the reference time for Antigravity's >10h check.
pub fn resolve_window(source_id: SourceId, reset_at_ms: Option<i64>, now_ms: i64) -> (String, u64) {
    match source_id {
        SourceId::Gemini => ("daily".to_string(), DAILY_SECONDS),
        SourceId::Antigravity => match reset_at_ms {
            Some(reset) if reset - now_ms > TEN_HOURS_MS => ("daily".to_string(), DAILY_SECONDS),
            _ => ("5h".to_string(), FIVE_HOUR_SECONDS),
        },
    }
}

// ---------- Response shapes for transforms_* functions ----------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RetrieveUserQuotaResponse {
    pub buckets: Vec<QuotaBucket>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct QuotaBucket {
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
    #[serde(rename = "quotaId")]
    pub quota_id: Option<String>,
    #[serde(rename = "remainingAmount")]
    pub remaining_amount: Option<String>,
    #[serde(rename = "remainingFraction")]
    pub remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    pub reset_time: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FetchModelsResponse {
    pub models: IndexMap<String, ModelData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ModelData {
    #[serde(rename = "quotaInfo")]
    pub quota_info: Option<QuotaInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct QuotaInfo {
    #[serde(rename = "remainingFraction")]
    pub remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    pub reset_time: Option<String>,
}

// ---------- loadCodeAssist response ----------
//
// Shape (observed against a free-tier account on cloudcode-pa.googleapis.com):
//
// {
//   "currentTier":            { "id": "free-tier", "name": "...", ... },
//   "allowedTiers":           [ { "id": "free-tier", "isDefault": true }, ... ],
//   "cloudaicompanionProject": "<project-id>",
//   "paidTier":               { "id": "...", "name": "...",
//                               "availableCredits": [ { ... } ] }  // optional
// }

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LoadCodeAssistResponse {
    #[serde(rename = "currentTier")]
    pub current_tier: Option<TierInfo>,
    #[serde(rename = "cloudaicompanionProject")]
    pub cloudaicompanion_project: Option<String>,
    #[serde(rename = "paidTier")]
    pub paid_tier: Option<PaidTierInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TierInfo {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PaidTierInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "availableCredits")]
    pub available_credits: Option<Vec<AvailableCredit>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AvailableCredit {
    /// Credit amount. The API returns this as a string (e.g. "100000").
    #[serde(rename = "creditAmount")]
    pub credit_amount: Option<String>,
    #[serde(rename = "creditType")]
    pub credit_type: Option<String>,
}

/// Translate a `QuotaBucket` into a scoped (model, window_label, UsageWindow) triple.
/// Returns None if `modelId` is missing (we can't scope the entry).
pub fn transform_quota_bucket(
    bucket: &QuotaBucket,
    source_id: SourceId,
    now_ms: i64,
) -> Option<(String, String, UsageWindow)> {
    let model_id = bucket
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let scoped = scope_name(source_id, model_id);
    let reset_at = bucket.reset_time.as_deref().and_then(iso_to_epoch_ms);
    let used_percent = bucket
        .remaining_fraction
        .map(|f| (1.0 - f) * 100.0)
        .map(|v| clamp_percent(Some(v)).unwrap_or(v));
    let value_label = quota_value_label(
        bucket.remaining_amount.as_deref(),
        bucket.remaining_fraction,
    );
    let (label, seconds) = resolve_window(source_id, reset_at, now_ms);
    Some((
        scoped,
        label,
        UsageWindow {
            used_percent,
            window_seconds: Some(seconds),
            reset_at,
            value_label,
        },
    ))
}

fn quota_value_label(
    remaining_amount: Option<&str>,
    remaining_fraction: Option<f64>,
) -> Option<String> {
    let remaining = remaining_amount?.trim().parse::<u64>().ok()?;
    let fraction = remaining_fraction?;
    if !fraction.is_finite() || fraction <= 0.0 {
        return Some(format!("{remaining} left"));
    }
    let limit = (remaining as f64 / fraction).round() as u64;
    if limit == 0 {
        Some(format!("{remaining} left"))
    } else {
        Some(format!("{remaining} / {limit} left"))
    }
}

/// Translate a `ModelData` into a scoped (model, window_label, UsageWindow)
/// triple using the model's raw name from the `models` map key.
pub fn transform_model_data(
    model_name: &str,
    data: &ModelData,
    source_id: SourceId,
    now_ms: i64,
) -> (String, String, UsageWindow) {
    let scoped = scope_name(source_id, model_name);
    let info = data.quota_info.as_ref();
    let reset_at = info
        .and_then(|i| i.reset_time.as_deref())
        .and_then(iso_to_epoch_ms);
    let used_percent = info
        .and_then(|i| i.remaining_fraction)
        .map(|f| (1.0 - f) * 100.0)
        .and_then(|v| clamp_percent(Some(v)));
    let (label, seconds) = resolve_window(source_id, reset_at, now_ms);
    (
        scoped,
        label,
        UsageWindow {
            used_percent,
            window_seconds: Some(seconds),
            reset_at,
            value_label: None,
        },
    )
}

fn scope_name(source_id: SourceId, model_name: &str) -> String {
    let prefix = source_id.label();
    if model_name.starts_with(&format!("{prefix}/")) {
        model_name.to_string()
    } else {
        format!("{prefix}/{model_name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW_MS: i64 = 1_777_075_200_000; // 2026-04-21T12:00:00Z

    #[test]
    fn parse_refresh_all_three_parts() {
        let p = parse_refresh_token(Some("TOK|proj-a|mproj-a"));
        assert_eq!(p.refresh_token.as_deref(), Some("TOK"));
        assert_eq!(p.project_id.as_deref(), Some("proj-a"));
        assert_eq!(p.managed_project_id.as_deref(), Some("mproj-a"));
    }

    #[test]
    fn parse_refresh_token_only() {
        let p = parse_refresh_token(Some("TOK"));
        assert_eq!(p.refresh_token.as_deref(), Some("TOK"));
        assert_eq!(p.project_id, None);
        assert_eq!(p.managed_project_id, None);
    }

    #[test]
    fn parse_refresh_empty_segment_yields_none() {
        let p = parse_refresh_token(Some("TOK||mproj"));
        assert_eq!(p.refresh_token.as_deref(), Some("TOK"));
        assert_eq!(p.project_id, None);
        assert_eq!(p.managed_project_id.as_deref(), Some("mproj"));
    }

    #[test]
    fn parse_refresh_none_or_empty() {
        assert!(parse_refresh_token(None).refresh_token.is_none());
        assert!(parse_refresh_token(Some("")).refresh_token.is_none());
        assert!(parse_refresh_token(Some("   ")).refresh_token.is_none());
    }

    #[test]
    fn gemini_window_always_daily() {
        let (l, s) = resolve_window(SourceId::Gemini, None, NOW_MS);
        assert_eq!(l, "daily");
        assert_eq!(s, DAILY_SECONDS);

        let (l, s) = resolve_window(SourceId::Gemini, Some(NOW_MS + 1_000), NOW_MS);
        assert_eq!(l, "daily");
        assert_eq!(s, DAILY_SECONDS);
    }

    #[test]
    fn antigravity_window_picks_5h_when_reset_imminent() {
        // 3 hours from now → 5h window.
        let reset = NOW_MS + 3 * 3_600 * 1000;
        let (l, s) = resolve_window(SourceId::Antigravity, Some(reset), NOW_MS);
        assert_eq!(l, "5h");
        assert_eq!(s, FIVE_HOUR_SECONDS);
    }

    #[test]
    fn antigravity_window_picks_daily_when_far() {
        // 20 hours from now → daily.
        let reset = NOW_MS + 20 * 3_600 * 1000;
        let (l, s) = resolve_window(SourceId::Antigravity, Some(reset), NOW_MS);
        assert_eq!(l, "daily");
        assert_eq!(s, DAILY_SECONDS);
    }

    #[test]
    fn transform_bucket_scopes_model_name() {
        let b = QuotaBucket {
            model_id: Some("gemini-2.5-pro".into()),
            remaining_amount: Some("62".into()),
            remaining_fraction: Some(0.62),
            reset_time: Some("2026-04-22T00:00:00Z".into()),
            ..QuotaBucket::default()
        };
        let (scoped, label, window) = transform_quota_bucket(&b, SourceId::Gemini, NOW_MS).unwrap();
        assert_eq!(scoped, "gemini/gemini-2.5-pro");
        assert_eq!(label, "daily");
        assert_eq!(window.used_percent, Some(38.0));
        assert_eq!(window.value_label.as_deref(), Some("62 / 100 left"));
    }

    #[test]
    fn transform_bucket_preserves_already_scoped_name() {
        let b = QuotaBucket {
            model_id: Some("gemini/gemini-2.0-flash".into()),
            remaining_fraction: Some(0.4),
            ..QuotaBucket::default()
        };
        let (scoped, _, _) = transform_quota_bucket(&b, SourceId::Gemini, NOW_MS).unwrap();
        assert_eq!(scoped, "gemini/gemini-2.0-flash");
    }

    #[test]
    fn transform_bucket_skips_missing_model_id() {
        let b = QuotaBucket {
            model_id: None,
            ..QuotaBucket::default()
        };
        assert!(transform_quota_bucket(&b, SourceId::Gemini, NOW_MS).is_none());
    }
}
