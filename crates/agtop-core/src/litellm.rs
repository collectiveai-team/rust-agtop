//! LiteLLM pricing cache.
//!
//! The upstream [`model_prices_and_context_window.json`][src] file is the
//! industry de-facto source for per-model API prices. We mirror it to
//! `$XDG_CACHE_HOME/agtop/litellm-pricing.json` (falling back to
//! `~/.cache/agtop/` on unix) with a 24h TTL and expose a
//! [`PricingIndex`] that [`crate::pricing::lookup`] consults before falling
//! back to the hard-coded built-in tables.
//!
//! [src]: https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json
//!
//! Design choices:
//! - **Sync HTTP, no tokio**: agtop is a short-lived CLI; an async runtime
//!   would be strict overkill. `ureq` with `rustls` is ~1 MB of extra deps
//!   and needs no OpenSSL.
//! - **No panics on network failure**: every fetch path returns a `Result`
//!   and the caller degrades to the built-in tables.
//! - **Cache-first**: we never block on a network request when a usable
//!   cache file exists. When the cache is missing or stale, the caller
//!   chooses whether to refresh synchronously or skip.
//! - **Conservative parsing**: we only read the fields we need. Unknown
//!   fields and non-chat modes (`image_generation`, `embedding`, …) are
//!   ignored so new LiteLLM entries can't break us.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::pricing::Rates;
use crate::session::ProviderKind;

/// Upstream URL of the LiteLLM model pricing table.
pub const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// How long a cached copy is considered fresh before we want to refresh.
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// HTTP timeout for a single fetch attempt. Short on purpose: we should
/// never keep the user waiting if GitHub is slow.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Soft ceiling on the JSON response size (10 MB). The current file is
/// ~1.5 MB as of this writing; anything an order of magnitude larger is
/// almost certainly pathological.
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Raw schema (only the fields we use)
// ---------------------------------------------------------------------------

/// One model entry in `model_prices_and_context_window.json`.
///
/// Fields are `input_cost_per_token` (USD *per single token*; multiply by
/// 1e6 to get USD/M which is what `Rates` stores). Everything except
/// `input_cost_per_token` is optional because many entries are partial.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawEntry {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    input_cost_per_token: Option<f64>,
    #[serde(default)]
    output_cost_per_token: Option<f64>,
    #[serde(default)]
    cache_read_input_token_cost: Option<f64>,
    /// Claude-style 5-minute ephemeral cache write price.
    #[serde(default)]
    cache_creation_input_token_cost: Option<f64>,
    /// Claude-style 1-hour ephemeral cache write price.
    #[serde(default)]
    cache_creation_input_token_cost_above_1hr: Option<f64>,
}

/// Optional wrapper written alongside the cache to record the timestamp
/// of the last successful fetch. Kept separate from the JSON payload so
/// the cache file itself is a verbatim copy of upstream.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct CacheMeta {
    fetched_at: chrono::DateTime<chrono::Utc>,
    source_url: String,
    entry_count: usize,
}

// ---------------------------------------------------------------------------
// Public index
// ---------------------------------------------------------------------------

/// Model → rates lookup built from a LiteLLM JSON dump.
#[derive(Debug, Default, Clone)]
pub struct PricingIndex {
    by_key: HashMap<String, Rates>,
}

