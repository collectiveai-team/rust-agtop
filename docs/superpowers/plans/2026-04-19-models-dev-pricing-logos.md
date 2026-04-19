# models.dev Pricing + Logos Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace LiteLLM as the primary pricing source with models.dev, keeping LiteLLM as fallback. Add provider logo rendering in the dashboard plan panel.

**Architecture:** New `models_dev.rs` module fetches/parses `models.dev/api.json` into a provider-aware `ModelsDevIndex`. New `logo.rs` fetches/caches SVG logos. `pricing.rs` lookup chain becomes: models.dev → LiteLLM → hardcoded. Logos render via `ratatui-image` in `dashboard_plan.rs`.

**Tech Stack:** `ureq` (existing), `serde`/`serde_json` (existing), `ratatui-image` v10 (new), `image` crate (new).

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/agtop-core/src/models_dev.rs` | **New** — fetch, parse, cache models.dev API; `ModelsDevIndex` with provider-aware lookup |
| `crates/agtop-core/src/logo.rs` | **New** — fetch, cache, decode SVG logos from models.dev |
| `crates/agtop-core/src/pricing.rs` | **Modify** — three-tier lookup (`PricingSource` struct replaces single `PricingIndex`) |
| `crates/agtop-core/src/lib.rs` | **Modify** — declare `models_dev`, `logo` modules |
| `crates/agtop-cli/src/main.rs` | **Modify** — `setup_pricing` fetches both sources; logo loading |
| `crates/agtop-cli/Cargo.toml` | **Modify** — add `ratatui-image`, `image` deps |
| `crates/agtop-cli/src/tui/app/mod.rs` | **Modify** — `App` gains `logos` field |
| `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs` | **Modify** — render logos next to subscription names |
| `crates/agtop-core/src/litellm.rs` | **Unchanged** — remains as-is for fallback |

---

### Task 1: Create `models_dev.rs` — raw schema + `ModelsDevIndex`

**Files:**
- Create: `crates/agtop-core/src/models_dev.rs`
- Modify: `crates/agtop-core/src/lib.rs`

- [ ] **Step 1: Add `models_dev` module declaration to `lib.rs`**

In `crates/agtop-core/src/lib.rs`, add after the `pub mod litellm;` line:

```rust
pub mod models_dev;
```

- [ ] **Step 2: Write `models_dev.rs` with raw schema structs, `ModelsDevIndex`, and lookup logic**

Create `crates/agtop-core/src/models_dev.rs`:

```rust
//! models.dev pricing cache.
//!
//! The upstream [models.dev API](https://models.dev/api.json) provides
//! per-model pricing organized by provider, with costs already expressed
//! as USD per million tokens. We cache to
//! `$XDG_CACHE_HOME/agtop/models-dev-pricing.json` with a 24h TTL and
//! expose a [`ModelsDevIndex`] that [`crate::pricing::lookup`] consults
//! as the first-tier source before LiteLLM and built-in tables.

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

/// Which models.dev provider IDs to search for a given [`ClientKind`].
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

/// Provider-aware pricing index built from the models.dev API JSON.
#[derive(Debug, Default, Clone)]
pub struct ModelsDevIndex {
    by_key: HashMap<(String, String), Rates>,
    by_model: HashMap<String, Rates>,
    ctx_by_key: HashMap<(String, String), u64>,
}

