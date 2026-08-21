// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use dreamseq::cloud::{CloudClient, CredentialStore, Credentials, DeviceAuthorization};
use dreamseq::report::Anthology;
use dreamseq::{Dreamseq, DreamseqConfig};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Manager, State};

/// Mobile-facing view of the current configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileConfig {
    pub harnesses: Vec<dreamseq::config::HarnessConfig>,
    pub output_dir: PathBuf,
    pub anthologies_dir: PathBuf,
    pub allow_remote_analysis: bool,
    pub auto_approve_remote_analysis: bool,
}

impl From<&DreamseqConfig> for MobileConfig {
    fn from(config: &DreamseqConfig) -> Self {
        Self {
            harnesses: config.harnesses.clone(),
            output_dir: config.output_dir.clone(),
            anthologies_dir: config.anthologies_dir.clone(),
            allow_remote_analysis: config.allow_remote_analysis,
            auto_approve_remote_analysis: config.auto_approve_remote_analysis,
        }
    }
}

impl From<MobileConfig> for DreamseqConfig {
    fn from(mobile: MobileConfig) -> Self {
        Self {
            harnesses: mobile.harnesses,
            output_dir: mobile.output_dir,
            anthologies_dir: mobile.anthologies_dir,
            allow_remote_analysis: mobile.allow_remote_analysis,
            auto_approve_remote_analysis: mobile.auto_approve_remote_analysis,
            enable_tts: false,
            enable_kaptaind: false,
            groq_api_key: std::env::var("GROQ_API_KEY").unwrap_or_default(),
            groq_base_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AnthologySummary {
    pub id: String,
    pub date: String,
    pub generated_at: DateTime<Utc>,
    pub executive_summary: String,
    pub candidate_tools_count: usize,
}

impl From<&Anthology> for AnthologySummary {
    fn from(anthology: &Anthology) -> Self {
        Self {
            id: anthology.id.clone(),
            date: anthology.date.clone(),
            generated_at: anthology.generated_at,
            executive_summary: anthology.executive_summary.clone(),
            candidate_tools_count: anthology.candidate_tools.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountInfo {
    pub account_id: String,
    pub device_id: String,
    pub api_url: String,
}

impl From<&Credentials> for AccountInfo {
    fn from(credentials: &Credentials) -> Self {
        Self {
            account_id: credentials.account_id.clone(),
            device_id: credentials.device_id.clone(),
            api_url: credentials.api_url.clone(),
        }
    }
}

pub struct MobileState {
    config_path: PathBuf,
    credentials_path: PathBuf,
    config: Mutex<DreamseqConfig>,
}

impl MobileState {
    fn new(app_handle: &tauri::AppHandle) -> Result<Self> {
        let app_dir = app_handle
            .path()
            .app_local_data_dir()
            .context("cannot determine app local data directory")?;
        let config_path = app_dir.join("config.json");
        let credentials_path = app_dir.join("credentials.json");

        let config = if config_path.exists() {
            DreamseqConfig::load_from_path(&config_path)?
        } else {
            DreamseqConfig {
                output_dir: app_dir.join("output"),
                anthologies_dir: app_dir.join("anthologies"),
                ..Default::default()
            }
        };

        Ok(Self {
            config_path,
            credentials_path,
            config: Mutex::new(config),
        })
    }

    fn config(&self) -> DreamseqConfig {
        self.config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_config(&self, config: DreamseqConfig) -> Result<()> {
        let config_dir = self
            .config_path
            .parent()
            .context("mobile configuration path has no parent directory")?;
        std::fs::create_dir_all(config_dir)?;
        let content = serde_json::to_string_pretty(&config)?;
        std::fs::write(&self.config_path, content)?;
        *self
            .config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        Ok(())
    }

    fn credential_store(&self) -> CredentialStore {
        CredentialStore::at(&self.credentials_path)
    }

    fn default_mobile_log_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("logs")
    }
}

fn discover_harnesses_in(log_dir: &Path) -> Vec<dreamseq::config::HarnessConfig> {
    let formats = [
        ("mobile-json", dreamseq::config::LogFormat::Json),
        ("mobile-plain", dreamseq::config::LogFormat::Plain),
        ("mobile-markdown", dreamseq::config::LogFormat::Markdown),
    ];
    formats
        .iter()
        .filter_map(|(name, format)| {
            let path = log_dir.join(name);
            path.exists().then(|| dreamseq::config::HarnessConfig {
                name: (*name).to_string(),
                log_path: path,
                log_format: format.clone(),
                bound_filter: None,
            })
        })
        .collect()
}

fn list_anthologies_in(anthologies_dir: &Path) -> Result<Vec<AnthologySummary>> {
    if !anthologies_dir.exists() {
        return Ok(Vec::new());
    }
    let mut summaries = Vec::new();
    for entry in std::fs::read_dir(anthologies_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read anthology {}", path.display()))?;
        match serde_json::from_str::<Anthology>(&content) {
            Ok(anthology) => summaries.push(AnthologySummary::from(&anthology)),
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "ignoring invalid anthology")
            }
        }
    }
    summaries.sort_by_key(|summary| std::cmp::Reverse(summary.generated_at));
    Ok(summaries)
}

fn load_anthology_in(anthologies_dir: &Path, id: &str) -> Result<Option<Anthology>> {
    if !anthologies_dir.exists() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(anthologies_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read anthology {}", path.display()))?;
        match serde_json::from_str::<Anthology>(&content) {
            Ok(anthology) if anthology.id == id => return Ok(Some(anthology)),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "ignoring invalid anthology")
            }
        }
    }
    Ok(None)
}

#[tauri::command]
fn load_config(state: State<'_, MobileState>) -> Result<MobileConfig, String> {
    Ok(MobileConfig::from(&state.config()))
}

#[tauri::command]
fn save_config(
    state: State<'_, MobileState>,
    config: MobileConfig,
) -> Result<MobileConfig, String> {
    let config: DreamseqConfig = config.into();
    state
        .set_config(config.clone())
        .map_err(|e| e.to_string())?;
    Ok(MobileConfig::from(&config))
}

#[tauri::command]
fn discover_harnesses(state: State<'_, MobileState>) -> Vec<dreamseq::config::HarnessConfig> {
    discover_harnesses_in(&state.default_mobile_log_dir())
}

#[tauri::command]
async fn run_analysis(state: State<'_, MobileState>) -> Result<AnthologySummary, String> {
    let config = state.config();
    let mut dreamseq = Dreamseq::new(config).map_err(|e| e.to_string())?;
    let mut anthology = dreamseq.run().await.map_err(|e| e.to_string())?;
    anthology.generate().map_err(|e| e.to_string())?;
    let summary = AnthologySummary::from(&anthology);
    anthology.save().map_err(|e| e.to_string())?;
    Ok(summary)
}

#[tauri::command]
fn list_anthologies(state: State<'_, MobileState>) -> Result<Vec<AnthologySummary>, String> {
    let anthologies_dir = state.config().anthologies_dir;
    list_anthologies_in(&anthologies_dir).map_err(|error| error.to_string())
}

#[tauri::command]
fn load_anthology(state: State<'_, MobileState>, id: String) -> Result<Option<Anthology>, String> {
    let anthologies_dir = state.config().anthologies_dir;
    load_anthology_in(&anthologies_dir, &id).map_err(|error| error.to_string())
}

#[tauri::command]
async fn request_authorization(api_url: Option<String>) -> Result<DeviceAuthorization, String> {
    let client = CloudClient::new(api_url.as_deref()).map_err(|e| e.to_string())?;
    client
        .request_authorization()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn complete_pairing(
    state: State<'_, MobileState>,
    api_url: Option<String>,
    device_code: String,
    expires_in: u64,
    interval: u64,
) -> Result<AccountInfo, String> {
    let client = CloudClient::new(api_url.as_deref()).map_err(|e| e.to_string())?;
    let store = state.credential_store();
    let credentials = client
        .poll_token(&device_code, expires_in, interval, &store)
        .await
        .map_err(|e| e.to_string())?;
    Ok(AccountInfo::from(&credentials))
}

#[tauri::command]
async fn cloud_status(state: State<'_, MobileState>) -> Result<Option<AccountInfo>, String> {
    let store = state.credential_store();
    match store.load_optional().map_err(|e| e.to_string())? {
        Some(credentials) => Ok(Some(AccountInfo::from(&credentials))),
        None => Ok(None),
    }
}

#[tauri::command]
async fn logout(state: State<'_, MobileState>) -> Result<(), String> {
    let store = state.credential_store();
    if let Some(credentials) = store.load_optional().map_err(|e| e.to_string())? {
        let client = CloudClient::new(Some(&credentials.api_url)).map_err(|e| e.to_string())?;
        client
            .revoke(&credentials)
            .await
            .map_err(|e| e.to_string())?;
        store.remove().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn sync_with_cloud(
    state: State<'_, MobileState>,
) -> Result<dreamseq::cloud::SyncSummary, String> {
    let store = state.credential_store();
    let credentials = store.load().map_err(|e| e.to_string())?;
    let client = CloudClient::new(Some(&credentials.api_url)).map_err(|e| e.to_string())?;
    let config = state.config();
    let directories = vec![config.anthologies_dir, config.output_dir];
    client
        .sync_directories(&credentials, &directories)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = MobileState::new(app.app_handle())?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            discover_harnesses,
            run_analysis,
            list_anthologies,
            load_anthology,
            request_authorization,
            complete_pairing,
            cloud_status,
            logout,
            sync_with_cloud,
            open_url,
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        tracing::error!(error = %error, "Tauri application terminated with an error");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_directory(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dreamseq-mobile-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("fixture directory should be created");
        root
    }

    #[test]
    fn mobile_config_round_trips_to_dreamseq_config() {
        let mobile = MobileConfig {
            harnesses: vec![dreamseq::config::HarnessConfig {
                name: "mobile-json".to_string(),
                log_path: PathBuf::from("/tmp/logs/mobile-json"),
                log_format: dreamseq::config::LogFormat::Json,
                bound_filter: None,
            }],
            output_dir: PathBuf::from("/tmp/output"),
            anthologies_dir: PathBuf::from("/tmp/anthologies"),
            allow_remote_analysis: true,
            auto_approve_remote_analysis: true,
        };
        let dreamseq_config: DreamseqConfig = mobile.clone().into();
        assert!(dreamseq_config.allow_remote_analysis);
        assert!(dreamseq_config.auto_approve_remote_analysis);
        assert_eq!(dreamseq_config.harnesses.len(), 1);

        let back = MobileConfig::from(&dreamseq_config);
        assert_eq!(back.allow_remote_analysis, mobile.allow_remote_analysis);
        assert_eq!(
            back.auto_approve_remote_analysis,
            mobile.auto_approve_remote_analysis
        );
        assert_eq!(back.harnesses.len(), mobile.harnesses.len());
    }

    #[test]
    fn anthology_summary_counts_tools() {
        let config = DreamseqConfig::default();
        let mut anthology = Anthology::new(vec![], vec![], config);
        anthology.executive_summary = "test summary".to_string();
        anthology
            .candidate_tools
            .push(dreamseq::report::CandidateTool {
                id: "DS-0001".to_string(),
                name: "test-tool".to_string(),
                priority: dreamseq::report::Priority::High,
                category: dreamseq::report::InterventionCategory::MissingCapability,
                reason: "because".to_string(),
                estimated_time_saved: "10 min/day".to_string(),
                confidence: 0.9,
                affected_projects: vec![],
                existing_matches: vec![],
                mutation_fitness: 0.0,
                capability_overlap: 0.0,
                implementation_cost: "low".to_string(),
            });
        let summary = AnthologySummary::from(&anthology);
        assert_eq!(summary.candidate_tools_count, 1);
        assert_eq!(summary.executive_summary, "test summary");
    }

    #[test]
    fn discovers_only_existing_mobile_log_formats() {
        let root = temp_directory("harnesses");
        std::fs::create_dir_all(root.join("mobile-json"))
            .expect("JSON log directory should be created");
        std::fs::create_dir_all(root.join("mobile-markdown"))
            .expect("Markdown log directory should be created");
        let harnesses = discover_harnesses_in(&root);
        assert_eq!(harnesses.len(), 2);
        assert_eq!(harnesses[0].name, "mobile-json");
        assert_eq!(harnesses[1].name, "mobile-markdown");
        std::fs::remove_dir_all(root).expect("fixture directory should be removable");
    }

    #[test]
    fn lists_and_loads_valid_anthologies_in_newest_first_order() {
        let root = temp_directory("anthologies");
        let mut older = Anthology::new(vec![], vec![], DreamseqConfig::default());
        older.generated_at = Utc::now() - chrono::Duration::hours(1);
        let mut newer = older.clone();
        newer.id = uuid::Uuid::new_v4().to_string();
        newer.generated_at = Utc::now();
        for (name, anthology) in [("older.json", &older), ("newer.json", &newer)] {
            std::fs::write(
                root.join(name),
                serde_json::to_vec(anthology).expect("anthology should serialize"),
            )
            .expect("anthology fixture should write");
        }
        std::fs::write(root.join("broken.json"), b"not-json")
            .expect("invalid fixture should write");
        std::fs::write(root.join("ignored.txt"), b"not-json")
            .expect("ignored fixture should write");

        let summaries = list_anthologies_in(&root).expect("listing should succeed");
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].id, newer.id);
        assert_eq!(
            load_anthology_in(&root, &older.id)
                .expect("lookup should succeed")
                .expect("anthology should exist")
                .id,
            older.id
        );
        assert!(
            load_anthology_in(&root, "missing")
                .expect("missing lookup should succeed")
                .is_none()
        );
        std::fs::remove_dir_all(root).expect("fixture directory should be removable");
    }

    #[test]
    fn anthology_helpers_accept_missing_directories() {
        let root =
            std::env::temp_dir().join(format!("dreamseq-mobile-missing-{}", uuid::Uuid::new_v4()));
        assert!(
            list_anthologies_in(&root)
                .expect("missing directory should not error")
                .is_empty()
        );
        assert!(
            load_anthology_in(&root, "missing")
                .expect("missing directory should not error")
                .is_none()
        );
    }
}
