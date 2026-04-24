//! Google credential resolution. Two sources are collected independently:
//!
//! 1. **Gemini CLI** — from opencode's auth.json under the `google` or
//!    `google.oauth` key. May have a nested `oauth` sub-object (openchamber
//!    tolerates both nesting depths; we do the same).
//! 2. **Antigravity** — from a platform-specific accounts.json file on
//!    disk. The active account index (or 0) wins; its `refreshToken` is
//!    shaped as `token|projectId|managedProjectId`.
//!
//! OAuth client credentials are hardcoded from openchamber's source —
//! these are the registered clients that Gemini CLI and Antigravity use.

use crate::quota::auth::{AuthEntry, OpencodeAuth};
use crate::quota::providers::google::transforms::{parse_refresh_token, RefreshTokenParts};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceId {
    Gemini,
    Antigravity,
}

impl SourceId {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Antigravity => "antigravity",
        }
    }
}

pub const DEFAULT_PROJECT_ID: &str = "rising-fact-p41fc";

// ---------------------------------------------------------------------------
// NOTE ON "SECRETS" BELOW
// ---------------------------------------------------------------------------
// The four constants below are public OAuth **installed-application** client
// credentials that ship inside the official Gemini CLI and Antigravity
// clients. They are NOT private secrets — Google explicitly documents that
// installed-app client secrets embedded in distributed source code are not
// treated as secrets:
//   https://developers.google.com/identity/protocols/oauth2#installed
//
// Upstream references (Apache-2.0):
//   - google-gemini/gemini-cli
//       packages/core/src/code_assist/oauth2.ts
//       (OAUTH_CLIENT_ID / OAUTH_CLIENT_SECRET — matches GEMINI_* below,
//        with an explicit upstream comment: "It's ok to save this in git
//        because this is an installed application ... the client secret is
//        obviously not treated as a secret.")
//   - Antigravity client (extracted from the distributed application, same
//     installed-app flow).
//
// We reproduce them here so this crate can speak the same OAuth protocol as
// Gemini CLI / Antigravity when refreshing user-owned tokens that those
// clients obtained. Each end user authenticates with their own Google
// account — these constants only identify *which application* is asking.
//
// GitHub's push-protection secret scanner pattern-matches the `GOCSPX-`
// prefix regardless of context. If a push is blocked, use the one-click
// "allow secret" bypass URL in the error output; do not rotate these
// values or treat them as leaked credentials.
// ---------------------------------------------------------------------------

/// Gemini CLI OAuth client ID. Public installed-app credential.
/// Source: google-gemini/gemini-cli (Apache-2.0).
pub const GEMINI_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
/// Gemini CLI OAuth client "secret". Public installed-app credential — not
/// actually secret. Source: google-gemini/gemini-cli (Apache-2.0).
pub const GEMINI_CLIENT_SECRET: &str = "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";

/// Antigravity OAuth client ID. Public installed-app credential extracted
/// from the distributed Antigravity application.
pub const ANTIGRAVITY_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
/// Antigravity OAuth client "secret". Public installed-app credential — not
/// actually secret. Extracted from the distributed Antigravity application.
pub const ANTIGRAVITY_CLIENT_SECRET: &str = "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";

#[derive(Debug, Clone)]
pub struct AuthSource {
    pub source_id: SourceId,
    pub source_label: String,
    /// The current access token if known. None means "refresh would be
    /// needed" — we report HTTP 401 in that case rather than refresh.
    pub access_token: Option<String>,
    /// The refresh token. Kept for future own-login-suite use only.
    /// Phase 5 never calls Google's token endpoint with it.
    pub refresh_token: Option<String>,
    /// Epoch ms of access-token expiry, when known.
    pub expires: Option<i64>,
    pub project_id: Option<String>,
}

impl AuthSource {
    pub fn client_id(&self) -> &'static str {
        match self.source_id {
            SourceId::Gemini => GEMINI_CLIENT_ID,
            SourceId::Antigravity => ANTIGRAVITY_CLIENT_ID,
        }
    }
    pub fn client_secret(&self) -> &'static str {
        match self.source_id {
            SourceId::Gemini => GEMINI_CLIENT_SECRET,
            SourceId::Antigravity => ANTIGRAVITY_CLIENT_SECRET,
        }
    }
}

