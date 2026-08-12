use crate::goblin_gateway::{GatewayDecision, validate_run_envelope};
use crate::report::{Anthology, CandidateTool, Priority};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use walkdir::WalkDir;

const LEGACY_API_URL: &str = "https://dreamsequence.pro";
const DEFAULT_API_URL: &str = "https://padagonia.dreamsequence.pro/dreamsequence";
const SCHEMA_VERSION: u8 = 1;

#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub api_url: String,
    pub access_token: String,
    pub account_id: String,
    pub device_id: String,
    pub paired_at: DateTime<Utc>,
}

pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn discover() -> Result<Self> {
        if let Some(path) = std::env::var_os("DREAMSEQ_CREDENTIALS_PATH") {
            return Ok(Self::at(path));
        }
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
        Ok(Self::at(
            home.join(".config")
                .join("dreamseq")
                .join("credentials.json"),
        ))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<Credentials> {
        let bytes = fs::read(&self.path).with_context(|| {
            format!(
                "Dreamseq is not paired. Run `dreamseq login` first ({})",
                self.path.display()
            )
        })?;
        serde_json::from_slice(&bytes).context("stored Dreamsequence credentials are invalid")
    }

    pub fn load_optional(&self) -> Result<Option<Credentials>> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes)
                    .context("stored Dreamsequence credentials are invalid")?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %self.path.display(), "Dreamsequence credentials are not present");
                Ok(None)
            }
            Err(error) => {
                tracing::error!(path = %self.path.display(), error = %error, "failed to read Dreamsequence credentials");
                Err(error.into())
            }
        }
    }

    pub fn save(&self, credentials: &Credentials) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("credentials path has no parent"))?;
        fs::create_dir_all(parent)?;
        crate::fs_security::write_private_atomic(
            &self.path,
            &serde_json::to_vec_pretty(credentials)?,
        )?;
        Ok(())
    }

    pub fn remove(&self) -> Result<bool> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %self.path.display(), "Dreamsequence credentials were already absent");
                Ok(false)
            }
            Err(error) => {
                tracing::error!(path = %self.path.display(), error = %error, "failed to remove Dreamsequence credentials");
                Err(error.into())
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Serialize)]
struct DevicePoll<'a> {
    device_code: &'a str,
}

#[derive(Debug, Deserialize)]
struct DeviceToken {
    access_token: String,
    account_id: String,
    device_id: String,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RunEnvelope {
    schema_version: u8,
    run: CloudRun,
}

#[derive(Debug, Serialize)]
struct CloudRun {
    id: String,
    generated_at: DateTime<Utc>,
    source: CloudSource,
    summary: String,
    pipeline: CloudPipeline,
    opportunities: Vec<CloudOpportunity>,
}

#[derive(Debug, Serialize)]
struct CloudSource {
    id: String,
    cli_version: &'static str,
}

#[derive(Debug, Serialize)]
struct CloudPipeline {
    raw_entries: usize,
    normalized_entries: usize,
    segments: usize,
    estimated_input_tokens: usize,
}

#[derive(Debug, Serialize)]
struct CloudOpportunity {
    id: String,
    title: String,
    summary: String,
    priority: &'static str,
    confidence: u8,
    estimated_hours: f64,
    evidence_count: usize,
    repositories: usize,
    projects: Vec<String>,
    decision: &'static str,
}

impl RunEnvelope {
    pub fn from_anthology(anthology: &Anthology, source_id: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            run: CloudRun {
                id: anthology.id.clone(),
                generated_at: anthology.generated_at,
                source: CloudSource {
                    id: source_id.to_string(),
                    cli_version: env!("CARGO_PKG_VERSION"),
                },
                summary: anthology.executive_summary.clone(),
                pipeline: CloudPipeline {
                    raw_entries: anthology.pipeline.raw_entries,
                    normalized_entries: anthology.pipeline.normalized_entries,
                    segments: anthology.pipeline.segments,
                    estimated_input_tokens: anthology.pipeline.estimated_input_tokens,
                },
                opportunities: anthology
                    .candidate_tools
                    .iter()
                    .map(CloudOpportunity::from)
                    .collect(),
            },
        }
    }
}

impl From<&CandidateTool> for CloudOpportunity {
    fn from(tool: &CandidateTool) -> Self {
        let projects = if tool.existing_matches.is_empty() {
            tool.affected_projects.clone()
        } else {
            tool.existing_matches.clone()
        };
        Self {
            id: tool.id.clone(),
            title: tool.name.clone(),
            summary: tool.reason.clone(),
            priority: match tool.priority {
                Priority::High => "high",
                Priority::Medium => "medium",
                Priority::Low => "low",
            },
            confidence: (tool.confidence.clamp(0.0, 1.0) * 100.0).round() as u8,
            estimated_hours: parse_hours(&tool.estimated_time_saved),
            evidence_count: 1,
            repositories: projects.len(),
            projects,
            decision: if tool.existing_matches.is_empty() {
                "generate"
            } else {
                "extend"
            },
        }
    }
}

