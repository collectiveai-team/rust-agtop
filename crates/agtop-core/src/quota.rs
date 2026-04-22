//! Quota subsystem — fetches current usage/quota state from coding-plan
//! provider APIs. See `docs/superpowers/specs/2026-04-21-quota-provider-fetchers-design.md`.

pub mod auth;
pub mod config;
pub mod http;
pub mod providers;
pub mod time;
pub mod types;

pub use auth::{AuthEntry, AuthLoadError, OpencodeAuth};
pub use config::QuotaConfig;
pub use http::{HttpClient, HttpRequest, HttpResponse, Method, TransportError, UreqClient};
pub use providers::{Provider, ProviderMeta};
pub use types::{ErrorKind, ProviderId, ProviderResult, QuotaError, Usage, UsageExtra, UsageWindow};
