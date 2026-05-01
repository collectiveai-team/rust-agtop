//! Startup check for a newer `agtop` release on GitHub, with optional
//! atomic in-place upgrade.
//!
//! Public entry point: [`check_and_maybe_prompt`]. Called once near the
//! top of `main` (after subcommand dispatch, before TUI init). All
//! errors inside the check path are swallowed at `tracing::debug!` so
//! a flaky network never blocks startup. Errors during the actual
//! self-update (after the user said "y") ARE surfaced to the user.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where the y/N prompt is displayed (interactive) versus a one-line
/// stderr notice (banner). Picked by `main` based on flags + tty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    /// Default TUI / dashboard runs on a real terminal.
    Interactive,
    /// `--list`, `--json`, `--watch`, piped stdin, CI.
    Banner,
}

/// Repo coordinates and binary identity. Static so we don't have to
/// pass them through call sites; all construction happens at
/// [`agtop_opts`].
pub struct UpdateOpts {
    pub current_version: &'static str,
    pub repo_owner: &'static str,
    pub repo_name: &'static str,
    pub bin_name: &'static str,
}

/// Coordinates for the production `agtop` binary on GitHub.
pub fn agtop_opts() -> UpdateOpts {
    UpdateOpts {
        current_version: crate::version::DISPLAY_VERSION,
        repo_owner: "collectiveai-team",
        repo_name: "rust-agtop",
        bin_name: "agtop",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedCheck {
    checked_at: DateTime<Utc>,
    latest_version: String,
}

impl CachedCheck {
    const TTL: Duration = Duration::hours(24);

    fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        let age = now.signed_duration_since(self.checked_at);
        age >= Duration::zero() && age <= Self::TTL
    }
}

fn cache_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("agtop").join("update-check.json"))
}

fn load_cache() -> Option<CachedCheck> {
    let path = cache_path()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_cache(entry: &CachedCheck) {
    let Some(path) = cache_path() else {
        tracing::debug!("update check: no cache_dir, skipping cache save");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::debug!("update check: mkdir {} failed: {e}", parent.display());
            return;
        }
    }
    let json = match serde_json::to_vec(entry) {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("update check: serialize failed: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, json) {
        tracing::debug!("update check: write {} failed: {e}", path.display());
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseInfo {
    tag_name: String,
}

fn parse_release_payload(body: &str) -> Result<ReleaseInfo> {
    let info: ReleaseInfo =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("malformed release JSON: {e}"))?;
    if info.tag_name.is_empty() {
        anyhow::bail!("release JSON missing tag_name");
    }
    Ok(info)
}

const GITHUB_API_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn fetch_latest_release(opts: &UpdateOpts) -> Result<ReleaseInfo> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        opts.repo_owner, opts.repo_name,
    );
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(GITHUB_API_TIMEOUT))
        .user_agent(concat!("rust-agtop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();

    let mut resp = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| anyhow::anyhow!("GitHub API request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("GitHub API returned HTTP {status}");
    }

    const MAX_BYTES: usize = 256 * 1024;
    let mut body = Vec::with_capacity(64 * 1024);
    use std::io::Read;
    resp.body_mut()
        .as_reader()
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| anyhow::anyhow!("read GitHub API body: {e}"))?;
    if body.len() > MAX_BYTES {
        anyhow::bail!("GitHub API body unexpectedly large: {} bytes", body.len());
    }
    let body =
        String::from_utf8(body).map_err(|e| anyhow::anyhow!("GitHub API body not UTF-8: {e}"))?;
    parse_release_payload(&body)
}

