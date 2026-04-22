//! Read-only reader for opencode's auth.json.
//!
//! Schema is a JSON object mapping provider alias → entry. Entries may
//! contain any subset of: `access`, `refresh`, `token`, `key`, `accountId`,
//! plus a nested `oauth` object (Google only) with the same access/refresh
//! fields. See spec section "Auth types".

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthEntry {
    #[serde(default)]
    pub access: Option<String>,
    #[serde(default)]
    pub refresh: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default, rename = "accountId")]
    pub account_id: Option<String>,
    /// Google nests its oauth fields inside an `oauth` sub-object.
    #[serde(default)]
    pub oauth: Option<Box<AuthEntry>>,
    /// Google's access-token expiry (epoch ms). Present only inside the nested
    /// `oauth` object.
    #[serde(default)]
    pub expires: Option<i64>,
}

pub struct OpencodeAuth {
    entries: HashMap<String, AuthEntry>,
}

#[derive(Debug)]
pub enum AuthLoadError {
    NotFound,
    Permission(io::Error),
    Malformed(serde_json::Error),
}

impl std::fmt::Display for AuthLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "opencode auth.json not found"),
            Self::Permission(e) => write!(f, "opencode auth.json: permission error: {e}"),
            Self::Malformed(e) => write!(f, "opencode auth.json: malformed JSON: {e}"),
        }
    }
}

impl std::error::Error for AuthLoadError {}

impl OpencodeAuth {
    /// Load from the default location with env override support:
    /// 1. `$AGTOP_QUOTA_OPENCODE_AUTH_PATH`
    /// 2. `$XDG_DATA_HOME/opencode/auth.json`
    /// 3. `~/.local/share/opencode/auth.json`
    pub fn load() -> Result<Self, AuthLoadError> {
        let path = default_auth_path().ok_or(AuthLoadError::NotFound)?;
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self, AuthLoadError> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(AuthLoadError::NotFound),
            Err(e) => return Err(AuthLoadError::Permission(e)),
        };
        let entries: HashMap<String, AuthEntry> =
            serde_json::from_slice(&bytes).map_err(AuthLoadError::Malformed)?;
        Ok(Self { entries })
    }

    pub fn empty() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Look up an entry by any of the provided alias keys, first match wins.
    pub fn lookup(&self, aliases: &[&str]) -> Option<&AuthEntry> {
        for alias in aliases {
            if let Some(entry) = self.entries.get(*alias) {
                return Some(entry);
            }
        }
        None
    }
}

fn default_auth_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AGTOP_QUOTA_OPENCODE_AUTH_PATH") {
        return Some(PathBuf::from(p));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg).join("opencode").join("auth.json"));
    }
    dirs::home_dir().map(|h| h.join(".local/share/opencode/auth.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/auth");
        p.push(name);
        p
    }

    #[test]
    fn loads_full_fixture() {
        let auth = OpencodeAuth::load_from(&fixture("opencode_full.json")).unwrap();
        let anth = auth.lookup(&["anthropic", "claude"]).unwrap();
        assert_eq!(anth.access.as_deref(), Some("ANTHROPIC_ACCESS_PLACEHOLDER"));
        let zai = auth.lookup(&["zai-coding-plan", "zai", "z.ai"]).unwrap();
        assert_eq!(zai.key.as_deref(), Some("ZAI_KEY_PLACEHOLDER"));
        let openai = auth.lookup(&["openai", "codex", "chatgpt"]).unwrap();
        assert_eq!(
            openai.account_id.as_deref(),
            Some("00000000-0000-0000-0000-000000000000")
        );
    }

    #[test]
    fn alias_order_first_match_wins() {
        let auth = OpencodeAuth::load_from(&fixture("opencode_full.json")).unwrap();
        // anthropic wins over claude because it appears first in aliases
        let anth = auth.lookup(&["anthropic", "claude"]).unwrap();
        assert!(anth.access.is_some());
    }

    #[test]
    fn minimal_fixture_has_only_anthropic() {
        let auth = OpencodeAuth::load_from(&fixture("opencode_minimal.json")).unwrap();
        assert!(auth.lookup(&["anthropic"]).is_some());
        assert!(auth.lookup(&["openai"]).is_none());
        assert!(auth.lookup(&["zai-coding-plan", "zai"]).is_none());
    }

    #[test]
    fn malformed_fixture_yields_malformed_error() {
        let err = OpencodeAuth::load_from(&fixture("opencode_malformed.json"));
        assert!(matches!(err, Err(AuthLoadError::Malformed(_))));
    }

    #[test]
    fn nested_google_oauth_is_accessible() {
        let auth = OpencodeAuth::load_from(&fixture("opencode_nested_google_oauth.json")).unwrap();
        let g = auth.lookup(&["google", "google.oauth"]).unwrap();
        let oauth = g.oauth.as_deref().expect("google.oauth nested field");
        assert_eq!(oauth.access.as_deref(), Some("NESTED_GOOGLE_ACCESS"));
        assert_eq!(oauth.expires, Some(1_800_000_000_000));
    }

    #[test]
    fn not_found_when_path_missing() {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/auth/does_not_exist.json");
        let err = OpencodeAuth::load_from(&p);
        assert!(matches!(err, Err(AuthLoadError::NotFound)));
    }
}
