// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use crate::goblin_gateway::{GatewayDecision, validate_run_envelope};
use crate::report::Anthology;
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[path = "cloud_credentials.rs"]
mod cloud_credentials;
#[path = "cloud_envelope.rs"]
mod cloud_envelope;
#[path = "cloud_protocol.rs"]
mod cloud_protocol;
#[path = "cloud_sync.rs"]
mod cloud_sync;
pub use cloud_envelope::RunEnvelope;
pub use cloud_protocol::DeviceAuthorization;
use cloud_protocol::{DevicePoll, DeviceToken, parse_response};
pub use cloud_sync::SyncSummary;

const LEGACY_API_URL: &str = "https://dreamsequence.pro";
const DEFAULT_API_URL: &str = "https://padagonia.dreamsequence.pro/dreamsequence";

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
        self.load_credentials()
    }

    pub fn load_optional(&self) -> Result<Option<Credentials>> {
        self.load_optional_credentials()
    }

    pub fn save(&self, credentials: &Credentials) -> Result<()> {
        self.save_credentials(credentials)
    }

    pub fn remove(&self) -> Result<bool> {
        self.remove_credentials()
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

    /// Start a device-authorization flow and return the verification payload
    /// without printing or opening a browser.
    // traci: allow -- compatibility wrapper creates and propagates a trace_id.
    pub async fn request_authorization(&self) -> Result<DeviceAuthorization> {
        let trace_id = crate::telemetry::new_trace_id();
        self.request_authorization_with_trace_id(&trace_id).await
    }

    #[tracing::instrument(skip_all, fields(trace_id = %trace_id))]
    pub async fn request_authorization_with_trace_id(
        &self,
        trace_id: &str,
    ) -> Result<DeviceAuthorization> {
        let response = self
            .http
            .post(format!("{}/api/v1/device/authorize", self.base_url))
            .json(&serde_json::json!({ "client": "dreamseq-cli" }))
            .send()
            .await?;
        parse_response(response, trace_id).await
    }

    /// Poll the device-token endpoint until the user approves the request or
    /// the deadline expires.
    // traci: allow -- compatibility wrapper creates and propagates a trace_id.
    pub async fn poll_token(
        &self,
        device_code: &str,
        expires_in: u64,
        interval: u64,
        store: &CredentialStore,
    ) -> Result<Credentials> {
        let trace_id = crate::telemetry::new_trace_id();
        self.poll_token_with_trace_id(device_code, expires_in, interval, store, &trace_id)
            .await
    }

    #[tracing::instrument(skip_all, fields(trace_id = %trace_id))]
    pub async fn poll_token_with_trace_id(
        &self,
        device_code: &str,
        expires_in: u64,
        interval: u64,
        store: &CredentialStore,
        trace_id: &str,
    ) -> Result<Credentials> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
        let interval = Duration::from_secs(interval.max(2));
        loop {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("device code expired; run `dreamseq login` again");
            }
            tokio::time::sleep(interval).await;
            let response = self
                .http
                .post(format!("{}/api/v1/device/token", self.base_url))
                .json(&DevicePoll { device_code })
                .send()
                .await?;
            if response.status() == StatusCode::ACCEPTED {
                continue;
            }
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                tokio::time::sleep(interval).await;
                continue;
            }
            let token: DeviceToken = parse_response(response, trace_id).await?;
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

    // traci: allow -- compatibility wrapper creates and propagates a trace_id.
    pub async fn pair(&self, store: &CredentialStore, open: bool) -> Result<Credentials> {
        let trace_id = crate::telemetry::new_trace_id();
        self.pair_with_trace_id(store, open, &trace_id).await
    }

    #[tracing::instrument(skip_all, fields(trace_id = %trace_id))]
    pub async fn pair_with_trace_id(
        &self,
        store: &CredentialStore,
        open: bool,
        trace_id: &str,
    ) -> Result<Credentials> {
        let authorization = self.request_authorization_with_trace_id(trace_id).await?;

        println!("Open {}", authorization.verification_uri);
        println!("Enter code: {}", authorization.user_code);
        if open && !open_browser(&authorization.verification_uri_complete) {
            eprintln!("Could not open a browser automatically. Use the URL above.");
        }

        self.poll_token_with_trace_id(
            &authorization.device_code,
            authorization.expires_in,
            authorization.interval,
            store,
            trace_id,
        )
        .await
    }

    // traci: allow -- compatibility wrapper creates and propagates a trace_id.
    pub async fn revoke(&self, credentials: &Credentials) -> Result<()> {
        let trace_id = crate::telemetry::new_trace_id();
        self.revoke_with_trace_id(credentials, &trace_id).await
    }

    #[tracing::instrument(skip_all, fields(trace_id = %trace_id))]
    pub async fn revoke_with_trace_id(
        &self,
        credentials: &Credentials,
        trace_id: &str,
    ) -> Result<()> {
        let response = self
            .http
            .post(format!("{}/api/v1/device/logout", self.base_url))
            .bearer_auth(&credentials.access_token)
            .send()
            .await?;
        let _: serde_json::Value = parse_response(response, trace_id).await?;
        Ok(())
    }

    // traci: allow -- compatibility wrapper creates and propagates a trace_id.
    pub async fn upload(&self, credentials: &Credentials, anthology: &Anthology) -> Result<()> {
        let trace_id = crate::telemetry::new_trace_id();
        self.upload_with_trace_id(credentials, anthology, &trace_id)
            .await
    }

    #[tracing::instrument(skip_all, fields(trace_id = %trace_id, anthology_id = %anthology.id))]
    pub async fn upload_with_trace_id(
        &self,
        credentials: &Credentials,
        anthology: &Anthology,
        trace_id: &str,
    ) -> Result<()> {
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
        let _: serde_json::Value = parse_response(response, trace_id).await?;
        Ok(())
    }
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
    use crate::report::{CandidateTool, Priority};
    use crate::report::{InterventionCategory, PipelineStats};
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn test_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0_u8; 32 * 1024];
                let read = socket.read(&mut buffer).await.unwrap();
                requests.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (format!("http://{address}"), task)
    }

    fn test_credentials(api_url: String) -> Credentials {
        Credentials {
            api_url,
            access_token: "ds_test_token".into(),
            account_id: "account_1".into(),
            device_id: "device_1".into(),
            paired_at: Utc::now(),
        }
    }

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
            remote_analysis_consent: Some(crate::RemoteAnalysisConsent::PreConfigured),
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
        assert_eq!(
            effective_api_url("https://dreamsequence.pro"),
            DEFAULT_API_URL
        );
        assert_eq!(
            effective_api_url("https://dreamsequence.pro/"),
            DEFAULT_API_URL
        );
        assert_eq!(
            effective_api_url("https://self-hosted.example"),
            "https://self-hosted.example"
        );
        assert_eq!(
            CloudClient::new(Some(LEGACY_API_URL)).unwrap().base_url,
            DEFAULT_API_URL
        );
    }

    #[test]
    fn estimated_hours_are_projected() {
        assert_eq!(cloud_envelope::parse_hours("14.7 hours/week"), 14.7);
        assert_eq!(cloud_envelope::parse_hours("unknown"), 0.0);
    }

    #[tokio::test]
    async fn authorization_posts_the_device_contract() {
        let body = r#"{
            "device_code":"device-code",
            "user_code":"ABCD-EFGH",
            "verification_uri":"https://example.com/device",
            "verification_uri_complete":"https://example.com/device?code=ABCD-EFGH",
            "expires_in":600,
            "interval":5
        }"#;
        let (base_url, server) = test_server(vec![("200 OK", body)]).await;
        let authorization = CloudClient::new(Some(&base_url))
            .unwrap()
            .request_authorization()
            .await
            .unwrap();
        let requests = server.await.unwrap();

        assert_eq!(authorization.user_code, "ABCD-EFGH");
        assert!(requests[0].starts_with("POST /api/v1/device/authorize HTTP/1.1"));
        assert!(requests[0].contains("dreamseq-cli"));
    }

    #[tokio::test]
    async fn authorization_surfaces_api_and_json_failures() {
        let (base_url, server) = test_server(vec![
            (
                "401 Unauthorized",
                r#"{"error":"invalid_client","message":"Client is not registered"}"#,
            ),
            ("200 OK", "not-json"),
        ])
        .await;
        let client = CloudClient::new(Some(&base_url)).unwrap();

        let api_error = client.request_authorization().await.unwrap_err();
        assert!(api_error.to_string().contains("Client is not registered"));
        let json_error = client.request_authorization().await.unwrap_err();
        assert!(json_error.to_string().contains("invalid JSON"));
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn sync_counts_uploaded_skipped_missing_and_failed_inputs() {
        let root =
            std::env::temp_dir().join(format!("dreamseq-cloud-sync-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let anthology = Anthology::new(vec![], vec![], DreamseqConfig::default());
        fs::write(
            root.join("anthology.json"),
            serde_json::to_vec(&anthology).unwrap(),
        )
        .unwrap();
        fs::write(root.join("other.json"), br#"{"kind":"other"}"#).unwrap();
        fs::write(root.join("notes.txt"), b"ignored").unwrap();

        let (base_url, server) = test_server(vec![("200 OK", r#"{"ok":true}"#)]).await;
        let client = CloudClient::new(Some(&base_url)).unwrap();
        let credentials = test_credentials(base_url);
        let summary = client
            .sync_directories(&credentials, &[root.clone(), root.join("missing")])
            .await
            .unwrap();
        let requests = server.await.unwrap();

        assert_eq!(
            summary,
            SyncSummary {
                uploaded: 1,
                skipped: 1,
                failed: 0,
            }
        );
        assert!(requests[0].starts_with("POST /api/v1/runs HTTP/1.1"));
        assert!(requests[0].contains("authorization: Bearer ds_test_token"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sync_records_upload_failures_without_aborting_the_scan() {
        let root =
            std::env::temp_dir().join(format!("dreamseq-cloud-fail-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let anthology = Anthology::new(vec![], vec![], DreamseqConfig::default());
        fs::write(
            root.join("run.json"),
            serde_json::to_vec(&anthology).unwrap(),
        )
        .unwrap();

        let (base_url, server) = test_server(vec![(
            "503 Service Unavailable",
            r#"{"message":"maintenance"}"#,
        )])
        .await;
        let client = CloudClient::new(Some(&base_url)).unwrap();
        let credentials = test_credentials(base_url);
        let summary = client
            .sync_directories(&credentials, std::slice::from_ref(&root))
            .await
            .unwrap();

        assert_eq!(summary.failed, 1);
        assert_eq!(summary.uploaded, 0);
        assert_eq!(server.await.unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
