//! Core data types for the quota subsystem.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    Claude,
    Codex,
    Copilot,
    CopilotAddon,
    Zai,
    Google,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::CopilotAddon => "copilot-addon",
            Self::Zai => "zai",
            Self::Google => "google",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex / ChatGPT Plus",
            Self::Copilot => "GitHub Copilot",
            Self::CopilotAddon => "GitHub Copilot Add-on",
            Self::Zai => "z.ai",
            Self::Google => "Google",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageWindow {
    /// 0.0..=100.0. None when the provider cannot or will not report a
    /// percentage (e.g. Copilot "unlimited" plans, credits-only windows).
    pub used_percent: Option<f64>,
    /// Window duration in seconds. None for windows with no fixed duration.
    pub window_seconds: Option<u64>,
    /// Epoch milliseconds (UTC) when the window resets. None if unknown.
    pub reset_at: Option<i64>,
    /// Optional display string for non-percentage windows.
    pub value_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Usage {
    pub windows: IndexMap<String, UsageWindow>,
    pub models: IndexMap<String, IndexMap<String, UsageWindow>>,
    pub extras: IndexMap<String, UsageExtra>,
}

#[derive(Debug, Clone, Serialize)]
pub enum UsageExtra {
    OverageBudget {
        monthly_limit: Option<f64>,
        used: Option<f64>,
        utilization: Option<f64>,
        currency: Option<String>,
        enabled: bool,
    },
    PerToolCounts {
        items: Vec<(String, u64)>,
        total_cap: Option<u64>,
        reset_at: Option<i64>,
    },
    KeyValue(IndexMap<String, String>),
}

#[derive(Debug, Clone, Serialize)]
pub struct QuotaError {
    pub kind: ErrorKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub enum ErrorKind {
    NotConfigured,
    Transport,
    Http {
        status: u16,
        retry_after: Option<u64>,
    },
    Parse,
    Provider {
        code: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderResult {
    pub provider_id: ProviderId,
    pub provider_name: &'static str,
    pub configured: bool,
    pub ok: bool,
    pub usage: Option<Usage>,
    pub error: Option<QuotaError>,
    pub fetched_at: i64,
    pub meta: BTreeMap<String, String>,
}

impl ProviderResult {
    pub fn ok(
        id: ProviderId,
        name: &'static str,
        usage: Usage,
        meta: BTreeMap<String, String>,
    ) -> Self {
        Self {
            provider_id: id,
            provider_name: name,
            configured: true,
            ok: true,
            usage: Some(usage),
            error: None,
            fetched_at: now_epoch_ms(),
            meta,
        }
    }

    pub fn not_configured(id: ProviderId, name: &'static str) -> Self {
        Self {
            provider_id: id,
            provider_name: name,
            configured: false,
            ok: false,
            usage: None,
            error: Some(QuotaError {
                kind: ErrorKind::NotConfigured,
                detail: String::from("provider not configured"),
            }),
            fetched_at: now_epoch_ms(),
            meta: BTreeMap::new(),
        }
    }

    pub fn err(id: ProviderId, name: &'static str, error: QuotaError) -> Self {
        Self {
            provider_id: id,
            provider_name: name,
            configured: true,
            ok: false,
            usage: None,
            error: Some(error),
            fetched_at: now_epoch_ms(),
            meta: BTreeMap::new(),
        }
    }
}

fn now_epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
