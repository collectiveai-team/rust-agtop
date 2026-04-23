//! Quota-specific helpers that don't belong on `App` itself.

#![allow(dead_code)]

use agtop_core::quota::{ProviderId, Usage, UsageWindow};

/// Resolve the "preferred window" for a provider — the single window
/// used in the Classic tab cards and Dashboard list short view.
///
/// Returns `(label, &UsageWindow)` pairs from the provider's `Usage`
/// according to the per-provider preference table in the spec.
///
/// Falls back chains:
/// - Claude / Codex / z.ai → `5h` → first window
/// - Copilot / CopilotAddon → `premium` → first window
/// - Google → first model's `5h` → first model's `daily` → first model's first window
///
/// Returns `None` when the provider has no windows at all.
pub fn preferred_window(provider_id: ProviderId, usage: &Usage) -> Option<(String, &UsageWindow)> {
    match provider_id {
        ProviderId::Claude | ProviderId::Codex | ProviderId::Zai => find_any(usage, &["5h"]),
        ProviderId::Copilot | ProviderId::CopilotAddon => find_any(usage, &["premium"]),
        ProviderId::Google => preferred_google(usage),
    }
}

fn find_any<'a>(usage: &'a Usage, preferred_labels: &[&str]) -> Option<(String, &'a UsageWindow)> {
    for pref in preferred_labels {
        if let Some(w) = usage.windows.get(*pref) {
            return Some(((*pref).to_string(), w));
        }
    }
    usage.windows.iter().next().map(|(k, v)| (k.clone(), v))
}

fn preferred_google(usage: &Usage) -> Option<(String, &UsageWindow)> {
    // Google: top-level windows is empty by spec; look into models.
    let (first_model_key, first_model_windows) = usage.models.iter().next()?;
    for pref in &["5h", "daily"] {
        if let Some(w) = first_model_windows.get(*pref) {
            return Some((format!("{}::{}", first_model_key, pref), w));
        }
    }
    first_model_windows
        .iter()
        .next()
        .map(|(k, v)| (format!("{}::{}", first_model_key, k), v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agtop_core::quota::{Usage, UsageWindow};
    use indexmap::IndexMap;

    fn uw(pct: f64) -> UsageWindow {
        UsageWindow {
            used_percent: Some(pct),
            window_seconds: None,
            reset_at: None,
            value_label: None,
        }
    }

    fn usage_with(pairs: &[(&str, f64)]) -> Usage {
        let mut windows: IndexMap<String, UsageWindow> = IndexMap::new();
        for (k, v) in pairs {
            windows.insert((*k).to_string(), uw(*v));
        }
        Usage {
            windows,
            models: Default::default(),
            extras: Default::default(),
        }
    }

    #[test]
    fn claude_prefers_5h() {
        let u = usage_with(&[("7d", 10.0), ("5h", 72.0)]);
        let (label, w) = preferred_window(ProviderId::Claude, &u).unwrap();
        assert_eq!(label, "5h");
        assert_eq!(w.used_percent, Some(72.0));
    }

    #[test]
    fn claude_falls_back_to_first_when_5h_missing() {
        let u = usage_with(&[("7d", 10.0)]);
        let (label, _) = preferred_window(ProviderId::Claude, &u).unwrap();
        assert_eq!(label, "7d");
    }

    #[test]
    fn copilot_prefers_premium() {
        let u = usage_with(&[("chat", 0.0), ("premium", 50.0)]);
        let (label, w) = preferred_window(ProviderId::Copilot, &u).unwrap();
        assert_eq!(label, "premium");
        assert_eq!(w.used_percent, Some(50.0));
    }

    #[test]
    fn copilot_addon_prefers_premium() {
        let u = usage_with(&[("premium", 85.0)]);
        let (label, _) = preferred_window(ProviderId::CopilotAddon, &u).unwrap();
        assert_eq!(label, "premium");
    }

    #[test]
    fn zai_prefers_5h_falls_through_to_first() {
        let u = usage_with(&[("monthly", 31.0)]);
        let (label, _) = preferred_window(ProviderId::Zai, &u).unwrap();
        assert_eq!(label, "monthly");
    }

    #[test]
    fn google_uses_first_model_with_5h_preference() {
        use indexmap::IndexMap;
        let mut m1: IndexMap<String, UsageWindow> = IndexMap::new();
        m1.insert("daily".into(), uw(20.0));
        m1.insert("5h".into(), uw(95.0));
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        models.insert("gemini/gemini-2.5-pro".into(), m1);

        let u = Usage {
            windows: Default::default(),
            models,
            extras: Default::default(),
        };
        let (label, w) = preferred_window(ProviderId::Google, &u).unwrap();
        assert_eq!(label, "gemini/gemini-2.5-pro::5h");
        assert_eq!(w.used_percent, Some(95.0));
    }

    #[test]
    fn google_with_only_daily() {
        use indexmap::IndexMap;
        let mut m1: IndexMap<String, UsageWindow> = IndexMap::new();
        m1.insert("daily".into(), uw(33.0));
        let mut models: IndexMap<String, IndexMap<String, UsageWindow>> = IndexMap::new();
        models.insert("gemini/gemini-2.5-pro".into(), m1);

        let u = Usage {
            windows: Default::default(),
            models,
            extras: Default::default(),
        };
        let (label, _) = preferred_window(ProviderId::Google, &u).unwrap();
        assert_eq!(label, "gemini/gemini-2.5-pro::daily");
    }

    #[test]
    fn empty_usage_returns_none() {
        let u = usage_with(&[]);
        assert!(preferred_window(ProviderId::Claude, &u).is_none());
    }

    #[test]
    fn google_empty_models_returns_none() {
        let u = Usage {
            windows: Default::default(),
            models: Default::default(),
            extras: Default::default(),
        };
        assert!(preferred_window(ProviderId::Google, &u).is_none());
    }
}
