// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
mod curly_expand;

use anyhow::Result;
use clap::{Parser, Subcommand};
use dreamseq::cloud::{CloudClient, CredentialStore};
use dreamseq::color::Colorize;
use dreamseq::present::CompletionReport;
use dreamseq::{Anthology, Dreamseq, DreamseqConfig};
use tracing_subscriber::EnvFilter;

mod cli_support;
use cli_support::*;

#[derive(Parser)]
#[command(name = "dreamseq")]
#[command(version)]
#[command(about = "🌙 End-of-day agent reflection protocol", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// ▶️ Run the Dreamseq pipeline
    Run {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
        /// Show full diagnostics and artifact paths
        #[arg(short, long)]
        verbose: bool,
        /// Emit the completion report as JSON
        #[arg(long)]
        json: bool,
        /// Publish intervention backlogs into registered Speck projects
        #[arg(long)]
        publish_dreams: bool,
    },
    /// 🛠️ Initialize configuration
    Init,
    /// 🔐 Pair this device with Dreamsequence
    Login {
        /// Dreamsequence service URL
        #[arg(long)]
        api_url: Option<String>,
        /// Print the verification URL without opening a browser
        #[arg(long)]
        no_open: bool,
    },
    /// 🔓 Revoke this device and remove its local credential
    Logout,
    /// ☁️ Upload existing Dreamseq anthologies from output directories
    Sync {
        /// Directory to scan; repeat to include more than one directory
        #[arg(long = "dir")]
        directories: Vec<std::path::PathBuf>,
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
        /// Emit the sync summary as JSON
        #[arg(long)]
        json: bool,
    },
    /// 📋 Generate report from an existing anthology
    Report {
        /// Date of the anthology (YYYY-MM-DD)
        date: String,
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
        /// Show full diagnostics
        #[arg(short, long)]
        verbose: bool,
        /// Emit the completion report as JSON
        #[arg(long)]
        json: bool,
        /// Publish intervention backlogs into registered Speck projects
        #[arg(long)]
        publish_dreams: bool,
    },
    /// 📈 Show trend analysis
    Trends {
        /// Number of days to analyze
        #[arg(short, long, default_value = "30")]
        days: i64,
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// ⚙️ Interactively edit configuration: providers, fallback, harnesses
    Config {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// 🧭 Compare remotes against local repos with azusa and show cached drift scores
    Drift {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
        /// Emit the drift view as JSON
        #[arg(long)]
        json: bool,
        /// Skip the azusa scan and show only cached scores
        #[arg(long)]
        cached: bool,
    },
    /// 🧰 Detect agent harnesses, dev tools, and workstation specs, and report them
    StackReport {
        /// Print the composed payload without sending it
        #[arg(long)]
        dry_run: bool,
        /// Print the API JSON response on success
        #[arg(long)]
        json: bool,
        /// Comma-separated devtools registry batches to detect (default: all)
        #[arg(long, value_parser = dreamseq::stack_report::parse_batches)]
        batches: Option<std::collections::HashSet<u32>>,
    },
    /// 🔎 Probe agent harnesses for provider/model/auth capability evidence
    Stack {
        #[command(subcommand)]
        action: StackCommands,
    },
    /// 🧭 Ingest Lucid agent sessions (lucid.session/v1)
    Lucid {
        #[command(subcommand)]
        action: LucidCommands,
    },
}

#[derive(Subcommand)]
enum LucidCommands {
    /// Fetch a project's session export since the last pull into a directory
    /// that a `lucid_session` harness reads
    Pull {
        /// Lucid origin, e.g. https://lucid.dreamsequence.pro
        #[arg(long)]
        url: String,
        /// Lucid project id
        #[arg(long)]
        project: String,
        /// Output directory (the harness `log_path`)
        #[arg(long)]
        out: std::path::PathBuf,
        /// Days to fetch on the first pull (at most 90)
        #[arg(long, default_value = "7")]
        days: i64,
        /// Environment variable holding the Lucid bearer token
        #[arg(long, default_value = "LUCID_API_TOKEN")]
        token_env: String,
        /// Emit the pull summary as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum StackCommands {
    /// Probe one harness (default: every detected harness) for provider,
    /// model, auth, and connectivity evidence — never fails on probe issues
    Probe {
        /// Harness name to probe (e.g. claude, codex, kimi); omit to probe all
        harness: Option<String>,
        /// Emit the probe entries as JSON instead of human-readable blocks
        #[arg(long)]
        json: bool,
    },
}

fn init_tracing(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("info")
    } else {
        EnvFilter::new("warn")
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn __curly_original_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            config,
            verbose,
            json,
            publish_dreams,
        } => {
            init_tracing(verbose);
            let trace_id = dreamseq::telemetry::new_trace_id();

            let config = if let Some(config_path) = config {
                DreamseqConfig::load_from_path(std::path::Path::new(&config_path))?
            } else {
                DreamseqConfig::load()?
            };

            let repository = std::env::current_dir()?;
            let mut dreamseq = Dreamseq::new(config)?;
            let mut anthology = dreamseq.run_with_trace_id(&trace_id).await?;

            anthology.generate()?;
            let saved_path = anthology.save()?;
            if let Err(error) = sync_if_paired(&anthology, &trace_id).await {
                eprintln!("{} Cloud sync failed: {error}", "⚠️".yellow());
            }
            // "Upon next contact" remote stack-report scheduling: fulfil a
            // pending request now if this device is paired and one exists.
            match dreamseq::stack_report::check_in_if_paired(&trace_id).await {
                Ok(Some(true)) => {
                    tracing::info!("submitted a stack report requested from the dashboard")
                }
                Ok(Some(false)) | Ok(None) => {}
                Err(error) => {
                    eprintln!("{} Stack-report check-in failed: {error}", "⚠️".yellow())
                }
            }
            if let Some(observations) = dreamseq.last_drift_observations()
                && !observations.is_empty()
            {
                match dreamseq::azusa::upload_if_paired(observations, &trace_id).await {
                    Ok(Some(count)) => {
                        tracing::info!(count, "uploaded drift scores to Dreamsequence")
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("{} Drift score upload failed: {error}", "⚠️".yellow())
                    }
                }
            }
            let (dreams_roots, dreams_paths) = if publish_dreams {
                save_dreams_to_speck_projects(&anthology)?
            } else {
                (Vec::new(), Vec::new())
            };

            // The renderer expects the anthology path to be relative to the
            // repository root for a clean report. Fix it if `save()` returned
            // an absolute path outside the repo.
            let relative_anthology = make_relative_to(&saved_path, &repository);

            let mut report =
                CompletionReport::from_anthology(&anthology, &repository, &dreams_roots);
            // Override with the actual saved path and handoff file paths.
            report.artifacts.anthology_path = relative_anthology;
            report.artifacts.dreams_paths = dreams_paths;

            render_report(&report, json, verbose)?;
            if anthology.config.enable_tts {
                notify_completion(&anthology);
            }
        }
        Commands::Init => {
            init_tracing(false);
            let config = DreamseqConfig::discover();
            config.save()?;
            println!(
                "{} Configuration initialized at {}",
                "✅".green(),
                DreamseqConfig::path()?.display().to_string().cyan()
            );
            println!(
                "{} Discovered {} harness log sources.",
                "🔍".blue(),
                config.harnesses.len()
            );
            println!(
                "{} Add a Groq API key if you want remote semantic analysis.",
                "💡".yellow()
            );
        }
        Commands::Login { api_url, no_open } => {
            init_tracing(false);
            let trace_id = dreamseq::telemetry::new_trace_id();
            let store = CredentialStore::discover()?;
            let client = CloudClient::new(api_url.as_deref())?;
            let credentials = client
                .pair_with_trace_id(&store, !no_open, &trace_id)
                .await?;
            println!(
                "{} Device paired with account {}.",
                "✅".green(),
                credentials.account_id.cyan()
            );
            // Mandatory first-contact reporting: every newly onboarded
            // device submits a stack report immediately, not just when a
            // user happens to run `stack-report` or `run` later. Best-effort
            // like every other post-pairing side effect in this codebase —
            // a network hiccup here shouldn't fail an otherwise-successful
            // pairing, and dreamseq run's own "upon next contact" check-in
            // will retry it regardless.
            if let Err(error) = dreamseq::stack_report::run_stack_report(false, false, None).await {
                eprintln!("{} Initial stack report failed: {error}", "⚠️".yellow());
            }
        }
        Commands::Logout => {
            init_tracing(false);
            let trace_id = dreamseq::telemetry::new_trace_id();
            let store = CredentialStore::discover()?;
            if let Some(credentials) = store.load_optional()? {
                CloudClient::new(Some(&credentials.api_url))?
                    .revoke_with_trace_id(&credentials, &trace_id)
                    .await?;
                store.remove()?;
                println!(
                    "{} Device logged out and local credential removed.",
                    "✅".green()
                );
            } else {
                println!("{} This device is already logged out.", "ℹ️".blue());
            }
        }
        Commands::Sync {
            mut directories,
            config,
            json,
        } => {
            init_tracing(false);
            let trace_id = dreamseq::telemetry::new_trace_id();
            let config = if let Some(config_path) = config {
                DreamseqConfig::load_from_path(std::path::Path::new(&config_path))?
            } else {
                DreamseqConfig::load()?
            };
            if directories.is_empty() {
                directories.push(config.anthologies_dir);
                directories.push(config.output_dir);
            }
            directories.sort();
            directories.dedup();
            let credentials = CredentialStore::discover()?.load()?;
            let client = CloudClient::new(Some(&credentials.api_url))?;
            let summary = client
                .sync_directories_with_trace_id(&credentials, &directories, &trace_id)
                .await?;
            if json {
                println!("{}", serde_json::to_string(&summary)?);
            } else {
                println!(
                    "{} Uploaded {} anthologies; skipped {} unrelated JSON files; {} failed.",
                    if summary.failed == 0 {
                        "✅".green()
                    } else {
                        "⚠️".yellow()
                    },
                    summary.uploaded,
                    summary.skipped,
                    summary.failed
                );
            }
            if summary.failed > 0 {
                anyhow::bail!("one or more anthologies could not be uploaded");
            }
        }
        Commands::Report {
            date,
            config,
            verbose,
            json,
            publish_dreams,
        } => {
            init_tracing(verbose);
            let config = if let Some(config_path) = config {
                DreamseqConfig::load_from_path(std::path::Path::new(&config_path))?
            } else {
                DreamseqConfig::load()?
            };
            let anthology_path = find_anthology_for_date(&config.anthologies_dir, &date)?
                .ok_or_else(|| {
                    anyhow::anyhow!("{} Anthology not found for date: {}", "❌".red(), date)
                })?;

            let content = std::fs::read_to_string(&anthology_path)?;
            let anthology: Anthology = serde_json::from_str(&content)?;
            let repository = std::env::current_dir()?;
            let (dreams_roots, dreams_paths) = if publish_dreams {
                save_dreams_to_speck_projects(&anthology)?
            } else {
                (Vec::new(), Vec::new())
            };
            let mut report =
                CompletionReport::from_anthology(&anthology, &repository, &dreams_roots);
            report.artifacts.anthology_path = make_relative_to(&anthology_path, &repository);
            report.artifacts.dreams_paths = dreams_paths;
            render_report(&report, json, verbose)?;
        }
        Commands::Trends { days, config } => {
            init_tracing(false);
            let trace_id = dreamseq::telemetry::new_trace_id();
            let config = if let Some(config_path) = config {
                DreamseqConfig::load_from_path(std::path::Path::new(&config_path))?
            } else {
                DreamseqConfig::load()?
            };
            let analyzer = dreamseq::TrendAnalyzer::with_directory(config.anthologies_dir.clone());

            let most_recent = find_latest_anthology(&config.anthologies_dir)?;

            if let Some(anthology) = most_recent {
                let trends = analyzer
                    .analyze_for_days_with_trace_id(&anthology, days, &trace_id)
                    .await?;
                print_trends(&trends);
            } else {
                println!(
                    "{} No anthologies found. Run {} first.",
                    "⚠️".yellow(),
                    "dreamseq run".cyan()
                );
            }
        }
        Commands::Config { config } => {
            init_tracing(false);
            let save_path = config.as_ref().map(std::path::PathBuf::from);
            let loaded = if let Some(config_path) = &config {
                DreamseqConfig::load_from_path(std::path::Path::new(config_path))?
            } else {
                DreamseqConfig::load()?
            };
            dreamseq::config_tui::run(loaded, save_path)?;
        }
        Commands::Drift {
            config,
            json,
            cached,
        } => {
            init_tracing(false);
            let config = if let Some(config_path) = config {
                DreamseqConfig::load_from_path(std::path::Path::new(&config_path))?
            } else {
                DreamseqConfig::load()?
            };
            dreamseq::azusa::run_drift(&config, cached, json).await?;
        }
        Commands::StackReport {
            dry_run,
            json,
            batches,
        } => {
            init_tracing(false);
            dreamseq::stack_report::run_stack_report(dry_run, json, batches).await?;
        }
        Commands::Stack { action } => match action {
            StackCommands::Probe { harness, json } => {
                init_tracing(false);
                dreamseq::stack_probe::run_probe_command(harness, json).await?;
            }
        },
        Commands::Lucid { action } => match action {
            LucidCommands::Pull {
                url,
                project,
                out,
                days,
                token_env,
                json,
            } => {
                init_tracing(false);
                let summary = dreamseq::lucid::pull(&dreamseq::lucid::PullOptions {
                    base_url: url,
                    project_id: project,
                    out_dir: out,
                    initial_days: days,
                    token_env,
                })
                .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&summary)?);
                } else {
                    println!(
                        "Pulled {} Lucid event(s) into {} file(s); {} rejected.",
                        summary.events,
                        summary.files_written.len(),
                        summary.rejected
                    );
                }
            }
        },
    }

    Ok(())
}

