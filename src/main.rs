use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use dreamseq::present::{CompletionReport, HumanRenderer, JsonRenderer};
use dreamseq::{Anthology, Dreamseq, DreamseqConfig, TrendAnalysis};
use tracing_subscriber::EnvFilter;

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

            if json {
                println!("{}", JsonRenderer::render(&report)?);
            } else {
                let renderer = HumanRenderer::from_env(verbose);
                println!("{}", renderer.render(&report));
            }
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
            if json {
                println!("{}", JsonRenderer::render(&report)?);
            } else {
                let renderer = HumanRenderer::from_env(verbose);
                println!("{}", renderer.render(&report));
            }
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

fn make_relative_to(path: &std::path::Path, base: &std::path::Path) -> std::path::PathBuf {
    path.strip_prefix(base)
        .map_or_else(|_| path.to_path_buf(), |p| p.to_path_buf())
}

fn save_dreams_to_speck_projects(
    anthology: &Anthology,
) -> Result<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let speck_path = home.join(".speckrc");
    let mut roots = Vec::new();
    match std::fs::read_to_string(&speck_path) {
        Ok(content) => {
            let document: toml::Value = toml::from_str(&content)?;
            if let Some(tools) = document.get("tools").and_then(toml::Value::as_table) {
                for tool in tools.values() {
                    if let Some(path) = tool.get("path").and_then(toml::Value::as_str) {
                        let root = std::path::PathBuf::from(path);
                        if root.is_dir() && !roots.contains(&root) {
                            roots.push(root);
                        }
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(path = %speck_path.display(), "Speck registry not found; publishing only to the current repository");
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "Cannot read {}: {}",
                speck_path.display(),
                error
            ));
        }
    }
    if roots.is_empty() {
        roots.push(std::env::current_dir()?);
    }
    roots.sort();

    let mut paths = Vec::with_capacity(roots.len());
    for root in &roots {
        paths.push(anthology.save_dreams(root)?);
    }
    Ok((roots, paths))
}

fn find_anthology_for_date(
    directory: &std::path::Path,
    date: &str,
) -> Result<Option<std::path::PathBuf>> {
    if !directory.exists() {
        return Ok(None);
    }
    let prefix = format!("dreamseq-{date}");
    let mut newest: Option<(chrono::DateTime<chrono::Utc>, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) || path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not read anthology candidate");
                continue;
            }
        };
        let anthology = match serde_json::from_str::<Anthology>(&content) {
            Ok(anthology) => anthology,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not parse anthology candidate");
                continue;
            }
        };
        if newest
            .as_ref()
            .is_none_or(|(generated_at, _)| anthology.generated_at > *generated_at)
        {
            newest = Some((anthology.generated_at, path));
        }
    }
    Ok(newest.map(|(_, path)| path))
}

fn find_latest_anthology(directory: &std::path::Path) -> Result<Option<Anthology>> {
    if !directory.exists() {
        return Ok(None);
    }
    let mut newest: Option<Anthology> = None;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not read anthology candidate");
                continue;
            }
        };
        let anthology = match serde_json::from_str::<Anthology>(&content) {
            Ok(anthology) => anthology,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not parse anthology candidate");
                continue;
            }
        };
        if newest
            .as_ref()
            .is_none_or(|current| anthology.generated_at > current.generated_at)
        {
            newest = Some(anthology);
        }
    }
    Ok(newest)
}

fn notify_completion(anthology: &Anthology) {
    let summary = format!(
        "Dreamseq analyzed {} entries and found {} intervention candidates.",
        anthology.pipeline.raw_entries,
        anthology.candidate_tools.len()
    );
    match std::process::Command::new("voxd-cli")
        .args(["speak", "--system", &summary])
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => tracing::warn!(
            status = %output.status,
            error = %String::from_utf8_lossy(&output.stderr),
            "TTS notification failed"
        ),
        Err(error) => {
            tracing::warn!(error = %error, "voxd-cli is unavailable; TTS notification skipped")
        }
    }
}

fn print_trends(trends: &TrendAnalysis) {
    println!("{} {}", "📅 Period:".blue(), trends.period);
    println!();

    if trends.trends.is_empty() {
        println!("   {}", "No trend data available.".dimmed());
        return;
    }

    for trend_data in trends.trends.values() {
        let direction = match trend_data.trend_direction {
            dreamseq::trends::TrendDirection::Increasing => "📈 Increasing".red(),
            dreamseq::trends::TrendDirection::Decreasing => "📉 Decreasing".green(),
            dreamseq::trends::TrendDirection::Stable => "➡️ Stable".white(),
        };

        println!(
            "   {} {} — {}",
            "▸".cyan(),
            trend_data.metric_name.cyan().bold(),
            direction
        );
        println!("     Current:  {:.2}", trend_data.current_value);
        println!("     Previous: {:.2}", trend_data.previous_value);
        println!();
        println!("{}", trend_data.visualization);
        println!();
    }
}
