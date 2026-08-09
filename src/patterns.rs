use crate::groq::Analysis;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    pub pattern_type: PatternType,
    pub description: String,
    pub frequency: usize,
    pub confidence: f64,
    pub impact_score: f64,
    pub affected_harnesses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternType {
    ModelFailure,
    HarnessFriction,
    MissingTool,
    WorkflowBottleneck,
    RepeatedCommand,
    RepeatedPrompt,
    ContextLoss,
    AutomationOpportunity,
}

pub struct PatternExtractor;

impl Default for PatternExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternExtractor {
    pub fn new() -> Self {
        Self
    }

    pub fn extract(&self, analysis: &Analysis) -> Result<Vec<Pattern>> {
        let mut patterns = Vec::new();

        // Extract model failure patterns
        for failure in &analysis.model_failures {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::ModelFailure,
                description: format!("{}: {}", failure.model, failure.issue),
                frequency: failure.frequency,
                confidence: 0.8,
                impact_score: self.calculate_impact_score(failure.frequency as f64, 0.7),
                affected_harnesses: vec![failure.model.clone()],
            });
        }

        // Extract harness friction patterns
        for friction in &analysis.harness_friction {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::HarnessFriction,
                description: format!("{} friction: {}", friction.harness, friction.issue),
                frequency: 1,
                confidence: friction.severity,
                impact_score: friction.severity,
                affected_harnesses: vec![friction.harness.clone()],
            });
        }

        // Extract missing tool patterns
        for tool in &analysis.missing_tooling {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::MissingTool,
                description: format!("Missing tool: {} - {}", tool.tool_name, tool.purpose),
                frequency: 1,
                confidence: 0.9,
                impact_score: tool.estimated_value,
                affected_harnesses: vec![],
            });
        }

        // Extract workflow bottleneck patterns
        for bottleneck in &analysis.workflow_bottlenecks {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::WorkflowBottleneck,
                description: bottleneck.description.clone(),
                frequency: bottleneck.frequency,
                confidence: 0.85,
                impact_score: self.calculate_impact_score(
                    bottleneck.frequency as f64,
                    bottleneck.time_impact_minutes / 60.0,
                ),
                affected_harnesses: vec![],
            });
        }

        // Extract repeated command patterns
        for command in &analysis.repeated_commands {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::RepeatedCommand,
                description: format!("Repeated command: {}", command.command),
                frequency: command.frequency,
                confidence: 0.95,
                impact_score: self.calculate_impact_score(command.frequency as f64, 0.5),
                affected_harnesses: vec![command.context.clone()],
            });
        }

        // Extract repeated prompt patterns
        for prompt in &analysis.repeated_prompts {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::RepeatedPrompt,
                description: format!("Repeated prompt pattern: {}", prompt.prompt_pattern),
                frequency: prompt.frequency,
                confidence: 0.9,
                impact_score: self.calculate_impact_score(prompt.frequency as f64, 0.3),
                affected_harnesses: vec![],
            });
        }

        // Extract context loss patterns
        for loss in &analysis.context_loss {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::ContextLoss,
                description: loss.description.clone(),
                frequency: loss.affected_segments.len(),
                confidence: 0.75,
                impact_score: self.calculate_impact_score(loss.affected_segments.len() as f64, 0.8),
                affected_harnesses: loss.affected_segments.clone(),
            });
        }

        // Extract automation opportunities
        for opportunity in &analysis.automation_opportunities {
            patterns.push(Pattern {
                id: uuid::Uuid::new_v4().to_string(),
                pattern_type: PatternType::AutomationOpportunity,
                description: opportunity.description.clone(),
                frequency: 1,
                confidence: opportunity.confidence,
                impact_score: self
                    .calculate_impact_score(1.0, opportunity.estimated_time_saved / 60.0),
                affected_harnesses: vec![],
            });
        }

        // Sort by impact score
        patterns.sort_by(|a, b| b.impact_score.total_cmp(&a.impact_score));

        Ok(patterns)
    }

    fn calculate_impact_score(&self, frequency: f64, time_hours: f64) -> f64 {
        // Impact score combines frequency and time impact
        let frequency_score = (frequency.min(100.0) / 100.0) * 0.5;
        let time_score = (time_hours.min(10.0) / 10.0) * 0.5;
        frequency_score + time_score
    }
}
