use crate::aggregator::LogEntry;
use crate::segmentation::Segment;
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};

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
        if let Some(category) = self.detect_missing_tool(&content) {
            return Some(self.create_steering_event(
                entry,
                category,
                "User indicated missing tool or capability",
            ));
        }

        if let Some(category) = self.detect_missing_context(&content) {
            return Some(self.create_steering_event(
                entry,
                category,
                "Model forgot previous context or decisions",
            ));
        }

        if let Some(category) = self.detect_wrong_abstraction(&content) {
            return Some(self.create_steering_event(
                entry,
                category,
                "Model solved the wrong problem",
            ));
        }

        if let Some(category) = self.detect_excess_verbosity(&content) {
            return Some(self.create_steering_event(entry, category, "User requested conciseness"));
        }

        if let Some(category) = self.detect_hallucination(&content) {
            return Some(self.create_steering_event(
                entry,
                category,
                "Model invented API or command",
            ));
        }

        if let Some(category) = self.detect_architectural_mismatch(&content) {
            return Some(self.create_steering_event(
                entry,
                category,
                "Existing tool no longer fits workflow",
            ));
        }

        if let Some(category) = self.detect_manual_repetition(&content) {
            return Some(self.create_steering_event(
                entry,
                category,
                "User repeated same sequence manually",
            ));
        }

        None
    }

    fn detect_missing_tool(&self, content: &str) -> Option<SteeringCategory> {
        let patterns = [
            r"i wish i had",
            r"we need a tool for",
            r"there should be a command",
            r"missing.*tool",
            r"would be nice if.*could",
        ];

        for pattern in &patterns {
            if Regex::new(pattern).unwrap().is_match(content) {
                return Some(SteeringCategory::MissingTool);
            }
        }
        None
    }

    fn detect_missing_context(&self, content: &str) -> Option<SteeringCategory> {
        let patterns = [
            r"we already decided",
            r"you forgot",
            r"as i mentioned",
            r"remember that",
            r"we discussed",
            r"losing context",
        ];

        for pattern in &patterns {
            if Regex::new(pattern).unwrap().is_match(content) {
                return Some(SteeringCategory::MissingContext);
            }
        }
        None
    }

    fn detect_wrong_abstraction(&self, content: &str) -> Option<SteeringCategory> {
        let patterns = [
            r"that's not what i asked",
            r"you're solving the wrong problem",
            r"wrong approach",
            r"not the right abstraction",
            r"misunderstood the requirement",
        ];

        for pattern in &patterns {
            if Regex::new(pattern).unwrap().is_match(content) {
                return Some(SteeringCategory::WrongAbstraction);
            }
        }
        None
    }

    fn detect_excess_verbosity(&self, content: &str) -> Option<SteeringCategory> {
        let patterns = [
            r"be more concise",
            r"too verbose",
            r"shorter",
            r"get to the point",
            r"less detail",
            r"keep it brief",
        ];

        for pattern in &patterns {
            if Regex::new(pattern).unwrap().is_match(content) {
                return Some(SteeringCategory::ExcessVerbosity);
            }
        }
        None
    }

    fn detect_hallucination(&self, content: &str) -> Option<SteeringCategory> {
        let patterns = [
            r"that.*doesn't exist",
            r"no such.*api",
            r"that command doesn't",
            r"invented",
            r"hallucinated",
            r"not a real",
        ];

        for pattern in &patterns {
            if Regex::new(pattern).unwrap().is_match(content) {
                return Some(SteeringCategory::Hallucination);
            }
        }
        None
    }

    fn detect_architectural_mismatch(&self, content: &str) -> Option<SteeringCategory> {
        let patterns = [
            r"doesn't fit the workflow",
            r"architectural mismatch",
            r"wrong for this use case",
            r"doesn't scale",
            r"not the right fit",
        ];

        for pattern in &patterns {
            if Regex::new(pattern).unwrap().is_match(content) {
                return Some(SteeringCategory::ArchitecturalMismatch);
            }
        }
        None
    }

    fn detect_manual_repetition(&self, content: &str) -> Option<SteeringCategory> {
        let patterns = [
            r"again",
            r"repeat",
            r"same as before",
            r"do it again",
            r"like last time",
        ];

        for pattern in &patterns {
            if Regex::new(pattern).unwrap().is_match(content) {
                return Some(SteeringCategory::ManualRepetition);
            }
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

fn is_conversation_opener(content: &str) -> bool {
    matches!(
        content.trim(),
        "hi" | "hello" | "hey" | "hello!" | "hi!" | "hey!" | "good morning" | "good evening"
    )
}