impl ModelsDevIndex {
    pub fn from_json(raw: &serde_json::Value) -> Self {
        let mut by_key: HashMap<(String, String), Rates> = HashMap::new();
        let mut by_model: HashMap<String, Rates> = HashMap::new();
        let mut ctx_by_key: HashMap<(String, String), u64> = HashMap::new();

        let Some(providers) = raw.as_object() else {
            return Self { by_key, by_model, ctx_by_key };
        };

        for (provider_id, provider_val) in providers {
            let provider: RawProvider = match serde_json::from_value(provider_val.clone()) {
                Ok(p) => p,
                Err(_) => continue,
            };

            for (model_id, model) in provider.models {
                let Some(input_cost) = model.cost.input else {
                    continue;
                };
                if input_cost < 0.0 {
                    continue;
                }

                let rates = Rates {
                    input_per_m: input_cost,
                    cached_input_per_m: model.cost.cache_read.unwrap_or(0.0),
                    output_per_m: model.cost.output.unwrap_or(0.0),
                    cache_write_5m_per_m: model.cost.cache_write.unwrap_or(0.0),
                    cache_write_1h_per_m: 0.0,
                    cache_read_per_m: model.cost.cache_read.unwrap_or(0.0),
                };

                let key = (provider_id.clone(), model_id.clone());
                by_key.insert(key.clone(), rates);
                by_model.entry(model_id.clone()).or_insert(rates);

                if let Some(ctx) = model.limit.context.filter(|&c| c > 0) {
                    ctx_by_key.insert(key, ctx);
                }
            }
        }

        Self { by_key, by_model, ctx_by_key }
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn lookup(&self, client: ClientKind, model: &str) -> Option<Rates> {
        let providers = provider_ids_for(client);

        let model_clean = if let Some((_, suffix)) = model.rsplit_once('/') {
            suffix
        } else {
            model
        };

        for pid in providers {
            if let Some(r) = self.by_key.get(&(*pid.to_string(), model_clean.to_string())) {
                return Some(*r);
            }
            let trimmed = strip_date_suffix(model_clean);
            if trimmed != model_clean {
                if let Some(r) = self.by_key.get(&(*pid.to_string(), trimmed.to_string())) {
                    return Some(*r);
                }
            }
        }

        if let Some(r) = self.by_model.get(model_clean) {
            return Some(*r);
        }

        let trimmed = strip_date_suffix(model_clean);
        if trimmed != model_clean {
            if let Some(r) = self.by_model.get(trimmed) {
                return Some(*r);
            }
        }

        let mut best: Option<(usize, Rates)> = None;
        for (k, r) in &self.by_model {
            if model_clean.starts_with(k.as_str()) {
                if best.map(|(prev, _)| prev < k.len()).unwrap_or(true) {
                    best = Some((k.len(), *r));
                }
            }
        }
        best.map(|(_, r)| r)
    }

    pub fn lookup_context_window(&self, client: ClientKind, model: &str) -> Option<u64> {
        let providers = provider_ids_for(client);

        let model_clean = if let Some((_, suffix)) = model.rsplit_once('/') {
            suffix
        } else {
            model
        };

        for pid in providers {
            if let Some(w) = self.ctx_by_key.get(&(*pid.to_string(), model_clean.to_string())) {
                return Some(*w);
            }
            let trimmed = strip_date_suffix(model_clean);
            if trimmed != model_clean {
                if let Some(w) = self.ctx_by_key.get(&(*pid.to_string(), trimmed.to_string())) {
                    return Some(*w);
                }
            }
        }
        None
    }
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
    if idx.is_empty() { None } else { Some(idx) }
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
                "id": "anthropic",
                "name": "Anthropic",
                "models": {
                    "claude-opus-4-7": {
                        "name": "Claude Opus 4.7",
                        "cost": { "input": 5.0, "output": 25.0, "cache_read": 0.5, "cache_write": 6.25 },
                        "limit": { "context": 200000 }
                    },
                    "claude-haiku-4-5": {
                        "name": "Claude Haiku 4.5",
                        "cost": { "input": 1.0, "output": 5.0, "cache_read": 0.1, "cache_write": 1.25 },
                        "limit": { "context": 200000 }
                    }
                }
            },
            "openai": {
                "id": "openai",
                "name": "OpenAI",
                "models": {
                    "gpt-5.4": {
                        "name": "GPT-5.4",
                        "cost": { "input": 2.5, "output": 15.0, "cache_read": 0.25 },
                        "limit": { "context": 400000 }
                    }
                }
            },
            "github-copilot": {
                "id": "github-copilot",
                "name": "GitHub Copilot",
                "models": {
                    "gpt-4.1": {
                        "cost": { "input": 0.0, "output": 0.0 }
                    }
                }
            }
        })
    }

    #[test]
    fn index_parses_provider_aware() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        let r = idx.lookup(ClientKind::Claude, "claude-opus-4-7").unwrap();
        assert!((r.input_per_m - 5.0).abs() < 1e-9);
        assert!((r.output_per_m - 25.0).abs() < 1e-9);
        assert!((r.cache_write_5m_per_m - 6.25).abs() < 1e-9);
        assert!((r.cache_read_per_m - 0.5).abs() < 1e-9);
    }

    #[test]
    fn index_resolves_codex_via_openai_provider() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        let r = idx.lookup(ClientKind::Codex, "gpt-5.4").unwrap();
        assert!((r.input_per_m - 2.5).abs() < 1e-9);
        assert!((r.output_per_m - 15.0).abs() < 1e-9);
    }

    #[test]
    fn index_strips_slash_prefix_for_opencode() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        assert!(idx.lookup(ClientKind::OpenCode, "anthropic/claude-opus-4-7").is_some());
    }

    #[test]
    fn index_returns_none_for_cursor() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        assert!(idx.lookup(ClientKind::Cursor, "claude-sonnet-4-5").is_none());
    }

    #[test]
    fn index_context_window_lookup() {
        let idx = ModelsDevIndex::from_json(&sample_json());
        assert_eq!(idx.lookup_context_window(ClientKind::Claude, "claude-opus-4-7"), Some(200_000));
        assert_eq!(idx.lookup_context_window(ClientKind::Codex, "gpt-5.4"), Some(400_000));
    }

    #[test]
    fn index_empty_on_garbage() {
        let idx = ModelsDevIndex::from_json(&serde_json::json!("not an object"));
        assert!(idx.is_empty());
    }

    #[test]
    fn strip_date_suffix_same_behavior() {
        assert_eq!(strip_date_suffix("claude-sonnet-4-5"), "claude-sonnet-4-5");
        assert_eq!(strip_date_suffix("claude-sonnet-4-5-20250929"), "claude-sonnet-4-5");
        assert_eq!(strip_date_suffix("gpt-4o-123"), "gpt-4o-123");
    }
}
```

- [ ] **Step 3: Run tests to verify models_dev module compiles and passes**

Run: `cargo test -p agtop-core -- models_dev`
Expected: All 7 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-core/src/models_dev.rs crates/agtop-core/src/lib.rs
git commit -m "feat(core): add models.dev pricing module with provider-aware lookup"
```

