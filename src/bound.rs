use crate::aggregator::{LogEntry, LogMetadata};
use crate::config::HarnessConfig;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Output schema produced by `bound --json --meta`.
#[derive(Debug, Deserialize)]
pub struct BoundOutput {
    pub tree: Option<String>,
    pub files: Vec<BoundFile>,
}

#[derive(Debug, Deserialize)]
pub struct BoundFile {
    pub metadata: Option<BoundMetadata>,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct BoundMetadata {
    pub relative_path: String,
    pub size_bytes: u64,
    pub line_count: usize,
    pub modified_unix: i64,
    pub sha256: Option<String>,
}

pub struct BoundClient;

impl Default for BoundClient {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundClient {
    pub fn new() -> Self {
        Self
    }

    /// Aggregate files from a harness using the `bound` binary.
    ///
    /// Missing Bound installations are returned as contextual errors so the
    /// ingestion report can account for the rejected source.
    pub async fn aggregate(&self, harness: &HarnessConfig) -> Result<Vec<LogEntry>> {
        let binary = std::env::var("BOUND_BINARY").unwrap_or_else(|_| "bound".to_string());

        if std::process::Command::new(&binary)
            .arg("--version")
            .output()
            .is_err()
        {
            anyhow::bail!("bound binary not found for harness {}", harness.name);
        }

        // Use a private temp directory (0700) to avoid symlink races on
        // world-writable /tmp. Bound writes inside; we clean the directory.
        let temp_dir =
            std::env::temp_dir().join(format!("dreamseq-bound-{}", uuid::Uuid::new_v4()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&temp_dir)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(&temp_dir)?;
        }
        let output_path = temp_dir.join("out.json");
        let output_path_str = output_path.to_string_lossy().to_string();

        let mut command = std::process::Command::new(&binary);
        command.args(["--json", "--meta", "--out", &output_path_str]);

        if let Some(filter) = &harness.bound_filter {
            command.arg(filter);
            command.arg(&harness.log_path);
        } else {
            // No filter: run bound with the harness directory as the working
            // directory so it scans every non-hidden file there.
            command.current_dir(&harness.log_path);
        }

        let status = command.status()?;
        if !status.success() {
            if let Err(error) = std::fs::remove_dir_all(&temp_dir)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %temp_dir.display(), error = %error, "failed to remove Bound temp dir after command failure");
            }
            anyhow::bail!("bound failed for harness {}", harness.name);
        }

        let content_result = std::fs::read_to_string(&output_path);
        if let Err(error) = std::fs::remove_dir_all(&temp_dir) {
            tracing::warn!(path = %temp_dir.display(), error = %error, "failed to remove temporary Bound output");
        }
        let content = content_result?;

        let output: BoundOutput = serde_json::from_str(&content)?;

        Ok(output
            .files
            .into_iter()
            .filter_map(|file| {
                let entry = self.bound_file_to_log_entry(file, &harness.name);
                if entry.is_none() {
                    tracing::warn!(harness = %harness.name, "Bound record had no metadata and was rejected");
                }
                entry
            })
            .collect())
    }

    fn bound_file_to_log_entry(&self, file: BoundFile, harness: &str) -> Option<LogEntry> {
        let metadata = file.metadata?;
        let timestamp = DateTime::from_timestamp(metadata.modified_unix, 0)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Some(LogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            harness: harness.to_string(),
            timestamp,
            content: file.content,
            metadata: LogMetadata {
                model: None,
                provider: Some(format!("bound:{}", metadata.relative_path)),
                tool_calls: vec![],
                user_messages: 0,
                assistant_messages: 0,
            },
        })
    }
}

/// Convenience helper used by the main aggregator when it sees a
/// `LogFormat::Bound` harness.
pub async fn aggregate_bound_harness(harness: &HarnessConfig) -> Result<Vec<LogEntry>> {
    BoundClient::new().aggregate(harness).await
}
