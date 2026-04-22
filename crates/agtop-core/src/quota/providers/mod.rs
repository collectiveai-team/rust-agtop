//! Provider implementations for the quota subsystem.
//!
//! Each file in this directory implements `Provider` for one provider
//! (Google gets a sub-directory because it has multiple auth sources and
//! fallback URLs). `register_all()` returns every registered provider in
//! display order.

use crate::quota::auth::OpencodeAuth;
use crate::quota::http::HttpClient;
use crate::quota::types::{ProviderId, ProviderResult};

pub mod claude;
pub mod codex;
pub mod copilot;
pub mod copilot_addon;
pub mod google;
pub mod zai; // Phase 5

/// Contract every registered provider implements.
///
/// - `id` / `display_name` are compile-time constants.
/// - `is_configured` must not make network calls. It only inspects `auth`.
/// - `fetch` is blocking (ureq-based) and must not mutate auth, must not
///   refresh tokens. On any failure it returns `ProviderResult` with
///   `ok: false` — errors are data, not control flow.
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn display_name(&self) -> &'static str;
    fn is_configured(&self, auth: &OpencodeAuth) -> bool;
    fn fetch(&self, auth: &OpencodeAuth, http: &dyn HttpClient) -> ProviderResult;
}

/// Read-only metadata view, used by `list_providers()` to describe every
/// provider without exposing fetch behavior.
pub trait ProviderMeta: Send + Sync {
    fn id(&self) -> ProviderId;
    fn display_name(&self) -> &'static str;
}

impl<P: Provider + ?Sized> ProviderMeta for P {
    fn id(&self) -> ProviderId {
        Provider::id(self)
    }
    fn display_name(&self) -> &'static str {
        Provider::display_name(self)
    }
}

/// Ordered list of every registered provider. Order determines TUI display
/// order. Enable additional entries as each phase lands.
pub fn register_all() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(claude::Claude),
        Box::new(codex::Codex),
        Box::new(copilot::Copilot),
        Box::new(copilot_addon::CopilotAddon),
        Box::new(zai::Zai),
        Box::new(google::Google),
    ]
}

/// Find a registered provider by id. Returns `None` if unknown.
/// Used by `fetch_one` to avoid a double dispatch through string labels.
pub fn find(id: ProviderId) -> Option<Box<dyn Provider>> {
    register_all().into_iter().find(|p| p.id() == id)
}
