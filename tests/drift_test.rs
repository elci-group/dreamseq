// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use chrono::{Duration, Utc};
use dreamseq::azusa::{
    AzusaDriftReport, DriftCache, LocalDriftSnapshot, RepoDriftEntry, record_observation,
    resolve_device_name, view_cache, view_entry,
};
use dreamseq::config::AccountTier;

fn report(score: f32, direction: &str, risk: &str, compared_to: &str) -> AzusaDriftReport {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "repository": {"name": "dreamseq", "root": "/repo"},
        "state_b": {"spec": compared_to, "label": compared_to, "digest": "abc"},
        "direction": direction,
        "score": {"overall": score, "layers": {}},
        "risk": {"level": risk, "rationale": "", "factors": []},
    }))
    .expect("report fixture parses")
}

fn snapshot(device: &str, score: f32, minutes_ago: i64) -> LocalDriftSnapshot {
    LocalDriftSnapshot {
        device: device.to_string(),
        score,
        direction: "a-to-b".to_string(),
        risk: "medium".to_string(),
        compared_to: "origin/main".to_string(),
        local_captured_at: Utc::now() - Duration::minutes(minutes_ago),
        remote_captured_at: Utc::now() - Duration::minutes(minutes_ago),
    }
}

fn entry_with(locals: Vec<LocalDriftSnapshot>) -> RepoDriftEntry {
    RepoDriftEntry {
        identity: "git@github.com:elci-group/dreamseq.git".to_string(),
        name: "dreamseq".to_string(),
        local_path: None,
        remote_captured_at: Some(Utc::now()),
        locals,
    }
}

#[test]
fn tier_limits_map_to_free_pro_enterprise() {
    assert_eq!(AccountTier::Free.max_local_scores(), 1);
    assert_eq!(AccountTier::Pro.max_local_scores(), 4);
    assert_eq!(AccountTier::Enterprise.max_local_scores(), 16);
}

#[test]
fn pro_view_shows_up_to_four_individual_scores() {
    let locals: Vec<LocalDriftSnapshot> = (0..6)
        .map(|i| snapshot(&format!("device-{i}"), 2.0 + i as f32, i))
        .collect();
    let entry = entry_with(locals);

    let pro = view_entry(&entry, AccountTier::Pro);
    assert_eq!(pro.locals.len(), 4);
    assert!(pro.average.is_none(), "pro shows individual scores, not an average");

    let free = view_entry(&entry, AccountTier::Free);
    assert_eq!(free.locals.len(), 1);
}

#[test]
fn enterprise_view_averages_up_to_sixteen_scores() {
    let locals: Vec<LocalDriftSnapshot> = (0..20)
        .map(|i| snapshot(&format!("device-{i}"), 4.0, i))
        .collect();
    let entry = entry_with(locals);

    let view = view_entry(&entry, AccountTier::Enterprise);
    assert!(view.locals.is_empty(), "enterprise collapses to the average");
    let average = view.average.expect("enterprise average present");
    assert_eq!(average.devices, 16);
    assert!((average.score - 4.0).abs() < f32::EPSILON);
}

#[test]
fn enterprise_average_rounds_to_two_decimals() {
    let locals = vec![snapshot("a", 1.0, 10), snapshot("b", 2.0, 5), snapshot("c", 2.0, 1)];
    let view = view_entry(&entry_with(locals), AccountTier::Enterprise);
    assert!((view.average.expect("average").score - 1.67).abs() < 0.001);
}

#[test]
fn observation_replaces_same_device_snapshot_and_keeps_newest_first() {
    let mut cache = DriftCache::default();
    let identity = "git@github.com:elci-group/dreamseq.git".to_string();
    let first = report(3.5, "a-to-b", "medium", "origin/main");

    record_observation(
        &mut cache,
        identity.clone(),
        "dreamseq".to_string(),
        Some(std::path::PathBuf::from("/repo")),
        &first,
        "laptop",
        Utc::now(),
        Utc::now(),
    );
    let newer = report(6.25, "bidirectional", "high", "origin/main");
    record_observation(
        &mut cache,
        identity.clone(),
        "dreamseq".to_string(),
        Some(std::path::PathBuf::from("/repo")),
        &newer,
        "laptop",
        Utc::now(),
        Utc::now(),
    );

    let entry = cache.repos.get(&identity).expect("entry exists");
    assert_eq!(entry.locals.len(), 1, "one snapshot per device");
    assert!((entry.locals[0].score - 6.25).abs() < f32::EPSILON);
    assert_eq!(entry.locals[0].risk, "high");
    assert_eq!(entry.locals[0].compared_to, "origin/main");
}

