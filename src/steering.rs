// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use crate::aggregator::LogEntry;
use crate::segmentation::Segment;
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteeringEvent {
    pub id: String,
    pub category: SteeringCategory,
    pub description: String,
    pub entry_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub context: String,
    pub severity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SteeringCategory {
    MissingTool,
    MissingContext,
    WrongAbstraction,
    ExcessVerbosity,
    Hallucination,
    ArchitecturalMismatch,
    ManualRepetition,
    Other,
}

static MISSING_TOOL_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile_patterns(
        "missing_tool",
        &[
            r"i wish i had",
            r"we need a tool for",
            r"there should be a command",
            r"missing.*tool",
            r"would be nice if.*could",
        ],
    )
});

static MISSING_CONTEXT_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile_patterns(
        "missing_context",
        &[
            r"we already decided",
            r"you forgot",
            r"as i mentioned",
            r"remember that",
            r"we discussed",
            r"losing context",
        ],
    )
});

static WRONG_ABSTRACTION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile_patterns(
        "wrong_abstraction",
        &[
            r"that's not what i asked",
            r"you're solving the wrong problem",
            r"wrong approach",
            r"not the right abstraction",
            r"misunderstood the requirement",
        ],
    )
});

static EXCESS_VERBOSITY_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile_patterns(
        "excess_verbosity",
        &[
            r"be more concise",
            r"too verbose",
            r"\bshorter\b",
            r"get to the point",
            r"less detail",
            r"keep it brief",
        ],
    )
});

static HALLUCINATION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile_patterns(
        "hallucination",
        &[
            r"that.*doesn't exist",
            r"no such.*api",
            r"that command doesn't",
            r"\binvented\b",
            r"\bhallucinated\b",
            r"not a real",
        ],
    )
});

static ARCHITECTURAL_MISMATCH_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile_patterns(
        "architectural_mismatch",
        &[
            r"doesn't fit the workflow",
            r"architectural mismatch",
            r"wrong for this use case",
            r"doesn't scale",
            r"not the right fit",
        ],
    )
});

static MANUAL_REPETITION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile_patterns(
        "manual_repetition",
        &[
            // Match "again" only when it's tied to an action, avoiding stray words
            // like "... failed again" in telemetry noise.
            r"\b(?:do|run|execute|perform|say|write|try)\s+(?:it|that|this)\s+again\b",
            r"\brepeat(?:\s+(?:that|this|it|the\s+\w+))?\b",
            r"\bsame as before\b",
            r"\bdo it again\b",
            r"\blike last time\b",
        ],
    )
});

fn compile_patterns(group: &'static str, patterns: &[&'static str]) -> Vec<Regex> {
    patterns
        .iter()
        .filter_map(|pattern| match Regex::new(pattern) {
            Ok(regex) => Some(regex),
            Err(error) => {
                tracing::error!(group, pattern, error = %error, "built-in steering regex compilation failed");
                None
            }
        })
        .collect()
}

pub struct SteeringDetector;

impl Default for SteeringDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl SteeringDetector {
    pub fn new() -> Self {
        Self
    }

    pub fn detect(&self, segments: &[Segment]) -> Result<Vec<SteeringEvent>> {
        let mut events = Vec::new();

        for segment in segments {
            for entry in &segment.entries {
                if let Some(event) = self.detect_steering_in_entry(entry) {
                    events.push(event);
                }
            }
        }

        events.sort_by_key(|a| a.timestamp);
        Ok(events)
    }

    fn detect_steering_in_entry(&self, entry: &LogEntry) -> Option<SteeringEvent> {
        let content = entry.content.to_lowercase();

        // Conversation openers are not evidence of missing capability. Keep
        // them out of friction clusters so greetings cannot inflate tooling ROI.
        if is_conversation_opener(&content) {
            return None;
        }

        // Check for various steering patterns
        if matches_patterns(&content, &MISSING_TOOL_PATTERNS) {
            return Some(self.create_steering_event(
                entry,
                SteeringCategory::MissingTool,
                "User indicated missing tool or capability",
            ));
        }

        if matches_patterns(&content, &MISSING_CONTEXT_PATTERNS) {
            return Some(self.create_steering_event(
                entry,
                SteeringCategory::MissingContext,
                "Model forgot previous context or decisions",
            ));
        }

        if matches_patterns(&content, &WRONG_ABSTRACTION_PATTERNS) {
            return Some(self.create_steering_event(
                entry,
                SteeringCategory::WrongAbstraction,
                "Model solved the wrong problem",
            ));
        }

        if matches_patterns(&content, &EXCESS_VERBOSITY_PATTERNS) {
            return Some(self.create_steering_event(
                entry,
                SteeringCategory::ExcessVerbosity,
                "User requested conciseness",
            ));
        }

        if matches_patterns(&content, &HALLUCINATION_PATTERNS) {
            return Some(self.create_steering_event(
                entry,
                SteeringCategory::Hallucination,
                "Model invented API or command",
            ));
        }

        if matches_patterns(&content, &ARCHITECTURAL_MISMATCH_PATTERNS) {
            return Some(self.create_steering_event(
                entry,
                SteeringCategory::ArchitecturalMismatch,
                "Existing tool no longer fits workflow",
            ));
        }

        if matches_patterns(&content, &MANUAL_REPETITION_PATTERNS) {
            return Some(self.create_steering_event(
                entry,
                SteeringCategory::ManualRepetition,
                "User repeated same sequence manually",
            ));
        }

        None
    }

    fn create_steering_event(
        &self,
        entry: &LogEntry,
        category: SteeringCategory,
        description: &str,
    ) -> SteeringEvent {
        SteeringEvent {
            id: uuid::Uuid::new_v4().to_string(),
            category,
            description: description.to_string(),
            entry_id: entry.id.clone(),
            timestamp: entry.timestamp,
            context: entry.content.clone(),
            severity: self.calculate_severity(&category),
        }
    }

    fn calculate_severity(&self, category: &SteeringCategory) -> f64 {
        match category {
            SteeringCategory::MissingTool => 0.8,
            SteeringCategory::MissingContext => 0.7,
            SteeringCategory::WrongAbstraction => 0.9,
            SteeringCategory::ExcessVerbosity => 0.3,
            SteeringCategory::Hallucination => 0.95,
            SteeringCategory::ArchitecturalMismatch => 0.85,
            SteeringCategory::ManualRepetition => 0.6,
            SteeringCategory::Other => 0.5,
        }
    }
}

fn matches_patterns(content: &str, patterns: &[Regex]) -> bool {
    patterns.iter().any(|pattern| pattern.is_match(content))
}

fn is_conversation_opener(content: &str) -> bool {
    matches!(
        content.trim(),
        "hi" | "hello" | "hey" | "hello!" | "hi!" | "hey!" | "good morning" | "good evening"
    )
}
