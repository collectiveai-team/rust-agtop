use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::session::ClientKind;

pub const LOGO_BASE_URL: &str = "https://models.dev/logos";

pub const LOGO_TTL_SECS: u64 = 7 * 24 * 60 * 60;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

pub fn logo_provider_id(client: ClientKind) -> Option<&'static str> {
    match client {
        ClientKind::Claude => Some("anthropic"),
        ClientKind::Codex => Some("openai"),
        ClientKind::OpenCode => Some("opencode"),
        ClientKind::Copilot => Some("github-copilot"),
        ClientKind::GeminiCli => Some("google"),
        ClientKind::Cursor => None,
        ClientKind::Antigravity => None,
    }
}

pub fn logo_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("agtop").join("logos"))
}

pub fn logo_cache_path(provider_id: &str) -> Option<PathBuf> {
    Some(logo_dir()?.join(format!("{provider_id}.svg")))
}

pub fn is_logo_fresh(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() == 0 {
        return false;
    }
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime
        .elapsed()
        .map(|age| age < Duration::from_secs(LOGO_TTL_SECS))
        .unwrap_or(false)
}

pub fn fetch_and_cache(provider_id: &str) -> Result<Vec<u8>, String> {
    let path = logo_cache_path(provider_id).ok_or("cache dir unavailable")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .user_agent(concat!("rust-agtop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();

    let url = format!("{LOGO_BASE_URL}/{provider_id}.svg");
    let mut resp = agent.get(&url).call().map_err(|e| e.to_string())?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }

    let mut body = Vec::with_capacity(64 * 1024);
    resp.body_mut()
        .as_reader()
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;

    let tmp = path.with_extension("svg.tmp");
    fs::write(&tmp, &body).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;

    tracing::debug!(provider = provider_id, bytes = body.len(), "cached logo");
    Ok(body)
}

pub fn load_or_fetch(provider_id: &str) -> Option<Vec<u8>> {
    if let Some(path) = logo_cache_path(provider_id) {
        if is_logo_fresh(&path) {
            if let Ok(data) = fs::read(&path) {
                return Some(data);
            }
        }
    }
    fetch_and_cache(provider_id).ok()
}

pub fn load_all_logos() -> HashMap<ClientKind, Vec<u8>> {
    let mut out = HashMap::new();
    for &kind in ClientKind::all() {
        if let Some(pid) = logo_provider_id(kind) {
            if let Some(data) = load_or_fetch(pid) {
                out.insert(kind, data);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_provider_id_mapping() {
        assert_eq!(logo_provider_id(ClientKind::Claude), Some("anthropic"));
        assert_eq!(logo_provider_id(ClientKind::Codex), Some("openai"));
        assert_eq!(logo_provider_id(ClientKind::OpenCode), Some("opencode"));
        assert_eq!(
            logo_provider_id(ClientKind::Copilot),
            Some("github-copilot")
        );
        assert_eq!(logo_provider_id(ClientKind::GeminiCli), Some("google"));
        assert_eq!(logo_provider_id(ClientKind::Cursor), None);
        assert_eq!(logo_provider_id(ClientKind::Antigravity), None);
    }
}
