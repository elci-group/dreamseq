// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use crate::config::HarnessConfig;
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::LazyLock;
use walkdir::WalkDir;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestionReport {
    pub harnesses: Vec<HarnessIngestion>,
    pub files_seen: usize,
    pub files_failed: usize,
    pub entries_accepted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessIngestion {
    pub harness: String,
    pub path: std::path::PathBuf,
    pub files_seen: usize,
    pub files_failed: usize,
    pub entries_accepted: usize,
    pub warnings: Vec<String>,
}

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
    .unwrap_or_else(|error| {
        tracing::error!(error = %error, pattern = "timestamp_prefix", "built-in regex compilation failed");
        panic!("invalid built-in timestamp regex")
    })
});

pub struct LogAggregator;

#[derive(Debug, Deserialize)]
struct CodexSqliteRow {
    ts: i64,
    feedback_log_body: String,
}

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
        Ok(self.aggregate_with_report(harnesses).await?.0)
    }

    pub async fn aggregate_with_report(
        &self,
        harnesses: &[HarnessConfig],
    ) -> Result<(Vec<LogEntry>, IngestionReport)> {
        let mut all_entries = Vec::new();
        let mut report = IngestionReport::default();

        for harness in harnesses {
            let (entries, harness_report) = self.aggregate_harness(harness).await;
            report.files_seen += harness_report.files_seen;
            report.files_failed += harness_report.files_failed;
            report.entries_accepted += harness_report.entries_accepted;
            report.harnesses.push(harness_report);
            all_entries.extend(entries);
        }

        all_entries.sort_by_key(|e| e.timestamp);
        Ok((all_entries, report))
    }

    async fn aggregate_harness(
        &self,
        harness: &HarnessConfig,
    ) -> (Vec<LogEntry>, HarnessIngestion) {
        let mut report = HarnessIngestion {
            harness: harness.name.clone(),
            path: harness.log_path.clone(),
            files_seen: 0,
            files_failed: 0,
            entries_accepted: 0,
            warnings: Vec::new(),
        };
        if !harness.log_path.exists() {
            let warning = format!("log path does not exist: {}", harness.log_path.display());
            tracing::warn!(
                harness = %harness.name,
                path = %harness.log_path.display(),
                "log path does not exist"
            );
            report.warnings.push(warning);
            return (Vec::new(), report);
        }

        if matches!(harness.log_format, crate::config::LogFormat::Bound) {
            report.files_seen = 1;
            return match crate::bound::aggregate_bound_harness(harness).await {
                Ok(entries) => {
                    report.entries_accepted = entries.len();
                    (entries, report)
                }
                // traci: allow -- branch emits harness, path, and error below.
                Err(error) => {
                    report.files_failed = 1;
                    report.warnings.push(error.to_string());
                    tracing::error!(
                        harness = %harness.name,
                        path = %harness.log_path.display(),
                        error = %error,
                        "Bound ingestion failed"
                    );
                    (Vec::new(), report)
                }
            };
        }

        let mut entries = Vec::new();
        for entry in WalkDir::new(&harness.log_path)
            .follow_links(false)
            .into_iter()
        {
            match entry {
                Ok(entry) if entry.file_type().is_file() => {
                    if is_binary_artifact(entry.path(), &harness.log_format) {
                        tracing::debug!(
                            harness = %harness.name,
                            path = %entry.path().display(),
                            "skipping known binary log artifact"
                        );
                        continue;
                    }
                    report.files_seen += 1;
                    match self.parse_log_file(entry.path(), harness).await {
                        Ok(log_entries) => entries.extend(log_entries),
                        // traci: allow -- branch emits harness, path, and error below.
                        Err(error) => {
                            report.files_failed += 1;
                            let warning = format!("{}: {error}", entry.path().display());
                            report.warnings.push(warning);
                            tracing::error!(
                                harness = %harness.name,
                                path = %entry.path().display(),
                                error = %error,
                                "failed to parse log file"
                            );
                        }
                    }
                }
                Ok(_) => {}
                // traci: allow -- traversal failures are recorded with harness and error.
                Err(error) => {
                    report.files_failed += 1;
                    report.warnings.push(error.to_string());
                    tracing::error!(
                        harness = %harness.name,
                        error = %error,
                        "failed to traverse log source"
                    );
                }
            }
        }

        report.entries_accepted = entries.len();
        (entries, report)
    }

    async fn parse_log_file(&self, path: &Path, harness: &HarnessConfig) -> Result<Vec<LogEntry>> {
        match &harness.log_format {
            crate::config::LogFormat::CodexSqlite => self.parse_codex_sqlite(path, &harness.name),
            format => {
                let content = std::fs::read_to_string(path)?;
                let fallback_timestamp = file_timestamp(path);
                match format {
                    crate::config::LogFormat::Json => {
                        self.parse_json_logs(&content, &harness.name, fallback_timestamp)
                    }
                    crate::config::LogFormat::Markdown => {
                        self.parse_markdown_logs(&content, &harness.name, fallback_timestamp)
                    }
                    crate::config::LogFormat::Plain => {
                        self.parse_plain_logs(&content, &harness.name, fallback_timestamp)
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
        if std::process::Command::new("sqlite3")
            .arg("--version")
            .output()
            .is_err()
        {
            anyhow::bail!(
                "sqlite3 is unavailable; cannot ingest Codex SQLite source {}",
                path.display()
            );
        }

        let output = std::process::Command::new("sqlite3")
            .arg("-json")
            .arg(path)
            .arg("SELECT ts, feedback_log_body FROM logs WHERE feedback_log_body IS NOT NULL ORDER BY ts")
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "sqlite3 could not read Codex logs: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let rows: Vec<CodexSqliteRow> = serde_json::from_slice(&output.stdout)
            .context("sqlite3 returned invalid JSON while reading Codex logs")?;
        let mut entries = Vec::with_capacity(rows.len());
        for (row_number, row) in rows.into_iter().enumerate() {
            let timestamp = chrono::DateTime::from_timestamp(row.ts, 0);
            let Some(timestamp) = timestamp else {
                tracing::warn!(
                    harness,
                    row_number,
                    timestamp = row.ts,
                    "invalid Codex timestamp"
                );
                continue;
            };
            entries.push(LogEntry {
                id: uuid::Uuid::new_v4().to_string(),
                harness: harness.to_string(),
                timestamp,
                content: row.feedback_log_body,
                metadata: LogMetadata {
                    model: None,
                    provider: Some("openai".to_string()),
                    tool_calls: vec![],
                    user_messages: 0,
                    assistant_messages: 0,
                },
            });
        }
        Ok(entries)
    }

    fn parse_json_logs(
        &self,
        content: &str,
        harness: &str,
        fallback_timestamp: DateTime<Utc>,
    ) -> Result<Vec<LogEntry>> {
        if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(content) {
            return Ok(values
                .into_iter()
                .map(|value| self.json_to_log_entry(value, harness, fallback_timestamp))
                .collect());
        }

        let mut entries = Vec::new();
        let mut rejected = 0usize;

        for (line_number, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(entry) => {
                    entries.push(self.json_to_log_entry(entry, harness, fallback_timestamp))
                }
                Err(error) => {
                    rejected += 1;
                    tracing::warn!(
                        harness,
                        line_number = line_number + 1,
                        error = %error,
                        "rejected malformed JSON log record"
                    );
                }
            }
        }

        if entries.is_empty() && rejected > 0 {
            anyhow::bail!("no valid JSON records; rejected {rejected} malformed records");
        }

        Ok(entries)
    }

    fn json_to_log_entry(
        &self,
        value: serde_json::Value,
        harness: &str,
        fallback_timestamp: DateTime<Utc>,
    ) -> LogEntry {
        LogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            harness: harness.to_string(),
            timestamp: parse_json_timestamp(&value["timestamp"])
                .or_else(|| parse_json_timestamp(&value["ts"]))
                .unwrap_or(fallback_timestamp),
            content: extract_json_content(&value),
            metadata: LogMetadata {
                model: value["model"].as_str().map(String::from),
                provider: value["provider"].as_str().map(String::from),
                tool_calls: parse_tool_calls(&value["tool_calls"]),
                user_messages: value["user_messages"].as_u64().unwrap_or(0) as usize,
                assistant_messages: value["assistant_messages"].as_u64().unwrap_or(0) as usize,
            },
        }
    }

    fn parse_markdown_logs(
        &self,
        content: &str,
        harness: &str,
        fallback_time: DateTime<Utc>,
    ) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();

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

    fn parse_plain_logs(
        &self,
        content: &str,
        harness: &str,
        fallback_time: DateTime<Utc>,
    ) -> Result<Vec<LogEntry>> {
        let mut entries = Vec::new();

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

fn is_binary_artifact(path: &Path, format: &crate::config::LogFormat) -> bool {
    if matches!(format, crate::config::LogFormat::CodexSqlite) {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let file_name = file_name.to_ascii_lowercase();
    [".db", ".db-wal", ".db-shm", ".pb"]
        .iter()
        .any(|suffix| file_name.ends_with(suffix))
}

fn file_timestamp(path: &Path) -> DateTime<Utc> {
    match std::fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(timestamp) => DateTime::<Utc>::from(timestamp),
        Err(error) => {
            tracing::warn!(path = %path.display(), error = %error, "using Unix epoch for log file timestamp");
            DateTime::<Utc>::UNIX_EPOCH
        }
    }
}

fn parse_json_timestamp(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    if let Some(text) = value.as_str() {
        // traci: allow -- inspect_err records the rejected timestamp and parser error.
        return DateTime::parse_from_rfc3339(text)
            .map(|dt| dt.with_timezone(&Utc))
            .inspect_err(|error| {
                tracing::warn!(timestamp = text, error = %error, "invalid JSON timestamp");
            })
            // traci: allow -- inspect_err above records the rejected value and parser failure.
            .ok();
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
        let Some(ts_match) = captures.name("ts") else {
            return (fallback, line);
        };
        let ts_str = ts_match.as_str();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_sqlite_preserves_multiline_feedback_rows() {
        if std::process::Command::new("sqlite3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let path = std::env::temp_dir().join(format!(
            "dreamseq-codex-sqlite-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let sql = "CREATE TABLE logs (ts INTEGER NOT NULL, feedback_log_body TEXT);\
                   INSERT INTO logs VALUES (1710000000, 'first line\nsecond line\twith tab — done');";
        let status = std::process::Command::new("sqlite3")
            .arg(&path)
            .arg(sql)
            .status()
            .expect("sqlite3 should create the fixture database");
        assert!(status.success());

        let result = LogAggregator::new().parse_codex_sqlite(&path, "codex");
        let _ = std::fs::remove_file(&path);
        let entries = result.expect("the multiline row should parse");

        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].content,
            "first line\nsecond line\twith tab — done"
        );
        assert_eq!(entries[0].timestamp.timestamp(), 1_710_000_000);
    }

    #[test]
    fn skips_known_binary_artifacts_for_text_formats() {
        use crate::config::LogFormat;

        for name in [
            "conversation.db",
            "conversation.db-wal",
            "conversation.db-shm",
            "conversation.pb",
        ] {
            assert!(is_binary_artifact(Path::new(name), &LogFormat::Plain));
            assert!(is_binary_artifact(Path::new(name), &LogFormat::Json));
        }
        assert!(!is_binary_artifact(
            Path::new("conversation.log"),
            &LogFormat::Plain
        ));
        assert!(!is_binary_artifact(
            Path::new("conversation.db"),
            &LogFormat::CodexSqlite
        ));
    }

    #[tokio::test]
    async fn ignores_database_artifacts_during_plain_ingestion() {
        let root = std::env::temp_dir().join(format!(
            "dreamseq-gemini-artifacts-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("conversation"), "a valid text entry\n").unwrap();
        for name in [
            "conversation.db",
            "conversation.db-wal",
            "conversation.db-shm",
            "conversation.pb",
        ] {
            std::fs::write(root.join(name), [0, 159, 146, 150]).unwrap();
        }

        let (entries, report) = LogAggregator::new()
            .aggregate_with_report(&[crate::config::HarnessConfig {
                name: "gemini".to_string(),
                log_path: root.clone(),
                log_format: crate::config::LogFormat::Plain,
                bound_filter: None,
            }])
            .await
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(report.files_seen, 1);
        assert_eq!(report.files_failed, 0);
        assert!(report.harnesses[0].warnings.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
