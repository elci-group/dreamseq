use crate::config::DreamseqConfig;
use crate::patterns::Pattern;
use crate::steering::SteeringEvent;
use crate::trends::TrendAnalysis;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
            Self::MissingCapability => write!(f, "Missing capability"),
            Self::WorkflowAcceleration => write!(f, "Workflow acceleration"),
            Self::PackageManagerFriction => write!(f, "Package-manager friction"),
            Self::ModelReliability => write!(f, "Model reliability"),
            Self::ContextManagement => write!(f, "Context management"),
            Self::Other => write!(f, "Other friction"),
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