---

### Task 2: Modify `pricing.rs` — three-tier lookup

**Files:**
- Modify: `crates/agtop-core/src/pricing.rs`

- [ ] **Step 1: Replace single `PricingIndex` storage with `PricingSource`**

In `crates/agtop-core/src/pricing.rs`, replace the imports and static storage section. Replace:

```rust
use crate::litellm::PricingIndex;
use crate::session::{ClientKind, CostBreakdown, TokenTotals};
```

with:

```rust
use crate::litellm::PricingIndex;
use crate::models_dev::ModelsDevIndex;
use crate::session::{ClientKind, CostBreakdown, TokenTotals};
```

Replace the static storage and accessors (lines 106–139):

```rust
struct PricingSource {
    models_dev: Option<ModelsDevIndex>,
    litellm: Option<PricingIndex>,
}

static PRICING_SOURCE: OnceLock<RwLock<PricingSource>> = OnceLock::new();

fn pricing_slot() -> &'static RwLock<PricingSource> {
    PRICING_SOURCE.get_or_init(|| RwLock::new(PricingSource { models_dev: None, litellm: None }))
}

pub fn set_pricing_index(index: PricingIndex) {
    if let Ok(mut slot) = pricing_slot().write() {
        slot.litellm = Some(index);
    }
}

pub fn set_models_dev_index(index: ModelsDevIndex) {
    if let Ok(mut slot) = pricing_slot().write() {
        slot.models_dev = Some(index);
    }
}

fn autoload_index() {
    let needs_load = pricing_slot()
        .read()
        .ok()
        .map(|s| s.models_dev.is_none() && s.litellm.is_none())
        .unwrap_or(true);
    if !needs_load {
        return;
    }
    if let Some(md) = crate::models_dev::load_from_cache() {
        set_models_dev_index(md);
    }
    if let Some(lt) = crate::litellm::load_from_cache() {
        set_pricing_index(lt);
    }
}
```

