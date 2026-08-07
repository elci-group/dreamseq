use crate::report::Anthology;
use anyhow::Result;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysis {
    pub period: String,
    pub trends: HashMap<String, TrendData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendData {
    pub metric_name: String,
    pub current_value: f64,
    pub previous_value: f64,
    pub trend_direction: TrendDirection,
    pub visualization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    Increasing,
    Decreasing,
    Stable,
}

pub struct TrendAnalyzer {
    anthologies_dir: PathBuf,
}

impl Default for TrendAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl TrendAnalyzer {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            anthologies_dir: home.join("dreamseq").join("anthologies"),
        }
    }

    pub fn with_directory(anthologies_dir: PathBuf) -> Self {
        Self { anthologies_dir }
    }

    pub async fn analyze(&self, current_anthology: &Anthology) -> Result<TrendAnalysis> {
        self.analyze_for_days(current_anthology, 30).await
    }

    pub async fn analyze_for_days(
        &self,
        current_anthology: &Anthology,
        days: i64,
    ) -> Result<TrendAnalysis> {
        let previous_anthologies = self.load_previous_anthologies(days.max(1))?;

        let mut trends = HashMap::new();

        // Analyze context-loss trends
        if let Some(context_trend) =
            self.analyze_context_loss(current_anthology, &previous_anthologies)
        {
            trends.insert("context_loss".to_string(), context_trend);
        }

        // Analyze git workflow friction
        if let Some(git_trend) = self.analyze_git_friction(current_anthology, &previous_anthologies)
        {
            trends.insert("git_friction".to_string(), git_trend);
        }

        // Analyze documentation friction
        if let Some(doc_trend) =
            self.analyze_documentation_friction(current_anthology, &previous_anthologies)
        {
            trends.insert("documentation_friction".to_string(), doc_trend);
        }

        // Analyze prompt repetition
        if let Some(prompt_trend) =
            self.analyze_prompt_repetition(current_anthology, &previous_anthologies)
        {
            trends.insert("prompt_repetition".to_string(), prompt_trend);
        }

        // Analyze repeated shell commands
        if let Some(command_trend) =
            self.analyze_command_repetition(current_anthology, &previous_anthologies)
        {
            trends.insert("command_repetition".to_string(), command_trend);
        }

        Ok(TrendAnalysis {
            period: format!("Past {} days", days.max(1)),
            trends,
        })
    }

    fn load_previous_anthologies(&self, days: i64) -> Result<Vec<Anthology>> {
        let mut anthologies = Vec::new();
        let cutoff_date = Utc::now() - Duration::days(days);

        if !self.anthologies_dir.exists() {
            return Ok(anthologies);
        }

        for entry in fs::read_dir(&self.anthologies_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|ext| ext == "json")
                && let Ok(content) = fs::read_to_string(&path)
                && let Ok(anthology) = serde_json::from_str::<Anthology>(&content)
                && anthology.generated_at > cutoff_date
            {
                anthologies.push(anthology);
            }
        }

        anthologies.sort_by_key(|a| a.generated_at);
        Ok(anthologies)
    }

    fn analyze_context_loss(
        &self,
        current: &Anthology,
        previous: &[Anthology],
    ) -> Option<TrendData> {
        let current_count = current
            .steering_events
            .iter()
            .filter(|e| {
                matches!(
                    e.category,
                    crate::steering::SteeringCategory::MissingContext
                )
            })
            .count() as f64;

        let previous_count = if previous.is_empty() {
            0.0
        } else {
            previous
                .iter()
                .map(|a| {
                    a.steering_events
                        .iter()
                        .filter(|e| {
                            matches!(
                                e.category,
                                crate::steering::SteeringCategory::MissingContext
                            )
                        })
                        .count() as f64
                })
                .sum::<f64>()
                / previous.len() as f64
        };

        Some(self.create_trend_data("Context-loss issues", current_count, previous_count))
    }

    fn analyze_git_friction(
        &self,
        current: &Anthology,
        previous: &[Anthology],
    ) -> Option<TrendData> {
        let current_count = current.user_behaviour.repeated_git_workflows.len() as f64;

        let previous_count = if previous.is_empty() {
            0.0
        } else {
            previous
                .iter()
                .map(|a| a.user_behaviour.repeated_git_workflows.len() as f64)
                .sum::<f64>()
                / previous.len() as f64
        };

        Some(self.create_trend_data("Git workflow friction", current_count, previous_count))
    }

    fn analyze_documentation_friction(
        &self,
        current: &Anthology,
        previous: &[Anthology],
    ) -> Option<TrendData> {
        let current_count = current
            .patterns
            .iter()
            .filter(|p| p.description.to_lowercase().contains("document"))
            .count() as f64;

        let previous_count = if previous.is_empty() {
            0.0
        } else {
            previous
                .iter()
                .map(|a| {
                    a.patterns
                        .iter()
                        .filter(|p| p.description.to_lowercase().contains("document"))
                        .count() as f64
                })
                .sum::<f64>()
                / previous.len() as f64
        };

        Some(self.create_trend_data("Documentation friction", current_count, previous_count))
    }

    fn analyze_prompt_repetition(
        &self,
        current: &Anthology,
        previous: &[Anthology],
    ) -> Option<TrendData> {
        let current_count = current
            .patterns
            .iter()
            .filter(|p| matches!(p.pattern_type, crate::patterns::PatternType::RepeatedPrompt))
            .count() as f64;

        let previous_count = if previous.is_empty() {
            0.0
        } else {
            previous
                .iter()
                .map(|a| {
                    a.patterns
                        .iter()
                        .filter(|p| {
                            matches!(p.pattern_type, crate::patterns::PatternType::RepeatedPrompt)
                        })
                        .count() as f64
                })
                .sum::<f64>()
                / previous.len() as f64
        };

        Some(self.create_trend_data("Prompt repetition", current_count, previous_count))
    }

    fn analyze_command_repetition(
        &self,
        current: &Anthology,
        previous: &[Anthology],
    ) -> Option<TrendData> {
        let current_count = current
            .patterns
            .iter()
            .filter(|p| {
                matches!(
                    p.pattern_type,
                    crate::patterns::PatternType::RepeatedCommand
                )
            })
            .count() as f64;

        let previous_count = if previous.is_empty() {
            0.0
        } else {
            previous
                .iter()
                .map(|a| {
                    a.patterns
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.pattern_type,
                                crate::patterns::PatternType::RepeatedCommand
                            )
                        })
                        .count() as f64
                })
                .sum::<f64>()
                / previous.len() as f64
        };

        Some(self.create_trend_data("Repeated shell commands", current_count, previous_count))
    }

    fn create_trend_data(
        &self,
        metric_name: &str,
        current_value: f64,
        previous_value: f64,
    ) -> TrendData {
        let trend_direction = if current_value > previous_value * 1.1 {
            TrendDirection::Increasing
        } else if current_value < previous_value * 0.9 {
            TrendDirection::Decreasing
        } else {
            TrendDirection::Stable
        };

        let visualization = self.create_visualization(current_value, previous_value);

        TrendData {
            metric_name: metric_name.to_string(),
            current_value,
            previous_value,
            trend_direction,
            visualization,
        }
    }

    fn create_visualization(&self, current: f64, previous: f64) -> String {
        // Create a simple ASCII bar chart
        let max_bars = 20;
        let scale = max_bars as f64 / current.max(previous).max(1.0);

        let current_bars = (current * scale) as usize;
        let previous_bars = (previous * scale) as usize;

        format!(
            "Current: {}█\nPrevious: {}█",
            "█".repeat(current_bars),
            "█".repeat(previous_bars)
        )
    }
}
