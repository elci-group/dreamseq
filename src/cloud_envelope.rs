// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use crate::report::{Anthology, CandidateTool, Priority};
use chrono::{DateTime, Utc};
use serde::Serialize;

const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Serialize)]
pub struct RunEnvelope {
    schema_version: u8,
    run: CloudRun,
}

#[derive(Debug, Serialize)]
struct CloudRun {
    id: String,
    generated_at: DateTime<Utc>,
    source: CloudSource,
    summary: String,
    pipeline: CloudPipeline,
    opportunities: Vec<CloudOpportunity>,
}

#[derive(Debug, Serialize)]
struct CloudSource {
    id: String,
    cli_version: &'static str,
}

#[derive(Debug, Serialize)]
struct CloudPipeline {
    raw_entries: usize,
    normalized_entries: usize,
    segments: usize,
    estimated_input_tokens: usize,
}

#[derive(Debug, Serialize)]
struct CloudOpportunity {
    id: String,
    title: String,
    summary: String,
    priority: &'static str,
    confidence: u8,
    estimated_hours: f64,
    evidence_count: usize,
    repositories: usize,
    projects: Vec<String>,
    decision: &'static str,
}

impl RunEnvelope {
    pub fn from_anthology(anthology: &Anthology, source_id: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            run: CloudRun {
                id: anthology.id.clone(),
                generated_at: anthology.generated_at,
                source: CloudSource {
                    id: source_id.to_string(),
                    cli_version: env!("CARGO_PKG_VERSION"),
                },
                summary: anthology.executive_summary.clone(),
                pipeline: CloudPipeline {
                    raw_entries: anthology.pipeline.raw_entries,
                    normalized_entries: anthology.pipeline.normalized_entries,
                    segments: anthology.pipeline.segments,
                    estimated_input_tokens: anthology.pipeline.estimated_input_tokens,
                },
                opportunities: anthology
                    .candidate_tools
                    .iter()
                    .map(CloudOpportunity::from)
                    .collect(),
            },
        }
    }
}

impl From<&CandidateTool> for CloudOpportunity {
    fn from(tool: &CandidateTool) -> Self {
        let projects = if tool.existing_matches.is_empty() {
            tool.affected_projects.clone()
        } else {
            tool.existing_matches.clone()
        };
        Self {
            id: tool.id.clone(),
            title: tool.name.clone(),
            summary: tool.reason.clone(),
            priority: match tool.priority {
                Priority::High => "high",
                Priority::Medium => "medium",
                Priority::Low => "low",
            },
            confidence: (tool.confidence.clamp(0.0, 1.0) * 100.0).round() as u8,
            estimated_hours: parse_hours(&tool.estimated_time_saved),
            evidence_count: 1,
            repositories: projects.len(),
            projects,
            decision: if tool.existing_matches.is_empty() {
                "generate"
            } else {
                "extend"
            },
        }
    }
}

pub(super) fn parse_hours(value: &str) -> f64 {
    value
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .find_map(|part| {
            (!part.is_empty())
                // traci: allow -- non-numeric fragments are expected while scanning prose.
                .then(|| part.parse::<f64>().ok())
                .flatten()
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DreamseqConfig;
    use crate::report::InterventionCategory;

    #[test]
    fn serializes_the_minimal_cloud_contract() {
        let mut anthology = Anthology::new(vec![], vec![], DreamseqConfig::default());
        anthology.candidate_tools.push(CandidateTool {
            id: "DS-1".into(),
            name: "Route cache".into(),
            priority: Priority::High,
            category: InterventionCategory::WorkflowAcceleration,
            reason: "Repeated route lookup".into(),
            estimated_time_saved: "1.5 hours/day".into(),
            confidence: 0.875,
            affected_projects: vec!["dreamseq".into()],
            existing_matches: vec![],
            mutation_fitness: 0.8,
            capability_overlap: 0.1,
            implementation_cost: "low".into(),
        });
        let value = serde_json::to_value(RunEnvelope::from_anthology(&anthology, "device"))
            .expect("envelope should serialize");
        let opportunity = &value["run"]["opportunities"][0];
        assert_eq!(value["schema_version"], 1);
        assert_eq!(opportunity["estimated_hours"], 1.5);
        assert_eq!(opportunity["confidence"], 88);
        assert_eq!(opportunity["decision"], "generate");
        assert!(value.get("config").is_none());
    }
}