- [ ] **Step 2: Update `lookup()` to three-tier chain**

Replace the `lookup` function body (the existing two-tier becomes three-tier):

```rust
pub fn lookup(client: ClientKind, model: &str) -> Option<Rates> {
    autoload_index();

    if let Ok(slot) = pricing_slot().read() {
        if let Some(idx) = &slot.models_dev {
            if let Some(r) = idx.lookup(client, model) {
                return Some(r);
            }
        }
    }

    if let Ok(slot) = pricing_slot().read() {
        if let Some(idx) = &slot.litellm {
            if let Some(r) = idx.lookup(client, model) {
                return Some(r);
            }
        }
    }

    builtin_lookup(client, model)
}
```

- [ ] **Step 3: Update `context_window()` to three-tier chain**

Replace the `context_window` function body:

```rust
pub fn context_window(client: ClientKind, model: &str) -> Option<u64> {
    autoload_index();

    if let Ok(slot) = pricing_slot().read() {
        if let Some(idx) = &slot.models_dev {
            if let Some(w) = idx.lookup_context_window(client, model) {
                return Some(w);
            }
        }
    }

    if let Ok(slot) = pricing_slot().read() {
        if let Some(idx) = &slot.litellm {
            if let Some(w) = idx.lookup_context_window(client, model) {
                return Some(w);
            }
        }
    }

    builtin_context_window(client, model)
}
```

- [ ] **Step 4: Run tests to verify pricing still works**

