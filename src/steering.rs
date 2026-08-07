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
    vec![
        Regex::new(r"i wish i had").unwrap(),
        Regex::new(r"we need a tool for").unwrap(),
        Regex::new(r"there should be a command").unwrap(),
        Regex::new(r"missing.*tool").unwrap(),
        Regex::new(r"would be nice if.*could").unwrap(),
    ]
});

static MISSING_CONTEXT_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"we already decided").unwrap(),
        Regex::new(r"you forgot").unwrap(),
        Regex::new(r"as i mentioned").unwrap(),
        Regex::new(r"remember that").unwrap(),
        Regex::new(r"we discussed").unwrap(),
        Regex::new(r"losing context").unwrap(),
    ]
});

static WRONG_ABSTRACTION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"that's not what i asked").unwrap(),
        Regex::new(r"you're solving the wrong problem").unwrap(),
        Regex::new(r"wrong approach").unwrap(),
        Regex::new(r"not the right abstraction").unwrap(),
        Regex::new(r"misunderstood the requirement").unwrap(),
    ]
});

static EXCESS_VERBOSITY_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"be more concise").unwrap(),
        Regex::new(r"too verbose").unwrap(),
        Regex::new(r"\bshorter\b").unwrap(),
        Regex::new(r"get to the point").unwrap(),
        Regex::new(r"less detail").unwrap(),
        Regex::new(r"keep it brief").unwrap(),
    ]
});

static HALLUCINATION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"that.*doesn't exist").unwrap(),
        Regex::new(r"no such.*api").unwrap(),
        Regex::new(r"that command doesn't").unwrap(),
        Regex::new(r"\binvented\b").unwrap(),
        Regex::new(r"\bhallucinated\b").unwrap(),
        Regex::new(r"not a real").unwrap(),
    ]
});

static ARCHITECTURAL_MISMATCH_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"doesn't fit the workflow").unwrap(),
        Regex::new(r"architectural mismatch").unwrap(),
        Regex::new(r"wrong for this use case").unwrap(),
        Regex::new(r"doesn't scale").unwrap(),
        Regex::new(r"not the right fit").unwrap(),
    ]
});

static MANUAL_REPETITION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // Match "again" only when it's tied to an action, avoiding stray words
        // like "... failed again" in telemetry noise.
        Regex::new(r"\b(?:do|run|execute|perform|say|write|try)\s+(?:it|that|this)\s+again\b")
            .unwrap(),
        Regex::new(r"\brepeat(?:\s+(?:that|this|it|the\s+\w+))?\b").unwrap(),
        Regex::new(r"\bsame as before\b").unwrap(),
        Regex::new(r"\bdo it again\b").unwrap(),
        Regex::new(r"\blike last time\b").unwrap(),
    ]
});

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