pub struct CloudClient {
    base_url: String,
    http: Client,
}

impl CloudClient {
    pub fn new(api_url: Option<&str>) -> Result<Self> {
        let base_url = normalize_api_url(
            api_url
                .map(effective_api_url)
                .map(str::to_owned)
                // traci: allow -- an absent optional environment override is expected control flow.
                .or_else(|| std::env::var("DREAMSEQUENCE_API_URL").ok())
                .unwrap_or_else(|| DEFAULT_API_URL.to_string()),
        )?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("dreamseq/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base_url, http })
    }

    pub async fn pair(&self, store: &CredentialStore, open: bool) -> Result<Credentials> {
        let response = self
            .http
            .post(format!("{}/api/v1/device/authorize", self.base_url))
            .json(&serde_json::json!({ "client": "dreamseq-cli" }))
            .send()
            .await?;
        let authorization: DeviceAuthorization = parse_response(response).await?;

        println!("Open {}", authorization.verification_uri);
        println!("Enter code: {}", authorization.user_code);
        if open && !open_browser(&authorization.verification_uri_complete) {
            eprintln!("Could not open a browser automatically. Use the URL above.");
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs(authorization.expires_in);
        let interval = Duration::from_secs(authorization.interval.max(2));
        loop {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("device code expired; run `dreamseq login` again");
            }
            tokio::time::sleep(interval).await;
            let response = self
                .http
                .post(format!("{}/api/v1/device/token", self.base_url))
                .json(&DevicePoll {
                    device_code: &authorization.device_code,
                })
                .send()
                .await?;
            if response.status() == StatusCode::ACCEPTED {
                continue;
            }
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                tokio::time::sleep(interval).await;
                continue;
            }
            let token: DeviceToken = parse_response(response).await?;
            let credentials = Credentials {
                api_url: self.base_url.clone(),
                access_token: token.access_token,
                account_id: token.account_id,
                device_id: token.device_id,
                paired_at: Utc::now(),
            };
            store.save(&credentials)?;
            return Ok(credentials);
        }
    }

    pub async fn revoke(&self, credentials: &Credentials) -> Result<()> {
        let response = self
            .http
            .post(format!("{}/api/v1/device/logout", self.base_url))
            .bearer_auth(&credentials.access_token)
            .send()
            .await?;
        let _: serde_json::Value = parse_response(response).await?;
        Ok(())
    }

    pub async fn upload(&self, credentials: &Credentials, anthology: &Anthology) -> Result<()> {
        let envelope = RunEnvelope::from_anthology(anthology, &credentials.device_id);
        let envelope_value = serde_json::to_value(&envelope)?;
        let decision = validate_run_envelope(&envelope_value, 0.90);
        tracing::info!(decision = %decision, "Goblin run-envelope preflight completed");
        if std::env::var("DREAMSEQ_GOBLIN_ENFORCE").as_deref() == Ok("1")
            && !matches!(decision, GatewayDecision::Accept)
        {
            anyhow::bail!("Goblin rejected the run envelope: {decision}");
        }
        let response = self
            .http
            .post(format!("{}/api/v1/runs", self.base_url))
            .bearer_auth(&credentials.access_token)
            .json(&envelope)
            .send()
            .await?;
        let _: serde_json::Value = parse_response(response).await?;
        Ok(())
    }

    pub async fn sync_directories(
        &self,
        credentials: &Credentials,
        directories: &[PathBuf],
    ) -> Result<SyncSummary> {
        let mut summary = SyncSummary::default();
        for directory in directories {
            if !directory.exists() {
                continue;
            }
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
                let content = match fs::read(entry.path()) {
                    Ok(content) => content,
                    Err(error) => {
                        summary.failed += 1;
                        tracing::warn!(path = %entry.path().display(), error = %error, "could not read sync candidate");
                        continue;
                    }
                };
                let anthology = match serde_json::from_slice::<Anthology>(&content) {
                    Ok(anthology) => anthology,
                    Err(_) => {
                        summary.skipped += 1;
                        continue;
                    }
                };
                match self.upload(credentials, &anthology).await {
                    Ok(()) => summary.uploaded += 1,
                    Err(error) => {
                        summary.failed += 1;
                        tracing::warn!(path = %entry.path().display(), error = %error, "could not upload anthology");
                    }
                }
            }
        }
        Ok(summary)
    }
}

#[derive(Debug, Default, Serialize)]
pub struct SyncSummary {
    pub uploaded: usize,
    pub skipped: usize,
    pub failed: usize,
}