Run: `cargo test -p agtop-core`
Expected: All existing tests PASS (no behavioral change from the caller's perspective).

- [ ] **Step 5: Commit**

```bash
git add crates/agtop-core/src/pricing.rs
git commit -m "feat(core): three-tier pricing lookup: models.dev → LiteLLM → built-in"
```

---

### Task 3: Update `main.rs` — `setup_pricing` fetches both sources

**Files:**
- Modify: `crates/agtop-cli/src/main.rs`

- [ ] **Step 1: Update `setup_pricing` to fetch models.dev alongside LiteLLM**

Replace the `setup_pricing` function (lines 253–295) with:

```rust
fn setup_pricing(refresh: bool, disable: bool) {
    use agtop_core::{litellm, models_dev};

    if disable {
        agtop_core::pricing::set_pricing_index(litellm::PricingIndex::default());
        agtop_core::pricing::set_models_dev_index(models_dev::ModelsDevIndex::default());
        return;
    }

    // models.dev (primary)
    let md_cache = models_dev::cache_path();
    let md_have_any = md_cache.as_deref().map(|p| p.exists()).unwrap_or(false);
    let should_fetch_md = refresh || !md_have_any;

    if should_fetch_md {
        match models_dev::refresh_cache() {
            Ok(idx) => {
                tracing::info!(entries = idx.len(), "installed fresh models.dev pricing index");
                agtop_core::pricing::set_models_dev_index(idx);
            }
            Err(e) => {
                tracing::warn!(error = %e, "models.dev refresh failed; falling back");
            }
        }
    } else if let Some(path) = md_cache.as_deref() {
        if !models_dev::is_cache_fresh(path) {
            if let Ok(idx) = models_dev::refresh_cache() {
                agtop_core::pricing::set_models_dev_index(idx);
            }
        }
    }

    // LiteLLM (fallback)
    let cache = litellm::cache_path();
    let have_fresh_cache = cache
        .as_deref()
        .map(litellm::is_cache_fresh)
        .unwrap_or(false);
    let have_any_cache = cache.as_deref().map(|p| p.exists()).unwrap_or(false);

    let should_fetch = refresh || !have_any_cache;
    if should_fetch {
        match litellm::refresh_cache() {
            Ok(idx) => {
                tracing::info!(entries = idx.len(), "installed fresh LiteLLM pricing index");
                agtop_core::pricing::set_pricing_index(idx);
            }
            Err(e) => {
                tracing::warn!(error = %e, "LiteLLM refresh failed; falling back");
            }
        }
    } else if !have_fresh_cache {
        if let Ok(idx) = litellm::refresh_cache() {
            agtop_core::pricing::set_pricing_index(idx);
        }
    }
}
```

- [ ] **Step 2: Run full build + tests**

Run: `cargo build && cargo test -p agtop-cli`
Expected: Compiles and all CLI tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/src/main.rs
git commit -m "feat(cli): setup_pricing fetches models.dev + LiteLLM"
```

---

### Task 4: Create `logo.rs` — SVG fetch, cache, decode

**Files:**
- Create: `crates/agtop-core/src/logo.rs`
- Modify: `crates/agtop-core/src/lib.rs`

- [ ] **Step 1: Add `logo` module to `lib.rs`**

In `crates/agtop-core/src/lib.rs`, add after `pub mod litellm;`:

```rust
pub mod logo;
```

- [ ] **Step 2: Write `logo.rs`**

Create `crates/agtop-core/src/logo.rs`:

```rust
//! Provider logo fetching and caching from models.dev.
//!
//! SVG logos are fetched from `https://models.dev/logos/{provider}.svg`,
//! cached to `~/.cache/agtop/logos/`, and decoded into pixel buffers
//! suitable for ratatui-image rendering.

use std::fs;
use std::path::{Path, PathBuf};

use crate::session::ClientKind;

pub const LOGO_BASE_URL: &str = "https://models.dev/logos";
pub const LOGO_TTL_SECS: u64 = 7 * 24 * 60 * 60;
pub const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Which models.dev provider ID to use for each [`ClientKind`]'s logo.
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

/// Directory where cached SVG logos are stored.
pub fn logo_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("agtop").join("logos"))
}

/// Resolve the cache path for a given logo provider ID.
pub fn logo_cache_path(provider_id: &str) -> Option<PathBuf> {
    Some(logo_dir()?.join(format!("{provider_id}.svg")))
}

/// Check if a cached logo is fresh (< LOGO_TTL_SECS old).
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
        .map(|age| age < std::time::Duration::from_secs(LOGO_TTL_SECS))
        .unwrap_or(false)
}

