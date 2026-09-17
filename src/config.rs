// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamseqConfig {
    /// Loaded from legacy configuration files for compatibility, but never
    /// written back to disk. Prefer the `GROQ_API_KEY` environment variable.
    #[serde(default, skip_serializing)]
    pub groq_api_key: String,
    pub harnesses: Vec<HarnessConfig>,
    pub output_dir: PathBuf,
    pub anthologies_dir: PathBuf,
    pub enable_tts: bool,
    pub enable_kaptaind: bool,
    /// Explicit consent to send redacted log excerpts to Dreamsequence or a
    /// configured BYOK inference endpoint. Remote analysis is disabled by default.
    #[serde(default)]
    pub allow_remote_analysis: bool,
    /// When true and `allow_remote_analysis` is false, the pipeline will
    /// automatically grant one-time consent instead of prompting interactively.
    /// Intended for non-interactive use after reviewing the privacy implications.
    #[serde(default)]
    pub auto_approve_remote_analysis: bool,
    /// Legacy direct-provider override. It bypasses cloud routing and is kept
    /// for compatibility and local integration tests.
    #[serde(default)]
    pub groq_base_url: Option<String>,
    /// Whether the Dreamsequence cloud route is tried at all when this
    /// device is paired. Disabling it here (via `dreamseq config`) is the
    /// persisted equivalent of the `DREAMSEQ_DISABLE_CLOUD_INFERENCE`
    /// environment variable.
    #[serde(default = "default_true")]
    pub cloud_inference_enabled: bool,
    /// Whether BYOK providers may be tried when the cloud route is disabled,
    /// unpaired, or failing. Turning this off makes inference fail fast
    /// instead of falling through to a BYOK provider the user didn't expect.
    #[serde(default = "default_true")]
    pub byok_fallback_enabled: bool,
    /// BYOK providers configured explicitly via `dreamseq config`, in
    /// priority order. Non-empty, this list entirely replaces the implicit
    /// env-var auto-detection in `inference_providers::add_named_provider_routes`
    /// — see `configured_byok_routes` for why the two aren't merged.
    #[serde(default)]
    pub providers: Vec<ProviderSettings>,
    /// Account tier, gating how many per-device azusa drift scores dreamseq
    /// shows per repository: free shows 1, pro up to 4, enterprise averages
    /// up to 16 into a single collaboration score.
    #[serde(default)]
    pub tier: AccountTier,
    /// Display name for this device, attached to azusa drift observations so
    /// scores from a user's machines can be told apart. When unset, the OS
    /// hostname is used.
    #[serde(default)]
    pub device_name: Option<String>,
    /// Whether `dreamseq run` measures repository drift automatically with
    /// azusa (current repo plus every cached repo available locally) and
    /// uploads fresh scores when this device is paired. Defaults to `true`;
    /// disable to keep runs on log analysis alone.
    #[serde(default = "default_true")]
    pub enable_drift: bool,
}

/// Product tier of the account using this installation. Gates display limits
/// for shared features such as azusa drift scores (see `azusa::view_entry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AccountTier {
    #[default]
    Free,
    Pro,
    Enterprise,
}

impl AccountTier {
    /// How many local-repo drift scores per repo this tier may display.
    /// Enterprise gets the widest window, averaged in the view so
    /// collaboration does not produce a wall of largely meaningless numbers.
    #[must_use]
    pub fn max_local_scores(self) -> usize {
        match self {
            Self::Free => 1,
            Self::Pro => 4,
            Self::Enterprise => 16,
        }
    }
}

impl std::fmt::Display for AccountTier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Free => "free",
            Self::Pro => "pro",
            Self::Enterprise => "enterprise",
        };
        formatter.write_str(name)
    }
}

fn default_true() -> bool {
    true
}

/// A single BYOK provider, as the user configured it through the
/// `dreamseq config` TUI rather than through environment variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// Matches a well-known provider name from
    /// `GroqClient::known_provider_templates` (e.g. "openai", "openrouter"),
    /// or any custom label for a fully custom BYOK endpoint.
    pub name: String,
    pub enabled: bool,
    pub base_url: String,
    /// Model used for batches classified as heavy (the default tier).
    pub model: String,
    /// Cheaper/faster model used for batches classified as light. Also used
    /// to rank this provider ahead of ones without a light model for those
    /// batches — see `GroqClient`'s `ranked_byok_routes`.
    #[serde(default)]
    pub light_model: Option<String>,
    /// Stored directly (in this 0600 config file — see
    /// `fs_security::write_private_atomic`) so a provider can be configured
    /// end-to-end without exporting an environment variable. Left blank to
    /// fall back to that provider's well-known environment variable(s), if
    /// it has any and one is set.
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub protocol: crate::groq::Protocol,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    pub name: String,
    pub log_path: PathBuf,
    pub log_format: LogFormat,
    /// Optional Bound filter (e.g. "[.json]"). When `log_format` is `Bound`,
    /// this filter is passed to the `bound` binary. If omitted, Bound scans
    /// every non-hidden file in `log_path`.
    #[serde(default)]
    pub bound_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    #[serde(alias = "Json")]
    Json,
    #[serde(alias = "Markdown")]
    Markdown,
    #[serde(alias = "Plain")]
    Plain,
    #[serde(alias = "CodexSqlite")]
    CodexSqlite,
    #[serde(alias = "Bound")]
    Bound,
}

