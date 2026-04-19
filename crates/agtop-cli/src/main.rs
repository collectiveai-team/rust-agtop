//! agtop — htop-style dashboard for AI coding agents (Rust port).
//!
//! Default mode is an interactive ratatui TUI (see [`tui`]). `--list`
//! and `--json` remain one-shot non-interactive paths for scripting.

mod fmt;
mod tui;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::Serialize;

use crate::tui::widgets::state_display::display_state;
use agtop_core::{
    analyze_all, default_clients, discover_all, pricing::Plan, session::SessionAnalysis, ClientKind,
};

#[derive(Parser, Debug)]
#[command(
    name = "agtop",
    version,
    about = "htop-style dashboard for AI coding agent sessions (Claude Code, Codex, OpenCode)",
    long_about = None,
)]
struct Cli {
    /// Print discovered sessions as a table and exit.
    #[arg(short = 'l', long)]
    list: bool,

    /// Print full analysis (tokens + cost per session) as JSON and exit.
    #[arg(short = 'j', long)]
    json: bool,

    /// Billing plan: retail | max | included (default: retail).
    #[arg(short = 'p', long, default_value = "retail")]
    plan: String,

    /// Only include sessions from this agentic client (claude, codex,
    /// opencode, copilot, gemini-cli, cursor, antigravity). May be given
    /// multiple times. Affects the current run only — persisted
    /// enable/disable lives in the TUI's Config tab.
    #[arg(
        long = "client",
        alias = "backend",
        alias = "agentic-client",
        value_name = "KIND"
    )]
    clients: Vec<String>,

    /// Re-render the `--list` table every `--delay` seconds until Ctrl-C.
    /// Ignored in `--json` mode.
    #[arg(short = 'w', long)]
    watch: bool,

    /// Seconds between refreshes in `--watch` / TUI mode (default: 5).
    /// Press `r` in the TUI to force an immediate refresh.
    #[arg(short = 'd', long, default_value_t = 5u64, value_name = "SECS")]
    delay: u64,

    /// Start directly in the btop-style dashboard view.
    #[arg(short = 'D', long)]
    dashboard: bool,

    /// Force a synchronous fetch of the LiteLLM pricing table before
    /// analyzing sessions. Network required; errors are logged and the
    /// built-in tables are used as a fallback.
    #[arg(long)]
    refresh_pricing: bool,

    /// Skip the on-disk pricing cache entirely and never touch the
    /// network. Useful in air-gapped environments. Takes precedence
    /// over `--refresh-pricing`.
    #[arg(long)]
    no_pricing_refresh: bool,

    /// Verbose logging to stderr (sets RUST_LOG=info if unset).
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // TUI mode = bare `agtop` with no --list / --json / --watch.
    let tui_mode = !cli.json && !cli.list && !cli.watch;
    init_logging(cli.verbose, tui_mode);

    let plan = Plan::parse(&cli.plan)
        .with_context(|| format!("unknown plan '{}'; try retail|max|included", cli.plan))?;

    setup_pricing(cli.refresh_pricing, cli.no_pricing_refresh);

    // Always build the full client set. `--client` only affects the
    // in-memory enabled set for this run; disabling from the TUI is what
    // persists.
    let clients = default_clients();

    // Build initial enabled set:
    //   1. --client CLI flag (one-shot, does NOT write to disk)
    //   2. Otherwise the persisted ColumnConfig (~/.config/agtop/columns.json)
    //   3. Otherwise every client
    let enabled_initial: std::collections::HashSet<agtop_core::ClientKind> =
        if !cli.clients.is_empty() {
            let wanted: std::collections::HashSet<agtop_core::ClientKind> = cli
                .clients
                .iter()
                .filter_map(|s| parse_client_kind(s))
                .collect();
            if wanted.is_empty() {
                anyhow::bail!(
                    "no recognized --client values (got: {:?}). expected one of: {}",
                    cli.clients,
                    agtop_core::ClientKind::all()
                        .iter()
                        .map(|client| client.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            wanted
        } else {
            tui::column_config::ColumnConfig::load().enabled_clients()
        };

    if cli.json {
        if cli.watch {
            anyhow::bail!("--watch is not supported with --json (JSON is a one-shot dump)");
        }
        let live = filtered_clients(&clients, &enabled_initial);
        let analyses = analyze_all(&live, plan);
        let out = JsonOutput {
            plan: cli.plan.clone(),
            sessions: analyses.iter().map(JsonSession::from).collect(),
            totals: JsonTotals::from(&analyses),
        };
        serde_json::to_writer_pretty(std::io::stdout(), &out)?;
        println!();
        return Ok(());
    }

    // `--list` (with or without `--watch`) keeps the scriptable flat
    // table output. Bare `agtop` (no `--list`, no `--json`) launches
    // the interactive TUI.
    if cli.watch {
        if !cli.list {
            anyhow::bail!(
                "--watch without --list is redundant: the TUI refreshes automatically. \
                 Use `agtop --list --watch` for the non-interactive refresh loop, or \
                 run `agtop --delay <secs>` for the TUI."
            );
        }
        let live = filtered_clients(&clients, &enabled_initial);
        run_watch(&live, plan, cli.delay.max(1))?;
    } else if cli.list {
        let live = filtered_clients(&clients, &enabled_initial);
        let analyses = analyze_all(&live, plan);
        let summaries = discover_all(&live);
        render_table(&summaries, &analyses);
    } else {
        // Default: launch the TUI. Any rendering error is bubbled up
        // after the terminal has been restored (tui::run guarantees
        // teardown on both success and failure paths).
        tui::run(
            clients,
            enabled_initial,
            plan,
            std::time::Duration::from_secs(cli.delay.max(1)),
            cli.dashboard,
        )?;
    }

    Ok(())
}

/// Non-TUI refresh loop: clears the screen, re-renders the table, sleeps.
/// Runs until Ctrl-C (SIGINT). Intended for CI-ish use (`--list --watch`).
fn run_watch(
    clients: &[std::sync::Arc<dyn agtop_core::Client>],
    plan: Plan,
    delay_secs: u64,
) -> Result<()> {
    use crossterm::{cursor, execute, terminal};
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        // Best-effort Ctrl-C handler: flip the flag so the loop exits, the
        // cursor gets restored, and the shell prompt isn't left mangled.
        // If installation fails (handler already set by a parent process),
        // fall back to default SIGINT behavior — the cursor may not be
        // restored in that edge case, but `reset` fixes it.
        let _ = ctrlc::set_handler(move || {
            running.store(false, Ordering::SeqCst);
        });
    }

    let mut stdout = std::io::stdout();
    // Hide the cursor while redrawing to avoid flicker.
    let _ = execute!(stdout, cursor::Hide);
    let result = (|| -> Result<()> {
        while running.load(Ordering::SeqCst) {
            // Clear screen + move to top-left before each render.
            execute!(
                stdout,
                terminal::Clear(terminal::ClearType::All),
                cursor::MoveTo(0, 0)
            )
            .context("clear screen")?;

            let analyses = analyze_all(clients, plan);
            let summaries = discover_all(clients);
            render_table(&summaries, &analyses);
            writeln!(
                stdout,
                "\n(watch: refreshing every {}s — Ctrl-C to exit)",
                delay_secs
            )?;
            stdout.flush().ok();

            // Sleep in short chunks so Ctrl-C feels responsive even with
            // a large --delay.
            let mut remaining = delay_secs;
            while remaining > 0 && running.load(Ordering::SeqCst) {
                let chunk = remaining.min(1);
                std::thread::sleep(std::time::Duration::from_secs(chunk));
                remaining -= chunk;
            }
        }
        Ok(())
    })();

    // Restore the cursor regardless of how the loop exited.
    let _ = execute!(stdout, cursor::Show);
    result
}

/// Prime the LiteLLM pricing index. The first lookup will auto-load
/// from the on-disk cache regardless; this function only handles the
/// explicit-refresh and no-network paths:
///
/// - `--no-pricing-refresh`: install an empty index so `lookup` never
///   reads the cache file. The built-in tables alone apply.
/// - `--refresh-pricing`: fetch from upstream and swap in the fresh
///   index. On failure, log and fall through to the on-disk cache.
/// - Otherwise (default): if the cache is missing *or* stale, do a
///   synchronous refresh once at startup. The stale-but-present case is
///   tolerated silently: we'd rather start fast than block users behind
///   GitHub.
fn setup_pricing(refresh: bool, disable: bool) {
    use agtop_core::litellm;

    if disable {
        // Install an empty index so auto-load never fires. The built-in
        // tables stay in charge.
        agtop_core::pricing::set_pricing_index(litellm::PricingIndex::default());
        return;
    }

    let cache = litellm::cache_path();
    let have_fresh_cache = cache
        .as_deref()
        .map(litellm::is_cache_fresh)
        .unwrap_or(false);
    let have_any_cache = cache.as_deref().map(|p| p.exists()).unwrap_or(false);

    // Explicit refresh, or: no cache at all and we're online-permissive.
    let should_fetch = refresh || !have_any_cache;
    if should_fetch {
        match litellm::refresh_cache() {
            Ok(idx) => {
                tracing::info!(entries = idx.len(), "installed fresh LiteLLM pricing index");
                agtop_core::pricing::set_pricing_index(idx);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "LiteLLM refresh failed; falling back to on-disk cache (if any) + built-ins"
                );
            }
        }
    } else if !have_fresh_cache {
        // Cache exists but is stale. Do a quiet background-ish refresh
        // (synchronous, but bounded by FETCH_TIMEOUT in litellm.rs).
        // Failure is silent: the stale cache still loads via autoload.
        if let Ok(idx) = litellm::refresh_cache() {
            agtop_core::pricing::set_pricing_index(idx);
        }
    }
    // Otherwise cache is fresh — let `pricing::lookup`'s autoload
    // handle the on-disk read at first use.
}