#[test]
fn cache_round_trips_through_json_and_tolerates_missing_fields() {
    let mut cache = DriftCache::default();
    record_observation(
        &mut cache,
        "git@github.com:elci-group/dreamseq.git".to_string(),
        "dreamseq".to_string(),
        None,
        &report(1.0, "in-sync", "low", "origin/main"),
        "laptop",
        Utc::now(),
        Utc::now(),
    );

    let json = serde_json::to_string(&cache).expect("serializes");
    let loaded: DriftCache = serde_json::from_str(&json).expect("round trips");
    assert_eq!(loaded.repos.len(), 1);

    // Config files written before the drift feature exist without these
    // blocks; defaults must carry them.
    let legacy: DriftCache = serde_json::from_str(r#"{"schema_version":1}"#)
        .expect("legacy cache without repos field parses");
    assert!(legacy.repos.is_empty());
}

#[test]
fn view_cache_covers_every_cached_repo() {
    let mut cache = DriftCache::default();
    for name in ["alpha", "beta"] {
        record_observation(
            &mut cache,
            format!("https://example.com/{name}.git"),
            name.to_string(),
            None,
            &report(0.0, "in-sync", "low", "origin/main"),
            "laptop",
            Utc::now(),
            Utc::now(),
        );
    }
    let views = view_cache(&cache, AccountTier::Pro);
    assert_eq!(views.len(), 2);
}

#[test]
fn device_name_prefers_config_then_hostname_then_placeholder() {
    assert_eq!(resolve_device_name(Some("workstation"), Some("host")), "workstation");
    assert_eq!(resolve_device_name(Some("  "), Some("host")), "host");
    assert_eq!(resolve_device_name(None, None), "unknown-device");
    assert_eq!(resolve_device_name(None, Some("host")), "host");
}

#[test]
fn default_config_carries_free_tier_and_no_device_name() {
    let config = dreamseq::DreamseqConfig::default();
    assert_eq!(config.tier, AccountTier::Free);
    assert!(config.device_name.is_none());
    assert!(config.enable_drift, "drift measurement is on by default");
}

#[test]
fn scan_candidates_dedup_skip_missing_and_skip_non_git_dirs() {
    use dreamseq::azusa::local_scan_candidates;

    let root = std::env::temp_dir().join(format!("dreamseq-scan-{}", uuid::Uuid::new_v4()));
    let git_repo = root.join("repo");
    let plain_dir = root.join("plain");
    std::fs::create_dir_all(&plain_dir).unwrap();
    std::fs::create_dir_all(&git_repo).unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&git_repo)
        .status();
    if init.map(|status| !status.success()).unwrap_or(true) {
        eprintln!("git is unavailable; skipping scan-candidates test");
        std::fs::remove_dir_all(&root).ok();
        return;
    }

    let mut cache = DriftCache::default();
    record_observation(
        &mut cache,
        "git@github.com:elci-group/dreamseq.git".to_string(),
        "dreamseq".to_string(),
        Some(git_repo.clone()),
        &report(0.0, "in-sync", "low", "origin/main"),
        "laptop",
        Utc::now(),
        Utc::now(),
    );

    let candidates = local_scan_candidates(
        &cache,
        &[
            git_repo.clone(),               // duplicate of the cached path
            root.join("does-not-exist"),    // missing on this device
            plain_dir,                      // not a git worktree
        ],
    );

    assert_eq!(candidates, vec![git_repo.canonicalize().unwrap()]);
    std::fs::remove_dir_all(&root).ok();
}

/// Tiny one-shot HTTP server: captures the first request and replies 200.
fn mock_api() -> (String, std::sync::mpsc::Receiver<String>) {
    use std::io::{BufRead, Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let address = listener.local_addr().expect("mock server address");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept one request");
        let mut reader = std::io::BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        tx.send(format!(
            "{}{}",
            request_line,
            String::from_utf8_lossy(&body)
        ))
        .unwrap();
        reader
            .get_mut()
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"ok\":true}",
            )
            .unwrap();
    });
    (format!("http://127.0.0.1:{}", address.port()), rx)
}

#[tokio::test]
async fn drift_upload_posts_envelope_to_the_drift_endpoint() {
    use dreamseq::azusa::RecordedObservation;
    use dreamseq::cloud::{CloudClient, Credentials};

    let (url, rx) = mock_api();
    let client = CloudClient::new(Some(&url)).expect("client builds for localhost http");
    let credentials = Credentials {
        api_url: url,
        access_token: "token-123".to_string(),
        account_id: "acct-1".to_string(),
        device_id: "device-1".to_string(),
        paired_at: Utc::now(),
    };
    let observation = RecordedObservation {
        identity: "git@github.com:elci-group/dreamseq.git".to_string(),
        name: "dreamseq".to_string(),
        local_path: std::path::PathBuf::from("/repo"),
        remote_captured_at: Utc::now(),
        snapshot: snapshot("device-1", 4.2, 0),
    };

    client
        .upload_drift_with_trace_id(&credentials, std::slice::from_ref(&observation), "trace")
        .await
        .expect("upload succeeds against mock server");

    let raw = rx.recv().expect("mock server captured the request");
    let (request_line, body) = raw.split_once('\n').expect("request line and body");
    assert!(request_line.starts_with("POST /api/v1/drift "), "got: {request_line}");

    let body: serde_json::Value = serde_json::from_str(body).expect("envelope is JSON");
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["device"]["id"], "device-1");
    let repo = &body["repos"][0];
    assert_eq!(repo["identity"], "git@github.com:elci-group/dreamseq.git");
    assert_eq!(repo["name"], "dreamseq");
    assert_eq!(repo["device"], "device-1");
    assert_eq!(repo["score"], 4.2);
    assert_eq!(repo["compared_to"], "origin/main");
    assert!(repo.get("local_path").is_none(), "device-local paths are not uploaded");
    assert!(body.get("config").is_none());
}