impl DreamseqConfig {
    pub fn path() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
            .join(".config")
            .join("dreamseq")
            .join("config.json"))
    }

    pub fn load() -> Result<Self> {
        let config_path = Self::path()?;

        if !config_path.exists() {
            let legacy_path = config_path.with_file_name("config.toml");
            if legacy_path.exists() {
                return Self::load_from_path(&legacy_path);
            }
            return Ok(Self::discover());
        }

        Self::load_from_path(&config_path)
    }

    /// Discover the local harnesses supported by the MVP. Missing locations are
    /// skipped so a new harness can be added without breaking existing runs.
    pub fn discover() -> Self {
        Self {
            harnesses: Self::discovered_harnesses(),
            ..Self::default()
        }
    }

    fn discovered_harnesses() -> Vec<HarnessConfig> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::discovered_harnesses_in(&home)
    }

    fn discovered_harnesses_in(home: &Path) -> Vec<HarnessConfig> {
        let candidates = [
            ("claude", home.join(".claude/telemetry"), LogFormat::Json),
            ("grok", home.join(".grok/logs"), LogFormat::Json),
            ("kimi", home.join(".kimi/logs"), LogFormat::Plain),
            ("kimi-code", home.join(".kimi-code/logs"), LogFormat::Plain),
            ("crush", home.join(".crush/logs"), LogFormat::Plain),
            ("factory", home.join(".factory/logs"), LogFormat::Plain),
            ("hermes", home.join(".hermes/logs"), LogFormat::Plain),
            ("openclaw", home.join(".openclaw/logs"), LogFormat::Plain),
            (
                "openclaw-workspace",
                home.join(".openclaw/workspace/memory"),
                LogFormat::Plain,
            ),
            (
                "gemini",
                home.join(".gemini/antigravity-cli/conversations"),
                LogFormat::Plain,
            ),
            (
                "codex",
                home.join(".codex/logs_2.sqlite"),
                LogFormat::CodexSqlite,
            ),
        ];

        candidates
            .into_iter()
            .filter(|(_, path, _)| path.exists())
            .map(|(name, log_path, log_format)| HarnessConfig {
                name: name.to_string(),
                log_path,
                log_format,
                bound_filter: None,
            })
            .collect()
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Self = match path.extension().and_then(|ext| ext.to_str()) {
            Some("toml") => toml::from_str(&content)?,
            _ => serde_json::from_str(&content)?,
        };
        // Keep credentials out of the config file while still honoring the
        // standard shell-based configuration used by the Groq CLI/API.
        if config.groq_api_key.trim().is_empty()
            && let Ok(api_key) = std::env::var("GROQ_API_KEY")
            && !api_key.trim().is_empty()
        {
            config.groq_api_key = api_key;
        }
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::path()?;
        self.save_to_path(&config_path)
    }

    pub fn save_to_path(&self, config_path: &std::path::Path) -> Result<()> {
        if let Some(config_dir) = config_path.parent() {
            std::fs::create_dir_all(config_dir)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        crate::fs_security::write_private_atomic(config_path, content.as_bytes())?;
        Ok(())
    }
}

impl Default for DreamseqConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            groq_api_key: std::env::var("GROQ_API_KEY").unwrap_or_default(),
            harnesses: vec![],
            output_dir: home.join("dreamseq").join("output"),
            anthologies_dir: home.join("dreamseq").join("anthologies"),
            enable_tts: false,
            enable_kaptaind: false,
            allow_remote_analysis: false,
            auto_approve_remote_analysis: false,
            groq_base_url: None,
            cloud_inference_enabled: true,
            byok_fallback_enabled: true,
            providers: vec![],
            tier: AccountTier::default(),
            device_name: None,
            enable_drift: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DreamseqConfig;

    #[test]
    fn discovery_finds_only_existing_harness_sources() {
        let root =
            std::env::temp_dir().join(format!("dreamseq-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".grok/logs")).unwrap();
        std::fs::create_dir_all(root.join(".kimi/logs")).unwrap();

        let harnesses = DreamseqConfig::discovered_harnesses_in(&root);
        assert_eq!(harnesses.len(), 2);
        assert!(harnesses.iter().any(|harness| harness.name == "grok"));
        assert!(harnesses.iter().any(|harness| harness.name == "kimi"));
        assert!(!harnesses.iter().any(|harness| harness.name == "codex"));

        std::fs::remove_dir_all(root).unwrap();
    }
}