/// Collect every available Google auth source, in order of preference.
/// Empty vec means Google is not configured.
///
/// Order:
///   1. Gemini CLI native credentials (`~/.gemini/oauth_creds.json`) — primary
///   2. opencode auth.json `google` entry — fallback when Gemini CLI absent
///   3. Antigravity accounts.json
pub fn resolve_sources(auth: &OpencodeAuth) -> Vec<AuthSource> {
    let mut out = Vec::new();

    // Primary: Gemini CLI's own credential file.
    if let Some(src) = resolve_gemini_cli_native() {
        out.push(src);
    } else if let Some(src) = resolve_gemini_from_auth(auth) {
        // Fallback: opencode auth.json `google` entry.
        out.push(src);
    }

    if let Some(src) = resolve_antigravity(antigravity_accounts_path().as_deref()) {
        out.push(src);
    }
    out
}

/// Read Gemini CLI's own credential file: `~/.gemini/oauth_creds.json`.
///
/// Fields: `access_token`, `refresh_token`, `expiry_date` (epoch ms).
///
/// Env override: `AGTOP_QUOTA_GEMINI_CLI_CREDS` — set to an empty or
/// non-existent path in tests to disable native credential discovery.
fn resolve_gemini_cli_native() -> Option<AuthSource> {
    let path = if let Ok(p) = std::env::var("AGTOP_QUOTA_GEMINI_CLI_CREDS") {
        std::path::PathBuf::from(p)
    } else {
        let home = dirs::home_dir()?;
        home.join(".gemini").join("oauth_creds.json")
    };
    let bytes = std::fs::read(&path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;

    let access_token = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let refresh_token = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let expires = v.get("expiry_date").and_then(|x| x.as_i64());

    if access_token.is_none() && refresh_token.is_none() {
        return None;
    }

    Some(AuthSource {
        source_id: SourceId::Gemini,
        source_label: "Gemini".to_string(),
        access_token,
        refresh_token,
        expires,
        project_id: None,
    })
}

fn resolve_gemini_from_auth(auth: &OpencodeAuth) -> Option<AuthSource> {
    let entry = auth.lookup(&["google", "google.oauth"])?;
    // Tolerate both shapes: top-level fields on the entry, and nested under
    // `oauth`. Prefer the nested form (openchamber does).
    let picked: &AuthEntry = entry.oauth.as_deref().unwrap_or(entry);

    let access_token = picked.access.clone().or_else(|| picked.token.clone());
    let RefreshTokenParts {
        refresh_token,
        project_id,
        managed_project_id,
    } = parse_refresh_token(picked.refresh.as_deref());

    if access_token.is_none() && refresh_token.is_none() {
        return None;
    }

    Some(AuthSource {
        source_id: SourceId::Gemini,
        source_label: "Gemini".to_string(),
        access_token,
        refresh_token,
        expires: picked.expires,
        project_id: project_id.or(managed_project_id),
    })
}

fn resolve_antigravity(path: Option<&Path>) -> Option<AuthSource> {
    let path = path?;
    let bytes = std::fs::read(path).ok()?;
    let file: AntigravityFile = serde_json::from_slice(&bytes).ok()?;
    let idx = file.active_index.unwrap_or(0).max(0) as usize;
    let account = file.accounts.get(idx).or_else(|| file.accounts.first())?;
    let refresh_raw = account.refresh_token.as_deref()?;
    let RefreshTokenParts {
        refresh_token,
        project_id: embedded_project,
        managed_project_id,
    } = parse_refresh_token(Some(refresh_raw));
    let project_id = account
        .project_id
        .clone()
        .or(account.managed_project_id.clone())
        .or(embedded_project)
        .or(managed_project_id);

    Some(AuthSource {
        source_id: SourceId::Antigravity,
        source_label: "Antigravity".to_string(),
        access_token: None, // Antigravity file never stores a live access token.
        refresh_token,
        expires: None,
        project_id,
    })
}

/// Candidate paths for Antigravity's accounts.json, in search order.
/// First existing path wins.
fn antigravity_accounts_path() -> Option<PathBuf> {
    // Env override for tests and advanced users.
    if let Ok(p) = std::env::var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS") {
        return Some(PathBuf::from(p));
    }

    let home = dirs::home_dir()?;

    // Platform-specific default locations (copied from openchamber).
    let candidates: Vec<PathBuf> = vec![
        home.join(".config/google-cloud-code/accounts.json"),
        home.join("Library/Application Support/google-cloud-code/accounts.json"),
        home.join("AppData/Roaming/google-cloud-code/accounts.json"),
    ];

    candidates.into_iter().find(|p| p.exists())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AntigravityFile {
    accounts: Vec<AntigravityAccount>,
    #[serde(rename = "activeIndex")]
    active_index: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AntigravityAccount {
    email: Option<String>,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(rename = "projectId")]
    project_id: Option<String>,
    #[serde(rename = "managedProjectId")]
    managed_project_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_auth(name: &str) -> OpencodeAuth {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/auth");
        p.push(name);
        OpencodeAuth::load_from(&p).unwrap()
    }

    fn fixture_path(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/google");
        p.push(name);
        p
    }

    #[test]
    fn gemini_from_flat_entry() {
        let auth = fixture_auth("opencode_full.json");
        let src = resolve_gemini_from_auth(&auth).expect("gemini source present");
        assert_eq!(src.source_id, SourceId::Gemini);
        assert_eq!(
            src.access_token.as_deref(),
            Some("GOOGLE_ACCESS_PLACEHOLDER")
        );
        assert_eq!(
            src.refresh_token.as_deref(),
            Some("GOOGLE_REFRESH_PLACEHOLDER")
        );
        assert_eq!(src.project_id.as_deref(), Some("test-project"));
    }

    #[test]
    fn gemini_from_nested_oauth() {
        let auth = fixture_auth("opencode_nested_google_oauth.json");
        let src = resolve_gemini_from_auth(&auth).expect("gemini source present");
        assert_eq!(src.access_token.as_deref(), Some("NESTED_GOOGLE_ACCESS"));
        assert_eq!(src.expires, Some(1_800_000_000_000));
    }

    #[test]
    fn gemini_missing_when_no_google_entry() {
        let auth = fixture_auth("opencode_minimal.json");
        assert!(resolve_gemini_from_auth(&auth).is_none());
    }

    #[test]
    fn antigravity_from_fixture_picks_active_index() {
        let path = fixture_path("antigravity_accounts.json");
        let src = resolve_antigravity(Some(&path)).expect("antigravity source");
        assert_eq!(src.source_id, SourceId::Antigravity);
        assert_eq!(src.project_id.as_deref(), Some("proj-a"));
        assert!(src.access_token.is_none());
        assert!(src.refresh_token.is_some());
    }

    #[test]
    fn antigravity_missing_when_path_absent() {
        assert!(resolve_antigravity(None).is_none());
    }

    #[test]
    fn client_credentials_match_openchamber() {
        let gemini = AuthSource {
            source_id: SourceId::Gemini,
            source_label: "Gemini".into(),
            access_token: None,
            refresh_token: None,
            expires: None,
            project_id: None,
        };
        assert_eq!(gemini.client_id(), GEMINI_CLIENT_ID);
        let antig = AuthSource {
            source_id: SourceId::Antigravity,
            source_label: "Antigravity".into(),
            access_token: None,
            refresh_token: None,
            expires: None,
            project_id: None,
        };
        assert_eq!(antig.client_id(), ANTIGRAVITY_CLIENT_ID);
    }

    #[test]
    fn resolve_sources_combines_both_when_available() {
        // Use env override to point at the fixture Antigravity file.
        let fixture = fixture_path("antigravity_accounts.json");
        std::env::set_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS", &fixture);
        let auth = fixture_auth("opencode_full.json");
        let sources = resolve_sources(&auth);
        std::env::remove_var("AGTOP_QUOTA_ANTIGRAVITY_ACCOUNTS");
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].source_id, SourceId::Gemini);
        assert_eq!(sources[1].source_id, SourceId::Antigravity);
    }
}