impl PricingIndex {
    /// Build an index from already-parsed JSON. `Rates` are only produced
    /// for chat/completion-style entries with a non-zero
    /// `input_cost_per_token`. Image generation, embeddings, and the like
    /// are dropped.
    pub fn from_json(raw: &serde_json::Value) -> Self {
        let mut by_key: HashMap<String, Rates> = HashMap::new();
        let Some(obj) = raw.as_object() else {
            return Self { by_key };
        };
        for (key, val) in obj {
            if key == "sample_spec" {
                continue;
            }
            let entry: RawEntry = match serde_json::from_value(val.clone()) {
                Ok(e) => e,
                Err(_) => continue,
            };
            // We only care about chat-ish modes. `mode` may be absent for
            // some older entries — in that case we keep them and let the
            // absence of `input_cost_per_token` filter them out.
            if let Some(mode) = entry.mode.as_deref() {
                match mode {
                    "chat" | "completion" | "responses" => {}
                    _ => continue,
                }
            }
            let Some(input_per_tok) = entry.input_cost_per_token else {
                continue;
            };
            if input_per_tok <= 0.0 {
                continue;
            }
            let per_m = |v: Option<f64>| v.unwrap_or(0.0) * 1_000_000.0;
            let input_per_m = input_per_tok * 1_000_000.0;
            let cache_read_per_m = per_m(entry.cache_read_input_token_cost);
            let cache_write_5m_per_m = per_m(entry.cache_creation_input_token_cost);
            let cache_write_1h_per_m = per_m(entry.cache_creation_input_token_cost_above_1hr);
            let output_per_m = per_m(entry.output_cost_per_token);

            let rates = Rates {
                input_per_m,
                // LiteLLM has no separate "cached input" bucket. Match the
                // semantics of `Rates::codex` / `Rates::claude` by mapping
                // it to cache_read.
                cached_input_per_m: cache_read_per_m,
                output_per_m,
                cache_write_5m_per_m,
                cache_write_1h_per_m,
                cache_read_per_m,
            };

            // Store the primary key as-is …
            by_key.insert(key.clone(), rates);
            // … and also store variants that strip LiteLLM's provider
            // prefixes so transcripts that report a bare model name still
            // hit. e.g. "anthropic.claude-opus-4-7" → also index under
            // "claude-opus-4-7"; "us.anthropic.claude-opus-4-7" → same.
            for stripped in strip_provider_prefixes(key) {
                by_key.entry(stripped).or_insert(rates);
            }
        }
        Self { by_key }
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Look up a model by best-effort matching. Tries the exact key, then
    /// a few prefix normalizations. Returns `None` when nothing matches;
    /// the caller is expected to fall back to the built-in table.
    pub fn lookup(&self, provider: ProviderKind, model: &str) -> Option<Rates> {
        // Exact.
        if let Some(r) = self.by_key.get(model) {
            return Some(*r);
        }
        // Date-suffix trim: "claude-sonnet-4-5-20250929" → "claude-sonnet-4-5".
        let trimmed = strip_date_suffix(model);
        if trimmed != model {
            if let Some(r) = self.by_key.get(trimmed) {
                return Some(*r);
            }
        }
        // Provider prefix ("anthropic.claude-opus-4-7", "openai/gpt-5.3-codex").
        for candidate in prefix_candidates(provider, model) {
            if let Some(r) = self.by_key.get(&candidate) {
                return Some(*r);
            }
        }
        // Loose: longest key that is a prefix of `model`.
        let mut best: Option<(usize, Rates)> = None;
        for (k, r) in &self.by_key {
            if model.starts_with(k.as_str()) {
                let len = k.len();
                if best.map(|(prev, _)| prev < len).unwrap_or(true) {
                    best = Some((len, *r));
                }
            }
        }
        best.map(|(_, r)| r)
    }
}

// ---------------------------------------------------------------------------
// Key-normalization helpers
// ---------------------------------------------------------------------------

/// Produce candidate keys for a model after adding/removing the common
/// provider prefixes LiteLLM uses.
fn prefix_candidates(provider: ProviderKind, model: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(4);
    // Strip "provider/model" → "model" (OpenCode reports these).
    if let Some((_, suffix)) = model.rsplit_once('/') {
        if !suffix.is_empty() {
            out.push(suffix.to_string());
        }
    }
    match provider {
        ProviderKind::Claude | ProviderKind::OpenCode => {
            out.push(format!("anthropic.{}", model));
            out.push(format!("anthropic/{}", model));
        }
        ProviderKind::Codex => {
            out.push(format!("openai/{}", model));
        }
    }
    out
}

/// Strip regional/partition prefixes LiteLLM adds (us., eu., global., ap., au.).
fn strip_provider_prefixes(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    const REGIONS: &[&str] = &["us.", "eu.", "global.", "ap.", "au."];
    let mut work = key.to_string();
    for r in REGIONS {
        if let Some(rest) = work.strip_prefix(r) {
            out.push(rest.to_string());
            work = rest.to_string();
            break;
        }
    }
    const PROVIDERS: &[&str] = &["anthropic.", "openai.", "bedrock_converse."];
    for p in PROVIDERS {
        if let Some(rest) = work.strip_prefix(p) {
            out.push(rest.to_string());
        }
    }
    out
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

// ---------------------------------------------------------------------------
// Cache location + I/O
// ---------------------------------------------------------------------------

/// Resolve the path to the pricing cache file. Returns `None` if we can't
/// figure out a cache root (very unusual — would mean $HOME is unset).
pub fn cache_path() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("agtop").join("litellm-pricing.json"))
}

