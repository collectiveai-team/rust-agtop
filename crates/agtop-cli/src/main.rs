//! agtop — htop-style dashboard for AI coding agents (Rust port).
//!
//! MVP scope: session discovery + cost estimation, with `--list` and
//! `--json` output. Interactive TUI will land in a follow-up.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::Serialize;

use agtop_core::{
    analyze_all, default_providers, discover_all, pricing::Plan, session::SessionAnalysis,
    ProviderKind,
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

    /// Only include sessions from this provider (claude, codex, opencode).
    /// May be given multiple times.
    #[arg(long = "provider", value_name = "KIND")]
    providers: Vec<String>,

    /// Re-render the `--list` table every `--delay` seconds until Ctrl-C.
    /// Ignored in `--json` mode.
    #[arg(short = 'w', long)]
    watch: bool,

    /// Seconds between refreshes in `--watch` mode (default: 2).
    #[arg(short = 'd', long, default_value_t = 2u64, value_name = "SECS")]
    delay: u64,

    /// Verbose logging to stderr (sets RUST_LOG=info if unset).
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    init_logging(cli.verbose);

    let plan = Plan::parse(&cli.plan)
        .with_context(|| format!("unknown plan '{}'; try retail|max|included", cli.plan))?;

    let mut providers = default_providers();
    if !cli.providers.is_empty() {
        let wanted: Vec<ProviderKind> = cli
            .providers
            .iter()
            .filter_map(|s| parse_provider_kind(s))
            .collect();
        if wanted.is_empty() {
            anyhow::bail!(
                "no recognized --provider values (got: {:?}). expected: claude, codex, opencode",
                cli.providers
            );
        }
        providers.retain(|p| wanted.contains(&p.kind()));
    }

    if cli.json {
        if cli.watch {
            anyhow::bail!("--watch is not supported with --json (JSON is a one-shot dump)");
        }
        let analyses = analyze_all(&providers, plan);
        let out = JsonOutput {
            plan: cli.plan.clone(),
            sessions: analyses.iter().map(JsonSession::from).collect(),
            totals: JsonTotals::from(&analyses),
        };
        serde_json::to_writer_pretty(std::io::stdout(), &out)?;
        println!();
        return Ok(());
    }

    // Default + --list both print the table. (Interactive TUI is TODO; for
    // now --list is the only mode.)
    if cli.watch {
        run_watch(&providers, plan, cli.delay.max(1))?;
    } else if cli.list || !cli.json {
        let analyses = analyze_all(&providers, plan);
        let summaries = discover_all(&providers);
        render_table(&summaries, &analyses);
        if !cli.list {
            eprintln!(
                "\n(interactive TUI coming soon — for now, agtop prints the table; pass --json for machine-readable output.)"
            );
        }
    }

    Ok(())
}

