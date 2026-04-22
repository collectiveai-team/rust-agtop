//! Project name resolution from a working directory.
//!
//! Given a `cwd` path, resolves a human-readable project name by:
//! 1. Walking parent directories to find a `.git` directory (the repo root).
//! 2. Running `git -C <root> remote get-url origin` to obtain the remote URL.
//! 3. Parsing the last path segment of the URL, stripping any `.git` suffix.
//! 4. Falling back to the basename of the git root if no remote is found.
//! 5. Falling back to the basename of `cwd` if no `.git` directory exists.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve a project name for the given working directory.
///
/// Returns `None` when `cwd` is empty, the path has no components, or every
/// resolution step fails. Never panics.
pub fn resolve_project_name(cwd: &Path) -> Option<String> {
    // Find the nearest .git ancestor.
    let git_root = find_git_root(cwd);

    let root = git_root.as_deref().unwrap_or(cwd);

    // Try `git remote get-url origin`.
    if let Some(name) = git_remote_name(root) {
        return Some(name);
    }

    // Fallback: basename of the git root (or cwd if no git root).
    path_basename(root)
}

/// Walk from `start` upward, looking for a directory that contains `.git`.
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Run `git -C dir remote get-url origin` and parse the repo name from the URL.
fn git_remote_name(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", dir.to_str()?, "remote", "get-url", "origin"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = std::str::from_utf8(&output.stdout).ok()?.trim();
    if url.is_empty() {
        return None;
    }

    parse_repo_name_from_url(url)
}

/// Extract a repository name from a git remote URL.
///
/// Handles:
/// - `https://github.com/user/rust-agtop.git` → `rust-agtop`
/// - `https://github.com/user/rust-agtop`     → `rust-agtop`
/// - `git@github.com:user/rust-agtop.git`     → `rust-agtop`
/// - `ssh://git@github.com/user/rust-agtop`   → `rust-agtop`
fn parse_repo_name_from_url(url: &str) -> Option<String> {
    // Strip trailing slashes.
    let url = url.trim_end_matches('/');

    // Last segment after '/' or ':'.
    let segment = url.rsplit(['/', ':']).next()?;

    // Strip .git suffix.
    let name = segment.strip_suffix(".git").unwrap_or(segment);

    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Return the final component of a path as a String.
fn path_basename(path: &Path) -> Option<String> {
    path.file_name()?.to_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_https_url_with_git_suffix() {
        assert_eq!(
            parse_repo_name_from_url("https://github.com/user/rust-agtop.git"),
            Some("rust-agtop".into())
        );
    }

    #[test]
    fn parse_https_url_without_git_suffix() {
        assert_eq!(
            parse_repo_name_from_url("https://github.com/user/rust-agtop"),
            Some("rust-agtop".into())
        );
    }

    #[test]
    fn parse_ssh_url() {
        assert_eq!(
            parse_repo_name_from_url("git@github.com:user/rust-agtop.git"),
            Some("rust-agtop".into())
        );
    }

    #[test]
    fn parse_ssh_url_scheme() {
        assert_eq!(
            parse_repo_name_from_url("ssh://git@github.com/user/rust-agtop"),
            Some("rust-agtop".into())
        );
    }

    #[test]
    fn parse_url_with_trailing_slash() {
        assert_eq!(
            parse_repo_name_from_url("https://github.com/user/rust-agtop/"),
            Some("rust-agtop".into())
        );
    }
}