async fn parse_response<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        let error = serde_json::from_slice::<ApiError>(&bytes).unwrap_or(ApiError {
            error: None,
            message: None,
        });
        anyhow::bail!(
            "Dreamsequence API returned {}: {}",
            status,
            error
                .message
                .or(error.error)
                .unwrap_or_else(|| "request failed".to_string())
        );
    }
    serde_json::from_slice(&bytes).context("Dreamsequence API returned invalid JSON")
}

fn normalize_api_url(value: String) -> Result<String> {
    let value = value.trim_end_matches('/').to_string();
    let local = value.starts_with("http://127.0.0.1") || value.starts_with("http://localhost");
    if !value.starts_with("https://") && !local {
        anyhow::bail!("Dreamsequence API URLs must use HTTPS (localhost is allowed for testing)");
    }
    Ok(value)
}

pub(crate) fn effective_api_url(value: &str) -> &str {
    if value.trim_end_matches('/') == LEGACY_API_URL {
        DEFAULT_API_URL
    } else {
        value
    }
}

fn parse_hours(value: &str) -> f64 {
    value
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .find_map(|part| {
            (!part.is_empty())
                // traci: allow -- non-numeric fragments are expected while scanning prose.
                .then(|| part.parse::<f64>().ok())
                .flatten()
        })
        .unwrap_or(0.0)
}

fn open_browser(url: &str) -> bool {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("open", &[])]
    } else if cfg!(target_os = "windows") {
        &[("cmd", &["/C", "start", ""])]
    } else {
        &[("xdg-open", &[])]
    };
    candidates.iter().any(|(program, arguments)| {
        Command::new(program)
            .args(*arguments)
            .arg(url)
            .spawn()
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DreamseqConfig;
    use crate::report::{InterventionCategory, PipelineStats};

    #[test]
    fn credentials_are_private_and_removable() {
        let root = std::env::temp_dir().join(format!("dreamseq-cloud-{}", uuid::Uuid::new_v4()));
        let store = CredentialStore::at(root.join("credentials.json"));
        let credentials = Credentials {
            api_url: "https://example.com".into(),
            access_token: "ds_secret".into(),
            account_id: "user_1".into(),
            device_id: "device_1".into(),
            paired_at: Utc::now(),
        };
        store.save(&credentials).unwrap();
        assert_eq!(store.load().unwrap().access_token, "ds_secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(store.remove().unwrap());
        assert!(!store.remove().unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cloud_envelope_excludes_local_configuration_and_raw_events() {
        let config = DreamseqConfig {
            groq_api_key: "gsk_must_not_leave_machine".into(),
            output_dir: PathBuf::from("/private/output"),
            ..DreamseqConfig::default()
        };
        let mut anthology = Anthology::new(Vec::new(), Vec::new(), config);
        anthology.pipeline = PipelineStats {
            raw_entries: 12,
            normalized_entries: 10,
            segments: 3,
            estimated_input_tokens: 800,
        };
        anthology.executive_summary = "Useful aggregate summary".into();
        anthology.candidate_tools.push(CandidateTool {
            id: "DS-1".into(),
            name: "release-evidence".into(),
            priority: Priority::High,
            category: InterventionCategory::MissingCapability,
            reason: "Repeated evidence collection".into(),
            estimated_time_saved: "4.5 hours/week".into(),
            confidence: 0.91,
            affected_projects: vec!["alpha".into()],
            existing_matches: Vec::new(),
            mutation_fitness: 0.0,
            capability_overlap: 0.0,
            implementation_cost: "low".into(),
        });
        let json =
            serde_json::to_string(&RunEnvelope::from_anthology(&anthology, "device_1")).unwrap();
        assert!(json.contains("release-evidence"));
        assert!(!json.contains("gsk_must_not_leave_machine"));
        assert!(!json.contains("/private/output"));
        assert!(!json.contains("config"));
    }

    #[test]
    fn production_api_requires_https() {
        assert!(CloudClient::new(Some("http://example.com")).is_err());
        assert!(CloudClient::new(Some("http://127.0.0.1:8787")).is_ok());
        assert!(CloudClient::new(Some("https://example.com/")).is_ok());
    }

    #[test]
    fn migrates_legacy_commercial_origin_without_repairing() {
        assert_eq!(effective_api_url("https://dreamsequence.pro"), DEFAULT_API_URL);
        assert_eq!(effective_api_url("https://dreamsequence.pro/"), DEFAULT_API_URL);
        assert_eq!(effective_api_url("https://self-hosted.example"), "https://self-hosted.example");
        assert_eq!(CloudClient::new(Some(LEGACY_API_URL)).unwrap().base_url, DEFAULT_API_URL);
    }

    #[test]
    fn estimated_hours_are_projected() {
        assert_eq!(parse_hours("14.7 hours/week"), 14.7);
        assert_eq!(parse_hours("unknown"), 0.0);
    }
}