/// Top-level entry point. See module docs.
///
/// Returns `Ok(())` in every "user did not update" path, including all
/// network failures. Returns `Err(_)` only when the user explicitly
/// confirmed the update and the actual self-replace failed.
pub fn check_and_maybe_prompt(mode: PromptMode, opts: &UpdateOpts) -> Result<()> {
    let now = Utc::now();

    let latest_version: String = match load_cache() {
        Some(c) if c.is_fresh(now) => {
            tracing::debug!(
                "update check: using cached latest_version={} (age={:?})",
                c.latest_version,
                now.signed_duration_since(c.checked_at),
            );
            c.latest_version
        }
        _ => match fetch_latest_release(opts) {
            Ok(info) => {
                let entry = CachedCheck {
                    checked_at: now,
                    latest_version: info.tag_name.clone(),
                };
                save_cache(&entry);
                info.tag_name
            }
            Err(e) => {
                tracing::debug!("update check: fetch failed: {e}");
                return Ok(());
            }
        },
    };

    if !is_newer(opts.current_version, &latest_version) {
        tracing::debug!(
            "update check: current={} latest={} -> up to date",
            opts.current_version,
            latest_version,
        );
        return Ok(());
    }

    let confirmed = match mode {
        PromptMode::Banner => {
            eprintln!(
                "agtop: {latest_version} is available (current: {current}). \
                 Run agtop interactively to upgrade, or set \
                 AGTOP_NO_UPDATE_CHECK=1 to silence this notice.",
                current = opts.current_version,
            );
            false
        }
        PromptMode::Interactive => {
            println!(
                "A new agtop release is available: {latest_version} \
                 (current: {current}).",
                current = opts.current_version,
            );
            prompt_yes_no("Update now? [y/N] ")
        }
    };

    if !confirmed {
        return Ok(());
    }

    run_self_update(opts, &latest_version)?;
    println!("Updated agtop to {latest_version}. Re-run agtop to start the new version.");
    std::process::exit(0);
}

fn prompt_yes_no(question: &str) -> bool {
    use std::io::{stdin, stdout, Write};
    print!("{question}");
    if stdout().flush().is_err() {
        return false;
    }
    let mut line = String::new();
    if stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn run_self_update(opts: &UpdateOpts, latest_version: &str) -> Result<()> {
    let target = current_target_triple().ok_or_else(|| {
        anyhow::anyhow!(
            "no published binary for target {}; please reinstall manually",
            self_update::get_target(),
        )
    })?;
    let asset = asset_name_for_target(opts.bin_name, target)
        .expect("target triple was just validated by current_target_triple()");

    let _ = latest_version;
    self_update::backends::github::Update::configure()
        .repo_owner(opts.repo_owner)
        .repo_name(opts.repo_name)
        .bin_name(opts.bin_name)
        .target(target)
        .identifier(&asset)
        .current_version(opts.current_version)
        .show_download_progress(true)
        .no_confirm(true)
        .build()
        .map_err(|e| anyhow::anyhow!("self_update build failed: {e}"))?
        .update()
        .map_err(|e| anyhow::anyhow!("self_update apply failed: {e}"))?;
    Ok(())
}

fn is_newer(current: &str, latest: &str) -> bool {
    let strip = |s: &str| s.strip_prefix('v').unwrap_or(s).to_owned();
    let current = match semver::Version::parse(&strip(current)) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let latest = match semver::Version::parse(&strip(latest)) {
        Ok(v) => v,
        Err(_) => return false,
    };
    latest > current
}

const SUPPORTED_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
];

fn current_target_triple() -> Option<&'static str> {
    let live = self_update::get_target();
    SUPPORTED_TARGETS.iter().copied().find(|t| *t == live)
}