/// Non-TUI refresh loop: clears the screen, re-renders the table, sleeps.
/// Runs until Ctrl-C (SIGINT). Intended for CI-ish use (`--list --watch`).
fn run_watch(
    providers: &[std::sync::Arc<dyn agtop_core::Provider>],
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

            let analyses = analyze_all(providers, plan);
            let summaries = discover_all(providers);
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

fn init_logging(verbose: bool) {
    use tracing_subscriber::{fmt, EnvFilter};
    let default = if verbose { "info" } else { "warn" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn parse_provider_kind(s: &str) -> Option<ProviderKind> {
    match s.to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claudecode" => Some(ProviderKind::Claude),
        "codex" => Some(ProviderKind::Codex),
        "opencode" | "open-code" => Some(ProviderKind::OpenCode),
        _ => None,
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
        "{:<10}  {:<10}  {:<16}  {:>4}  {:<22}  {:<22}  {:>9}  {:>9}  {:>9}  {:>8}",
        "PROVIDER", "SESSION", "STARTED", "AGE", "MODEL", "CWD", "IN", "OUT", "CACHE", "COST$"
    );
    println!("{}", "-".repeat(140));

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
                    compact(t.input),
                    compact(t.output),
                    compact(t.cache_read + t.cache_write_5m + t.cache_write_1h + t.cached_input),
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

        let short_session = short_id(&s.session_id);
        let model = s.model.clone().unwrap_or_else(|| "?".into());
        let cwd = s.cwd.clone().unwrap_or_else(|| "-".into());
        let started = s
            .started_at
            .map(format_local_datetime)
            .unwrap_or_else(|| "-".into());
        let age = s
            .last_active
            .map(|t| relative_age(t, now))
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<10}  {:<10}  {:<16}  {:>4}  {:<22}  {:<22}  {:>9}  {:>9}  {:>9}  {:>8}",
            s.provider.as_str(),
            fit(&short_session, 10),
            fit(&started, 16),
            fit(&age, 4),
            fit(&model, 22),
            fit(&shorten_path(&cwd), 22),
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
    println!("{}", "-".repeat(140));
    println!(
        "totals: {} sessions  in={}  out={}  cache={}  cost=${:.4} (billed)  incl.sessions={}",
        analyses.len(),
        compact(totals.tokens.input),
        compact(totals.tokens.output),
        compact(
            totals.tokens.cache_read
                + totals.tokens.cache_write_5m
                + totals.tokens.cache_write_1h
                + totals.tokens.cached_input
        ),
        totals.cost_total_billed,
        totals.included_sessions
    );
}

/// Format a UTC timestamp as a short local datetime: "YYYY-MM-DD HH:MM".
/// Uses the system's local timezone so the output is meaningful to the user.
fn format_local_datetime(ts: DateTime<Utc>) -> String {
    use chrono::Local;
    ts.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// htop/agtop-style relative age. Mirrors the original index.js helper.
fn relative_age(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    if secs < 60 {
        return "now".into();
    }
    if secs < 3600 {
        return format!("{}m", secs / 60);
    }
    if secs < 86_400 {
        return format!("{}h", secs / 3600);
    }
    if secs < 604_800 {
        return format!("{}d", secs / 86_400);
    }
    if secs < 2_592_000 {
        return format!("{}w", secs / 604_800);
    }
    if secs < 31_536_000 {
        return format!("{}mo", secs / 2_592_000);
    }
    format!("{}y", secs / 31_536_000)
}

fn compact(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}G", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

fn short_id(id: &str) -> String {
    // Claude: full UUID → first 8 chars. Codex: full UUID → first 8 chars.
    // OpenCode: ses_<id> → keep full (already short).
    if id.starts_with("ses_") {
        return id[..id.len().min(10)].to_string();
    }
    id.chars().take(8).collect()
}

fn fit(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        format!("{:<w$}", s, w = w)
    } else {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn shorten_path(p: &str) -> String {
    if let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) {
        if let Some(rest) = p.strip_prefix(&home) {
            return format!("~{}", rest);
        }
    }
    p.to_string()
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
    provider: &'static str,
    session_id: String,
    model: Option<String>,
    effective_model: Option<String>,
    cwd: Option<String>,
    started_at: Option<DateTime<Utc>>,
    last_active: Option<DateTime<Utc>>,
    tokens: agtop_core::TokenTotals,
    cost: agtop_core::CostBreakdown,
    data_path: String,
}

impl From<&SessionAnalysis> for JsonSession {
    fn from(a: &SessionAnalysis) -> Self {
        Self {
            provider: a.summary.provider.as_str(),
            session_id: a.summary.session_id.clone(),
            model: a.summary.model.clone(),
            effective_model: a.effective_model.clone(),
            cwd: a.summary.cwd.clone(),
            started_at: a.summary.started_at,
            last_active: a.summary.last_active,
            tokens: a.tokens.clone(),
            cost: a.cost.clone(),
            data_path: a.summary.data_path.display().to_string(),
        }
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
