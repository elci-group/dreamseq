use crate::config::HarnessConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: String,
    pub harness: String,
    pub timestamp: DateTime<Utc>,
    pub content: String,
    pub metadata: LogMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogMetadata {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub user_messages: usize,
    pub assistant_messages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub duration_ms: Option<u64>,
}

pub struct LogAggregator;

impl Default for LogAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl LogAggregator {
    pub fn new() -> Self {
        Self
    }

    pub async fn aggregate(&self, harnesses: &[HarnessConfig]) -> Result<Vec<LogEntry>> {
        let mut all_entries = Vec::new();

        for harness in harnesses {
            let entries = self.aggregate_harness(harness).await?;
            all_entries.extend(entries);
        }

        all_entries.sort_by_key(|e| e.timestamp);
        Ok(all_entries)
    }

    async fn aggregate_harness(&self, harness: &HarnessConfig) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();

        if !harness.log_path.exists() {
            tracing::warn!("Log path does not exist: {:?}", harness.log_path);
            return Ok(entries);
        }

        for entry in WalkDir::new(&harness.log_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file()
                && let Ok(log_entries) = self.parse_log_file(entry.path(), harness).await
            {
                entries.extend(log_entries);
            }
        }

        Ok(entries)
    }

    async fn parse_log_file(&self, path: &Path, harness: &HarnessConfig) -> Result<Vec<LogEntry>> {
        match &harness.log_format {
            crate::config::LogFormat::CodexSqlite => self.parse_codex_sqlite(path, &harness.name),
            format => {
                let content = std::fs::read_to_string(path)?;
                match format {
                    crate::config::LogFormat::Json => self.parse_json_logs(&content, &harness.name),
                    crate::config::LogFormat::Markdown => {
                        self.parse_markdown_logs(&content, &harness.name)
                    }
                    crate::config::LogFormat::Plain | crate::config::LogFormat::Custom(_) => {
                        self.parse_plain_logs(&content, &harness.name)
                    }
                    crate::config::LogFormat::CodexSqlite => unreachable!(),
                }
            }
        }
    }

    fn parse_codex_sqlite(&self, path: &Path, harness: &str) -> Result<Vec<LogEntry>> {
        let output = std::process::Command::new("sqlite3")
            .args(["-separator", "\t"])
            .arg(path)
            .arg("SELECT ts, feedback_log_body FROM logs WHERE feedback_log_body IS NOT NULL ORDER BY ts")
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "sqlite3 could not read Codex logs: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let (seconds, content) = line.split_once('\t')?;
                let timestamp = seconds
                    .parse::<i64>()
                    .ok()
                    .and_then(|value| chrono::DateTime::from_timestamp(value, 0))?;
                Some(LogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    harness: harness.to_string(),
                    timestamp,
                    content: content.to_string(),
                    metadata: LogMetadata {
                        model: None,
                        provider: Some("openai".to_string()),
                        tool_calls: vec![],
                        user_messages: 0,
                        assistant_messages: 0,
                    },
                })
            })
            .collect())
    }

    fn parse_json_logs(&self, content: &str, harness: &str) -> Result<Vec<LogEntry>> {
        if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(content) {
            return Ok(values
                .into_iter()
                .filter_map(|value| self.json_to_log_entry(value, harness).ok())
                .collect());
        }

        let mut entries = Vec::new();

        for line in content.lines() {
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line)
                && let Ok(log_entry) = self.json_to_log_entry(entry, harness)
            {
                entries.push(log_entry);
            }
        }

        Ok(entries)
    }

    fn json_to_log_entry(&self, value: serde_json::Value, harness: &str) -> Result<LogEntry> {
        Ok(LogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            harness: harness.to_string(),
            timestamp: value["timestamp"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
            content: value["content"].as_str().unwrap_or("").to_string(),
            metadata: LogMetadata {
                model: value["model"].as_str().map(String::from),
                provider: value["provider"].as_str().map(String::from),
                tool_calls: vec![],
                user_messages: value["user_messages"].as_u64().unwrap_or(0) as usize,
                assistant_messages: value["assistant_messages"].as_u64().unwrap_or(0) as usize,
            },
        })
    }

    fn parse_markdown_logs(&self, content: &str, harness: &str) -> Result<Vec<LogEntry>> {
        // Simple markdown parser - looks for code blocks and timestamps
        let mut entries = Vec::new();
        let current_time = Utc::now();

        // For now, treat each line as a potential entry
        // In production, this would be more sophisticated
        for line in content.lines() {
            if !line.trim().is_empty() {
                entries.push(LogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    harness: harness.to_string(),
                    timestamp: current_time,
                    content: line.to_string(),
                    metadata: LogMetadata {
                        model: None,
                        provider: None,
                        tool_calls: vec![],
                        user_messages: 0,
                        assistant_messages: 0,
                    },
                });
            }
        }

        Ok(entries)
    }

    fn parse_plain_logs(&self, content: &str, harness: &str) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();
        let current_time = Utc::now();

        for line in content.lines() {
            if !line.trim().is_empty() {
                entries.push(LogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    harness: harness.to_string(),
                    timestamp: current_time,
                    content: line.to_string(),
                    metadata: LogMetadata {
                        model: None,
                        provider: None,
                        tool_calls: vec![],
                        user_messages: 0,
                        assistant_messages: 0,
                    },
                });
            }
        }

        Ok(entries)
    }
}
