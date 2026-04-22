//! `agtop quota` subcommand: list and manually fetch provider quotas.
//!
//! Three sub-actions:
//! - `list`       — print every registered provider plus configured/disabled status.
//! - `fetch`      — fetch ALL configured, honor config.disabled, print results.
//! - `fetch-one`  — fetch exactly one provider by id.
//!
//! Output format toggled by --json. Default is a compact human-readable form.

use agtop_core::quota::{
    fetch_all, fetch_one, list_providers, OpencodeAuth, ProviderId, ProviderResult, QuotaConfig,
    UreqClient,
};
use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct QuotaArgs {
    /// Path to the agtop config file (TOML, `[quota]` section).
    /// Env override: AGTOP_CONFIG.
    #[arg(long)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub action: QuotaAction,
}

#[derive(Debug, Subcommand)]
pub enum QuotaAction {
    /// List every registered provider and its configuration state.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Fetch quota from every configured provider in parallel.
    Fetch {
        #[arg(long)]
        json: bool,
    },
    /// Fetch quota from a single provider by id.
    FetchOne {
        #[arg(value_enum)]
        provider: ProviderId,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(args: QuotaArgs) -> Result<()> {
    // Resolve config path: --config flag > AGTOP_CONFIG env var > None (defaults).
    let config_path = args
        .config
        .or_else(|| std::env::var_os("AGTOP_CONFIG").map(PathBuf::from));
    let config = QuotaConfig::load(config_path.as_deref())?;
    match args.action {
        QuotaAction::List { json } => run_list(&config, json),
        QuotaAction::Fetch { json } => run_fetch(&config, json),
        QuotaAction::FetchOne { provider, json } => run_fetch_one(provider, &config, json),
    }
}

fn load_auth(config: &QuotaConfig) -> Result<OpencodeAuth> {
    let auth = match &config.opencode_auth_path {
        Some(p) => OpencodeAuth::load_from(p)?,
        None => OpencodeAuth::load().unwrap_or_else(|_| OpencodeAuth::empty()),
    };
    Ok(auth)
}

fn run_list(config: &QuotaConfig, json: bool) -> Result<()> {
    let auth = load_auth(config)?;
    let disabled: std::collections::HashSet<String> = config
        .disabled
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let rows: Vec<ListRow> = list_providers()
        .into_iter()
        .map(|info| {
            // Re-create a provider instance to probe is_configured.
            let configured = agtop_core::quota::providers::find(info.id)
                .map(|p| p.is_configured(&auth))
                .unwrap_or(false);
            ListRow {
                id: info.id.as_str().to_string(),
                display_name: info.display_name.to_string(),
                configured,
                disabled: disabled.contains(info.id.as_str()),
            }
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "{:<16} {:<24} {:<10} {:<10}",
        "id", "name", "configured", "disabled"
    );
    println!("{}", "-".repeat(64));
    for r in rows {
        println!(
            "{:<16} {:<24} {:<10} {:<10}",
            r.id, r.display_name, r.configured, r.disabled
        );
    }
    Ok(())
}

fn run_fetch(config: &QuotaConfig, json: bool) -> Result<()> {
    let auth = load_auth(config)?;
    let http = UreqClient::new();
    let results = fetch_all(&auth, &http, config);

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    if results.is_empty() {
        println!("No providers configured.");
        return Ok(());
    }

    for r in &results {
        print_human(r);
    }
    Ok(())
}

fn run_fetch_one(id: ProviderId, config: &QuotaConfig, json: bool) -> Result<()> {
    let auth = load_auth(config)?;
    let http = UreqClient::new();
    let r = fetch_one(id, &auth, &http);

    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }

    print_human(&r);
    Ok(())
}

fn print_human(r: &ProviderResult) {
    let status = if r.ok {
        "OK"
    } else if r.configured {
        "ERR"
    } else {
        "n/c"
    };
    println!(
        "\n[{}] {} ({})",
        status,
        r.provider_name,
        r.provider_id.as_str()
    );

    if !r.meta.is_empty() {
        for (k, v) in &r.meta {
            println!("  {k}: {v}");
        }
    }

    if let Some(err) = &r.error {
        println!("  error: {:?}", err.kind);
        println!("  detail: {}", err.detail);
        return;
    }

    if let Some(usage) = &r.usage {
        for (label, w) in &usage.windows {
            let pct = match w.used_percent {
                Some(p) => format!("{:>5.1}% used", p),
                None => "—".to_string(),
            };
            let reset = w
                .reset_at
                .map(|ms| format!("resets @ {}", fmt_epoch_ms(ms)))
                .unwrap_or_default();
            let label_part = w
                .value_label
                .clone()
                .map(|l| format!(" [{l}]"))
                .unwrap_or_default();
            println!("  {label:<14} {pct}  {reset}{label_part}");
        }
        for (model, windows) in &usage.models {
            for (label, w) in windows {
                let pct = match w.used_percent {
                    Some(p) => format!("{:>5.1}% used", p),
                    None => "—".to_string(),
                };
                println!("  {}::{:<8} {}", model, label, pct);
            }
        }
        for (name, _extra) in &usage.extras {
            println!("  extras.{name}: (serialize with --json for detail)");
        }
    }
}

fn fmt_epoch_ms(ms: i64) -> String {
    use chrono::{DateTime, Utc};
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ms.to_string())
}

#[derive(Debug, serde::Serialize)]
struct ListRow {
    id: String,
    display_name: String,
    configured: bool,
    disabled: bool,
}
