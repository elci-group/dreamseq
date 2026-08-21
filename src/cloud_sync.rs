// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use super::{CloudClient, Credentials};
use crate::report::Anthology;
use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct SyncSummary {
    pub uploaded: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl CloudClient {
    // traci: allow -- compatibility wrapper creates and propagates a trace_id.
    pub async fn sync_directories(
        &self,
        credentials: &Credentials,
        directories: &[PathBuf],
    ) -> Result<SyncSummary> {
        let trace_id = crate::telemetry::new_trace_id();
        self.sync_directories_with_trace_id(credentials, directories, &trace_id)
            .await
    }

    #[tracing::instrument(skip_all, fields(trace_id = %trace_id))]
    pub async fn sync_directories_with_trace_id(
        &self,
        credentials: &Credentials,
        directories: &[PathBuf],
        trace_id: &str,
    ) -> Result<SyncSummary> {
        let mut summary = SyncSummary::default();
        for directory in directories.iter().filter(|directory| directory.exists()) {
            self.sync_directory(credentials, directory, trace_id, &mut summary)
                .await;
        }
        Ok(summary)
    }

    async fn sync_directory(
        &self,
        credentials: &Credentials,
        directory: &PathBuf,
        trace_id: &str,
        summary: &mut SyncSummary,
    ) {
        for entry in WalkDir::new(directory).follow_links(false) {
            let entry = match entry {
                Ok(entry) if entry.file_type().is_file() => entry,
                Ok(_) => continue,
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(error = %error, "could not inspect sync directory entry");
                    continue;
                }
            };
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let anthology = match read_anthology(entry.path()) {
                Ok(Some(anthology)) => anthology,
                Ok(None) => {
                    summary.skipped += 1;
                    continue;
                }
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(path = %entry.path().display(), error = %error, "could not read sync candidate");
                    continue;
                }
            };
            match self
                .upload_with_trace_id(credentials, &anthology, trace_id)
                .await
            {
                Ok(()) => summary.uploaded += 1,
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(path = %entry.path().display(), error = %error, "could not upload anthology");
                }
            }
        }
    }
}

fn read_anthology(path: &std::path::Path) -> Result<Option<Anthology>> {
    let content = fs::read(path)?;
    match serde_json::from_slice::<Anthology>(&content) {
        Ok(anthology) => Ok(Some(anthology)),
        Err(error) => {
            // Non-anthology JSON is expected in directories that contain mixed exports.
            tracing::debug!(path = %path.display(), error = %error, "skipping non-anthology JSON");
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_anthologies_from_other_json_and_io_errors() {
        let root = std::env::temp_dir().join(format!("dreamseq-sync-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let anthology_path = root.join("anthology.json");
        let other_path = root.join("other.json");
        let missing_path = root.join("missing.json");
        let anthology = Anthology::new(vec![], vec![], crate::config::DreamseqConfig::default());
        fs::write(&anthology_path, serde_json::to_vec(&anthology).unwrap()).unwrap();
        fs::write(&other_path, br#"{"kind":"unrelated"}"#).unwrap();

        assert!(read_anthology(&anthology_path).unwrap().is_some());
        assert!(read_anthology(&other_path).unwrap().is_none());
        assert!(read_anthology(&missing_path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