fn meta_path(cache_file: &Path) -> PathBuf {
    cache_file.with_extension("json.meta")
}

/// Is the cache file fresh (< CACHE_TTL old) and non-empty?
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

/// Load a [`PricingIndex`] from the on-disk cache if present and parseable.
/// Freshness is NOT enforced here — call [`is_cache_fresh`] first if you
/// want to respect the TTL.
pub fn load_from_cache() -> Option<PricingIndex> {
    let path = cache_path()?;
    let data = fs::read_to_string(&path).ok()?;
    let raw: serde_json::Value = serde_json::from_str(&data).ok()?;
    let idx = PricingIndex::from_json(&raw);
    if idx.is_empty() {
        None
    } else {
        Some(idx)
    }
}

/// Download the LiteLLM JSON, write it to the cache atomically, and
/// return a freshly-built index. Intended to be called on-demand (e.g.
/// `--refresh-pricing`) or when the cache is stale at startup.
///
/// Returns an error — rather than panicking or blocking forever — when
/// the network is unavailable. Callers should treat failure as "keep
/// using built-in pricing" and not abort the program.
pub fn refresh_cache() -> Result<PricingIndex, LiteLlmError> {
    let path = cache_path().ok_or(LiteLlmError::NoCacheDir)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(LiteLlmError::Io)?;
    }

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .user_agent(concat!("rust-agtop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();

    let mut resp = agent
        .get(LITELLM_URL)
        .call()
        .map_err(|e| LiteLlmError::Http(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(LiteLlmError::Http(format!("HTTP {status}")));
    }

    // Cap the body size so a misbehaving mirror can't exhaust memory.
    let mut body = Vec::with_capacity(2 * 1024 * 1024);
    resp.body_mut()
        .as_reader()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .map_err(LiteLlmError::Io)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(LiteLlmError::TooLarge(body.len()));
    }

    // Parse first — never overwrite a good cache with garbage.
    let parsed: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| LiteLlmError::Parse(e.to_string()))?;
    let idx = PricingIndex::from_json(&parsed);
    if idx.is_empty() {
        return Err(LiteLlmError::Parse(
            "LiteLLM response contained no usable model entries".into(),
        ));
    }

    // Atomic write: tmp file + rename.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &body).map_err(LiteLlmError::Io)?;
    fs::rename(&tmp, &path).map_err(LiteLlmError::Io)?;

    // Best-effort meta file; failure is non-fatal.
    let meta = CacheMeta {
        fetched_at: chrono::Utc::now(),
        source_url: LITELLM_URL.into(),
        entry_count: idx.len(),
    };
    if let Ok(s) = serde_json::to_string_pretty(&meta) {
        let _ = fs::write(meta_path(&path), s);
    }

    tracing::info!(entries = idx.len(), path = %path.display(), "refreshed LiteLLM pricing cache");
    Ok(idx)
}

/// Error kinds from the network path. Non-exhaustive on purpose.
#[derive(Debug, thiserror::Error)]
pub enum LiteLlmError {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal in-memory JSON covering the three shapes of entries we care
    /// about: OpenAI chat, Anthropic bedrock_converse, and a noise entry.
    fn sample_json() -> serde_json::Value {
        serde_json::json!({
            "sample_spec": {"mode": "chat"},
            "gpt-5.4": {
                "litellm_provider": "openai",
                "mode": "chat",
                "input_cost_per_token": 2.5e-06,
                "output_cost_per_token": 1.5e-05,
                "cache_read_input_token_cost": 2.5e-07
            },
            "gpt-5.3-codex": {
                "litellm_provider": "openai",
                "mode": "responses",
                "input_cost_per_token": 1.75e-06,
                "output_cost_per_token": 1.4e-05,
                "cache_read_input_token_cost": 1.75e-07
            },
            "us.anthropic.claude-opus-4-7": {
                "litellm_provider": "bedrock_converse",
                "mode": "chat",
                "input_cost_per_token": 5.5e-06,
                "output_cost_per_token": 2.75e-05,
                "cache_read_input_token_cost": 5.5e-07,
                "cache_creation_input_token_cost": 6.875e-06
            },
            // Image generation: must be dropped.
            "dall-e-3": {
                "litellm_provider": "openai",
                "mode": "image_generation",
                "output_cost_per_image": 0.04
            }
        })
    }

