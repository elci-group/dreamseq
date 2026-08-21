// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use crate::groq::Analysis;
use crate::text_similarity::{jaccard_similarity, meaningful_words};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    pub pattern_type: PatternType,
    pub description: String,
    pub frequency: usize,
    pub confidence: f64,
    pub impact_score: f64,
    pub affected_harnesses: Vec<String>,
    /// A genuine per-finding minutes estimate, carried through separately
    /// from `impact_score` rather than backed into it — only
    /// `WorkflowBottleneck` and `AutomationOpportunity` findings come with
    /// one from the model; every other pattern type has no time basis at
    /// all and must not present one. `None` means "not quantified", not
    /// zero impact.
    #[serde(default)]
    pub estimated_minutes_per_day: Option<f64>,
    /// How many separately-worded findings across batches were consolidated
    /// into this one pattern (see `consolidate` below). 1 for a pattern that
    /// wasn't merged with anything.
    #[serde(default = "one")]
    pub manifestation_count: usize,
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

// Jaccard similarity (shared distinct words / all distinct words), not
// TF-IDF cosine — see text_similarity::jaccard_similarity for why idf
// weighting works against this specific comparison. Calibrated so that two
// findings sharing most of their words but differing in one technical
// detail (e.g. two dotted event names) clear it, while two findings that
// merely share a couple of generic words ("missing", "handler") don't.
const PATTERN_SIMILARITY_THRESHOLD: f64 = 0.4;

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
            patterns.push((
                Pattern {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern_type: PatternType::ModelFailure,
                    description: format!("{}: {}", failure.model, failure.issue),
                    frequency: failure.frequency,
                    confidence: 0.8,
                    impact_score: self.calculate_impact_score(failure.frequency as f64, 0.7),
                    affected_harnesses: vec![failure.model.clone()],
                    estimated_minutes_per_day: None,
                    manifestation_count: 1,
                },
                failure.issue.clone(),
            ));
        }

        // Extract harness friction patterns
        for friction in &analysis.harness_friction {
            patterns.push((
                Pattern {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern_type: PatternType::HarnessFriction,
                    description: format!("{} friction: {}", friction.harness, friction.issue),
                    frequency: 1,
                    confidence: friction.severity,
                    impact_score: friction.severity,
                    affected_harnesses: vec![friction.harness.clone()],
                    estimated_minutes_per_day: None,
                    manifestation_count: 1,
                },
                friction.issue.clone(),
            ));
        }

        // Extract missing tool patterns
        for tool in &analysis.missing_tooling {
            patterns.push((
                Pattern {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern_type: PatternType::MissingTool,
                    description: format!("Missing tool: {} - {}", tool.tool_name, tool.purpose),
                    frequency: 1,
                    confidence: 0.9,
                    impact_score: tool.estimated_value,
                    affected_harnesses: vec![],
                    estimated_minutes_per_day: None,
                    manifestation_count: 1,
                },
                format!("{} {}", tool.tool_name, tool.purpose),
            ));
        }

        // Extract workflow bottleneck patterns
        for bottleneck in &analysis.workflow_bottlenecks {
            patterns.push((
                Pattern {
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
                    estimated_minutes_per_day: Some(bottleneck.time_impact_minutes),
                    manifestation_count: 1,
                },
                bottleneck.description.clone(),
            ));
        }

        // Extract repeated command patterns
        for command in &analysis.repeated_commands {
            patterns.push((
                Pattern {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern_type: PatternType::RepeatedCommand,
                    description: format!("Repeated command: {}", command.command),
                    frequency: command.frequency,
                    confidence: 0.95,
                    impact_score: self.calculate_impact_score(command.frequency as f64, 0.5),
                    affected_harnesses: vec![command.context.clone()],
                    estimated_minutes_per_day: None,
                    manifestation_count: 1,
                },
                command.command.clone(),
            ));
        }

        // Extract repeated prompt patterns
        for prompt in &analysis.repeated_prompts {
            patterns.push((
                Pattern {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern_type: PatternType::RepeatedPrompt,
                    description: format!("Repeated prompt pattern: {}", prompt.prompt_pattern),
                    frequency: prompt.frequency,
                    confidence: 0.9,
                    impact_score: self.calculate_impact_score(prompt.frequency as f64, 0.3),
                    affected_harnesses: vec![],
                    estimated_minutes_per_day: None,
                    manifestation_count: 1,
                },
                prompt.prompt_pattern.clone(),
            ));
        }

        // Extract context loss patterns
        for loss in &analysis.context_loss {
            patterns.push((
                Pattern {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern_type: PatternType::ContextLoss,
                    description: loss.description.clone(),
                    frequency: loss.affected_segments.len(),
                    confidence: 0.75,
                    impact_score: self
                        .calculate_impact_score(loss.affected_segments.len() as f64, 0.8),
                    affected_harnesses: loss.affected_segments.clone(),
                    estimated_minutes_per_day: None,
                    manifestation_count: 1,
                },
                loss.description.clone(),
            ));
        }

        // Extract automation opportunities
        for opportunity in &analysis.automation_opportunities {
            patterns.push((
                Pattern {
                    id: uuid::Uuid::new_v4().to_string(),
                    pattern_type: PatternType::AutomationOpportunity,
                    description: opportunity.description.clone(),
                    frequency: 1,
                    confidence: opportunity.confidence,
                    impact_score: self
                        .calculate_impact_score(1.0, opportunity.estimated_time_saved / 60.0),
                    affected_harnesses: vec![],
                    estimated_minutes_per_day: Some(opportunity.estimated_time_saved),
                    manifestation_count: 1,
                },
                opportunity.description.clone(),
            ));
        }

        let mut patterns = consolidate(patterns);

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

