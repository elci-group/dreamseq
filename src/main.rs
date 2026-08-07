use anyhow::Result;
use clap::{Parser, Subcommand};
use dreamseq::{Anthology, Dreamseq, DreamseqConfig, Priority, TrendAnalysis};

#[derive(Parser)]
#[command(name = "dreamseq")]
#[command(about = "End-of-day agent reflection protocol", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the Dreamseq pipeline
    Run {
        /// Path to configuration file
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Initialize configuration
    Init,
    /// Generate report from existing anthology
    Report {
        /// Date of the anthology (YYYY-MM-DD)
        date: String,
    },
    /// Show trend analysis
    Trends {
        /// Number of days to analyze
        #[arg(short, long, default_value = "30")]
        days: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { config } => {
            let config = if let Some(config_path) = config {
                DreamseqConfig::load_from_path(std::path::Path::new(&config_path))?
            } else {
                DreamseqConfig::load()?
            };

            let dreamseq = Dreamseq::new(config)?;
            let mut anthology = dreamseq.run().await?;

            anthology.generate()?;
            let saved_path = anthology.save()?;
            let dreams_paths = save_dreams_to_speck_projects(&anthology)?;

            println!("Dreamseq analysis complete!");
            println!("Anthology saved to: {:?}", saved_path);
            println!(
                "Dreams handoff saved to {} project roots:",
                dreams_paths.len()
            );
            for path in &dreams_paths {
                println!("  - {:?}", path);
            }
            println!("\nExecutive Summary:\n{}", anthology.executive_summary);
            println!(
                "\nEvidence\n  Raw entries: {}\n  Unique events: {}\n  Segments: {}\n  Estimated input tokens: {}\n  Steering events: {}",
                anthology.pipeline.raw_entries,
                anthology.pipeline.normalized_entries,
                anthology.pipeline.segments,
                anthology.pipeline.estimated_input_tokens,
                anthology.steering_events.len()
            );
            println!(
                "\nCandidate interventions: {}",
                anthology.candidate_tools.len()
            );

            for tool in &anthology.candidate_tools {
                println!(
                    "\n  [{}] {}\n      Action: {}\n      Why: {}\n      Mutation fitness: {:.0}%\n      Existing capability overlap: {:.0}%\n      Implementation cost: {}\n      Expected value: {}\n      Confidence: {:.0}%\n      Evidence/context: {}",
                    match tool.priority {
                        Priority::High => "HIGH",
                        Priority::Medium => "MED",
                        Priority::Low => "LOW",
                    },
                    tool.name,
                    if tool.existing_matches.is_empty() {
                        "create new capability".to_string()
                    } else {
                        format!("extend {}", tool.existing_matches.join(", "))
                    },
                    tool.reason,
                    tool.mutation_fitness * 100.0,
                    tool.capability_overlap * 100.0,
                    tool.implementation_cost,
                    tool.estimated_time_saved,
                    tool.confidence * 100.0,
                    if !tool.existing_matches.is_empty() {
                        format!(
                            "existing projects to extend: {}",
                            tool.existing_matches.join(", ")
                        )
                    } else if tool.affected_projects.is_empty() {
                        "not specified".to_string()
                    } else {
                        tool.affected_projects.join(", ")
                    }
                );
            }
        }
        Commands::Init => {
            let config = DreamseqConfig::discover();
            config.save()?;
            println!("Configuration initialized at {:?}", DreamseqConfig::path()?);
            println!("Discovered {} harness log sources.", config.harnesses.len());
            println!("Add a Groq API key if you want remote semantic analysis.");
        }
        Commands::Report { date } => {
            let config = DreamseqConfig::load()?;
            let anthology_path = config
                .anthologies_dir
                .join(format!("dreamseq-{}.json", date));

            if !anthology_path.exists() {
                anyhow::bail!("Anthology not found for date: {}", date);
            }

            let content = std::fs::read_to_string(&anthology_path)?;
            let anthology: Anthology = serde_json::from_str(&content)?;

            print_anthology(&anthology);
        }
        Commands::Trends { days } => {
            let config = DreamseqConfig::load()?;
            let analyzer = dreamseq::TrendAnalyzer::with_directory(config.anthologies_dir.clone());

            // Load the most recent anthology
            let mut most_recent = None;
            let mut most_recent_date = None;

            if config.anthologies_dir.exists() {
                for entry in std::fs::read_dir(&config.anthologies_dir)? {
                    let entry = entry?;
                    let path = entry.path();

                    if path.extension().is_some_and(|ext| ext == "json")
                        && let Ok(content) = std::fs::read_to_string(&path)
                        && let Ok(anthology) = serde_json::from_str::<Anthology>(&content)
                        && most_recent_date.is_none_or(|date| anthology.generated_at > date)
                    {
                        most_recent_date = Some(anthology.generated_at);
                        most_recent = Some(anthology);
                    }
                }
            }

            if let Some(anthology) = most_recent {
                let trends = analyzer.analyze_for_days(&anthology, days).await?;
                print_trends(&trends);
            } else {
                println!("No anthologies found. Run dreamseq run first.");
            }
        }
    }

    Ok(())
}

