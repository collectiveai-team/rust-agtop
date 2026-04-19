use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::pricing::Rates;
use crate::session::ClientKind;

pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

pub const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Default, Deserialize)]
struct RawCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default)]
    reasoning: Option<f64>,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawLimit {
    #[serde(default)]
    context: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawModel {
    #[serde(default)]
    cost: RawCost,
    #[serde(default)]
    limit: RawLimit,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawProvider {
    #[serde(default)]
    models: HashMap<String, RawModel>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CacheMeta {
    fetched_at: chrono::DateTime<chrono::Utc>,
    source_url: String,
    provider_count: usize,
    model_count: usize,
}

#[derive(Debug, Default, Clone)]
pub struct ModelsDevIndex {
    by_key: HashMap<(String, String), Rates>,
    by_model: HashMap<String, Rates>,
    ctx_by_key: HashMap<(String, String), u64>,
    ctx_by_model: HashMap<String, u64>,
}

fn provider_ids_for(client: ClientKind) -> &'static [&'static str] {
    match client {
        ClientKind::Claude => &["anthropic"],
        ClientKind::Codex => &["openai"],
        ClientKind::OpenCode => &["opencode", "anthropic", "openai", "google"],
        ClientKind::Copilot => &["github-copilot"],
        ClientKind::GeminiCli => &["google"],
        ClientKind::Cursor => &[],
        ClientKind::Antigravity => &[],
    }
}

impl ModelsDevIndex {
    pub fn from_json(raw: &serde_json::Value) -> Self {
        let mut by_key: HashMap<(String, String), Rates> = HashMap::new();
        let mut by_model: HashMap<String, Rates> = HashMap::new();
        let mut ctx_by_key: HashMap<(String, String), u64> = HashMap::new();
        let mut ctx_by_model: HashMap<String, u64> = HashMap::new();

        let Some(obj) = raw.as_object() else {
            return Self {
                by_key,
                by_model,
                ctx_by_key,
                ctx_by_model,
            };
        };

        for (provider_id, provider_val) in obj {
            let provider: RawProvider = match serde_json::from_value(provider_val.clone()) {
                Ok(p) => p,
                Err(_) => continue,
            };
            for (model_id, model) in provider.models {
                let Some(input) = model.cost.input else {
                    continue;
                };
                if input < 0.0 {
                    continue;
                }
                let rates = Rates {
                    input_per_m: input,
                    cached_input_per_m: model.cost.cache_read.unwrap_or(0.0),
                    output_per_m: model.cost.output.unwrap_or(0.0),
                    cache_write_5m_per_m: model.cost.cache_write.unwrap_or(0.0),
                    cache_write_1h_per_m: 0.0,
                    cache_read_per_m: model.cost.cache_read.unwrap_or(0.0),
                };
                by_key.insert((provider_id.clone(), model_id.clone()), rates);
                by_model.entry(model_id.clone()).or_insert(rates);
                if let Some(ctx) = model.limit.context.filter(|w| *w > 0) {
                    ctx_by_key.insert((provider_id.clone(), model_id.clone()), ctx);
                    ctx_by_model.entry(model_id).or_insert(ctx);
                }
            }
        }

        Self {
            by_key,
            by_model,
            ctx_by_key,
            ctx_by_model,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn lookup(&self, client: ClientKind, model: &str) -> Option<Rates> {
        let model = strip_provider_prefix(model);
        let providers = provider_ids_for(client);

        for pid in providers {
            let key = ((*pid).to_string(), model.to_string());
            if let Some(r) = self.by_key.get(&key) {
                return Some(*r);
            }
        }

        let trimmed = strip_date_suffix(model);
        if trimmed != model {
            for pid in providers {
                let key = ((*pid).to_string(), trimmed.to_string());
                if let Some(r) = self.by_key.get(&key) {
                    return Some(*r);
                }
            }
        }

        if providers.is_empty() {
            return None;
        }

        if let Some(r) = self.by_model.get(model) {
            return Some(*r);
        }
        if trimmed != model {
            if let Some(r) = self.by_model.get(trimmed) {
                return Some(*r);
            }
        }

        let mut best: Option<(usize, Rates)> = None;
        for (k, r) in &self.by_model {
            if model.starts_with(k.as_str()) {
                let len = k.len();
                if best.map(|(prev, _)| prev < len).unwrap_or(true) {
                    best = Some((len, *r));
                }
            }
        }
        best.map(|(_, r)| r)
    }

    pub fn lookup_context_window(&self, client: ClientKind, model: &str) -> Option<u64> {
        let model = strip_provider_prefix(model);
        let providers = provider_ids_for(client);

        for pid in providers {
            let key = ((*pid).to_string(), model.to_string());
            if let Some(w) = self.ctx_by_key.get(&key).copied() {
                return Some(w);
            }
        }

        let trimmed = strip_date_suffix(model);
        if trimmed != model {
            for pid in providers {
                let key = ((*pid).to_string(), trimmed.to_string());
                if let Some(w) = self.ctx_by_key.get(&key).copied() {
                    return Some(w);
                }
            }
        }

        if providers.is_empty() {
            return None;
        }

        if let Some(w) = self.ctx_by_model.get(model).copied() {
            return Some(w);
        }
        if trimmed != model {
            if let Some(w) = self.ctx_by_model.get(trimmed).copied() {
                return Some(w);
            }
        }

        let mut best: Option<(usize, u64)> = None;
        for (k, w) in &self.ctx_by_model {
            if model.starts_with(k.as_str()) {
                let len = k.len();
                if best.map(|(prev, _)| prev < len).unwrap_or(true) {
                    best = Some((len, *w));
                }
            }
        }
        best.map(|(_, w)| w)
    }
}

fn strip_provider_prefix(model: &str) -> &str {
    if let Some((_, suffix)) = model.rsplit_once('/') {
        if !suffix.is_empty() {
            return suffix;
        }
    }
    model
}

fn strip_date_suffix(model: &str) -> &str {
    if let Some(idx) = model.rfind('-') {
        let tail = &model[idx + 1..];
        if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) {
            return &model[..idx];
        }
    }
    model
}

pub fn cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("agtop").join("models-dev-pricing.json"))
}

fn meta_path(cache_file: &Path) -> PathBuf {
    cache_file.with_extension("json.meta")
}

pub fn is_cache_fresh(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() == 0 {
        return false;
    }
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime.elapsed().map(|age| age < CACHE_TTL).unwrap_or(false)
}

pub fn load_from_cache() -> Option<ModelsDevIndex> {
    let path = cache_path()?;
    let data = fs::read_to_string(&path).ok()?;
    let raw: serde_json::Value = serde_json::from_str(&data).ok()?;
    let idx = ModelsDevIndex::from_json(&raw);
    if idx.is_empty() {
        None
    } else {
        Some(idx)
    }
}

pub fn refresh_cache() -> Result<ModelsDevIndex, ModelsDevError> {
    let path = cache_path().ok_or(ModelsDevError::NoCacheDir)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ModelsDevError::Io)?;
    }

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .user_agent(concat!("rust-agtop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();

    let mut resp = agent
        .get(MODELS_DEV_URL)
        .call()
        .map_err(|e| ModelsDevError::Http(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(ModelsDevError::Http(format!("HTTP {status}")));
    }

    let mut body = Vec::with_capacity(2 * 1024 * 1024);
    resp.body_mut()
        .as_reader()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .map_err(ModelsDevError::Io)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(ModelsDevError::TooLarge(body.len()));
    }

    let parsed: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| ModelsDevError::Parse(e.to_string()))?;
    let idx = ModelsDevIndex::from_json(&parsed);
    if idx.is_empty() {
        return Err(ModelsDevError::Parse(
            "models.dev response contained no usable model entries".into(),
        ));
    }

    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &body).map_err(ModelsDevError::Io)?;
    fs::rename(&tmp, &path).map_err(ModelsDevError::Io)?;

    let meta = CacheMeta {
        fetched_at: chrono::Utc::now(),
        source_url: MODELS_DEV_URL.into(),
        provider_count: 0,
        model_count: idx.len(),
    };
    if let Ok(s) = serde_json::to_string_pretty(&meta) {
        let _ = fs::write(meta_path(&path), s);
    }

    tracing::info!(entries = idx.len(), path = %path.display(), "refreshed models.dev pricing cache");
    Ok(idx)
}

