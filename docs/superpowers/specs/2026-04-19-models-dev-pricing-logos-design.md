# models.dev Pricing Backend + Provider Logos

Date: 2026-04-19

## Problem

agtop uses LiteLLM's `model_prices_and_context_window.json` as its sole
remote pricing source. LiteLLM is functional but its data is a flat per-token
index with inconsistent provider prefixes and no structured provider metadata.
We want to:

1. Adopt [models.dev](https://models.dev) as the primary pricing source —
   cleaner schema, provider-organized, costs already in USD/M, with cache
   read/write/reasoning buckets.
2. Use models.dev's logo API (`/logos/{provider}.svg`) to render provider
   icons next to subscription names in the TUI's dashboard plan panel.

LiteLLM remains as a fallback for models not covered by models.dev (e.g.
Cursor, legacy aliases).

## Approach

New `models_dev.rs` module alongside the existing `litellm.rs`. Logo fetching
in a new `logo.rs`. Three-tier pricing lookup: models.dev → LiteLLM →
hardcoded built-in tables.

## Design

### 1. `models_dev.rs` — Pricing Module

**Source**: `https://models.dev/api.json` (~1.7 MB JSON).

**Schema**: Top-level keys are provider IDs. Each provider has a `models` map
keyed by model ID, with costs already expressed as USD per million tokens:

```json
{
  "anthropic": {
    "id": "anthropic",
    "name": "Anthropic",
    "models": {
      "claude-opus-4-7": {
        "cost": {
          "input": 5.0,
          "output": 25.0,
          "cache_read": 0.5,
          "cache_write": 6.25
        }
      }
    }
  }
}
```

No per-token multiplication needed (unlike LiteLLM's `input_cost_per_token`).

**Cache**: Write to `~/.cache/agtop/models-dev-pricing.json` with 24h TTL.
Same atomic-write pattern as `litellm.rs` (tmp file + rename). Meta file at
`models-dev-pricing.json.meta` records fetch timestamp and entry count.

**HTTP**: Sync `ureq` fetch, 10s timeout, 10 MB size cap (same constraints as
LiteLLM).

**Index struct**:

```rust
pub struct ModelsDevIndex {
    // Keyed by (provider_id, model_id) for provider-aware lookup.
    by_key: HashMap<(String, String), Rates>,
    // Flat model-name index for cross-provider fallback (OpenCode sessions).
    by_model: HashMap<String, Rates>,
    // Context windows keyed by (provider_id, model_id).
    ctx_by_key: HashMap<(String, String), u64>,
}
```

**ClientKind → models.dev provider mapping**:

| ClientKind   | models.dev provider IDs                      |
|--------------|----------------------------------------------|
| `Claude`     | `["anthropic"]`                              |
| `Codex`      | `["openai"]`                                 |
| `OpenCode`   | `["opencode", "anthropic", "openai", "google"]` |
| `Copilot`    | `["github-copilot"]`                         |
| `GeminiCli`  | `["google"]`                                 |
| `Cursor`     | `[]` (not in models.dev)                     |
| `Antigravity`| `[]` (not in models.dev)                     |

OpenCode searches multiple provider IDs since it proxies models from various
upstreams. Lookup tries each mapped provider in order, then falls back to the
flat `by_model` index for any model regardless of provider.

**Lookup algorithm** (per provider ID in the mapping):
1. Exact match on `(provider_id, model)`.
2. Date-suffix trim: `claude-sonnet-4-5-20250929` → `claude-sonnet-4-5`.
3. Strip `provider/` prefix for OpenCode-style `anthropic/claude-haiku-4-5`.
4. Longest prefix match on model name within that provider.

### 2. `logo.rs` — Logo Fetching and Caching

**Source**: `https://models.dev/logos/{provider}.svg` — returns a clean SVG
using `currentColor`, typically 200–500 bytes.

**ClientKind → logo provider ID mapping**:

| ClientKind   | Logo fetch ID       |
|--------------|---------------------|
| `Claude`     | `anthropic`         |
| `Codex`      | `openai`            |
| `OpenCode`   | `opencode`          |
| `Copilot`    | `github-copilot`    |
| `GeminiCli`  | `google`            |
| `Cursor`     | no logo available   |
| `Antigravity`| no logo available   |

**Cache**: Write SVGs to `~/.cache/agtop/logos/{provider}.svg`. 7-day TTL
(logos rarely change). Silent failures — if a logo can't be fetched, the
subscription name renders as colored text (current behavior).

**Decode**: At startup, after cache check, decode each cached SVG into an
`image::DynamicImage` then into a `ratatui_image::ImageSource`. Store in the
`App` struct as `logos: HashMap<ClientKind, ImageSource>`.

**New dependencies**:
- `ratatui-image` v10 (sixel/kitty/iterm2/halfblocks protocol detection)
- `image` crate (for SVG → raster decode)

### 3. `pricing.rs` — Three-Tier Lookup

**Storage**: Replace the current `OnceLock<RwLock<Option<PricingIndex>>>` with:

```rust
struct PricingSource {
    models_dev: ModelsDevIndex,
    litellm: PricingIndex,
}

static PRICING_SOURCE: OnceLock<RwLock<Option<PricingSource>>> = OnceLock::new();
```

**`set_pricing_source(models_dev, litellm)`** installs both indexes atomically.

**Lookup chain** in `lookup(client, model)`:

1. `models_dev.lookup(client, model)` — provider-aware, clean schema.
2. `litellm.lookup(client, model)` — existing flat index with prefix stripping.
3. `builtin_lookup(client, model)` — existing hardcoded tables.

`context_window()` follows the same three-tier chain. models.dev provides
`limit.context` per model.

**Startup flow** (in `main.rs::setup_pricing()`):

1. Check `models-dev-pricing.json` freshness → load or fetch.
2. Check `litellm-pricing.json` freshness → load or fetch (existing logic).
3. Call `set_pricing_source(models_dev_idx, litellm_idx)`.
4. In parallel, fetch/update logos (non-blocking; absence doesn't stall
   pricing).

**CLI flags**:
- `--refresh-pricing`: Refreshes both models.dev + LiteLLM caches.
- `--no-pricing-refresh`: Skips all network for both sources.

**Unchanged**: `Rates`, `compute_cost()`, `Plan`, `PlanMode`, built-in tables.

### 4. TUI Changes — `dashboard_plan.rs`

**`MergedPlan`** gains a `clients: Vec<ClientKind>` field, collected from all
`PlanUsage` entries that merged into it. Used to look up the correct logo.

**Left pane subscription list** — each `ListItem` changes from:

```
  Subscription Name
  ████████████░░░░░░░░  60%
```

to:

```
  [logo] Subscription Name
  ████████████░░░░░░░░  60%
```

`[logo]` is rendered via `ratatui-image`'s `FitImage` or `FixedImage` widget,
sized to single-line height. Uses the first `ClientKind` in the merged plan's
clients list. Falls back to colored text when:
- No logo is cached for that client
- Terminal doesn't support image protocols (ratatui-image auto-degrades to
  halfblocks or the widget is skipped entirely)

**Right pane details** — header line gains the same inline logo:

```
  [logo] Subscription Name
```

**App state**: `App` struct gains `logos: HashMap<ClientKind, ImageSource>`,
populated at startup. Render functions access via `app.logos()`.

**No other TUI changes** — session table, info tab, cost tab, config tab
remain unchanged. Logos only appear in the subscription details panel.

### Files Changed

| File | Change |
|------|--------|
| `agtop-core/src/models_dev.rs` | **New** — models.dev fetch, parse, cache, index |
| `agtop-core/src/logo.rs` | **New** — logo fetch, cache, decode |
| `agtop-core/src/pricing.rs` | Modify — three-tier lookup, `PricingSource` struct |
| `agtop-core/src/lib.rs` | Modify — declare new modules |
| `agtop-cli/src/main.rs` | Modify — startup: fetch both sources, logos |
| `agtop-cli/src/tui/app/mod.rs` | Modify — `logos` field on `App` |
| `agtop-cli/src/tui/widgets/dashboard_plan.rs` | Modify — logo rendering |
| `agtop-cli/Cargo.toml` | Modify — add `ratatui-image`, `image` deps |
| `agtop-core/src/litellm.rs` | Unchanged — remains as fallback |
| `agtop-core/Cargo.toml` | Unchanged — `ureq` already present |

### Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| models.dev API down at startup | Falls back to LiteLLM → hardcoded, same as today |
| `ratatui-image` not supported on user's terminal | Auto-detects protocol; degrades to halfblocks or skips logo |
| SVG decode fails | Store `None` for that client, render colored text |
| models.dev missing a model (Cursor) | LiteLLM or hardcoded tables cover it |
| Larger dependency tree from `image` + `ratatui-image` | Acceptable for a TUI app; both are widely used |
