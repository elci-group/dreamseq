// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Native ingestion of Lucid agent sessions (`lucid.session/v1`).
//!
//! Lucid publishes each project's agent events as newline-delimited JSON at
//! `GET /api/lucid/v1/projects/{project}/signals/export`. Every line carries
//! its provenance (tenant, project, task, execution, attempt, sequence and
//! event id) and the agent's role, provider and model; payloads are already
//! reduced by Lucid to an allow-list of operational fields plus the SHA-256
//! of the full payload.
//!
//! A harness with `log_format = "lucid_session"` reads those files (`*.jsonl`
//! under its `log_path`). Each event becomes one [`LogEntry`] whose `id` is
//! the Lucid `event_id`, so the same event delivered twice (overlapping
//! exports, a retried pull) collapses to one entry while genuinely repeated
//! events, which have distinct ids, stay as frequency evidence.
//!
//! `dreamseq lucid pull` fetches the export incrementally into such a
//! directory. The bearer token is read from an environment variable (default
//! `LUCID_API_TOKEN`) and is never written to disk or logs.

use crate::aggregator::{LogEntry, LogMetadata, ToolCall};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const SESSION_V1: &str = "lucid.session/v1";
/// Lucid's largest export page.
pub const EXPORT_PAGE: usize = 10_000;
/// Lucid's longest export window.
pub const MAX_WINDOW_DAYS: i64 = 90;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentKey {
    pub role: String,
    pub provider: String,
    pub model: String,
}

/// One `lucid.session/v1` record. Unknown fields are ignored so a newer
/// Lucid can add fields without breaking older readers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub schema_version: String,
    pub event_id: String,
    pub tenant_id: String,
    pub project_id: String,
    pub task_id: String,
    pub execution_id: String,
    pub attempt_id: String,
    pub sequence: u64,
    pub occurred_at: DateTime<Utc>,
    pub kind: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub operation_id: Option<String>,
    pub agent: AgentKey,
    #[serde(default)]
    pub detail: serde_json::Value,
    pub payload_sha256: String,
}

impl SessionEvent {
    pub fn parse(line: &str) -> Result<Self> {
        let event: Self = serde_json::from_str(line).context("not a lucid.session record")?;
        if event.schema_version != SESSION_V1 {
            bail!(
                "schema_version {:?} is not {SESSION_V1}",
                event.schema_version
            );
        }
        if event.event_id.is_empty() {
            bail!("event_id is empty");
        }
        Ok(event)
    }

    fn detail_str(&self, key: &str) -> Option<&str> {
        self.detail.get(key).and_then(|v| v.as_str())
    }

    /// Human-readable line the pattern and steering stages read.
    fn content(&self) -> String {
        let mut parts = vec![format!("[lucid] {}", self.kind)];
        if let Some(obj) = self.detail.as_object() {
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            for k in keys {
                match &obj[k] {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {}
                    serde_json::Value::String(s) => parts.push(format!("{k}={s}")),
                    other => parts.push(format!("{k}={other}")),
                }
            }
        }
        parts.push(format!(
            "task={} attempt={} seq={}",
            self.task_id, self.attempt_id, self.sequence
        ));
        parts.join(" ")
    }

    /// The event as a Dreamseq log entry (see module docs).
    pub fn to_log_entry(&self, harness: &str) -> LogEntry {
        let tool_calls = match (self.kind.as_str(), self.detail_str("tool")) {
            ("tool.completed" | "policy.denied", Some(tool)) => {
                let mut parameters = self.detail.clone();
                if let Some(obj) = parameters.as_object_mut() {
                    obj.remove("tool");
                }
                let outcome = if self.kind == "policy.denied" {
                    "denied"
                } else {
                    self.detail_str("status").unwrap_or("unknown")
                };
                vec![ToolCall {
                    tool_name: tool.to_string(),
                    parameters,
                    result: Some(serde_json::json!({ "status": outcome })),
                    duration_ms: None,
                }]
            }
            _ => vec![],
        };
        LogEntry {
            id: self.event_id.clone(),
            harness: harness.to_string(),
            timestamp: self.occurred_at,
            content: self.content(),
            metadata: LogMetadata {
                model: Some(self.agent.model.clone()),
                provider: Some(self.agent.provider.clone()),
                tool_calls,
                user_messages: usize::from(self.kind == "message.applied"),
                assistant_messages: usize::from(self.kind == "model.response"),
            },
        }
    }
}

