//! Google (Gemini CLI + Antigravity) quota provider.
//!
//! See spec section 3.5 for the full behaviour. Key points:
//! - Reads credentials from TWO sources (Gemini CLI + Antigravity) and
//!   collects results from both. If at least one source succeeds, the
//!   overall result is ok=true with partial data.
//! - Gemini source calls `:retrieveUserQuota` and merges the result.
//!   Both sources call `:fetchAvailableModels` (with a 3-URL fallback chain).
//! - NO token refresh. If the stored access token is expired, the API
//!   returns 401 and we surface it as ErrorKind::Http{401}. The TUI's
//!   last-known-good policy provides the UX buffer.
//! - Per-model windows emitted under `Usage.models`, keyed by
//!   `<sourceId>/<modelName>`.

pub mod api;
pub mod auth;
pub mod transforms;

pub use auth::{AuthSource, SourceId, DEFAULT_PROJECT_ID};
pub use transforms::{parse_refresh_token, resolve_window, RefreshTokenParts};

use crate::quota::auth::OpencodeAuth;
use crate::quota::http::HttpClient;
use crate::quota::providers::Provider;
use crate::quota::types::{ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageWindow};
use indexmap::IndexMap;
use std::collections::BTreeMap;

const PROVIDER_ID: ProviderId = ProviderId::Google;
const PROVIDER_NAME: &str = "Google";

pub struct Google;

impl Provider for Google {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }
    fn display_name(&self) -> &'static str {
        PROVIDER_NAME
    }
    fn is_configured(&self, auth: &OpencodeAuth) -> bool {
        !auth::resolve_sources(auth).is_empty()
    }
    fn fetch(&self, auth: &OpencodeAuth, http: &dyn HttpClient) -> ProviderResult {
        fetch_impl(auth, http, now_epoch_ms())
    }
}

fn now_epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn fetch_impl(auth: &OpencodeAuth, http: &dyn HttpClient, now_ms: i64) -> ProviderResult {
    let sources = auth::resolve_sources(auth);
    if sources.is_empty() {
        return ProviderResult::not_configured(PROVIDER_ID, PROVIDER_NAME);
    }

    let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
    let mut last_error: Option<QuotaError> = None;
    let mut any_success = false;

    for src in &sources {
        let token = match src.access_token.as_deref() {
            Some(t) => t,
            None => {
                last_error = Some(QuotaError {
                    kind: ErrorKind::Http {
                        status: 401,
                        retry_after: None,
                    },
                    detail: format!(
                        "{}: no access token stored (upstream refresh not performed by agtop)",
                        src.source_label
                    ),
                });
                continue;
            }
        };
        let project = src.project_id.as_deref().or(Some(DEFAULT_PROJECT_ID));

        let mut source_progress = false;

        // Gemini-only: retrieve buckets.
        if src.source_id == SourceId::Gemini {
            match api::fetch_quota_buckets(http, token, project) {
                Ok(body) => {
                    match serde_json::from_slice::<transforms::RetrieveUserQuotaResponse>(&body) {
                        Ok(parsed) => {
                            for bucket in parsed.buckets.iter() {
                                if let Some((scoped, label, window)) =
                                    transforms::transform_quota_bucket(
                                        bucket,
                                        src.source_id,
                                        now_ms,
                                    )
                                {
                                    models.entry(scoped).or_default().insert(label, window);
                                    source_progress = true;
                                }
                            }
                        }
                        Err(e) => {
                            last_error = Some(QuotaError {
                                kind: ErrorKind::Parse,
                                detail: format!(
                                    "{}: retrieveUserQuota parse failure: {e}",
                                    src.source_label
                                ),
                            });
                        }
                    }
                }
                Err(e) => {
                    last_error = Some(with_source_prefix(
                        e,
                        &src.source_label,
                        ":retrieveUserQuota",
                    ));
                }
            }
        }

        // Both sources: fetch available models.
        match api::fetch_available_models(http, token, project) {
            Ok(body) => match serde_json::from_slice::<transforms::FetchModelsResponse>(&body) {
                Ok(parsed) => {
                    for (model_name, data) in parsed.models.iter() {
                        let (scoped, label, window) = transforms::transform_model_data(
                            model_name,
                            data,
                            src.source_id,
                            now_ms,
                        );
                        models.entry(scoped).or_default().insert(label, window);
                        source_progress = true;
                    }
                }
                Err(e) => {
                    last_error = Some(QuotaError {
                        kind: ErrorKind::Parse,
                        detail: format!(
                            "{}: fetchAvailableModels parse failure: {e}",
                            src.source_label
                        ),
                    });
                }
            },
            Err(e) => {
                last_error = Some(with_source_prefix(
                    e,
                    &src.source_label,
                    ":fetchAvailableModels",
                ));
            }
        }

        if source_progress {
            any_success = true;
        }
    }

    if !any_success {
        return ProviderResult::err(
            PROVIDER_ID,
            PROVIDER_NAME,
            last_error.unwrap_or_else(|| QuotaError {
                kind: ErrorKind::Transport,
                detail: "no data returned from any Google source".to_string(),
            }),
        );
    }

    // Include source labels in meta so TUI can show which sources contributed.
    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    let labels: Vec<String> = sources.iter().map(|s| s.source_label.clone()).collect();
    if !labels.is_empty() {
        meta.insert("sources".to_string(), labels.join(","));
    }

    let usage = Usage {
        windows: IndexMap::new(),
        models,
        extras: IndexMap::new(),
    };
    ProviderResult::ok(PROVIDER_ID, PROVIDER_NAME, usage, meta)
}

fn with_source_prefix(mut err: QuotaError, source_label: &str, endpoint: &str) -> QuotaError {
    err.detail = format!("{source_label} {endpoint}: {}", err.detail);
    err
}
