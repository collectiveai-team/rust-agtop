use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    emit_git_rerun_hints();

    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let version = git_describe()
        .map(|raw| normalize_git_describe(raw.trim(), &pkg_version))
        .unwrap_or_else(|| format!("v{pkg_version}"));

    println!("cargo:rustc-env=AGTOP_VERSION={version}");
}

fn git_describe() -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--dirty", "--always", "--match", "v*"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn normalize_git_describe(raw: &str, pkg_version: &str) -> String {
    if raw.starts_with('v') {
        // `git describe --dirty` may produce:
        //   v0.5.0              – exact tag, clean tree      → keep as-is
        //   v0.5.0-dirty        – exact tag, dirty tree      → strip -dirty
        //   v0.5.0-3-gabcdef    – N commits after tag        → strip -N-gHASH
        //   v0.5.0-3-gabcdef-dirty – commits after tag, dirty → strip both
        //
        // We want the stamped version to equal the nearest release tag so that
        // the update check does not fire when the local build is already at
        // (or based on) the latest published release.
        strip_git_describe_suffixes(raw)
    } else if raw.is_empty() {
        format!("v{pkg_version}")
    } else {
        format!("{pkg_version}+g{raw}")
    }
}

/// Strip `-dirty` and `-N-gHASH` (and `-N-gHASH-dirty`) suffixes that
/// `git describe --dirty` appends after the tag name.
fn strip_git_describe_suffixes(version: &str) -> String {
    // Work on the part after the leading 'v'.
    let (prefix, rest) = version.split_at(1); // prefix = "v"

    // Strip optional trailing -dirty.
    let rest = rest.strip_suffix("-dirty").unwrap_or(rest);

    // Strip optional -N-gHASH (exactly two dash-separated components where the
    // second starts with 'g' and the first is all digits).
    let rest = if let Some((base, commit_info)) = rest.rsplit_once('-') {
        if commit_info.starts_with('g') && commit_info[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            // base may still have -N at the end; strip that too.
            if let Some((base2, n)) = base.rsplit_once('-') {
                if n.chars().all(|c| c.is_ascii_digit()) {
                    base2
                } else {
                    base
                }
            } else {
                base
            }
        } else {
            rest
        }
    } else {
        rest
    };

    format!("{prefix}{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_exact_tag_clean() {
        assert_eq!(strip_git_describe_suffixes("v0.5.0"), "v0.5.0");
    }

    #[test]
    fn strip_dirty_suffix() {
        assert_eq!(strip_git_describe_suffixes("v0.5.0-dirty"), "v0.5.0");
    }

    #[test]
    fn strip_commits_ahead_suffix() {
        assert_eq!(strip_git_describe_suffixes("v0.5.0-3-gabcdef1"), "v0.5.0");
    }

    #[test]
    fn strip_commits_ahead_and_dirty() {
        assert_eq!(strip_git_describe_suffixes("v0.5.0-3-gabcdef1-dirty"), "v0.5.0");
    }

    #[test]
    fn preserve_rc_prerelease() {
        // rc tags are real version markers and must not be stripped.
        assert_eq!(strip_git_describe_suffixes("v0.5.0-rc1"), "v0.5.0-rc1");
        assert_eq!(strip_git_describe_suffixes("v0.5.0-rc2"), "v0.5.0-rc2");
    }

    #[test]
    fn normalize_passes_through_rc() {
        assert_eq!(normalize_git_describe("v0.5.0-rc1", "0.5.0"), "v0.5.0-rc1");
    }

    #[test]
    fn normalize_strips_dirty_from_release() {
        assert_eq!(normalize_git_describe("v0.5.0-dirty", "0.5.0"), "v0.5.0");
    }

    #[test]
    fn normalize_strips_dev_build_suffixes() {
        assert_eq!(normalize_git_describe("v0.5.0-3-gabcdef1", "0.5.0"), "v0.5.0");
        assert_eq!(normalize_git_describe("v0.5.0-3-gabcdef1-dirty", "0.5.0"), "v0.5.0");
    }

    #[test]
    fn normalize_non_v_prefix_uses_pkg_version() {
        // Hash-only output (no matching tag) falls back to pkg_version+ghash.
        assert_eq!(normalize_git_describe("abcdef1", "0.5.0"), "0.5.0+gabcdef1");
    }
}

fn emit_git_rerun_hints() {
    let Some(git_dir) = resolve_git_dir() else {
        return;
    };

    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    println!("cargo:rerun-if-changed={}", git_dir.join("index").display());

    if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) {
        if let Some(reference) = head.strip_prefix("ref: ") {
            println!(
                "cargo:rerun-if-changed={}",
                git_dir.join(reference.trim()).display()
            );
        }
    }
}

fn resolve_git_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let dot_git = manifest_dir.parent()?.parent()?.join(".git");

    if dot_git.is_dir() {
        return Some(dot_git);
    }

    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let git_dir = contents.strip_prefix("gitdir: ")?.trim();
    let git_dir = Path::new(git_dir);
    if git_dir.is_absolute() {
        Some(git_dir.to_path_buf())
    } else {
        Some(dot_git.parent()?.join(git_dir))
    }
}
