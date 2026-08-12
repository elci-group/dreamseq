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
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
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

            let repository = std::env::current_dir()?;
            let dreamseq = Dreamseq::new(config)?;
            let mut anthology = dreamseq.run().await?;

            anthology.generate()?;
            let saved_path = anthology.save()?;
            if let Err(error) = sync_if_paired(&anthology).await {
                eprintln!("{} Cloud sync failed: {error}", "⚠️".yellow());
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
            let store = CredentialStore::discover()?;
            let client = CloudClient::new(api_url.as_deref())?;
            let credentials = client.pair(&store, !no_open).await?;
            println!(
                "{} Device paired with account {}.",
                "✅".green(),
                credentials.account_id.cyan()
            );
        }
        Commands::Logout => {
            init_tracing(false);
            let store = CredentialStore::discover()?;
            if let Some(credentials) = store.load_optional()? {
                CloudClient::new(Some(&credentials.api_url))?
                    .revoke(&credentials)
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
            let summary = client.sync_directories(&credentials, &directories).await?;
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
            let config = if let Some(config_path) = config {
                DreamseqConfig::load_from_path(std::path::Path::new(&config_path))?
            } else {
                DreamseqConfig::load()?
            };
            let analyzer = dreamseq::TrendAnalyzer::with_directory(config.anthologies_dir.clone());

            let most_recent = find_latest_anthology(&config.anthologies_dir)?;

            if let Some(anthology) = most_recent {
                let trends = analyzer.analyze_for_days(&anthology, days).await?;
                print_trends(&trends);
            } else {
                println!(
                    "{} No anthologies found. Run {} first.",
                    "⚠️".yellow(),
                    "dreamseq run".cyan()
                );
            }
        }
    }

    Ok(())
}