/// Parse one export file. Returns the entries and how many lines were
/// rejected (malformed, or another schema).
pub fn parse_export(content: &str, harness: &str) -> (Vec<LogEntry>, usize) {
    let mut entries = Vec::new();
    let mut rejected = 0usize;
    for (n, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match SessionEvent::parse(line) {
            Ok(event) => entries.push(event.to_log_entry(harness)),
            Err(error) => {
                rejected += 1;
                tracing::warn!(
                    harness,
                    line_number = n + 1,
                    error = %error,
                    "rejected lucid.session record"
                );
            }
        }
    }
    (entries, rejected)
}

/// Collapse entries that share an id (the same Lucid event delivered more
/// than once); the first occurrence wins. Returns how many were removed.
pub fn dedupe_by_id(entries: &mut Vec<LogEntry>) -> usize {
    let mut seen = HashSet::new();
    let before = entries.len();
    entries.retain(|e| seen.insert(e.id.clone()));
    before - entries.len()
}

// ---- pull ----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PullOptions {
    /// Lucid origin, e.g. `https://lucid.dreamsequence.pro`.
    pub base_url: String,
    pub project_id: String,
    /// Directory the harness reads (`log_path`).
    pub out_dir: PathBuf,
    /// Start of the first pull when there is no saved state.
    pub initial_days: i64,
    /// Environment variable holding the bearer token.
    pub token_env: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PullSummary {
    pub files_written: Vec<PathBuf>,
    pub events: usize,
    pub rejected: usize,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PullState {
    /// End of the last completed window.
    last_to: Option<DateTime<Utc>>,
}

fn state_path(out_dir: &Path, project_id: &str) -> PathBuf {
    out_dir.join(format!(".lucid-pull-{project_id}.json"))
}

fn check_project_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'));
    if !ok || id == "." || id == ".." {
        bail!("project id must match [A-Za-z0-9_.:-]{{1,128}}");
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("partial");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

/// Fetch every event since the last pull (or `initial_days`) into
/// `out_dir`, one file per page. Pages are requested oldest first; a full
/// page continues from its last event's time (events at that instant may be
/// fetched twice and collapse on ingestion by `event_id`).
// traci: allow -- compatibility wrapper creates and propagates a trace_id.
pub async fn pull(options: &PullOptions) -> Result<PullSummary> {
    let trace_id = crate::telemetry::new_trace_id();
    pull_with_trace_id(options, &trace_id).await
}

fn read_state(path: &Path) -> PullState {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(state) => state,
            Err(error) => {
                // Starting over only re-fetches events, which collapse by id.
                tracing::warn!(path = %path.display(), error = %error, "unreadable Lucid pull state; starting from the initial window");
                PullState::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PullState::default(),
        Err(error) => {
            tracing::warn!(path = %path.display(), error = %error, "cannot read Lucid pull state; starting from the initial window");
            PullState::default()
        }
    }
}

#[tracing::instrument(skip_all, fields(trace_id = %trace_id, project = %options.project_id))]
pub async fn pull_with_trace_id(options: &PullOptions, trace_id: &str) -> Result<PullSummary> {
    check_project_id(&options.project_id)?;
    let token = match std::env::var(&options.token_env) {
        Ok(t) if !t.trim().is_empty() => t,
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            tracing::error!(variable = %options.token_env, "Lucid token variable is not set");
            bail!("{} is not set", options.token_env)
        }
        Err(error) => {
            tracing::error!(variable = %options.token_env, error = %error, "Lucid token variable is unreadable");
            bail!("{}: {error}", options.token_env)
        }
    };
    std::fs::create_dir_all(&options.out_dir)
        .with_context(|| format!("creating {}", options.out_dir.display()))?;
    let state_file = state_path(&options.out_dir, &options.project_id);
    let state = read_state(&state_file);
    let now = Utc::now();
    let mut from = state
        .last_to
        .unwrap_or(now - Duration::days(options.initial_days.clamp(1, MAX_WINDOW_DAYS)));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let base = options.base_url.trim_end_matches('/');
    let mut summary = PullSummary {
        from: Some(from),
        ..PullSummary::default()
    };
    loop {
        let to = (from + Duration::days(MAX_WINDOW_DAYS)).min(now);
        let url = format!(
            "{base}/api/lucid/v1/projects/{}/signals/export",
            options.project_id
        );
        let response = client
            .get(&url)
            .query(&[
                ("from", from.to_rfc3339()),
                ("to", to.to_rfc3339()),
                ("limit", EXPORT_PAGE.to_string()),
            ])
            .bearer_auth(&token)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?;
        let status = response.status();
        if !status.is_success() {
            tracing::error!(%status, project = %options.project_id, from = %from, to = %to, "Lucid export request failed");
            bail!("Lucid export returned HTTP {status}");
        }
        let body = response.text().await.context("reading the export")?;
        let mut events: Vec<SessionEvent> = Vec::new();
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            match SessionEvent::parse(line) {
                Ok(e) => events.push(e),
                Err(error) => {
                    tracing::warn!(error = %error, "rejected lucid.session record from the export");
                    summary.rejected += 1;
                }
            }
        }
        let full = events.len() >= EXPORT_PAGE;
        if !events.is_empty() {
            let file = options.out_dir.join(format!(
                "lucid-{}-{}.jsonl",
                options.project_id,
                events[0].occurred_at.format("%Y%m%dT%H%M%S%.6fZ")
            ));
            let mut bytes = Vec::new();
            for e in &events {
                serde_json::to_writer(&mut bytes, e)?;
                bytes.push(b'\n');
            }
            write_atomic(&file, &bytes)?;
            summary.files_written.push(file);
            summary.events += events.len();
        }
        let next_from = match (full, events.last()) {
            // A full page: continue from its last event.
            (true, Some(last)) if last.occurred_at > from => last.occurred_at,
            (true, _) => bail!("a full export page did not advance; refusing to loop"),
            // The window is done.
            (false, _) => to,
        };
        write_atomic(
            &state_file,
            &serde_json::to_vec(&PullState {
                last_to: Some(next_from),
            })?,
        )?;
        summary.to = Some(next_from);
        if next_from >= now {
            break;
        }
        from = next_from;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, kind: &str, detail: serde_json::Value) -> String {
        serde_json::json!({
            "schema_version": SESSION_V1,
            "event_id": id,
            "tenant_id": "t1",
            "project_id": "p1",
            "task_id": "task_1",
            "execution_id": "exec_1",
            "attempt_id": "att_1",
            "sequence": 3,
            "occurred_at": "2026-09-26T10:00:00Z",
            "kind": kind,
            "actor_id": null,
            "operation_id": null,
            "agent": {"role": "implementer", "provider": "anthropic", "model": "claude-x"},
            "detail": detail,
            "payload_sha256": "ab",
        })
        .to_string()
    }

    #[test]
    fn events_keep_provenance_and_tool_outcomes() {
        let content = [
            line(
                "ev1",
                "tool.completed",
                serde_json::json!({"tool": "write_file", "status": "rejected", "reject_reason": "path_escape"}),
            ),
            line(
                "ev2",
                "policy.denied",
                serde_json::json!({"tool": "delete_file", "reason": "tool_not_allowed"}),
            ),
            line("ev3", "message.applied", serde_json::json!({})),
            "not json".to_string(),
            line("ev4", "model.response", serde_json::json!({"usage": {"input_tokens": 5}}))
                .replace(SESSION_V1, "lucid.session/v2"),
        ]
        .join("\n");
        let (entries, rejected) = parse_export(&content, "lucid");
        assert_eq!(rejected, 2);
        assert_eq!(
            entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["ev1", "ev2", "ev3"]
        );
        let tool = &entries[0].metadata.tool_calls[0];
        assert_eq!(tool.tool_name, "write_file");
        assert_eq!(tool.result, Some(serde_json::json!({"status": "rejected"})));
        assert_eq!(tool.parameters["reject_reason"], "path_escape");
        assert!(tool.parameters.get("tool").is_none());
        assert_eq!(
            entries[1].metadata.tool_calls[0].result,
            Some(serde_json::json!({"status": "denied"}))
        );
        assert_eq!(entries[2].metadata.user_messages, 1);
        assert_eq!(entries[0].metadata.model.as_deref(), Some("claude-x"));
        assert!(entries[0].content.contains("reject_reason=path_escape"));
        assert!(
            entries[0]
                .content
                .contains("task=task_1 attempt=att_1 seq=3")
        );
        assert_eq!(
            entries[0].timestamp.to_rfc3339(),
            "2026-09-26T10:00:00+00:00"
        );
    }

    #[test]
    fn redelivered_events_collapse_but_repeats_stay() {
        let failed = serde_json::json!({"tool": "write_file", "status": "failed"});
        let content = [
            line("ev1", "tool.completed", failed.clone()),
            line("ev1", "tool.completed", failed.clone()),
            line("ev2", "tool.completed", failed),
        ]
        .join("\n");
        let (mut entries, _) = parse_export(&content, "lucid");
        assert_eq!(dedupe_by_id(&mut entries), 1);
        assert_eq!(entries.len(), 2, "a genuine repeat has its own id");
    }

    /// A one-connection-at-a-time HTTP server answering from `pages` in
    /// order and recording each request line.
    async fn serve(pages: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for body in pages {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                seen.push(request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/x-ndjson\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            seen
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn pull_is_incremental_and_never_logs_the_token() {
        let dir = std::env::temp_dir().join(format!("dreamseq-lucid-{}", uuid::Uuid::new_v4()));
        let page = format!(
            "{}\n{}\n",
            line("ev1", "task.running", serde_json::json!({})),
            line("ev2", "task.completed", serde_json::json!({}))
        );
        let (base, server) = serve(vec![page, String::new()]).await;
        // SAFETY: the variable name is unique to this test.
        unsafe { std::env::set_var("DREAMSEQ_TEST_LUCID_TOKEN", "secret-token-value") };
        let options = PullOptions {
            base_url: base,
            project_id: "p1".into(),
            out_dir: dir.clone(),
            initial_days: 7,
            token_env: "DREAMSEQ_TEST_LUCID_TOKEN".into(),
        };
        let first = pull(&options).await.unwrap();
        assert_eq!((first.events, first.files_written.len()), (2, 1));
        let written = std::fs::read_to_string(&first.files_written[0]).unwrap();
        assert!(!written.contains("secret-token-value"));
        let (entries, rejected) = parse_export(&written, "lucid");
        assert_eq!((entries.len(), rejected), (2, 0));
        // The second pull starts where the first ended and finds nothing.
        let second = pull(&options).await.unwrap();
        assert_eq!(second.events, 0);
        assert_eq!(second.from, first.to);
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /api/lucid/v1/projects/p1/signals/export?from="));
        assert!(
            requests[0]
                .to_lowercase()
                .contains("authorization: bearer secret-token-value")
        );
        let state = std::fs::read_to_string(state_path(&dir, "p1")).unwrap();
        assert!(!state.contains("secret"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(check_project_id("../etc").is_err());
    }
}