fn init_logging(verbose: bool, tui_mode: bool) {
    use tracing_subscriber::{fmt, EnvFilter};

    // In TUI mode the alternate screen occupies the whole terminal; any
    // log lines written to stderr corrupt the ratatui rendering and make
    // the UI unusable.  Unless the caller overrides via RUST_LOG, we
    // silence everything.  With --verbose we redirect to a temp file so
    // the user can `tail` it without affecting the TUI.
    if tui_mode && std::env::var("RUST_LOG").is_err() {
        if verbose {
            // Redirect info-level logs to a temp file so the TUI stays clean.
            // Users can `tail -f /tmp/agtop.log` in a separate terminal.
            let log_path = std::env::temp_dir().join("agtop.log");
            if let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
            {
                // std::fs::File implements MakeWriter (each call clones the fd).
                let filter = EnvFilter::new("info");
                let _ = fmt().with_env_filter(filter).with_writer(file).try_init();
                return;
            }
        }
        // No --verbose and no RUST_LOG: install an off filter so zero log
        // output reaches stderr (and therefore the TUI screen).
        let filter = EnvFilter::new("off");
        let _ = fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
        return;
    }

    let default = if verbose { "info" } else { "warn" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn parse_client_kind(s: &str) -> Option<ClientKind> {
    match s.to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" => Some(ClientKind::Claude),
        "codex" => Some(ClientKind::Codex),
        "opencode" | "open-code" => Some(ClientKind::OpenCode),
        "copilot" | "github-copilot" => Some(ClientKind::Copilot),
        "gemini" | "gemini-cli" => Some(ClientKind::GeminiCli),
        "cursor" => Some(ClientKind::Cursor),
        "antigravity" => Some(ClientKind::Antigravity),
        _ => None,
    }
}

fn filtered_clients(
    all: &[std::sync::Arc<dyn agtop_core::Client>],
    enabled: &std::collections::HashSet<agtop_core::ClientKind>,
) -> Vec<std::sync::Arc<dyn agtop_core::Client>> {
    all.iter()
        .filter(|client| enabled.contains(&client.kind()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod client_parse_tests {
    use super::*;
    use agtop_core::ClientKind;

    #[test]
    fn parse_client_kind_covers_all_variants() {
        assert_eq!(parse_client_kind("claude"), Some(ClientKind::Claude));
        assert_eq!(parse_client_kind("codex"), Some(ClientKind::Codex));
        assert_eq!(parse_client_kind("opencode"), Some(ClientKind::OpenCode));
        assert_eq!(parse_client_kind("copilot"), Some(ClientKind::Copilot));
        assert_eq!(parse_client_kind("gemini-cli"), Some(ClientKind::GeminiCli));
        assert_eq!(parse_client_kind("cursor"), Some(ClientKind::Cursor));
        assert_eq!(
            parse_client_kind("antigravity"),
            Some(ClientKind::Antigravity)
        );
        assert_eq!(parse_client_kind("bogus"), None);
    }
}

fn render_table(summaries: &[agtop_core::SessionSummary], analyses: &[SessionAnalysis]) {
    // Index analyses by session_id for quick lookup.
    use std::collections::HashMap;
    let by_id: HashMap<&str, &SessionAnalysis> = analyses
        .iter()
        .map(|a| (a.summary.session_id.as_str(), a))
        .collect();

    let now = Utc::now();
    println!(
        "{:<10}  {:<16}  {:<10}  {:<16}  {:>4}  {:<20}  {:<18}  {:>9}  {:>9}  {:>9}  {:>8}",
        "CLIENT",
        "SUBSCRIPTION",
        "SESSION",
        "STARTED",
        "AGE",
        "MODEL",
        "CWD",
        "IN",
        "OUT",
        "CACHE",
        "COST$"
    );
    println!("{}", "-".repeat(160));

    let mut printed = 0usize;
    for s in summaries {
        let a = by_id.get(s.session_id.as_str());
        let (input, output, cache, cost_str) = match a {
            Some(a) => {
                let t = &a.tokens;
                let c = &a.cost;
                let cost = if c.included {
                    "incl".to_string()
                } else {
                    format!("{:.4}", c.total)
                };
                (
                    fmt::compact(t.input),
                    fmt::compact(t.output),
                    fmt::compact(
                        t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input,
                    ),
                    cost,
                )
            }
            None => (
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "?".to_string(),
            ),
        };

        let mut short_session = fmt::short_id(&s.session_id);
        // Flag Claude sessions that folded in subagent sidechains:
        // "20cb0a50+2" = 2 subagent files merged. Only appears when > 0.
        if let Some(a) = a {
            if a.subagent_file_count > 0 {
                short_session.push_str(&format!("+{}", a.subagent_file_count));
            }
        }
        let model = s.model.clone().unwrap_or_else(|| "?".into());
        let subscription = s.subscription.clone().unwrap_or_else(|| "-".into());
        let cwd = s.cwd.clone().unwrap_or_else(|| "-".into());
        let started = s
            .started_at
            .map(fmt::format_local_datetime)
            .unwrap_or_else(|| "-".into());
        let age = s
            .last_active
            .map(|t| fmt::relative_age(t, now))
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<10}  {:<16}  {:<10}  {:<16}  {:>4}  {:<20}  {:<18}  {:>9}  {:>9}  {:>9}  {:>8}",
            s.client.as_str(),
            fmt::fit(&subscription, 16),
            fmt::fit(&short_session, 10),
            fmt::fit(&started, 16),
            fmt::fit(&age, 4),
            fmt::fit(&model, 20),
            fmt::fit(&fmt::shorten_path(&cwd), 18),
            input,
            output,
            cache,
            cost_str,
        );
        printed += 1;
        if printed >= 200 {
            let remaining = summaries.len().saturating_sub(printed);
            if remaining > 0 {
                println!(
                    "… {} more sessions not shown (use --json for all)",
                    remaining
                );
            }
            break;
        }
    }

    // Footer totals.
    let totals = JsonTotals::from(&analyses.to_vec());
    println!("{}", "-".repeat(160));
    println!(
        "totals: {} sessions  in={}  out={}  cache={}  cost=${:.4} (billed)  incl.sessions={}",
        analyses.len(),
        fmt::compact(totals.tokens.input),
        fmt::compact(totals.tokens.output),
        fmt::compact(
            totals.tokens.cache_read
                + totals.tokens.cache_write_5m
                + totals.tokens.cache_write_1h
                + totals.tokens.cached_input
        ),
        totals.cost_total_billed,
        totals.included_sessions
    );
}

// ---------------------------------------------------------------------------
// JSON output types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct JsonOutput {
    plan: String,
    sessions: Vec<JsonSession>,
    totals: JsonTotals,
}

#[derive(Debug, Serialize)]
struct JsonSession {
    client: &'static str,
    subscription: Option<String>,
    session_id: String,
    model: Option<String>,
    effective_model: Option<String>,
    cwd: Option<String>,
    started_at: Option<DateTime<Utc>>,
    last_active: Option<DateTime<Utc>>,
    state: Option<String>,
    display_state: String,
    state_detail: Option<String>,
    model_effort: Option<String>,
    model_effort_detail: Option<String>,
    tokens: agtop_core::TokenTotals,
    cost: agtop_core::CostBreakdown,
    /// Number of Claude subagent sidechains that were folded into
    /// `tokens` / `cost`. Zero for non-Claude providers and for Claude
    /// sessions without subagents.
    subagent_file_count: usize,
    tool_call_count: Option<u64>,
    duration_secs: Option<u64>,
    context_used_pct: Option<f64>,
    context_used_tokens: Option<u64>,
    context_window: Option<u64>,
    data_path: String,
}

impl From<&SessionAnalysis> for JsonSession {
    fn from(a: &SessionAnalysis) -> Self {
        Self::from_analysis(a, Utc::now())
    }
}

impl JsonSession {
    fn from_analysis(a: &SessionAnalysis, now: DateTime<Utc>) -> Self {
        let (display_state_label, _) = display_state(a, now);
        Self {
            client: a.summary.client.as_str(),
            subscription: a.summary.subscription.clone(),
            session_id: a.summary.session_id.clone(),
            model: a.summary.model.clone(),
            effective_model: a.effective_model.clone(),
            cwd: a.summary.cwd.clone(),
            started_at: a.summary.started_at,
            last_active: a.summary.last_active,
            state: a.summary.state.clone(),
            display_state: display_state_label.to_string(),
            state_detail: a.summary.state_detail.clone(),
            model_effort: a.summary.model_effort.clone(),
            model_effort_detail: a.summary.model_effort_detail.clone(),
            tokens: a.tokens.clone(),
            cost: a.cost.clone(),
            subagent_file_count: a.subagent_file_count,
            tool_call_count: a.tool_call_count,
            duration_secs: a.duration_secs,
            context_used_pct: a.context_used_pct,
            context_used_tokens: a.context_used_tokens,
            context_window: a.context_window,
            data_path: a.summary.data_path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod json_output_tests {
    use super::*;
    use agtop_core::session::{ClientKind, CostBreakdown, SessionSummary, TokenTotals};
    use std::path::PathBuf;

    #[test]
    fn json_session_keeps_raw_state_and_adds_display_state() {
        let now = Utc::now();
        let summary = SessionSummary::new(
            ClientKind::OpenCode,
            None,
            "sess".into(),
            Some(now - chrono::Duration::minutes(1)),
            Some(now - chrono::Duration::seconds(5)),
            Some("model".into()),
            Some("/tmp".into()),
            PathBuf::from("/tmp/sess.json"),
            Some("stopped".into()),
            Some("finish=stop".into()),
            None,
            None,
        );
        let analysis = SessionAnalysis::new(
            summary,
            TokenTotals::default(),
            CostBreakdown::default(),
            None,
            0,
            None,
            None,
            None,
            None,
            None,
        );

        let json = JsonSession::from_analysis(&analysis, now);

        assert_eq!(json.state.as_deref(), Some("stopped"));
        assert_eq!(json.display_state, "working");
    }
}

#[derive(Debug, Serialize, Default)]
struct JsonTotals {
    sessions: usize,
    included_sessions: usize,
    tokens: agtop_core::TokenTotals,
    /// Sum of billable cost (included sessions contribute 0).
    cost_total_billed: f64,
}

impl From<&Vec<SessionAnalysis>> for JsonTotals {
    fn from(v: &Vec<SessionAnalysis>) -> Self {
        let mut t = JsonTotals {
            sessions: v.len(),
            ..Default::default()
        };
        for a in v {
            if a.cost.included {
                t.included_sessions += 1;
            } else {
                t.cost_total_billed += a.cost.total;
            }
            t.tokens.input += a.tokens.input;
            t.tokens.cached_input += a.tokens.cached_input;
            t.tokens.output += a.tokens.output;
            t.tokens.reasoning_output += a.tokens.reasoning_output;
            t.tokens.cache_read += a.tokens.cache_read;
            t.tokens.cache_write_5m += a.tokens.cache_write_5m;
            t.tokens.cache_write_1h += a.tokens.cache_write_1h;
        }
        t
    }
}
