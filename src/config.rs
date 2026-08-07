use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamseqConfig {
    pub groq_api_key: String,
    pub harnesses: Vec<HarnessConfig>,
    pub output_dir: PathBuf,
    pub anthologies_dir: PathBuf,
    pub enable_tts: bool,
    pub enable_kaptaind: bool,
    /// Optional override for the Groq API base URL. Useful for testing against
    /// a local mock server.
    #[serde(default)]
    pub groq_base_url: Option<String>,
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
pub enum LogFormat {
    Json,
    Markdown,
    Plain,
    CodexSqlite,
    Bound,
    Custom(String),
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
        let config_dir = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
            .join(".config")
            .join("dreamseq");

        std::fs::create_dir_all(&config_dir)?;

        let config_path = Self::path()?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
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
            enable_kaptaind: true,
            groq_base_url: None,
        }
    }
}
