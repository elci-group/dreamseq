use crate::aggregator::LogEntry;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: String,
    pub topic: String,
    pub entries: Vec<LogEntry>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub confidence: f64,
}

pub struct SemanticSegmenter;

impl Default for SemanticSegmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticSegmenter {
    pub fn new() -> Self {
        Self
    }

    pub fn segment(&self, entries: Vec<LogEntry>) -> Result<Vec<Segment>> {
        if entries.is_empty() {
            return Ok(vec![]);
        }

        // Group entries by time proximity and topic similarity
        let mut segments = Vec::new();
        let mut current_segment: Vec<LogEntry> = vec![entries[0].clone()];

        for i in 1..entries.len() {
            let prev = &entries[i - 1];
            let current = &entries[i];

            // Check if this should start a new segment
            let time_gap = current
                .timestamp
                .signed_duration_since(prev.timestamp)
                .num_minutes();
            let topic_similarity = self.topic_similarity(prev, current);

            if time_gap > 30 || topic_similarity < 0.5 {
                // End current segment
                if !current_segment.is_empty() {
                    segments.push(self.create_segment(current_segment.clone())?);
                }
                current_segment = vec![current.clone()];
            } else {
                current_segment.push(current.clone());
            }
        }

        // Don't forget the last segment
        if !current_segment.is_empty() {
            segments.push(self.create_segment(current_segment)?);
        }

        Ok(segments)
    }

    fn create_segment(&self, entries: Vec<LogEntry>) -> Result<Segment> {
        let start_time = entries
            .first()
            .map(|e| e.timestamp)
            .unwrap_or_else(Utc::now);

        let end_time = entries.last().map(|e| e.timestamp).unwrap_or_else(Utc::now);

        let topic = self.infer_topic(&entries);
        let confidence = self.calculate_confidence(&entries);

        Ok(Segment {
            id: uuid::Uuid::new_v4().to_string(),
            topic,
            entries,
            start_time,
            end_time,
            confidence,
        })
    }

    fn topic_similarity(&self, entry1: &LogEntry, entry2: &LogEntry) -> f64 {
        // Simple similarity based on shared keywords
        let words1: HashSet<String> = entry1
            .content
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();

        let words2: HashSet<String> = entry2
            .content
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();

        if words1.is_empty() || words2.is_empty() {
            return 0.0;
        }

        let intersection = words1.intersection(&words2).count();
        let union = words1.union(&words2).count();

        intersection as f64 / union as f64
    }

    fn infer_topic(&self, entries: &[LogEntry]) -> String {
        // Extract most common keywords as topic
        let mut word_counts: HashMap<String, usize> = HashMap::new();

        for entry in entries {
            for word in entry.content.split_whitespace() {
                let word = word.to_lowercase();
                if word.len() > 3 {
                    // Ignore short words
                    *word_counts.entry(word).or_insert(0) += 1;
                }
            }
        }

        word_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(word, _)| word)
            .unwrap_or_else(|| "general".to_string())
    }

    fn calculate_confidence(&self, entries: &[LogEntry]) -> f64 {
        // Confidence based on segment coherence
        if entries.len() < 2 {
            return 1.0;
        }

        let mut similarities = Vec::new();
        for i in 0..entries.len() - 1 {
            similarities.push(self.topic_similarity(&entries[i], &entries[i + 1]));
        }

        let avg_similarity: f64 = similarities.iter().sum::<f64>() / similarities.len() as f64;
        avg_similarity
    }
}

use std::collections::HashSet;
