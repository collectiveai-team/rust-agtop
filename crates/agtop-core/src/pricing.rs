//! Pricing tables + billing-plan logic.
//!
//! Rates are USD per million tokens. Two data sources are consulted in
//! order:
//!
//! 1. A live [`crate::litellm::PricingIndex`] installed via
//!    [`set_pricing_index`] (built from the cached LiteLLM JSON, which
//!    covers far more models and tracks upstream price changes).
//! 2. The hard-coded tables in this file as a last-resort fallback so
//!    agtop works offline and keeps producing sensible cost figures when
//!    LiteLLM drops a model we care about.

use std::sync::{OnceLock, RwLock};

use crate::litellm::PricingIndex;
use crate::session::{CostBreakdown, ProviderKind, TokenTotals};

/// Billing plan selector. `Plan` decides whether sessions are priced at
/// retail or marked "included" for a given provider.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Standard API pricing for all providers.
    Retail,
    /// Claude Max / Pro: Claude sessions treated as included; Codex/OpenCode retail.
    Max,
    /// All sessions marked as included (enterprise / bundled).
    Included,
}

impl Plan {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "retail" | "default" | "api" => Some(Self::Retail),
            "max" | "claude-max" | "claude_max" => Some(Self::Max),
            "included" | "enterprise" | "not-billed" => Some(Self::Included),
            _ => None,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanMode {
    Retail,
    Included,
}

impl Plan {
    #[must_use]
    pub fn mode_for(self, provider: ProviderKind) -> PlanMode {
        match (self, provider) {
            (Self::Retail, _) => PlanMode::Retail,
            (Self::Included, _) => PlanMode::Included,
            (Self::Max, ProviderKind::Claude) => PlanMode::Included,
            (Self::Max, _) => PlanMode::Retail,
        }
    }
}

/// Per-model rate card. Fields denote USD per 1,000,000 tokens.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct Rates {
    pub input_per_m: f64,
    pub cached_input_per_m: f64,
    pub output_per_m: f64,
    pub cache_write_5m_per_m: f64,
    pub cache_write_1h_per_m: f64,
    pub cache_read_per_m: f64,
}

impl Rates {
    pub const fn codex(input: f64, cached_input: f64, output: f64) -> Self {
        Self {
            input_per_m: input,
            cached_input_per_m: cached_input,
            output_per_m: output,
            cache_write_5m_per_m: 0.0,
            cache_write_1h_per_m: 0.0,
            cache_read_per_m: 0.0,
        }
    }

    pub const fn claude(
        input: f64,
        cache_write_5m: f64,
        cache_write_1h: f64,
        cache_read: f64,
        output: f64,
    ) -> Self {
        Self {
            input_per_m: input,
            // Claude does not have a separate "cached input" knob the way
            // Codex does; we map cached input to cache_read (the cheap bucket).
            cached_input_per_m: cache_read,
            output_per_m: output,
            cache_write_5m_per_m: cache_write_5m,
            cache_write_1h_per_m: cache_write_1h,
            cache_read_per_m: cache_read,
        }
    }
}

/// Storage for a process-wide LiteLLM pricing index. Wrapped in an
/// `RwLock` because we allow the CLI to swap it out post-startup (after
/// an explicit `--refresh-pricing`). Initialized lazily on first lookup.
static PRICING_INDEX: OnceLock<RwLock<Option<PricingIndex>>> = OnceLock::new();

fn pricing_slot() -> &'static RwLock<Option<PricingIndex>> {
    PRICING_INDEX.get_or_init(|| RwLock::new(None))
}

/// Install a pricing index — typically built from the on-disk cache or a
/// fresh network fetch. Subsequent [`lookup`] calls will consult it before
/// the built-in tables.
pub fn set_pricing_index(index: PricingIndex) {
    if let Ok(mut slot) = pricing_slot().write() {
        *slot = Some(index);
    }
}

/// Best-effort auto-initialization: if no index has been installed yet
/// but a parseable cache file exists on disk, load it. Never fetches,
/// never blocks. Idempotent.
fn autoload_index() {
    if pricing_slot()
        .read()
        .ok()
        .map(|s| s.is_some())
        .unwrap_or(true)
    {
        return;
    }
    if let Some(idx) = crate::litellm::load_from_cache() {
        set_pricing_index(idx);
    }
}

