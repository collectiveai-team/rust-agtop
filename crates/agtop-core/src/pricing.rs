//! Pricing tables + billing-plan logic.
//!
//! Rates are USD per million tokens. Kept intentionally small and hard-coded:
//! in the MVP we do not fetch LiteLLM's JSON (the original does). Future work
//! can plug in a cache loader behind this API.

use crate::session::{CostBreakdown, ProviderKind, TokenTotals};

/// Billing plan selector. `Plan` decides whether sessions are priced at
/// retail or marked "included" for a given provider.
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
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "retail" | "default" | "api" => Some(Self::Retail),
            "max" | "claude-max" | "claude_max" => Some(Self::Max),
            "included" | "enterprise" | "not-billed" => Some(Self::Included),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanMode {
    Retail,
    Included,
}

impl Plan {
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

/// Look up rates for `(provider, model)`. Matches by exact model first, then
/// by loose prefix (e.g. `claude-sonnet-4-5-20250929` → `claude-sonnet-4-5`).
pub fn lookup(provider: ProviderKind, model: &str) -> Option<Rates> {
    let table: &[(&str, Rates)] = match provider {
        ProviderKind::Codex => CODEX_RATES,
        ProviderKind::Claude | ProviderKind::OpenCode => CLAUDE_RATES,
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
    // OpenCode frequently reports `provider/model`; retry with the suffix.
    if let Some((_, suffix)) = model.rsplit_once('/') {
        if suffix != model {
            return lookup(provider, suffix);
        }
    }
    None
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
    ("gpt-5.3-codex", Rates::codex(1.75, 0.175, 14.0)),
    ("codex-mini-latest", Rates::codex(1.50, 0.375, 6.0)),
    // Forward-compat aliases observed in real transcripts.
    ("gpt-5-codex", Rates::codex(1.75, 0.175, 14.0)),
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
