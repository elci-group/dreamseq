// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

pub struct KaptaindMonitor {
    project_path: PathBuf,
}

impl KaptaindMonitor {
    pub fn new(project_path: PathBuf) -> Self {
        Self { project_path }
    }

    /// Initialize kaptaind for the Dreamseq project
    pub fn init(&self) -> Result<()> {
        let status = Command::new("kaptaind-cli")
            .arg("init")
            .current_dir(&self.project_path)
            .status()?;

        if status.success() {
            tracing::info!(project = %self.project_path.display(), "Kaptaind initialized for Dreamseq project");
            Ok(())
        } else {
            anyhow::bail!("Failed to initialize kaptaind");
        }
    }

    /// Run kaptaind analysis
    pub fn analyze(&self) -> Result<String> {
        let output = Command::new("kaptaind-cli")
            .arg("analyze")
            .current_dir(&self.project_path)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Kaptaind analysis failed: {}", error);
        }
    }

    /// Check kaptaind daemon status
    pub fn status(&self) -> Result<String> {
        let output = Command::new("kaptaind-cli")
            .arg("status")
            .current_dir(&self.project_path)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to check kaptaind status: {}", error);
        }
    }

    /// Start kaptaind daemon
    pub fn start_daemon(&self) -> Result<()> {
        let status = Command::new("kaptaind")
            .arg("--daemon")
            .current_dir(&self.project_path)
            .status()?;

        if status.success() {
            tracing::info!(project = %self.project_path.display(), "Kaptaind daemon started");
            Ok(())
        } else {
            anyhow::bail!("Failed to start kaptaind daemon");
        }
    }

    /// Get recent commits from kaptaind
    pub fn get_commits(&self, limit: usize) -> Result<String> {
        let output = Command::new("kaptaind-cli")
            .args(["log", "--limit", &limit.to_string()])
            .current_dir(&self.project_path)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get kaptaind commits: {}", error);
        }
    }

    /// Create kaptaind configuration for Dreamseq
    pub fn create_config(&self) -> Result<()> {
        let config_content = r#"[watch]
path = "."
recursive = true
ignore_file = ".kaptainignore"

[cluster]
window = 5

[test]
command = "cargo test"
required = true

[push]
enabled = false
branch = "main"

[notify]
# Optional: Add webhook URL for notifications
# webhook_url = "https://hooks.slack.com/services/..."

[notify.tts]
enabled = false
provider = "auto"
"#;

        let config_path = self.project_path.join("kaptaind.toml");
        std::fs::write(&config_path, config_content)?;

        tracing::info!(path = %config_path.display(), "Kaptaind configuration created");
        Ok(())
    }

    /// Create .kaptainignore file
    pub fn create_ignore(&self) -> Result<()> {
        let ignore_content = r#"# Dreamseq output directories
output/
anthologies/

# Rust build artifacts
target/
Cargo.lock

# IDE
.vscode/
.idea/

# OS
.DS_Store
Thumbs.db
"#;

        let ignore_path = self.project_path.join(".kaptainignore");
        std::fs::write(&ignore_path, ignore_content)?;

        tracing::info!(path = %ignore_path.display(), "Kaptaind ignore file created");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, KaptaindMonitor) {
        let root = std::env::temp_dir().join(format!("dreamseq-kaptaind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("fixture directory should be created");
        let monitor = KaptaindMonitor::new(root.clone());
        (root, monitor)
    }

    #[test]
    fn creates_a_safe_project_configuration() {
        let (root, monitor) = fixture();
        monitor.create_config().expect("configuration should write");
        let config =
            std::fs::read_to_string(root.join("kaptaind.toml")).expect("configuration should read");
        assert!(config.contains("command = \"cargo test\""));
        assert!(config.contains("enabled = false"));
        assert!(!config.contains("hooks.slack.com/services/...\nwebhook_url"));
        std::fs::remove_dir_all(root).expect("fixture directory should be removable");
    }

    #[test]
    fn creates_ignore_rules_for_generated_and_local_files() {
        let (root, monitor) = fixture();
        monitor.create_ignore().expect("ignore file should write");
        let ignore =
            std::fs::read_to_string(root.join(".kaptainignore")).expect("ignore file should read");
        for expected in ["output/", "anthologies/", "target/", ".vscode/"] {
            assert!(ignore.lines().any(|line| line == expected));
        }
        std::fs::remove_dir_all(root).expect("fixture directory should be removable");
    }
}