fn save_dreams_to_speck_projects(anthology: &Anthology) -> Result<Vec<std::path::PathBuf>> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let speck_path = home.join(".speckrc");
    let content = std::fs::read_to_string(&speck_path)
        .map_err(|error| anyhow::anyhow!("Cannot read {}: {}", speck_path.display(), error))?;
    let document: toml::Value = toml::from_str(&content)?;
    let mut roots = Vec::new();
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
    if roots.is_empty() {
        roots.push(std::env::current_dir()?);
    }
    roots.sort();
    roots
        .into_iter()
        .map(|root| anthology.save_dreams(&root))
        .collect()
}

fn print_anthology(anthology: &Anthology) {
    println!("# Dreamseq Anthology");
    println!("Date: {}", anthology.date);
    println!(
        "Generated: {}",
        anthology.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!();

    println!("## Executive Summary");
    println!("{}", anthology.executive_summary);
    println!();

    println!("## Significant Milestones");
    for milestone in &anthology.significant_milestones {
        println!("- {}", milestone);
    }
    println!();

    println!("## User Behaviour");
    println!("### Repeated Git Workflows");
    for workflow in &anthology.user_behaviour.repeated_git_workflows {
        println!("- {}", workflow);
    }
    println!("### Repeated Package Installs");
    for install in &anthology.user_behaviour.repeated_package_installs {
        println!("- {}", install);
    }
    println!("### Repeated File Navigation");
    for nav in &anthology.user_behaviour.repeated_file_navigation {
        println!("- {}", nav);
    }
    println!();

    println!("## Model Weaknesses");
    for weakness in &anthology.model_weaknesses {
        println!(
            "- {}: {} (frequency: {})",
            weakness.model, weakness.weakness, weakness.frequency
        );
    }
    println!();

    println!("## Harness Weaknesses");
    for weakness in &anthology.harness_weaknesses {
        println!(
            "- {}: {} (severity: {:.2})",
            weakness.harness, weakness.weakness, weakness.severity
        );
    }
    println!();

    println!("## Candidate Tools");
    for tool in &anthology.candidate_tools {
        println!("### {} ({})", tool.name, tool.id);
        println!("Priority: {:?}", tool.priority);
        println!("Reason: {}", tool.reason);
        println!("Estimated Time Saved: {}", tool.estimated_time_saved);
        println!("Confidence: {:.2}", tool.confidence);
        println!();
    }

    println!("## Steering Events");
    println!("Total: {}", anthology.steering_events.len());
    for event in &anthology.steering_events {
        println!(
            "- [{:?}] {} (severity: {:.2})",
            event.category, event.description, event.severity
        );
    }

    if let Some(trends) = &anthology.trends {
        println!();
        println!("## Trend Analysis");
        print_trends(trends);
    }
}

fn print_trends(trends: &TrendAnalysis) {
    println!("Period: {}", trends.period);
    println!();

    for trend_data in trends.trends.values() {
        println!("### {}", trend_data.metric_name);
        println!("Current: {:.2}", trend_data.current_value);
        println!("Previous: {:.2}", trend_data.previous_value);
        println!("Direction: {:?}", trend_data.trend_direction);
        println!();
        println!("{}", trend_data.visualization);
        println!();
    }
}
