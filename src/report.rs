use crate::config::DreamseqConfig;
use crate::patterns::Pattern;
use crate::steering::SteeringEvent;
use crate::trends::TrendAnalysis;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anthology {
    pub id: String,
    pub generated_at: DateTime<Utc>,
    pub date: String,
    pub executive_summary: String,
    pub significant_milestones: Vec<String>,
    pub user_behaviour: UserBehaviour,
    pub model_weaknesses: Vec<ModelWeakness>,
    pub harness_weaknesses: Vec<HarnessWeakness>,
    pub candidate_tools: Vec<CandidateTool>,
    pub steering_events: Vec<SteeringEvent>,
    pub patterns: Vec<Pattern>,
    pub trends: Option<TrendAnalysis>,
    pub config: DreamseqConfig,
    #[serde(default)]
    pub pipeline: PipelineStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineStats {
    pub raw_entries: usize,
    pub normalized_entries: usize,
    pub segments: usize,
    pub estimated_input_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBehaviour {
    pub repeated_git_workflows: Vec<String>,
    pub repeated_package_installs: Vec<String>,
    pub repeated_file_navigation: Vec<String>,
    pub other_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelWeakness {
    pub model: String,
    pub weakness: String,
    pub frequency: usize,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessWeakness {
    pub harness: String,
    pub weakness: String,
    pub severity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateTool {
    pub id: String,
    pub name: String,
    pub priority: Priority,
    #[serde(default)]
    pub category: InterventionCategory,
    pub reason: String,
    pub estimated_time_saved: String,
    pub confidence: f64,
    pub affected_projects: Vec<String>,
    #[serde(default)]
    pub existing_matches: Vec<String>,
    #[serde(default)]
    pub mutation_fitness: f64,
    #[serde(default)]
    pub capability_overlap: f64,
    #[serde(default)]
    pub implementation_cost: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Priority {
    High,
    Medium,
    Low,
}

/// High-level category used by the human-facing renderer to group
/// candidate interventions. Kept intentionally coarse so the report stays
/// scannable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InterventionCategory {
    MissingCapability,
    WorkflowAcceleration,
    PackageManagerFriction,
    ModelReliability,
    ContextManagement,
    #[default]
    Other,
}

impl std::fmt::Display for InterventionCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterventionCategory::MissingCapability => write!(f, "Missing capability"),
            InterventionCategory::WorkflowAcceleration => write!(f, "Workflow acceleration"),
            InterventionCategory::PackageManagerFriction => write!(f, "Package-manager friction"),
            InterventionCategory::ModelReliability => write!(f, "Model reliability"),
            InterventionCategory::ContextManagement => write!(f, "Context management"),
            InterventionCategory::Other => write!(f, "Other friction"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directive {
    pub id: String,
    pub title: String,
    pub frequency: usize,
    pub estimated_time_saved: String,
    pub confidence: f64,
    pub automation_score: f64,
    pub implementation_effort: ImplementationEffort,
    pub affected_projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImplementationEffort {
    Low,
    Medium,
    High,
}

impl Anthology {
    pub fn new(
        patterns: Vec<Pattern>,
        steering_events: Vec<SteeringEvent>,
        config: DreamseqConfig,
    ) -> Self {
        let now = Utc::now();
        let date = now.format("%Y-%m-%d").to_string();

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            generated_at: now,
            date: date.clone(),
            executive_summary: String::new(),
            significant_milestones: Vec::new(),
            user_behaviour: UserBehaviour {
                repeated_git_workflows: Vec::new(),
                repeated_package_installs: Vec::new(),
                repeated_file_navigation: Vec::new(),
                other_patterns: Vec::new(),
            },
            model_weaknesses: Vec::new(),
            harness_weaknesses: Vec::new(),
            candidate_tools: Vec::new(),
            steering_events,
            patterns,
            trends: None,
            config,
            pipeline: PipelineStats::default(),
        }
    }

    pub fn set_pipeline_stats(&mut self, stats: PipelineStats) {
        self.pipeline = stats;
    }

    pub fn add_trends(&mut self, trends: TrendAnalysis) {
        self.trends = Some(trends);
    }

    pub fn generate(&mut self) -> Result<()> {
        self.generate_executive_summary();
        self.extract_milestones();
        self.analyze_user_behaviour();
        self.extract_model_weaknesses();
        self.extract_harness_weaknesses();
        self.generate_candidate_tools();
        Ok(())
    }

    fn generate_executive_summary(&mut self) {
        let pattern_count = self.patterns.len();
        let steering_count = self.steering_events.len();
        let high_impact = self
            .patterns
            .iter()
            .filter(|p| p.impact_score > 0.7)
            .count();

        self.executive_summary = format!(
            "{}: reviewed {} raw entries and retained {} normalized events across {} segments (~{} estimated input tokens). Found {} patterns and {} human steering events; {} high-impact opportunities require follow-up.",
            self.date,
            self.pipeline.raw_entries,
            self.pipeline.normalized_entries,
            self.pipeline.segments,
            self.pipeline.estimated_input_tokens,
            pattern_count,
            steering_count,
            high_impact
        );
    }

    fn extract_milestones(&mut self) {
        // Extract significant events from patterns and steering
        for pattern in &self.patterns {
            if pattern.impact_score > 0.8 {
                self.significant_milestones.push(format!(
                    "High-impact pattern identified: {} (score: {:.2})",
                    pattern.description, pattern.impact_score
                ));
            }
        }

        for event in &self.steering_events {
            if event.severity > 0.8 {
                self.significant_milestones.push(format!(
                    "Critical steering event: {:?} - {}",
                    event.category, event.description
                ));
            }
        }
    }

    fn analyze_user_behaviour(&mut self) {
        // Extract user behavior patterns from steering events and patterns
        for pattern in &self.patterns {
            match pattern.pattern_type {
                crate::patterns::PatternType::RepeatedCommand => {
                    if pattern.description.contains("git") {
                        self.user_behaviour
                            .repeated_git_workflows
                            .push(pattern.description.clone());
                    }
                }
                crate::patterns::PatternType::MissingTool
                    if pattern.description.contains("install")
                        || pattern.description.contains("package") =>
                {
                    self.user_behaviour
                        .repeated_package_installs
                        .push(pattern.description.clone());
                }
                _ => {}
            }
        }

        for event in &self.steering_events {
            if event.context.contains("cd")
                || event.context.contains("ls")
                || event.context.contains("find")
            {
                self.user_behaviour
                    .repeated_file_navigation
                    .push(event.context.clone());
            }
        }
    }

    fn extract_model_weaknesses(&mut self) {
        let mut model_weakness_map: HashMap<String, Vec<String>> = HashMap::new();

        for pattern in &self.patterns {
            if let crate::patterns::PatternType::ModelFailure = pattern.pattern_type {
                model_weakness_map
                    .entry(
                        pattern
                            .affected_harnesses
                            .first()
                            .unwrap_or(&"unknown".to_string())
                            .clone(),
                    )
                    .or_default()
                    .push(pattern.description.clone());
            }
        }

        for (model, weaknesses) in model_weakness_map {
            self.model_weaknesses.push(ModelWeakness {
                model: model.clone(),
                weakness: weaknesses.join("; "),
                frequency: weaknesses.len(),
                examples: weaknesses,
            });
        }
    }

    fn extract_harness_weaknesses(&mut self) {
        let mut harness_weakness_map: HashMap<String, Vec<f64>> = HashMap::new();

        for pattern in &self.patterns {
            if let crate::patterns::PatternType::HarnessFriction = pattern.pattern_type {
                for harness in &pattern.affected_harnesses {
                    harness_weakness_map
                        .entry(harness.clone())
                        .or_default()
                        .push(pattern.impact_score);
                }
            }
        }

        for (harness, severities) in harness_weakness_map {
            let avg_severity: f64 = severities.iter().sum::<f64>() / severities.len() as f64;
            self.harness_weaknesses.push(HarnessWeakness {
                harness,
                weakness: "General friction detected".to_string(),
                severity: avg_severity,
            });
        }
    }

    fn generate_candidate_tools(&mut self) {
        let mut tool_id = 0;

        for pattern in &self.patterns {
            if pattern.impact_score > 0.6 {
                tool_id += 1;
                let priority = if pattern.impact_score > 0.8 {
                    Priority::High
                } else if pattern.impact_score > 0.7 {
                    Priority::Medium
                } else {
                    Priority::Low
                };

                let name = self.suggest_tool_name(&pattern.description);
                let existing_matches = self.find_existing_matches(&name);
                let category = category_from_pattern_type(&pattern.pattern_type, &name);
                self.candidate_tools.push(CandidateTool {
                    id: format!("DS-{:04}", tool_id),
                    name: name.clone(),
                    priority,
                    category,
                    reason: truncate_rationale(&pattern.description, 240),
                    estimated_time_saved: format!(
                        "{} min/day",
                        (pattern.impact_score * 20.0) as i32
                    ),
                    confidence: pattern.confidence,
                    affected_projects: pattern.affected_harnesses.clone(),
                    existing_matches: existing_matches.clone(),
                    mutation_fitness: pattern.confidence * 0.7 + pattern.impact_score * 0.3,
                    capability_overlap: (existing_matches.len() as f64 / 3.0).min(1.0),
                    implementation_cost: if existing_matches.is_empty() {
                        "high".into()
                    } else {
                        "medium".into()
                    },
                });
            }
        }

        // Cluster human interventions so a single model suggestion cannot
        // hide the broader workflow pressure represented by steering events.
        let mut clusters: BTreeMap<crate::steering::SteeringCategory, Vec<&SteeringEvent>> =
            BTreeMap::new();
        for event in &self.steering_events {
            clusters.entry(event.category).or_default().push(event);
        }
        for (category, events) in clusters {
            if events.len() < 2 {
                continue;
            }
            let name = match category {
                crate::steering::SteeringCategory::MissingTool => "workflow-acceleration",
                crate::steering::SteeringCategory::MissingContext => "context-manager",
                crate::steering::SteeringCategory::WrongAbstraction => "requirement-guard",
                crate::steering::SteeringCategory::ExcessVerbosity => "response-compressor",
                crate::steering::SteeringCategory::Hallucination => "verification-gateway",
                crate::steering::SteeringCategory::ArchitecturalMismatch => "architecture-reviewer",
                crate::steering::SteeringCategory::ManualRepetition => "workflow-automator",
                crate::steering::SteeringCategory::Other => "workflow-friction-reducer",
            };
            if self.candidate_tools.iter().any(|tool| tool.name == name) {
                continue;
            }
            let average_severity =
                events.iter().map(|event| event.severity).sum::<f64>() / events.len() as f64;
            let existing_matches = self.find_existing_matches(name);
            let overlap = (existing_matches.len() as f64 / 3.0).min(1.0);
            let fitness = (average_severity * 0.55
                + (events.len() as f64 / 100.0).min(1.0) * 0.25
                + overlap * 0.2)
                .min(1.0);
            self.candidate_tools.push(CandidateTool {
                id: format!("DS-{:04}", self.candidate_tools.len() + 1),
                name: name.to_string(),
                priority: if events.len() >= 10 {
                    Priority::High
                } else {
                    Priority::Medium
                },
                category: category_from_steering_category(&category, name),
                reason: format!(
                    "{} steering events clustered as {:?}",
                    events.len(),
                    category
                ),
                estimated_time_saved: format!(
                    "{} min/day",
                    (average_severity * events.len() as f64 * 2.0) as i32
                ),
                confidence: (average_severity + (events.len() as f64 / 20.0).min(1.0)) / 2.0,
                affected_projects: Vec::new(),
                existing_matches,
                mutation_fitness: fitness,
                capability_overlap: overlap,
                implementation_cost: if overlap >= 0.34 {
                    "medium".into()
                } else {
                    "high".into()
                },
            });
        }
        self.candidate_tools
            .sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    }

    fn find_existing_matches(&self, intervention: &str) -> Vec<String> {
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        let Ok(content) = fs::read_to_string(home.join(".speckrc")) else {
            return Vec::new();
        };
        let Ok(document) = content.parse::<toml::Value>() else {
            tracing::warn!("could not parse ~/.speckrc while matching existing capabilities");
            return Vec::new();
        };
        let Some(tools) = document.get("tools").and_then(toml::Value::as_table) else {
            return Vec::new();
        };
        let keywords: Vec<String> = intervention
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| {
                word.len() >= 4
                    && !matches!(
                        word.to_ascii_lowercase().as_str(),
                        "tool" | "manager" | "assistant" | "helper"
                    )
            })
            .map(str::to_lowercase)
            .collect();
        let mut matches = Vec::new();
        for (name, tool) in tools {
            let Some(path) = tool.get("path").and_then(toml::Value::as_str) else {
                continue;
            };
            let root = PathBuf::from(path);
            if !root.is_dir() {
                continue;
            }
            let mut searchable = name.to_lowercase();
            for relative in [".speck/manifest.toml", "README.md"] {
                if let Ok(text) = fs::read_to_string(root.join(relative)) {
                    searchable.push(' ');
                    searchable.push_str(&text.to_lowercase());
                }
            }
            if !keywords.is_empty() && keywords.iter().any(|keyword| searchable.contains(keyword)) {
                matches.push(name.clone());
            }
        }
        matches.sort();
        matches.truncate(5);
        matches
    }

    fn suggest_tool_name(&self, description: &str) -> String {
        // Generate a tool name based on the description
        let desc_lower = description.to_lowercase();

        if desc_lower.contains("git") {
            "git-assistant".to_string()
        } else if desc_lower.contains("package") || desc_lower.contains("install") {
            "package-manager".to_string()
        } else if desc_lower.contains("search") || desc_lower.contains("find") {
            "smart-search".to_string()
        } else if desc_lower.contains("context") {
            "context-manager".to_string()
        } else {
            format!(
                "tool-{}",
                description.split_whitespace().next().unwrap_or("helper")
            )
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.config.anthologies_dir)?;

        let filename = format!("dreamseq-{}-{}.json", self.date, self.id);
        let filepath = self.config.anthologies_dir.join(filename);

        let content = serde_json::to_string_pretty(self)?;
        write_private(&filepath, content.as_bytes())?;
        set_private_permissions(&filepath)?;

        tracing::info!("Saved anthology to {:?}", filepath);
        Ok(filepath)
    }

    /// Write a concise, agent-consumable backlog of unresolved interventions.
    pub fn save_dreams(&self, repository: &std::path::Path) -> Result<PathBuf> {
        let dreams_dir = repository.join(".dreams");
        fs::create_dir_all(&dreams_dir)?;
        fs::create_dir_all(dreams_dir.join("history"))?;
        for lifecycle in ["completed.dreams", "rejected.dreams"] {
            let lifecycle_path = dreams_dir.join(lifecycle);
            if !lifecycle_path.exists() {
                fs::write(&lifecycle_path, "version: 1\ndreams: []\n")?;
            }
        }
        let path = dreams_dir.join("active.dreams");
        let mut output = String::new();
        let project = repository
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace");
        output.push_str("version: 1\n");
        output.push_str(&format!("project: {}\n", yaml_scalar(project)));
        output.push_str("generated:\n  by: dreamseq\n");
        output.push_str(&format!(
            "  date: {}\n  confidence: {:.2}\n",
            self.date,
            self.average_confidence()
        ));
        output.push_str("source:\n");
        output.push_str(&format!("    logs: {}\n", self.pipeline.raw_entries));
        output.push_str(&format!(
            "    normalized_events: {}\n",
            self.pipeline.normalized_entries
        ));
        output.push_str(&format!("    segments: {}\n", self.pipeline.segments));
        output.push_str(&format!(
            "    steering_events: {}\n",
            self.steering_events.len()
        ));
        output.push_str("\ndreams:\n");
        for (index, tool) in self.candidate_tools.iter().enumerate() {
            let priority = match tool.priority {
                Priority::High => "HIGH",
                Priority::Medium => "MEDIUM",
                Priority::Low => "LOW",
            };
            let action = if tool.existing_matches.is_empty() {
                "create capability"
            } else {
                "extend existing capability"
            };
            output.push_str(&format!(
                "\n  - id: dream-{:03}\n    priority: {}\n    category: workflow-friction\n    title: {}\n    problem: {}\n    evidence:\n      reason: {}\n      confidence: {:.2}\n      mutation_fitness: {:.2}\n    affected:\n{}    proposed_mutation:\n      action: {}\n      capability: {}\n    acceptance:\n      - Evidence-backed intervention reduces recurrence of this friction\n      - Outcome is verified by tests or deterministic operational checks\n    expected_value: {}\n",
                index + 1, priority, yaml_scalar(&tool.name), yaml_scalar(&tool.reason),
                yaml_scalar(&tool.reason), tool.confidence, tool.mutation_fitness,
                if tool.existing_matches.is_empty() { "      - none identified\n".to_string() } else { tool.existing_matches.iter().map(|name| format!("      - {}\n", yaml_scalar(name))).collect() },
                action, yaml_scalar(&tool.name), yaml_scalar(&tool.estimated_time_saved)
            ));
        }
        fs::write(&path, &output)?;
        fs::write(
            dreams_dir
                .join("history")
                .join(format!("{}-{}.dreams", self.date, self.id)),
            &output,
        )?;
        Ok(path)
    }

    fn average_confidence(&self) -> f64 {
        if self.candidate_tools.is_empty() {
            return 0.0;
        }
        self.candidate_tools
            .iter()
            .map(|tool| tool.confidence)
            .sum::<f64>()
            / self.candidate_tools.len() as f64
    }

    pub fn generate_directives(&self) -> Vec<Directive> {
        let mut directives = Vec::new();

        for tool in &self.candidate_tools {
            directives.push(Directive {
                id: tool.id.clone(),
                title: tool.name.clone(),
                frequency: tool.affected_projects.len(),
                estimated_time_saved: tool.estimated_time_saved.clone(),
                confidence: tool.confidence,
                automation_score: tool.confidence, // Simplified
                implementation_effort: match tool.priority {
                    Priority::High => ImplementationEffort::Medium,
                    Priority::Medium => ImplementationEffort::Low,
                    Priority::Low => ImplementationEffort::Low,
                },
                affected_projects: tool.affected_projects.clone(),
            });
        }

        directives.sort_by(|a, b| b.automation_score.total_cmp(&a.automation_score));

        directives
    }
}

fn yaml_scalar(value: &str) -> String {
    // Escape YAML double-quoted scalar per spec: backslash, quote, control chars.
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn category_from_pattern_type(
    pattern_type: &crate::patterns::PatternType,
    name: &str,
) -> InterventionCategory {
    if name.contains("package") {
        return InterventionCategory::PackageManagerFriction;
    }
    match pattern_type {
        crate::patterns::PatternType::MissingTool
        | crate::patterns::PatternType::AutomationOpportunity => {
            InterventionCategory::MissingCapability
        }
        crate::patterns::PatternType::WorkflowBottleneck
        | crate::patterns::PatternType::RepeatedCommand => {
            InterventionCategory::WorkflowAcceleration
        }
        crate::patterns::PatternType::RepeatedPrompt
        | crate::patterns::PatternType::ContextLoss => InterventionCategory::ContextManagement,
        crate::patterns::PatternType::ModelFailure => InterventionCategory::ModelReliability,
        crate::patterns::PatternType::HarnessFriction => InterventionCategory::Other,
    }
}

fn category_from_steering_category(
    category: &crate::steering::SteeringCategory,
    name: &str,
) -> InterventionCategory {
    if name.contains("package") {
        return InterventionCategory::PackageManagerFriction;
    }
    match category {
        crate::steering::SteeringCategory::MissingTool => InterventionCategory::MissingCapability,
        crate::steering::SteeringCategory::MissingContext => {
            InterventionCategory::ContextManagement
        }
        crate::steering::SteeringCategory::Hallucination => InterventionCategory::ModelReliability,
        crate::steering::SteeringCategory::ManualRepetition => {
            InterventionCategory::WorkflowAcceleration
        }
        crate::steering::SteeringCategory::WrongAbstraction
        | crate::steering::SteeringCategory::ExcessVerbosity
        | crate::steering::SteeringCategory::ArchitecturalMismatch
        | crate::steering::SteeringCategory::Other => InterventionCategory::Other,
    }
}

fn truncate_rationale(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        value.to_string()
    } else {
        format!("{}…", &value[..max_len.saturating_sub(1)])
    }
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}