/// Fetch a logo SVG from models.dev and cache it. Returns the SVG bytes
/// on success, or an error string on failure.
pub fn fetch_and_cache(provider_id: &str) -> Result<Vec<u8>, String> {
    let dir = logo_dir().ok_or("no cache dir")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let url = format!("{LOGO_BASE_URL}/{provider_id}.svg");
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .user_agent(concat!("rust-agtop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();

    let mut resp = agent.get(&url).call().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let mut body = Vec::with_capacity(4096);
    use std::io::Read;
    resp.body_mut()
        .as_reader()
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;

    let cache_path = dir.join(format!("{provider_id}.svg"));
    let tmp = dir.join(format!("{provider_id}.svg.tmp"));
    fs::write(&tmp, &body).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &cache_path).map_err(|e| e.to_string())?;

    tracing::debug!(provider = provider_id, bytes = body.len(), "cached logo");
    Ok(body)
}

/// Load a cached logo SVG if fresh, otherwise fetch and cache.
/// Returns the raw SVG bytes or None on any failure.
pub fn load_or_fetch(provider_id: &str) -> Option<Vec<u8>> {
    if let Some(path) = logo_cache_path(provider_id) {
        if path.exists() && is_logo_fresh(&path) {
            return fs::read(&path).ok();
        }
    }
    fetch_and_cache(provider_id).ok()
}

/// Load all available logos for the known clients. Returns a map from
/// ClientKind to raw SVG bytes. Failures are silently skipped.
pub fn load_all_logos() -> std::collections::HashMap<ClientKind, Vec<u8>> {
    let mut out = std::collections::HashMap::new();
    for client in ClientKind::all() {
        if let Some(pid) = logo_provider_id(*client) {
            if let Some(bytes) = load_or_fetch(pid) {
                out.insert(*client, bytes);
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
        assert_eq!(logo_provider_id(ClientKind::Copilot), Some("github-copilot"));
        assert_eq!(logo_provider_id(ClientKind::GeminiCli), Some("google"));
        assert_eq!(logo_provider_id(ClientKind::Cursor), None);
        assert_eq!(logo_provider_id(ClientKind::Antigravity), None);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p agtop-core -- logo`
Expected: 1 test PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-core/src/logo.rs crates/agtop-core/src/lib.rs
git commit -m "feat(core): add logo fetching/caching from models.dev"
```

---

### Task 5: Add `ratatui-image` + `image` dependencies to CLI crate

**Files:**
- Modify: `crates/agtop-cli/Cargo.toml`

- [ ] **Step 1: Add dependencies**

Add to the `[dependencies]` section of `crates/agtop-cli/Cargo.toml`:

```toml
ratatui-image = "10"
image = "0.25"
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p agtop-cli`
Expected: Compiles (may take time for first download of deps).

- [ ] **Step 3: Commit**

```bash
git add crates/agtop-cli/Cargo.toml Cargo.lock
git commit -m "build(cli): add ratatui-image and image dependencies"
```

---

### Task 6: Add logo state to `App` and load at TUI startup

**Files:**
- Modify: `crates/agtop-cli/src/tui/app/mod.rs`
- Modify: `crates/agtop-cli/src/tui/mod.rs`

- [ ] **Step 1: Add `logos` field to `App`**

In `crates/agtop-cli/src/tui/app/mod.rs`, add a new field to `App` struct after `enabled_arc`:

```rust
    logos: std::collections::HashMap<agtop_core::ClientKind, ratatui_image::ImageSource<'static>>,
```

Add a corresponding `None` initializer... no, we need to use the correct type. Since `ImageSource` is not `Default`, we need to handle this carefully.

Instead, store as `Option<ImageSource>` to handle clients without logos:

```rust
    logos: std::collections::HashMap<agtop_core::ClientKind, ratatui_image::ImageSource<'static>>,
```

And in `App::new()`, initialize as empty:

```rust
    logos: std::collections::HashMap::new(),
```

Then add a method to set logos after construction:

```rust
    pub fn set_logos(&mut self, logos: std::collections::HashMap<agtop_core::ClientKind, ratatui_image::ImageSource<'static>>) {
        self.logos = logos;
    }
```

And a read accessor:

```rust
    pub fn logo(&self, client: ClientKind) -> Option<&ratatui_image::ImageSource<'static>> {
        self.logos.get(&client)
    }
```

The `use` at the top of the file needs `agtop_core::session::ClientKind` — check if it's already imported. The `SessionAnalysis` import is there; we need to also import `ClientKind`. Add after the existing `use agtop_core::session::SessionAnalysis;`:

```rust
use agtop_core::ClientKind;
```

- [ ] **Step 2: Load logos at TUI startup in `tui/mod.rs`**

In `crates/agtop-cli/src/tui/mod.rs`, find where `App::new()` is called (inside the `run` function). After creating the app, load logos:

```rust
    let raw_logos = agtop_core::logo::load_all_logos();
    let mut logos = std::collections::HashMap::new();
    for (client, svg_bytes) in raw_logos {
        let img = match image::load_from_memory_with_format(&svg_bytes, image::ImageFormat::Svg) {
            Ok(img) => img,
            Err(_) => continue,
        };
        logos.insert(client, ratatui_image::ImageSource::new(img.clone(), app.area()));
    }
    app.set_logos(logos);
```

Note: The exact integration point depends on how `app` is created in `tui/mod.rs`. The engineer should read `tui/mod.rs` to find the `App::new()` call and add logo loading immediately after.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p agtop-cli`
Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/agtop-cli/src/tui/app/mod.rs crates/agtop-cli/src/tui/mod.rs
git commit -m "feat(tui): add logo state to App, load at startup"
```

---

### Task 7: Render logos in `dashboard_plan.rs`

**Files:**
- Modify: `crates/agtop-cli/src/tui/widgets/dashboard_plan.rs`

- [ ] **Step 1: Add `clients` field to `MergedPlan`**

In `dashboard_plan.rs`, add a field to `MergedPlan`:

```rust
struct MergedPlan<'a> {
    subscription_name: String,
    clients: Vec<ClientKind>,
    windows: Vec<&'a agtop_core::session::PlanWindow>,
    last_limit_hit: Option<DateTime<Utc>>,
    notes: Vec<String>,
}
```

Add `use agtop_core::session::ClientKind;` to the imports at the top.

- [ ] **Step 2: Collect clients during merge**

In `merge_plans`, when creating a new `MergedPlan`, collect the `client` from each `PlanUsage`. Update the merge function:

Where a new `MergedPlan` is created (the `!map.contains_key(&key)` branch), change:

```rust
                map.insert(
                    key.clone(),
                    MergedPlan {
                        subscription_name: display,
                        clients: vec![pu.client],
                        windows: Vec::new(),
                        last_limit_hit: pu.last_limit_hit,
                        notes: Vec::new(),
                    },
                );
```

And where an existing entry is updated, add client dedup:

```rust
        let entry = map.get_mut(&key).unwrap();
        if !entry.clients.contains(&pu.client) {
            entry.clients.push(pu.client);
        }
```

- [ ] **Step 3: Render logo in subscription list**

In `render_list`, update the `ListItem` construction. Change the name line from:

```rust
                Line::from(Span::styled(mp.subscription_name.clone(), name_style)),
```

to include the logo when available:

```rust
                let mut line_spans = Vec::new();
                if let Some(first_client) = mp.clients.first() {
                    if let Some(img_src) = app.logo(*first_client) {
                        // Render the logo as a fixed-size image inline.
                        // We create a small FixedImage widget and use its render output.
                        // For the list, we add a text placeholder with the logo style
                        // since ratatui List items support inline images via Image widget.
                        // Fall through to text if we can't render inline.
                        let _ = img_src; // TODO: render inline in next iteration
                    }
                }
                line_spans.push(Span::styled(mp.subscription_name.clone(), name_style));
                Line::from(line_spans),
```

Note: The exact ratatui-image integration for inline rendering within a `List` widget requires using `ratatui_image::widget::FixedImage` as a separate render call, not as part of a `Line`. The cleanest approach is to render logos as a separate pass after the list. Update the `render_list` function signature to accept `app: &App` and render logos separately.

Actually, the simplest approach that works with ratatui's List widget: render the logo above or below each list item using a custom rendering approach. Since ratatui-image needs to call `frame.render_widget()` with a specific `Rect`, the logo must be rendered as a separate widget in a computed area.

**Revised approach for render_list**: Split the left pane into per-item chunks and render each as a logo + text row:

Replace the entire `render_list` function:

```rust
fn render_list(frame: &mut Frame<'_>, area: Rect, merged: &[MergedPlan<'_>], selected: usize, app: &App) {
    const BAR_WIDTH: usize = 20;
    let item_height: u16 = 2;
    let total_items = merged.len() as u16;
    let visible = (area.height / item_height.max(1)).max(1) as usize;

    for (i, mp) in merged.iter().enumerate().take(visible) {
        let y = area.y + (i as u16) * item_height;
        if y + item_height > area.y + area.height {
            break;
        }

        let item_area = Rect { x: area.x, y, width: area.width, height: item_height };

        let util = mp
            .windows
            .iter()
            .filter_map(|w| w.reset_at.map(|t| (t, w.utilization)))
            .min_by_key(|(t, _)| *t)
            .and_then(|(_, util)| util)
            .or_else(|| mp.windows.iter().find_map(|w| w.utilization));

        let pct_str = util
            .map(|u| format!("{:>3.0}%", u * 100.0))
            .unwrap_or_else(|| "  - ".to_string());

        let name_style = if i == selected { th::PLAN_SELECTED } else { th::PLAN_LABEL };

        let [filled_span, empty_span] = bar_spans(util, BAR_WIDTH);

        let mut name_line_spans: Vec<Span<'static>> = vec![Span::raw("  ")];
        if let Some(first_client) = mp.clients.first() {
            if let Some(img_src) = app.logo(*first_client) {
                let img = ratatui_image::widget::FixedImage::new(img_src);
                let logo_area = Rect { x: area.x + 1, y, width: 1, height: 1 };
                frame.render_widget(img, logo_area);
                name_line_spans.push(Span::raw(" "));
            }
        }
        name_line_spans.push(Span::styled(mp.subscription_name.clone(), name_style));

        let bar_line = Line::from(vec![
            Span::raw("  "),
            filled_span,
            empty_span,
            Span::raw(format!(" {pct_str}")),
        ]);

        let lines = vec![Line::from(name_line_spans), bar_line];
        let p = Paragraph::new(lines);
        frame.render_widget(p, item_area);
    }
}
```

- [ ] **Step 4: Update `render_list` call site**

In the `render` function, update the call to pass `app`:

```rust
    render_list(frame, panes[0], &merged, selected, app);
```

And the `render_details` call — similarly pass `app` if we want logos there too:

```rust
    render_details(frame, panes[1], &merged, selected, app);
```

Update `render_details` signature to accept `app: &App` and add logo to the header:

In `render_details`, change the header line from:

```rust
    lines.push(Line::from(Span::styled(
        mp.subscription_name.clone(),
        th::PLAN_LABEL,
    )));
```

to:

```rust
    let mut header_spans: Vec<Span<'static>> = Vec::new();
    if let Some(first_client) = mp.clients.first() {
        if let Some(img_src) = app.logo(*first_client) {
            let img = ratatui_image::widget::FixedImage::new(img_src);
            let logo_area = Rect { x: area.x + 1, y: area.y, width: 1, height: 1 };
            frame.render_widget(img, logo_area);
            header_spans.push(Span::raw(" "));
        }
    }
    header_spans.push(Span::styled(mp.subscription_name.clone(), th::PLAN_LABEL));
    lines.push(Line::from(header_spans));
```

- [ ] **Step 5: Run build and tests**

Run: `cargo build -p agtop-cli && cargo test -p agtop-cli`
Expected: Compiles and tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/agtop-cli/src/tui/widgets/dashboard_plan.rs
git commit -m "feat(tui): render provider logos in subscription details panel"
```

---

### Task 8: End-to-end verification

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests PASS.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Run `--refresh-pricing` and verify both caches are populated**

Run: `cargo run -p agtop-cli -- --refresh-pricing --list`
Expected: Fetches both models.dev and LiteLLM. Prints session table. No errors.

- [ ] **Step 4: Verify cache files exist**

Run: `ls -la ~/.cache/agtop/`
Expected: Both `models-dev-pricing.json` and `litellm-pricing.json` present.

- [ ] **Step 5: Launch TUI and check dashboard plan panel**

Run: `cargo run -p agtop-cli -- --dashboard`
Expected: TUI launches. Dashboard plan panel shows subscriptions. If logos are cached, they appear next to subscription names (on supported terminals).

- [ ] **Step 6: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: end-to-end fixes for models.dev integration"
```