fn asset_name_for_target(bin_name: &str, target: &str) -> Option<String> {
    if SUPPORTED_TARGETS.contains(&target) {
        Some(format!("{bin_name}-{target}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_basic() {
        assert!(is_newer("0.4.2", "0.5.0"));
        assert!(is_newer("0.5.0", "0.5.1"));
        assert!(is_newer("0.5.0", "1.0.0"));
    }

    #[test]
    fn is_newer_same_or_older_returns_false() {
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("0.5.0", "0.4.9"));
        assert!(!is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn is_newer_strips_v_prefix() {
        assert!(is_newer("0.5.0", "v0.6.0"));
        assert!(is_newer("v0.5.0", "v0.6.0"));
        assert!(is_newer("v0.5.0", "0.6.0"));
    }

    #[test]
    fn is_newer_handles_prerelease() {
        assert!(is_newer("0.5.0-rc1", "0.5.0"));
        assert!(!is_newer("0.5.0", "0.5.0-rc1"));
    }

    #[test]
    fn is_newer_returns_false_on_unparseable() {
        assert!(!is_newer("not-a-version", "0.5.0"));
        assert!(!is_newer("0.5.0", "garbage"));
    }

    #[test]
    fn target_triple_is_known() {
        let t = current_target_triple();
        match t {
            Some("x86_64-unknown-linux-gnu")
            | Some("x86_64-apple-darwin")
            | Some("aarch64-apple-darwin") => {}
            Some(other) => panic!("unexpected target triple: {other}"),
            None => {
                #[cfg(any(
                    all(target_os = "linux", target_arch = "x86_64"),
                    all(target_os = "macos", target_arch = "x86_64"),
                    all(target_os = "macos", target_arch = "aarch64"),
                ))]
                panic!("supported platform returned None");
            }
        }
    }

    #[test]
    fn asset_name_for_known_triple() {
        assert_eq!(
            asset_name_for_target("agtop", "x86_64-unknown-linux-gnu"),
            Some("agtop-x86_64-unknown-linux-gnu".to_owned()),
        );
        assert_eq!(
            asset_name_for_target("agtop", "aarch64-apple-darwin"),
            Some("agtop-aarch64-apple-darwin".to_owned()),
        );
    }

    #[test]
    fn asset_name_for_unknown_triple_is_none() {
        assert_eq!(
            asset_name_for_target("agtop", "x86_64-pc-windows-msvc"),
            None,
        );
        assert_eq!(
            asset_name_for_target("agtop", "armv7-unknown-linux-gnueabihf"),
            None,
        );
    }

    #[test]
    fn parses_tag_name_from_fixture() {
        let payload = include_str!("../tests/fixtures/github_latest_release.json");
        let parsed: ReleaseInfo = parse_release_payload(payload).expect("fixture must parse");
        assert!(
            parsed.tag_name.starts_with("v") || !parsed.tag_name.is_empty(),
            "got tag_name = {:?}",
            parsed.tag_name,
        );
    }

    #[test]
    fn rejects_non_object_payload() {
        assert!(parse_release_payload("\"not-an-object\"").is_err());
        assert!(parse_release_payload("[]").is_err());
        assert!(parse_release_payload("not even json").is_err());
    }

    #[test]
    fn rejects_missing_tag_name() {
        assert!(parse_release_payload("{}").is_err());
        assert!(parse_release_payload(r#"{"name":"v0.6.0"}"#).is_err());
    }

    use chrono::{Duration, Utc};

    #[test]
    fn cache_is_fresh_within_24h() {
        let now = Utc::now();
        let entry = CachedCheck {
            checked_at: now - Duration::hours(23),
            latest_version: "0.6.0".to_owned(),
        };
        assert!(entry.is_fresh(now));
    }

    #[test]
    fn cache_is_stale_after_24h() {
        let now = Utc::now();
        let entry = CachedCheck {
            checked_at: now - Duration::hours(25),
            latest_version: "0.6.0".to_owned(),
        };
        assert!(!entry.is_fresh(now));
    }

    #[test]
    fn cache_in_future_is_stale() {
        let now = Utc::now();
        let entry = CachedCheck {
            checked_at: now + Duration::hours(1),
            latest_version: "0.6.0".to_owned(),
        };
        assert!(!entry.is_fresh(now));
    }

    #[test]
    fn cache_roundtrips_through_json() {
        let entry = CachedCheck {
            checked_at: Utc::now(),
            latest_version: "v0.6.0".to_owned(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: CachedCheck = serde_json::from_str(&json).unwrap();
        assert_eq!(back.latest_version, entry.latest_version);
        assert_eq!(back.checked_at.timestamp(), entry.checked_at.timestamp());
    }
}
