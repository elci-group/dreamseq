use crate::aggregator::LogEntry;
use anyhow::Result;
use std::collections::HashSet;

pub struct Normalizer;

impl Default for Normalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Normalizer {
    pub fn new() -> Self {
        Self
    }

    pub fn normalize(&self, entries: Vec<LogEntry>) -> Result<Vec<LogEntry>> {
        let mut normalized = Vec::new();
        let mut seen = HashSet::new();
        for entry in entries {
            let normalized_entry = self.normalize_entry(entry);
            if normalized_entry.content.is_empty() {
                continue;
            }
            // Remove duplicates based on content fingerprint
            let fingerprint = self.content_fingerprint(&normalized_entry.content);

            if seen.contains(&fingerprint) {
                continue;
            }
            seen.insert(fingerprint.clone());

            // Normalize timestamp
            normalized.push(normalized_entry);
        }

        tracing::info!(
            "Removed {} duplicate entries",
            seen.len().saturating_sub(normalized.len())
        );
        Ok(normalized)
    }

    fn content_fingerprint(&self, content: &str) -> String {
        // Simple fingerprinting - lowercase and trim
        // In production, this would use hashing
        content.trim().to_lowercase()
    }

    fn normalize_entry(&self, mut entry: LogEntry) -> LogEntry {
        // Normalize whitespace
        entry.content = entry
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Remove common noise patterns
        entry.content = self.remove_noise(&entry.content);

        // Normalize tool calls
        entry.metadata.tool_calls = self.normalize_tool_calls(&entry.metadata.tool_calls);

        entry
    }

    fn remove_noise(&self, content: &str) -> String {
        // Remove common noise patterns
        let noise_patterns = [
            r"\[system\]", // System messages
            r"\[debug\]",  // Debug messages
            r"^\s*$",      // Empty lines
        ];

        let mut result = content.to_string();
        for pattern in &noise_patterns {
            result = regex::Regex::new(pattern)
                .unwrap()
                .replace_all(&result, "")
                .to_string();
        }

        result
    }

    fn normalize_tool_calls(
        &self,
        tool_calls: &[crate::aggregator::ToolCall],
    ) -> Vec<crate::aggregator::ToolCall> {
        tool_calls
            .iter()
            .map(|call| crate::aggregator::ToolCall {
                tool_name: call.tool_name.to_lowercase(),
                parameters: self.normalize_parameters(&call.parameters),
                result: call.result.clone(),
                duration_ms: call.duration_ms,
            })
            .collect()
    }

    fn normalize_parameters(&self, params: &serde_json::Value) -> serde_json::Value {
        // Recursively normalize parameter structure
        match params {
            serde_json::Value::String(s) => serde_json::Value::String(s.trim().to_string()),
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| self.normalize_parameters(v)).collect())
            }
            serde_json::Value::Object(obj) => {
                let mut normalized = serde_json::Map::new();
                for (k, v) in obj {
                    let normalized_key = k.to_lowercase();
                    normalized.insert(normalized_key, self.normalize_parameters(v));
                }
                serde_json::Value::Object(normalized)
            }
            _ => params.clone(),
        }
    }
}
