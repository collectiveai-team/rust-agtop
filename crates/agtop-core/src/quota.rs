//! Quota subsystem — fetches current usage/quota state from coding-plan
//! provider APIs. See `docs/superpowers/specs/2026-04-21-quota-provider-fetchers-design.md`.

pub mod auth;
pub mod config;
pub mod http;
pub mod providers;
pub mod subscription;
pub mod time;
pub mod types;

pub use auth::{AuthEntry, AuthLoadError, OpencodeAuth};
pub use config::QuotaConfig;
pub use http::{HttpClient, HttpRequest, HttpResponse, Method, TransportError, UreqClient};
pub use providers::{Provider, ProviderMeta};
pub use types::{
    ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageExtra, UsageWindow,
};

use rayon::prelude::*;
use std::collections::HashSet;

/// Metadata view over every registered provider, in display order.
/// Unconfigured providers are still listed.
pub fn list_providers() -> Vec<ProviderInfo> {
    providers::register_all()
        .into_iter()
        .map(|p| ProviderInfo {
            id: p.id(),
            display_name: p.display_name(),
        })
        .collect()
}

/// Static-ish description of a provider for menus, help text, and CLI listing.
#[derive(Debug, Clone, Copy)]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub display_name: &'static str,
}

/// Fetch every configured provider in parallel.
///
/// - Unconfigured providers are filtered out BEFORE dispatch and are NOT
///   present in the result vec.
/// - Providers listed in `config.disabled` by their `ProviderId` string (e.g.
///   `"claude"`, `"copilot-addon"`) are also filtered out. Comparison is
///   case-insensitive; provider id strings are already lowercase constants.
/// - On any provider-level error (transport, HTTP, parse), the corresponding
///   entry is still present with `ok: false` and a populated `error` field.
///   Errors are data, not control flow — this function never returns `Err`.
pub fn fetch_all(
    auth: &OpencodeAuth,
    http: &(dyn HttpClient + Sync),
    config: &QuotaConfig,
) -> Vec<ProviderResult> {
    let disabled: HashSet<String> = config
        .disabled
        .iter()
        // Lowercase config values; provider id strings are already lowercase.
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();

    providers::register_all()
        .into_par_iter()
        .filter(|p| !disabled.contains(p.id().as_str()))
        .filter(|p| p.is_configured(auth))
        .map(|p| p.fetch(auth, http))
        .collect()
}

/// Fetch a single provider by id. Returns an error-result with
/// `ErrorKind::Transport` (detail "Unsupported provider") when the id
/// is unknown — matches the "errors are data" principle.
pub fn fetch_one(id: ProviderId, auth: &OpencodeAuth, http: &dyn HttpClient) -> ProviderResult {
    match providers::find(id) {
        Some(p) => p.fetch(auth, http),
        None => ProviderResult::err(
            id,
            id.display_name(),
            QuotaError {
                kind: ErrorKind::Transport,
                detail: format!("Unsupported provider: {}", id.as_str()),
            },
        ),
    }
}