/// Look up rates for `(provider, model)`. Tries in order:
/// 1. The live LiteLLM index (covers the full upstream catalog).
/// 2. The built-in tables below.
///
/// Uses exact match first, then strips `-YYYYMMDD` date suffixes, then
/// tries provider-prefix variants (`anthropic.foo`, `openai/foo`), and
/// finally falls back to the longest prefix match. OpenCode's
/// `provider/model` style is handled by retrying with the suffix.
pub fn lookup(provider: ProviderKind, model: &str) -> Option<Rates> {
    autoload_index();

    // 1) LiteLLM cache.
    if let Ok(slot) = pricing_slot().read() {
        if let Some(idx) = slot.as_ref() {
            if let Some(r) = idx.lookup(provider, model) {
                return Some(r);
            }
        }
    }

    // 2) Built-in fallback.
    builtin_lookup(provider, model)
}

/// Best-effort model context window lookup (tokens).
///
/// Uses the LiteLLM cache when available, then falls back to a tiny
/// built-in table for common models we observe in local transcripts.
pub fn context_window(provider: ProviderKind, model: &str) -> Option<u64> {
    autoload_index();
    if let Ok(slot) = pricing_slot().read() {
        if let Some(idx) = slot.as_ref() {
            if let Some(w) = idx.lookup_context_window(provider, model) {
                return Some(w);
            }
        }
    }
    builtin_context_window(provider, model)
}

/// Built-in (hard-coded) lookup. Exposed so tests can bypass the cache.
pub fn builtin_lookup(provider: ProviderKind, model: &str) -> Option<Rates> {
    let table: &[(&str, Rates)] = match provider {
        ProviderKind::Codex => CODEX_RATES,
        ProviderKind::Claude | ProviderKind::OpenCode => CLAUDE_RATES,
        ProviderKind::GeminiCli => GEMINI_RATES,
        ProviderKind::Copilot => COPILOT_RATES,
        ProviderKind::Cursor => CURSOR_RATES,
        ProviderKind::Antigravity => return None,
    };
    if let Some((_, r)) = table.iter().find(|(k, _)| *k == model) {
        return Some(*r);
    }
    // Fuzzy: try trimming trailing date suffix `-YYYYMMDD`.
    let trimmed = strip_date_suffix(model);
    if trimmed != model {
        if let Some((_, r)) = table.iter().find(|(k, _)| *k == trimmed) {
            return Some(*r);
        }
    }
    // Loose: any table key that is a prefix of the model (longest wins).
    let best = table
        .iter()
        .filter(|(k, _)| model.starts_with(k))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, r)| *r);
    if best.is_some() {
        return best;
    }
    // OpenCode / Cursor frequently report `provider/model`; retry with the suffix.
    if let Some((_, suffix)) = model.rsplit_once('/') {
        if suffix != model {
            return builtin_lookup(provider, suffix);
        }
    }
    None
}