fn main() {
    let raw_args: Vec<String> = std::env::args().collect();
    let mut positions: Vec<usize> = Vec::new();
    let mut fields: Vec<Vec<String>> = Vec::new();
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--binary" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--binary=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--binary={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--readme" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--readme=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--readme={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--license" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--license=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--license={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--format" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--format=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--format={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--output" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--output=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--output={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--output" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--output=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--output={}", v))
                    .collect(),
            );
            break;
        }
    }
    for (__i, __a) in raw_args.iter().enumerate() {
        if __a == "--manifest" {
            if let Some(__v) = raw_args.get(__i + 1) {
                positions.push(__i + 1);
                fields.push(curly_expand::expand_or_literal(__v));
            }
            break;
        } else if let Some(__v) = __a.strip_prefix("--manifest=") {
            positions.push(__i);
            fields.push(
                curly_expand::expand_or_literal(__v)
                    .into_iter()
                    .map(|v| format!("--manifest={}", v))
                    .collect(),
            );
            break;
        }
    }
    if let Some(__v) = raw_args.get(1) {
        if !__v.starts_with('-') {
            positions.push(1);
            fields.push(curly_expand::expand_or_literal(__v));
        }
    }

    if fields.is_empty() || fields.iter().all(|f| f.len() <= 1) {
        __curly_original_main();
        return;
    }

    let combos = curly_expand::cartesian(&fields);
    let exe = std::env::current_exe().expect("resolve current exe");
    let mut had_failure = false;
    for combo in &combos {
        let mut new_args = raw_args.clone();
        for (slot, value) in positions.iter().zip(combo.iter()) {
            new_args[*slot] = value.clone();
        }
        let status = std::process::Command::new(&exe)
            .args(&new_args[1..])
            .status()
            .expect("failed to re-exec self");
        if !status.success() {
            had_failure = true;
        }
    }
    if had_failure {
        std::process::exit(1);
    }
}
