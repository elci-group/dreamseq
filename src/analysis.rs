use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub model_failures: Vec<ModelFailure>,
    pub harness_friction: Vec<HarnessFriction>,
    pub missing_tooling: Vec<MissingTool>,
    pub workflow_bottlenecks: Vec<WorkflowBottleneck>,
    pub repeated_commands: Vec<RepeatedCommand>,
    pub repeated_prompts: Vec<RepeatedPrompt>,
    pub context_loss: Vec<ContextLoss>,
    pub automation_opportunities: Vec<AutomationOpportunity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFailure {
    pub model: String,
    pub issue: String,
    pub frequency: usize,
    pub example: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessFriction {
    pub harness: String,
    pub issue: String,
    pub severity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingTool {
    pub tool_name: String,
    pub purpose: String,
    pub estimated_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowBottleneck {
    pub description: String,
    pub frequency: usize,
    pub time_impact_minutes: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatedCommand {
    pub command: String,
    pub frequency: usize,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatedPrompt {
    pub prompt_pattern: String,
    pub frequency: usize,
    pub suggested_improvement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextLoss {
    pub description: String,
    pub affected_segments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationOpportunity {
    pub description: String,
    pub estimated_time_saved: f64,
    pub confidence: f64,
}

impl Analysis {
    pub(crate) fn merge(&mut self, other: Self) {
        merge_by(&mut self.model_failures, other.model_failures, |item| {
            (item.model.clone(), item.issue.clone())
        }, |existing, finding| {
            existing.frequency = existing.frequency.saturating_add(finding.frequency);
        });
        merge_by(&mut self.harness_friction, other.harness_friction, |item| {
            (item.harness.clone(), item.issue.clone())
        }, |existing, finding| {
            existing.severity = existing.severity.max(finding.severity);
        });
        merge_by(&mut self.missing_tooling, other.missing_tooling, |item| {
            (item.tool_name.clone(), item.purpose.clone())
        }, |existing, finding| {
            existing.estimated_value = existing.estimated_value.max(finding.estimated_value);
        });
        merge_by(&mut self.workflow_bottlenecks, other.workflow_bottlenecks, |item| {
            item.description.clone()
        }, |existing, finding| {
            existing.frequency = existing.frequency.saturating_add(finding.frequency);
            existing.time_impact_minutes = existing.time_impact_minutes.max(finding.time_impact_minutes);
        });
        merge_by(&mut self.repeated_commands, other.repeated_commands, |item| {
            item.command.clone()
        }, |existing, finding| {
            existing.frequency = existing.frequency.saturating_add(finding.frequency);
        });
        merge_by(&mut self.repeated_prompts, other.repeated_prompts, |item| {
            item.prompt_pattern.clone()
        }, |existing, finding| {
            existing.frequency = existing.frequency.saturating_add(finding.frequency);
        });
        for finding in other.context_loss {
            if let Some(existing) = self.context_loss.iter_mut().find(|item| item.description == finding.description) {
                for segment in finding.affected_segments {
                    if !existing.affected_segments.contains(&segment) {
                        existing.affected_segments.push(segment);
                    }
                }
            } else {
                self.context_loss.push(finding);
            }
        }
        merge_by(&mut self.automation_opportunities, other.automation_opportunities, |item| {
            item.description.clone()
        }, |existing, finding| {
            existing.estimated_time_saved = existing.estimated_time_saved.max(finding.estimated_time_saved);
            existing.confidence = existing.confidence.max(finding.confidence);
        });
    }

    pub(crate) fn sanitize(&mut self) {
        for friction in &mut self.harness_friction {
            friction.severity = finite_clamp(friction.severity, 0.0, 1.0);
        }
        for tool in &mut self.missing_tooling {
            tool.estimated_value = finite_clamp(tool.estimated_value, 0.0, 1.0);
        }
        for bottleneck in &mut self.workflow_bottlenecks {
            bottleneck.frequency = bottleneck.frequency.min(1_000_000);
            bottleneck.time_impact_minutes = finite_clamp(bottleneck.time_impact_minutes, 0.0, 525_600.0);
        }
        for failure in &mut self.model_failures {
            failure.frequency = failure.frequency.min(1_000_000);
        }
        for command in &mut self.repeated_commands {
            command.frequency = command.frequency.min(1_000_000);
        }
        for prompt in &mut self.repeated_prompts {
            prompt.frequency = prompt.frequency.min(1_000_000);
        }
        for opportunity in &mut self.automation_opportunities {
            opportunity.estimated_time_saved = finite_clamp(opportunity.estimated_time_saved, 0.0, 525_600.0);
            opportunity.confidence = finite_clamp(opportunity.confidence, 0.0, 1.0);
        }
    }
}

fn merge_by<T, K, F, M>(target: &mut Vec<T>, incoming: Vec<T>, key: F, merge: M)
where
    F: Fn(&T) -> K,
    K: PartialEq,
    M: Fn(&mut T, T),
{
    for finding in incoming {
        if let Some(existing) = target.iter_mut().find(|item| key(item) == key(&finding)) {
            merge(existing, finding);
        } else {
            target.push(finding);
        }
    }
}

fn finite_clamp(value: f64, min: f64, max: f64) -> f64 {
    if value.is_finite() { value.clamp(min, max) } else { min }
}
