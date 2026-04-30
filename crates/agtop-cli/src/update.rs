//! Startup check for a newer `agtop` release on GitHub, with optional
//! atomic in-place upgrade.
//!
//! Public entry point: [`check_and_maybe_prompt`]. Called once near the
//! top of `main` (after subcommand dispatch, before TUI init). All
//! errors inside the check path are swallowed at `tracing::debug!` so
//! a flaky network never blocks startup. Errors during the actual
//! self-update (after the user said "y") ARE surfaced to the user.

use anyhow::Result;

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
    pub repo_owner:      &'static str,
    pub repo_name:       &'static str,
    pub bin_name:        &'static str,
}

/// Coordinates for the production `agtop` binary on GitHub.
pub fn agtop_opts() -> UpdateOpts {
    UpdateOpts {
        current_version: crate::version::DISPLAY_VERSION,
        repo_owner:      "collectiveai-team",
        repo_name:       "rust-agtop",
        bin_name:        "agtop",
    }
}

/// Top-level entry point. See module docs.
///
/// Returns `Ok(())` in every "user did not update" path, including all
/// network failures. Returns `Err(_)` only when the user explicitly
/// confirmed the update and the actual self-replace failed.
pub fn check_and_maybe_prompt(_mode: PromptMode, _opts: &UpdateOpts) -> Result<()> {
    // Filled in by Tasks 3..7.
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
}
