use crate::aggregator::LogEntry;
use anyhow::Result;

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
        let original_len = entries.len();
        let mut normalized = Vec::new();
        let mut empty = 0usize;
        for entry in entries {
            let normalized_entry = self.normalize_entry(entry);
            if normalized_entry.content.is_empty() {
                empty += 1;
                continue;
            }
            // Repeated entries are evidence for repetition and steering
            // frequency. Keep each occurrence, including its timestamp,
            // harness, model metadata, and tool-call provenance.
            normalized.push(normalized_entry);
        }

        tracing::info!(
            "Normalized {} entries: retained repeated evidence and removed {} empty entries",
            original_len,
            empty
        );
        Ok(normalized)
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
        content.replace("[system]", "").replace("[debug]", "")
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
