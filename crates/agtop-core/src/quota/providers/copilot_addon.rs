//! GitHub Copilot Add-on quota provider.
//!
//! Same endpoint, auth, and parser as the main Copilot provider, but
//! filters the windows down to `premium` only. Matches the distinct
//! provider id opencode uses to bill add-on entitlements separately.
//!
//! See spec section 3.3.1.

use crate::quota::auth::OpencodeAuth;
use crate::quota::http::HttpClient;
use crate::quota::providers::copilot::{fetch_impl, WindowFilter, ALIASES};
use crate::quota::providers::Provider;
use crate::quota::types::{ProviderId, ProviderResult};

pub const PROVIDER_ID: ProviderId = ProviderId::CopilotAddon;
pub const PROVIDER_NAME: &str = "GitHub Copilot Add-on";

pub struct CopilotAddon;

impl Provider for CopilotAddon {
    fn id(&self) -> ProviderId {
        PROVIDER_ID
    }
    fn display_name(&self) -> &'static str {
        PROVIDER_NAME
    }
    fn is_configured(&self, auth: &OpencodeAuth) -> bool {
        auth.lookup(ALIASES)
            .map(|e| e.access.is_some() || e.token.is_some())
            .unwrap_or(false)
    }
    fn fetch(&self, auth: &OpencodeAuth, http: &dyn HttpClient) -> ProviderResult {
        fetch_impl(
            auth,
            http,
            WindowFilter::PremiumOnly,
            PROVIDER_ID,
            PROVIDER_NAME,
        )
    }
}