    #[test]
    fn index_filters_non_chat_modes() {
        let idx = PricingIndex::from_json(&sample_json());
        // dall-e-3 must be gone.
        assert!(idx.lookup(ProviderKind::Codex, "dall-e-3").is_none());
        // sample_spec never gets stored.
        assert!(idx.lookup(ProviderKind::Codex, "sample_spec").is_none());
    }

    #[test]
    fn index_resolves_bare_model_via_prefix_stripping() {
        let idx = PricingIndex::from_json(&sample_json());
        // Transcript reports "claude-opus-4-7"; LiteLLM key is
        // "us.anthropic.claude-opus-4-7". Must match through prefix
        // stripping during indexing.
        let r = idx
            .lookup(ProviderKind::Claude, "claude-opus-4-7")
            .expect("claude opus 4-7 must be resolvable");
        assert!((r.input_per_m - 5.5).abs() < 1e-9);
        assert!((r.output_per_m - 27.5).abs() < 1e-9);
        assert!((r.cache_write_5m_per_m - 6.875).abs() < 1e-9);
        // LiteLLM has no 1h field on this entry — should default to 0.
        assert_eq!(r.cache_write_1h_per_m, 0.0);
    }

    #[test]
    fn index_units_are_usd_per_million() {
        let idx = PricingIndex::from_json(&sample_json());
        let r = idx
            .lookup(ProviderKind::Codex, "gpt-5.4")
            .expect("gpt-5.4 present");
        // 2.5e-06 per token → $2.50 per million.
        assert!((r.input_per_m - 2.5).abs() < 1e-9);
        assert!((r.output_per_m - 15.0).abs() < 1e-9);
        assert!((r.cache_read_per_m - 0.25).abs() < 1e-9);
    }

    #[test]
    fn index_supports_responses_mode_for_codex() {
        let idx = PricingIndex::from_json(&sample_json());
        assert!(idx.lookup(ProviderKind::Codex, "gpt-5.3-codex").is_some());
    }

    #[test]
    fn index_date_suffix_fallback() {
        let idx = PricingIndex::from_json(&serde_json::json!({
            "claude-sonnet-4-5": {
                "litellm_provider": "anthropic",
                "mode": "chat",
                "input_cost_per_token": 3e-06,
                "output_cost_per_token": 1.5e-05,
                "cache_read_input_token_cost": 3e-07
            }
        }));
        assert!(idx
            .lookup(ProviderKind::Claude, "claude-sonnet-4-5-20250929")
            .is_some());
    }

    #[test]
    fn index_opencode_provider_prefix() {
        let idx = PricingIndex::from_json(&serde_json::json!({
            "claude-haiku-4-5": {
                "litellm_provider": "anthropic",
                "mode": "chat",
                "input_cost_per_token": 1e-06,
                "output_cost_per_token": 5e-06,
                "cache_read_input_token_cost": 1e-07
            }
        }));
        assert!(idx
            .lookup(ProviderKind::OpenCode, "anthropic/claude-haiku-4-5")
            .is_some());
    }

    #[test]
    fn index_ignores_missing_input_cost() {
        let idx = PricingIndex::from_json(&serde_json::json!({
            "deprecated-model": {
                "litellm_provider": "openai",
                "mode": "chat",
                "output_cost_per_token": 1e-05
            }
        }));
        assert!(idx.is_empty());
    }

    #[test]
    fn strip_date_suffix_behaviour() {
        assert_eq!(strip_date_suffix("claude-sonnet-4-5"), "claude-sonnet-4-5");
        assert_eq!(
            strip_date_suffix("claude-sonnet-4-5-20250929"),
            "claude-sonnet-4-5"
        );
        // Only 8-digit tails, not arbitrary numerics.
        assert_eq!(strip_date_suffix("gpt-4o-123"), "gpt-4o-123");
    }

    #[test]
    fn strip_provider_prefixes_handles_region_and_provider() {
        let v = strip_provider_prefixes("us.anthropic.claude-opus-4-7");
        assert!(v.iter().any(|s| s == "anthropic.claude-opus-4-7"));
        assert!(v.iter().any(|s| s == "claude-opus-4-7"));
    }
}
