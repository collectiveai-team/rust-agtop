//! Google (Gemini CLI + Antigravity) quota provider.
//!
//! Behaviour (revised — see the upstream Gemini CLI source for the reference
//! flow):
//!
//! 1. For the Gemini source, we start by calling `:loadCodeAssist` in
//!    `HEALTH_CHECK` mode. That endpoint works for both free-tier and paid
//!    accounts without a caller-supplied project id and returns the account's
//!    current tier plus its onboarded `cloudaicompanionProject`.
//! 2. When the account has an onboarded project id, we call
//!    `:retrieveUserQuota` with that project id — this matches Gemini CLI's
//!    `refreshUserQuota` path. Per-bucket quota becomes `Usage.models`
//!    entries.
//! 3. When no project id is available, we still mark the provider result as
//!    OK and surface tier metadata via `meta`, but there is no quantitative
//!    quota data to display.
//! 4. Antigravity is kept as a second source. Because its on-disk
//!    `accounts.json` never stores a live access token — and we explicitly
//!    don't refresh tokens — the Antigravity source generally contributes
//!    nothing to the aggregated result in practice.
//!
//! We no longer call `:fetchAvailableModels`: it isn't used by Gemini CLI
//! itself and returns 403 for every free-tier caller we tested against.
//! Per-model windows (when present) come exclusively from
//! `:retrieveUserQuota` buckets.

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
    let mut tier_label: Option<String> = None;
    let mut project_id_seen: Option<String> = None;

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

        let mut source_progress = false;

        if src.source_id == SourceId::Gemini {
            // Step 1: discover the account's tier and onboarded project via
            // :loadCodeAssist. This call is what Gemini CLI itself runs.
            match api::load_code_assist(http, token) {
                Ok(body) => {
                    match serde_json::from_slice::<transforms::LoadCodeAssistResponse>(&body) {
                        Ok(parsed) => {
                            source_progress = true;
                            if let Some(ref tier) = parsed.current_tier {
                                if let Some(ref name) = tier.name {
                                    tier_label = Some(name.clone());
                                } else if let Some(ref id) = tier.id {
                                    tier_label = Some(id.clone());
                                }
                            }
                            if let Some(ref pid) = parsed.cloudaicompanion_project {
                                project_id_seen = Some(pid.clone());
                            }

                            // Step 2: fetch per-model buckets using the
                            // onboarded project id. Current Gemini CLI calls
                            // retrieveUserQuota whenever CodeAssist has a
                            // project id; it is not limited to paid tiers.
                            let project = src
                                .project_id
                                .as_deref()
                                .or(parsed.cloudaicompanion_project.as_deref());
                            if project.is_some() {
                                match api::fetch_quota_buckets(http, token, project) {
                                    Ok(body) => {
                                        match serde_json::from_slice::<
                                            transforms::RetrieveUserQuotaResponse,
                                        >(&body)
                                        {
                                            Ok(parsed) => {
                                                for bucket in parsed.buckets.iter() {
                                                    if let Some((scoped, label, window)) =
                                                        transforms::transform_quota_bucket(
                                                            bucket,
                                                            src.source_id,
                                                            now_ms,
                                                        )
                                                    {
                                                        models
                                                            .entry(scoped)
                                                            .or_default()
                                                            .insert(label, window);
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
                        }
                        Err(e) => {
                            last_error = Some(QuotaError {
                                kind: ErrorKind::Parse,
                                detail: format!(
                                    "{}: loadCodeAssist parse failure: {e}",
                                    src.source_label
                                ),
                            });
                        }
                    }
                }
                Err(e) => {
                    last_error = Some(with_source_prefix(e, &src.source_label, ":loadCodeAssist"));
                }
            }
        }

        // Antigravity note: the source is kept around so the UI can still
        // display "Gemini, Antigravity" in the plan label when accounts.json
        // is present. We do not call any endpoint for it here — historically
        // that path was driven by :fetchAvailableModels, which we've removed
        // because it's 403 for every caller we've observed. Antigravity
        // never stores a live access token on disk, so in practice this
        // branch is also a no-op.

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
    let raw_sources = if labels.is_empty() {
        None
    } else {
        Some(labels.join(","))
    };
    let plan_label = crate::quota::subscription::google_plan(raw_sources.as_deref());
    // Append tier info (e.g. "free-tier") so the TUI subtitle is informative
    // even when there's no quantitative quota data to display.
    let plan_label = match tier_label.as_deref() {
        Some(t) if !t.is_empty() => format!("{plan_label} ({t})"),
        _ => plan_label,
    };
    meta.insert("plan".to_string(), plan_label);
    if let Some(ref s) = raw_sources {
        meta.insert("sources".to_string(), s.clone());
    }
    if let Some(ref t) = tier_label {
        meta.insert("tier".to_string(), t.clone());
    }
    if let Some(ref pid) = project_id_seen {
        meta.insert("project_id".to_string(), pid.clone());
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