#[derive(Debug, thiserror::Error)]
pub enum ModelsDevError {
    #[error("cache dir could not be resolved")]
    NoCacheDir,
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("response body exceeded {MAX_RESPONSE_BYTES} bytes (got {0})")]
    TooLarge(usize),
    #[error("parse error: {0}")]
    Parse(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> serde_json::Value {
        serde_json::json!({
            "anthropic": {
                "id": "anthropic", "name": "Anthropic",
                "models": {
                    "claude-opus-4-7": { "name": "Claude Opus 4.7", "cost": { "input": 5.0, "output": 25.0, "cache_read": 0.5, "cache_write": 6.25 }, "limit": { "context": 200000 } },
                    "claude-haiku-4-5": { "name": "Claude Haiku 4.5", "cost": { "input": 1.0, "output": 5.0, "cache_read": 0.1, "cache_write": 1.25 }, "limit": { "context": 200000 } }
                }
            },
            "openai": {
                "id": "openai", "name": "OpenAI",
                "models": {
                    "gpt-5.4": { "name": "GPT-5.4", "cost": { "input": 2.5, "output": 15.0, "cache_read": 0.25 }, "limit": { "context": 400000 } }
                }
            },
            "github-copilot": {
                "id": "github-copilot", "name": "GitHub Copilot",
                "models": {
                    "gpt-4.1": { "cost": { "input": 0.0, "output": 0.0 } }
                }
            }
        })
    }

    #[test]
    fn index_parses_provider_aware() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        let r = idx
            .lookup(ClientKind::Claude, "claude-opus-4-7")
            .expect("should find claude-opus-4-7 via anthropic provider");
        assert!((r.input_per_m - 5.0).abs() < 1e-9);
        assert!((r.output_per_m - 25.0).abs() < 1e-9);
        assert!((r.cache_read_per_m - 0.5).abs() < 1e-9);
        assert!((r.cache_write_5m_per_m - 6.25).abs() < 1e-9);
    }

    #[test]
    fn index_resolves_codex_via_openai_provider() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        let r = idx
            .lookup(ClientKind::Codex, "gpt-5.4")
            .expect("should find gpt-5.4 via openai provider");
        assert!((r.input_per_m - 2.5).abs() < 1e-9);
        assert!((r.output_per_m - 15.0).abs() < 1e-9);
    }

    #[test]
    fn index_strips_slash_prefix_for_opencode() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        let r = idx
            .lookup(ClientKind::OpenCode, "anthropic/claude-opus-4-7")
            .expect("should strip anthropic/ prefix and find via anthropic provider");
        assert!((r.input_per_m - 5.0).abs() < 1e-9);
    }

    #[test]
    fn index_returns_none_for_cursor() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        assert!(idx.lookup(ClientKind::Cursor, "claude-opus-4-7").is_none());
    }

    #[test]
    fn index_context_window_lookup() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        let ctx = idx
            .lookup_context_window(ClientKind::Claude, "claude-opus-4-7")
            .expect("should find context window");
        assert_eq!(ctx, 200_000);
    }

    #[test]
    fn index_empty_on_garbage() {
        let idx = ModelsDevIndex::from_json(&serde_json::json!("not an object"));
        assert!(idx.is_empty());
    }

    #[test]
    fn strip_date_suffix_same_behavior() {
        assert_eq!(strip_date_suffix("claude-sonnet-4-5"), "claude-sonnet-4-5");
        assert_eq!(
            strip_date_suffix("claude-sonnet-4-5-20250929"),
            "claude-sonnet-4-5"
        );
        assert_eq!(strip_date_suffix("gpt-4o-123"), "gpt-4o-123");
    }
}
