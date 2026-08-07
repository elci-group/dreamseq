use crate::aggregator::LogEntry;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: String,
    pub topic: String,
    pub entries: Vec<LogEntry>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub confidence: f64,
}

static STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
        "from", "as", "is", "was", "are", "this", "that", "it", "i", "you", "we", "he", "she",
        "they", "be", "been", "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "can", "not", "no", "yes", "ok", "so", "if", "then", "than",
        "when", "where", "what", "how", "why", "who", "which", "there", "here",
    ]
    .iter()
    .copied()
    .collect()
});

/// Two entries must be at least this similar (cosine over TF-IDF vectors) to
/// stay in the same segment.
const SIMILARITY_THRESHOLD: f64 = 0.3;

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

        let vectors = tfidf_vectors(&entries);

        let mut segments = Vec::new();
        let mut current_segment: Vec<LogEntry> = vec![entries[0].clone()];

        for i in 1..entries.len() {
            let prev = &entries[i - 1];
            let current = &entries[i];

            let time_gap = current
                .timestamp
                .signed_duration_since(prev.timestamp)
                .num_minutes();
            let topic_similarity = cosine_similarity(&vectors[i - 1], &vectors[i]);

            if time_gap > 30 || topic_similarity < SIMILARITY_THRESHOLD {
                if !current_segment.is_empty() {
                    segments.push(self.create_segment(current_segment.clone())?);
                }
                current_segment = vec![current.clone()];
            } else {
                current_segment.push(current.clone());
            }
        }

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

    fn infer_topic(&self, entries: &[LogEntry]) -> String {
        let mut word_counts: HashMap<String, usize> = HashMap::new();

        for entry in entries {
            for word in meaningful_words(&entry.content) {
                if word.len() > 3 {
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
        if entries.len() < 2 {
            return 1.0;
        }

        let vectors = tfidf_vectors(entries);
        let mut similarities = Vec::new();
        for i in 0..entries.len() - 1 {
            similarities.push(cosine_similarity(&vectors[i], &vectors[i + 1]));
        }

        similarities.iter().sum::<f64>() / similarities.len() as f64
    }
}

fn meaningful_words(content: &str) -> HashSet<String> {
    content
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(word.as_str()))
        .collect()
}

fn term_frequencies(content: &str) -> HashMap<String, f64> {
    let words: Vec<String> = content
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(word.as_str()))
        .collect();

    let total = words.len().max(1) as f64;
    let mut counts: HashMap<String, f64> = HashMap::new();
    for word in words {
        *counts.entry(word).or_insert(0.0) += 1.0;
    }
    for count in counts.values_mut() {
        *count /= total;
    }
    counts
}

fn tfidf_vectors(entries: &[LogEntry]) -> Vec<HashMap<String, f64>> {
    let term_frequencies: Vec<HashMap<String, f64>> = entries
        .iter()
        .map(|entry| term_frequencies(&entry.content))
        .collect();

    let mut document_frequency: HashMap<String, usize> = HashMap::new();
    for tf in &term_frequencies {
        for term in tf.keys() {
            *document_frequency.entry(term.clone()).or_insert(0) += 1;
        }
    }

    let n = entries.len() as f64;
    let mut vectors = Vec::with_capacity(entries.len());
    for tf in term_frequencies {
        let mut vector = HashMap::new();
        for (term, frequency) in tf {
            let df = *document_frequency.get(&term).unwrap_or(&1) as f64;
            let idf = ((n / df) + 1.0).ln();
            vector.insert(term, frequency * idf);
        }
        vectors.push(vector);
    }

    vectors
}

fn cosine_similarity(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for (term, weight) in a {
        norm_a += weight * weight;
        if let Some(other_weight) = b.get(term) {
            dot_product += weight * other_weight;
        }
    }

    for weight in b.values() {
        norm_b += weight * weight;
    }

    let denominator = norm_a.sqrt() * norm_b.sqrt();
    if denominator == 0.0 {
        return 0.0;
    }

    dot_product / denominator
}