/// Collapses patterns that describe the same underlying root cause with
/// different wording — the LLM analyzes each batch independently, so a
/// recurring defect (e.g. incomplete Responses-API event coverage) shows up
/// as many differently-phrased findings across batches rather than one.
/// `Analysis::merge` already deduplicates exact-string repeats across
/// batches before this runs; this catches the much larger set of
/// near-duplicates that merge missed because the wording differs.
///
/// Deliberately does not merge across `PatternType` — a `MissingTool` and a
/// `WorkflowBottleneck` are different kinds of finding even if their prose
/// overlaps, and merging them would blur a distinction the rest of the
/// pipeline (candidate-tool categorization, user-behaviour extraction)
/// still relies on.
///
/// Each pattern is paired with a separate `comparison_text` rather than
/// comparing on `Pattern::description` directly: several pattern types
/// format their description with a fixed prefix ("Repeated command: ",
/// "Missing tool: ", "{harness} friction: ") that every pattern of that type
/// shares. Since patterns are already grouped by type before comparison,
/// that boilerplate would be common to every pair and inflate Jaccard
/// similarity for short, otherwise-unrelated findings (e.g. "Repeated
/// command: ls" vs "Repeated command: cd" sharing "repeated"/"command").
/// `comparison_text` carries just the substantive part (the issue, purpose,
/// command, or prompt text) so similarity reflects real content overlap.
fn consolidate(patterns: Vec<(Pattern, String)>) -> Vec<Pattern> {
    let mut by_type: HashMap<PatternType, Vec<(Pattern, String)>> = HashMap::new();
    for (pattern, comparison_text) in patterns {
        by_type
            .entry(pattern.pattern_type)
            .or_default()
            .push((pattern, comparison_text));
    }

    let mut consolidated = Vec::new();
    for (_pattern_type, group) in by_type {
        consolidated.extend(consolidate_group(group));
    }
    consolidated
}

fn consolidate_group(group: Vec<(Pattern, String)>) -> Vec<Pattern> {
    if group.len() < 2 {
        return group.into_iter().map(|(pattern, _)| pattern).collect();
    }

    let word_sets: Vec<_> = group
        .iter()
        .map(|(_, comparison_text)| meaningful_words(comparison_text))
        .collect();

    let mut merged_into: Vec<Option<usize>> = vec![None; group.len()];
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for i in 0..group.len() {
        if merged_into[i].is_some() {
            continue;
        }
        let cluster_index = clusters.len();
        clusters.push(vec![i]);
        merged_into[i] = Some(cluster_index);
        for j in (i + 1)..group.len() {
            if merged_into[j].is_some() {
                continue;
            }
            if jaccard_similarity(&word_sets[i], &word_sets[j]) >= PATTERN_SIMILARITY_THRESHOLD {
                clusters[cluster_index].push(j);
                merged_into[j] = Some(cluster_index);
            }
        }
    }

    clusters
        .into_iter()
        .map(|indices| merge_cluster(indices.into_iter().map(|i| group[i].0.clone()).collect()))
        .collect()
}

fn merge_cluster(mut members: Vec<Pattern>) -> Pattern {
    if members.len() == 1 {
        return members.remove(0);
    }

    // The clearest single description to represent the cluster is whichever
    // member individually scored highest — not a synthesized summary, since
    // nothing here is re-running the LLM to write one.
    let representative_index = members
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.impact_score.total_cmp(&b.impact_score))
        .map(|(index, _)| index)
        .unwrap_or(0);

    let mut affected_harnesses: Vec<String> = members
        .iter()
        .flat_map(|pattern| pattern.affected_harnesses.iter().cloned())
        .collect();
    affected_harnesses.sort();
    affected_harnesses.dedup();

    let frequency = members.iter().map(|pattern| pattern.frequency).sum();
    let manifestation_count = members
        .iter()
        .map(|pattern| pattern.manifestation_count)
        .sum();
    // Max, not sum: these findings are different wordings of the *same*
    // root cause, so their individual time estimates are competing
    // descriptions of one impact, not additive minutes. Summing would
    // double-count the same underlying time the way a naive rollup of
    // "20 min/day" x N near-duplicates would.
    let estimated_minutes_per_day = members
        .iter()
        .filter_map(|pattern| pattern.estimated_minutes_per_day)
        .fold(None, |max, value| {
            Some(max.map_or(value, |m: f64| m.max(value)))
        });
    let confidence = members
        .iter()
        .map(|pattern| pattern.confidence)
        .fold(0.0, f64::max);
    let impact_score = members
        .iter()
        .map(|pattern| pattern.impact_score)
        .fold(0.0, f64::max);

    let representative = members.swap_remove(representative_index);

    Pattern {
        id: uuid::Uuid::new_v4().to_string(),
        pattern_type: representative.pattern_type,
        description: representative.description,
        frequency,
        confidence,
        impact_score,
        affected_harnesses,
        estimated_minutes_per_day,
        manifestation_count,
    }
}
