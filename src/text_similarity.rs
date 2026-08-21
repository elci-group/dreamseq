// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Deterministic, local text-similarity primitives (TF-IDF + cosine
//! similarity) shared by anything that needs to cluster short pieces of
//! text without an inference call — originally written for
//! `segmentation::SemanticSegmenter`, reused by `patterns` to consolidate
//! near-duplicate findings that an LLM described with different wording
//! across separate batches.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

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

/// Splits on any run of non-alphanumeric characters, not just whitespace —
/// dotted/underscored technical identifiers (`response.in_progress`,
/// `response_function_call_arguments_delta`) are exactly the kind of text
/// LLM-written findings use, and treating each one as a single opaque token
/// would defeat similarity matching between findings that share most of
/// their words but differ in one segment of a dotted event name.
fn tokenize(content: &str) -> impl Iterator<Item = String> + '_ {
    content
        .split(|c: char| !c.is_alphanumeric())
        .map(|word| word.to_lowercase())
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(word.as_str()))
}

pub(crate) fn meaningful_words(content: &str) -> HashSet<String> {
    tokenize(content).collect()
}

fn term_frequencies(content: &str) -> HashMap<String, f64> {
    let words: Vec<String> = tokenize(content).collect();

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

/// One TF-IDF vector per input document, weighted against the whole set.
pub(crate) fn tfidf_vectors<S: AsRef<str>>(documents: &[S]) -> Vec<HashMap<String, f64>> {
    let term_frequencies: Vec<HashMap<String, f64>> = documents
        .iter()
        .map(|document| term_frequencies(document.as_ref()))
        .collect();

    let mut document_frequency: HashMap<String, usize> = HashMap::new();
    for tf in &term_frequencies {
        for term in tf.keys() {
            *document_frequency.entry(term.clone()).or_insert(0) += 1;
        }
    }

    let n = documents.len() as f64;
    let mut vectors = Vec::with_capacity(documents.len());
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

pub(crate) fn cosine_similarity(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
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

/// Fraction of shared distinct words between two texts. Unlike TF-IDF
/// cosine similarity, this doesn't downweight a term for appearing in
/// several documents — which is exactly backwards for finding near-
/// duplicates among a *small* set of *short* texts, where the shared
/// vocabulary between the near-duplicates is the signal, and idf actively
/// penalizes it for not being "rare". Suited to comparing a handful of
/// short LLM-written finding descriptions (see `patterns::consolidate`);
/// TF-IDF cosine remains the right tool for segmentation, which clusters
/// many long, sequential log entries where document-frequency smoothing
/// over a larger corpus works as intended.
pub(crate) fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_finds_near_duplicate_short_phrases_that_tfidf_misses() {
        let a = meaningful_words(
            "Responses API missing handler for function_call_arguments.delta events",
        );
        let b = meaningful_words("Responses API missing handler for in_progress events");
        let unrelated = meaningful_words("Docker healthcheck exceeds latency threshold repeatedly");

        let near_duplicate = jaccard_similarity(&a, &b);
        let dissimilar = jaccard_similarity(&a, &unrelated);
        assert!(
            near_duplicate > 0.4,
            "expected near-duplicate phrases to score high, got {near_duplicate}"
        );
        assert!(
            dissimilar < 0.1,
            "expected an unrelated phrase to score near zero, got {dissimilar}"
        );
    }

    #[test]
    fn identical_documents_are_maximally_similar() {
        let docs = vec![
            "missing tool for quota monitoring",
            "unrelated topic entirely",
        ];
        let vectors = tfidf_vectors(&docs);
        assert!(cosine_similarity(&vectors[0], &vectors[0]) > 0.99);
    }

    #[test]
    fn unrelated_documents_are_dissimilar() {
        let docs = vec![
            "missing tool for quota monitoring",
            "docker healthcheck latency issue",
        ];
        let vectors = tfidf_vectors(&docs);
        assert!(cosine_similarity(&vectors[0], &vectors[1]) < 0.3);
    }

    #[test]
    fn near_duplicate_wording_ranks_closer_than_unrelated_documents() {
        // A realistic-sized corpus, not just the two documents being
        // compared — TF-IDF's idf term is dominated by noise at n=2 (every
        // word looks equally "rare"), same as it would be in production
        // clustering, which runs over dozens of patterns per type, not a
        // single pair.
        let docs = vec![
            "unhandled response.function_call_arguments.delta event",
            "unhandled response.in_progress event in responses api stream",
            "docker healthcheck latency exceeds threshold",
            "sql query took over one second to complete",
            "shell command latency high during large repo scans",
        ];
        let vectors = tfidf_vectors(&docs);
        let near_duplicate = cosine_similarity(&vectors[0], &vectors[1]);
        let unrelated = cosine_similarity(&vectors[0], &vectors[2]);
        assert!(
            near_duplicate > unrelated,
            "expected the two responses-api findings ({near_duplicate}) to rank closer than an unrelated docker finding ({unrelated})"
        );
    }
}
