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
        raw.to_owned()
    } else if raw.is_empty() {
        format!("v{pkg_version}")
    } else {
        format!("{pkg_version}+g{raw}")
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
