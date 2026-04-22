//! Quota subsystem configuration loaded via figment.
//!
//! Precedence (highest wins): CLI flags → env vars (AGTOP_QUOTA_*) →
//! config file (~/.config/agtop/agtop.toml, `[quota]` section) → defaults.
//! CLI-flag merging is done by the caller (agtop-cli) — this module only
//! knows how to load defaults, file, and env.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct QuotaConfig {
    /// Background refresh cadence while the quota pane is focused.
    pub refresh_interval_secs: u64,

    /// Per-request HTTP timeout.
    pub request_timeout_secs: u64,

    /// Per-request HTTP timeout for Google endpoints.
    pub google_request_timeout_secs: u64,

    /// Override path to opencode's auth.json.
    pub opencode_auth_path: Option<PathBuf>,

    /// Providers to disable even if configured. ProviderId strings.
    pub disabled: Vec<String>,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 60,
            request_timeout_secs: 10,
            google_request_timeout_secs: 15,
            opencode_auth_path: None,
            disabled: Vec::new(),
        }
    }
}

impl QuotaConfig {
    /// Load configuration following the standard precedence. `file_path`
    /// is the optional TOML file; pass `None` to skip file loading.
    // figment::Error is inherently large (208 bytes); we can't reduce its size.
    #[allow(clippy::result_large_err)]
    pub fn load(file_path: Option<&std::path::Path>) -> Result<Self, figment::Error> {
        use figment::providers::{Env, Format, Serialized, Toml};
        use figment::Figment;

        // Defaults go into Profile::Default (fallback layer).
        let mut fig = Figment::from(Serialized::defaults(Self::default()));
        if let Some(path) = file_path {
            if path.exists() {
                // nested() makes top-level TOML sections become profiles,
                // so [quota] lands in the "quota" profile.
                fig = fig.merge(Toml::file(path).nested());
            }
        }
        // Env vars also target the "quota" profile so they override file values
        // (later merge wins within the same profile).
        fig = fig.merge(Env::prefixed("AGTOP_QUOTA_").split("__").profile("quota"));
        // select("quota") reads from the "quota" profile with Profile::Default
        // as the fallback, giving precedence: env > file > defaults.
        fig.select("quota").extract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let c = QuotaConfig::default();
        assert_eq!(c.refresh_interval_secs, 60);
        assert_eq!(c.request_timeout_secs, 10);
        assert_eq!(c.google_request_timeout_secs, 15);
        assert!(c.disabled.is_empty());
        assert!(c.opencode_auth_path.is_none());
    }

    #[test]
    fn env_override_applies() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("AGTOP_QUOTA_REFRESH_INTERVAL_SECS", "30");
            let c = QuotaConfig::load(None)?;
            assert_eq!(c.refresh_interval_secs, 30);
            Ok(())
        });
    }

    #[test]
    fn env_override_disabled_list() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("AGTOP_QUOTA_DISABLED", "[\"zai\", \"google\"]");
            let c = QuotaConfig::load(None)?;
            assert_eq!(c.disabled, vec!["zai".to_string(), "google".to_string()]);
            Ok(())
        });
    }

    #[test]
    fn file_then_env_env_wins() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "agtop.toml",
                "[quota]\nrefresh_interval_secs = 45\nrequest_timeout_secs = 20\n",
            )?;
            jail.set_env("AGTOP_QUOTA_REFRESH_INTERVAL_SECS", "90");

            let path = jail.directory().join("agtop.toml");
            let c = QuotaConfig::load(Some(&path))?;
            assert_eq!(c.refresh_interval_secs, 90); // env wins
            assert_eq!(c.request_timeout_secs, 20); // file over default
            assert_eq!(c.google_request_timeout_secs, 15); // default
            Ok(())
        });
    }
}