fn builtin_context_window(provider: ProviderKind, model: &str) -> Option<u64> {
    let key = strip_date_suffix(model);
    if key != model {
        return builtin_context_window(provider, key);
    }

    // OpenCode often reports `provider/model`.
    if let Some((_, suffix)) = model.rsplit_once('/') {
        if suffix != model {
            if let Some(w) = builtin_context_window(provider, suffix) {
                return Some(w);
            }
        }
    }

    match provider {
        ProviderKind::Codex => {
            if model.starts_with("gpt-5") || model.starts_with("codex") {
                Some(258_400)
            } else {
                None
            }
        }
        ProviderKind::Claude | ProviderKind::OpenCode => {
            if model.starts_with("claude") {
                Some(1_000_000)
            } else {
                None
            }
        }
        ProviderKind::GeminiCli => {
            if model.starts_with("gemini-2.5")
                || model.starts_with("gemini-2.0")
                || model.starts_with("gemini-1.5")
            {
                Some(1_048_576) // 1M tokens
            } else {
                None
            }
        }
        ProviderKind::Copilot | ProviderKind::Cursor => {
            // These proxy OpenAI models; context window varies but 128k is a
            // safe lower-bound for GPT-4.1/4o class models.
            if model.starts_with("gpt-4") || model.starts_with("o3") || model.starts_with("o4") {
                Some(128_000)
            } else {
                None
            }
        }
        ProviderKind::Antigravity => None,
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

/// Compute a [`CostBreakdown`] given token totals + a resolved rate card.
#[must_use]
pub fn compute_cost(totals: &TokenTotals, rates: &Rates, included: bool) -> CostBreakdown {
    if included {
        return CostBreakdown {
            included: true,
            ..Default::default()
        };
    }
    let per_m = |tokens: u64, rate: f64| (tokens as f64) * rate / 1_000_000.0;
    let uncached_input = totals.input.saturating_sub(totals.cached_input);
    let b = CostBreakdown {
        input: per_m(uncached_input, rates.input_per_m),
        cached_input: per_m(totals.cached_input, rates.cached_input_per_m),
        output: per_m(totals.output, rates.output_per_m),
        cache_write_5m: per_m(totals.cache_write_5m, rates.cache_write_5m_per_m),
        cache_write_1h: per_m(totals.cache_write_1h, rates.cache_write_1h_per_m),
        cache_read: per_m(totals.cache_read, rates.cache_read_per_m),
        included: false,
        total: 0.0,
    };
    let total =
        b.input + b.cached_input + b.output + b.cache_write_5m + b.cache_write_1h + b.cache_read;
    CostBreakdown { total, ..b }
}

// ---------------------------------------------------------------------------
// Built-in rate tables. Values mirror the original agtop index.js, which
// itself snapshots public API pricing (plus a few internal aliases).
// Extend by appending new entries.
// ---------------------------------------------------------------------------

const CODEX_RATES: &[(&str, Rates)] = &[
    // Values mirror the LiteLLM `model_prices_and_context_window.json` as
    // of April 2026. Kept in sync manually; the LiteLLM cache (see
    // `litellm::refresh_cache`) is preferred when it's available.
    //
    // Naming: OpenAI's "Codex" models are variants of the gpt-5 family.
    // Input is $ USD per M tokens; cached-input is cache_read; output is
    // typically 8× input for the chat models.
    ("gpt-5-codex", Rates::codex(1.25, 0.125, 10.0)),
    ("gpt-5.1-codex", Rates::codex(1.25, 0.125, 10.0)),
    ("gpt-5.1-codex-max", Rates::codex(1.25, 0.125, 10.0)),
    ("gpt-5.1-codex-mini", Rates::codex(0.25, 0.025, 2.0)),
    ("gpt-5.2-codex", Rates::codex(1.75, 0.175, 14.0)),
    ("gpt-5.3-codex", Rates::codex(1.75, 0.175, 14.0)),
    // `gpt-5.4` and friends (seen in live transcripts as of April 2026).
    ("gpt-5.4", Rates::codex(2.50, 0.25, 15.0)),
    ("gpt-5.4-mini", Rates::codex(0.75, 0.075, 4.5)),
    ("gpt-5.4-nano", Rates::codex(0.20, 0.02, 1.25)),
    // Non-codex variants agtop sometimes encounters via OpenCode sessions.
    ("gpt-5", Rates::codex(1.25, 0.125, 10.0)),
    ("gpt-5.1", Rates::codex(1.25, 0.125, 10.0)),
    ("gpt-5.2", Rates::codex(1.75, 0.175, 14.0)),
    ("gpt-5-mini", Rates::codex(0.25, 0.025, 2.0)),
    ("gpt-5-nano", Rates::codex(0.05, 0.005, 0.4)),
    ("codex-mini-latest", Rates::codex(1.50, 0.375, 6.0)),
];

const CLAUDE_RATES: &[(&str, Rates)] = &[
    // Opus 4.x family
    ("claude-opus-4-6", Rates::claude(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("claude-opus-4-5", Rates::claude(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("claude-opus-4-7", Rates::claude(5.0, 6.25, 10.0, 0.5, 25.0)),
    // Sonnet 4.x family
    (
        "claude-sonnet-4-6",
        Rates::claude(3.0, 3.75, 6.0, 0.3, 15.0),
    ),
    (
        "claude-sonnet-4-5",
        Rates::claude(3.0, 3.75, 6.0, 0.3, 15.0),
    ),
    // Haiku 4.x family
    ("claude-haiku-4-5", Rates::claude(1.0, 1.25, 2.0, 0.1, 5.0)),
    ("claude-haiku-4.5", Rates::claude(1.0, 1.25, 2.0, 0.1, 5.0)),
];

/// Gemini CLI model rates (USD per million tokens, April 2026 pricing).
/// Input/output only — Gemini CLI does not expose cache-write buckets.
const GEMINI_RATES: &[(&str, Rates)] = &[
    // Gemini 2.5 Flash (thinking)
    ("gemini-2.5-flash", Rates::codex(0.15, 0.0375, 0.60)),
    (
        "gemini-2.5-flash-thinking",
        Rates::codex(0.15, 0.0375, 3.50),
    ),
    // Gemini 2.5 Pro
    ("gemini-2.5-pro", Rates::codex(1.25, 0.31, 10.0)),
    // Gemini 2.0 Flash
    ("gemini-2.0-flash", Rates::codex(0.10, 0.025, 0.40)),
    ("gemini-2.0-flash-lite", Rates::codex(0.075, 0.02, 0.30)),
    // Gemini 1.5 Pro / Flash (older sessions)
    ("gemini-1.5-pro", Rates::codex(1.25, 0.31, 5.0)),
    ("gemini-1.5-flash", Rates::codex(0.075, 0.02, 0.30)),
];

/// Copilot model rates — proxied OpenAI/Anthropic models at retail pricing.
/// Copilot does not expose token counts locally, so these rates are used
/// only if a future data source provides per-session token counts.
const COPILOT_RATES: &[(&str, Rates)] = &[
    ("gpt-4.1", Rates::codex(2.0, 0.50, 8.0)),
    ("gpt-4o", Rates::codex(2.5, 1.25, 10.0)),
    ("gpt-4o-mini", Rates::codex(0.15, 0.075, 0.60)),
    ("o3", Rates::codex(10.0, 2.5, 40.0)),
    ("o4-mini", Rates::codex(1.1, 0.275, 4.4)),
    ("copilot/auto", Rates::codex(2.0, 0.50, 8.0)),
];

/// Cursor proxies various models; rates match retail pricing.
const CURSOR_RATES: &[(&str, Rates)] = &[
    ("gpt-4.1", Rates::codex(2.0, 0.50, 8.0)),
    ("gpt-4o", Rates::codex(2.5, 1.25, 10.0)),
    (
        "claude-sonnet-4-5",
        Rates::claude(3.0, 3.75, 6.0, 0.3, 15.0),
    ),
    ("claude-opus-4-5", Rates::claude(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("cursor-small", Rates::codex(0.10, 0.025, 0.30)),
    ("cursor-fast", Rates::codex(2.0, 0.50, 8.0)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_parse_recognized() {
        assert_eq!(Plan::parse("retail"), Some(Plan::Retail));
        assert_eq!(Plan::parse("MAX"), Some(Plan::Max));
        assert_eq!(Plan::parse("included"), Some(Plan::Included));
        assert_eq!(Plan::parse("unknown"), None);
    }

    #[test]
    fn plan_mode_semantics() {
        assert_eq!(Plan::Max.mode_for(ProviderKind::Claude), PlanMode::Included);
        assert_eq!(Plan::Max.mode_for(ProviderKind::Codex), PlanMode::Retail);
        assert_eq!(
            Plan::Included.mode_for(ProviderKind::OpenCode),
            PlanMode::Included
        );
    }

    #[test]
    fn lookup_exact_and_dated() {
        assert!(lookup(ProviderKind::Codex, "gpt-5.3-codex").is_some());
        // Dated Claude model
        assert!(lookup(ProviderKind::Claude, "claude-sonnet-4-5-20250929").is_some());
    }

    #[test]
    fn lookup_opencode_via_suffix() {
        // OpenCode often reports "provider/model"
        assert!(lookup(ProviderKind::OpenCode, "anthropic/claude-haiku-4.5").is_some());
    }

    #[test]
    fn builtin_lookup_antigravity_has_no_rates() {
        // Antigravity has no built-in rate table.
        assert!(builtin_lookup(ProviderKind::Antigravity, "sonnet").is_none());
        // Copilot, GeminiCli, and Cursor now have real rate tables.
        assert!(builtin_lookup(ProviderKind::Copilot, "gpt-4.1").is_some());
        assert!(builtin_lookup(ProviderKind::GeminiCli, "gemini-2.5-pro").is_some());
    }

    #[test]
    fn compute_cost_retail() {
        let totals = TokenTotals {
            input: 1_000_000,
            cached_input: 250_000,
            output: 500_000,
            ..Default::default()
        };
        let rates = Rates::codex(1.0, 0.25, 2.0);
        let cost = compute_cost(&totals, &rates, false);
        // uncached_input = 750k * $1/M = 0.75
        assert!((cost.input - 0.75).abs() < 1e-9);
        // cached_input = 250k * $0.25/M = 0.0625
        assert!((cost.cached_input - 0.0625).abs() < 1e-9);
        // output = 500k * $2/M = 1.0
        assert!((cost.output - 1.0).abs() < 1e-9);
        assert!((cost.total - 1.8125).abs() < 1e-9);
        assert!(!cost.included);
    }

    #[test]
    fn compute_cost_included_zero() {
        let totals = TokenTotals {
            input: 9_999_999,
            output: 9_999_999,
            ..Default::default()
        };
        let rates = Rates::codex(1.0, 0.25, 2.0);
        let cost = compute_cost(&totals, &rates, true);
        assert_eq!(cost.total, 0.0);
        assert!(cost.included);
    }
}
