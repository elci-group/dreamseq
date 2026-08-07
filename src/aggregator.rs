use crate::config::HarnessConfig;
use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::LazyLock;
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

static TIMESTAMP_PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<ts>\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)\s*(?P<rest>.*)$",
    )
    .unwrap()
});

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
        if !harness.log_path.exists() {
            tracing::warn!("Log path does not exist: {:?}", harness.log_path);
            return Ok(Vec::new());
        }

        if matches!(harness.log_format, crate::config::LogFormat::Bound) {
            return crate::bound::aggregate_bound_harness(harness).await;
        }

        let mut entries = Vec::new();
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
                    crate::config::LogFormat::Bound => {
                        unreachable!("Bound harnesses are handled directly in aggregate_harness")
                    }
                    crate::config::LogFormat::CodexSqlite => unreachable!(),
                }
            }
        }
    }

    fn parse_codex_sqlite(&self, path: &Path, harness: &str) -> Result<Vec<LogEntry>> {
        // If sqlite3 is not installed, skip this source rather than failing the
        // whole aggregation pipeline.
        if std::process::Command::new("sqlite3")
            .arg("--version")
            .output()
            .is_err()
        {
            tracing::warn!(
                "sqlite3 not available; skipping Codex SQLite source {:?}",
                path
            );
            return Ok(Vec::new());
        }

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
            timestamp: parse_json_timestamp(&value["timestamp"])
                .or_else(|| parse_json_timestamp(&value["ts"]))
                .unwrap_or_else(Utc::now),
            content: extract_json_content(&value),
            metadata: LogMetadata {
                model: value["model"].as_str().map(String::from),
                provider: value["provider"].as_str().map(String::from),
                tool_calls: parse_tool_calls(&value["tool_calls"]),
                user_messages: value["user_messages"].as_u64().unwrap_or(0) as usize,
                assistant_messages: value["assistant_messages"].as_u64().unwrap_or(0) as usize,
            },
        })
    }

    fn parse_markdown_logs(&self, content: &str, harness: &str) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();
        let fallback_time = Utc::now();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (timestamp, content) = extract_inline_timestamp(trimmed, fallback_time);
            entries.push(LogEntry {
                id: uuid::Uuid::new_v4().to_string(),
                harness: harness.to_string(),
                timestamp,
                content: content.to_string(),
                metadata: LogMetadata {
                    model: None,
                    provider: None,
                    tool_calls: vec![],
                    user_messages: 0,
                    assistant_messages: 0,
                },
            });
        }

        Ok(entries)
    }

    fn parse_plain_logs(&self, content: &str, harness: &str) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();
        let fallback_time = Utc::now();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (timestamp, content) = extract_inline_timestamp(trimmed, fallback_time);
            entries.push(LogEntry {
                id: uuid::Uuid::new_v4().to_string(),
                harness: harness.to_string(),
                timestamp,
                content: content.to_string(),
                metadata: LogMetadata {
                    model: None,
                    provider: None,
                    tool_calls: vec![],
                    user_messages: 0,
                    assistant_messages: 0,
                },
            });
        }

        Ok(entries)
    }
}

fn parse_json_timestamp(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    if let Some(text) = value.as_str() {
        return DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|dt| dt.with_timezone(&Utc));
    }
    if let Some(seconds) = value.as_i64() {
        // Heuristic: values larger than 1e12 are milliseconds.
        if seconds > 1_000_000_000_000 {
            return DateTime::from_timestamp_millis(seconds).map(|dt| dt.with_timezone(&Utc));
        }
        return DateTime::from_timestamp(seconds, 0).map(|dt| dt.with_timezone(&Utc));
    }
    None
}

fn extract_json_content(value: &serde_json::Value) -> String {
    for field in ["content", "message", "text", "body", "msg"] {
        if let Some(text) = value.get(field).and_then(|v| v.as_str()) {
            return text.to_string();
        }
    }
    String::new()
}

fn parse_tool_calls(value: &serde_json::Value) -> Vec<ToolCall> {
    value
        .as_array()
        .map(|array| {
            array
                .iter()
                .filter_map(|item| {
                    Some(ToolCall {
                        tool_name: item["tool_name"].as_str()?.to_string(),
                        parameters: item["parameters"].clone(),
                        result: item.get("result").cloned(),
                        duration_ms: item["duration_ms"].as_u64(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_inline_timestamp(line: &str, fallback: DateTime<Utc>) -> (DateTime<Utc>, &str) {
    if let Some(captures) = TIMESTAMP_PREFIX_RE.captures(line) {
        let ts_str = captures.name("ts").unwrap().as_str();
        let rest = captures.name("rest").map(|m| m.as_str()).unwrap_or("");

        if let Ok(dt) = DateTime::parse_from_rfc3339(ts_str) {
            return (dt.with_timezone(&Utc), rest);
        }
        if let Ok(naive) = NaiveDateTime::parse_from_str(ts_str, "%Y-%m-%d %H:%M:%S") {
            return (naive.and_utc(), rest);
        }
        if let Ok(naive) = NaiveDateTime::parse_from_str(ts_str, "%Y-%m-%dT%H:%M:%S") {
            return (naive.and_utc(), rest);
        }
    }
    (fallback, line)
}
