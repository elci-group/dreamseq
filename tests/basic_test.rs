use chrono::{DateTime, Duration, Utc};
use dreamseq::DreamseqConfig;
use dreamseq::aggregator::{LogEntry, LogMetadata};
use dreamseq::config::HarnessConfig;
use dreamseq::groq::GroqClient;
use dreamseq::normalization::Normalizer;
use dreamseq::segmentation::Segment;
use dreamseq::steering::SteeringDetector;
use std::fs;
use std::path::PathBuf;

fn log_entry(content: &str) -> LogEntry {
    log_entry_with_ts(content, Utc::now())
}

fn log_entry_with_ts(content: &str, timestamp: DateTime<Utc>) -> LogEntry {
    LogEntry {
        id: content.to_string(),
        harness: "test".to_string(),
        timestamp,
        content: content.to_string(),
        metadata: LogMetadata {
            model: None,
            provider: None,
            tool_calls: vec![],
            user_messages: 0,
            assistant_messages: 0,
        },
    }
}

fn segment_with_content(content: &str) -> Segment {
    let now = Utc::now();
    Segment {
        id: "test".to_string(),
        topic: "test".to_string(),
        entries: vec![log_entry(content)],
        start_time: now,
        end_time: now,
        confidence: 1.0,
    }
}

#[test]
fn test_config_default() {
    let config = DreamseqConfig::default();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

    assert!(config.output_dir.ends_with("dreamseq/output"));
    assert!(config.anthologies_dir.ends_with("dreamseq/anthologies"));
    assert!(config.harnesses.is_empty());
    assert!(!config.enable_tts);
    assert!(!config.enable_kaptaind);
    assert!(!config.allow_remote_analysis);
    assert_eq!(
        config.groq_api_key,
        std::env::var("GROQ_API_KEY").unwrap_or_default()
    );
    assert_eq!(config.output_dir, home.join("dreamseq").join("output"));
}

#[test]
fn readme_style_lowercase_log_formats_load() {
    let config: DreamseqConfig = serde_json::from_str(
        r#"{
            "harnesses": [{"name":"logs","log_path":"/tmp/logs","log_format":"json"}],
            "output_dir":"/tmp/output",
            "anthologies_dir":"/tmp/anthologies",
            "enable_tts":false,
            "enable_kaptaind":false
        }"#,
    )
    .unwrap();
    assert!(matches!(
        config.harnesses[0].log_format,
        dreamseq::config::LogFormat::Json
    ));
}

#[test]
fn serialized_config_never_contains_api_key() {
    let config = DreamseqConfig {
        groq_api_key: "super-secret-test-key".into(),
        ..DreamseqConfig::default()
    };
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(!serialized.contains("super-secret-test-key"));
    assert!(!serialized.contains("groq_api_key"));
}

#[test]
fn saved_config_is_private_and_secret_free() {
    let root = std::env::temp_dir().join(format!("dreamseq-private-{}", uuid::Uuid::new_v4()));
    let path = root.join("config.json");
    let config = DreamseqConfig {
        groq_api_key: "must-not-be-written".into(),
        ..DreamseqConfig::default()
    };
    config.save_to_path(&path).unwrap();
    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.contains("must-not-be-written"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovered_config_uses_safe_defaults() {
    let config = DreamseqConfig::discover();
    let default = DreamseqConfig::default();

    // Discovery adds existing log sources while preserving safe defaults.
    assert_eq!(config.anthologies_dir, default.anthologies_dir);
    assert_eq!(config.output_dir, default.output_dir);
    assert_eq!(config.enable_tts, default.enable_tts);
    assert_eq!(config.enable_kaptaind, default.enable_kaptaind);
}

#[test]
fn test_config_loads_json_and_toml() {
    let root = std::env::temp_dir().join(format!("dreamseq-config-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let config = DreamseqConfig::default();
    let json_path = root.join("config.json");
    fs::write(&json_path, serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(
        DreamseqConfig::load_from_path(&json_path)
            .unwrap()
            .enable_tts,
        config.enable_tts
    );

    let toml_path = root.join("config.toml");
    fs::write(&toml_path, toml::to_string(&config).unwrap()).unwrap();
    assert_eq!(
        DreamseqConfig::load_from_path(&toml_path)
            .unwrap()
            .enable_tts,
        config.enable_tts
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn config_falls_back_to_groq_environment_key() {
    let root = std::env::temp_dir().join(format!("dreamseq-env-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("config.json");
    let mut disk_config = DreamseqConfig::default();
    disk_config.groq_api_key.clear();
    fs::write(&path, serde_json::to_string(&disk_config).unwrap()).unwrap();
    let previous = std::env::var("GROQ_API_KEY").ok();
    unsafe { std::env::set_var("GROQ_API_KEY", "test-key") };
    assert_eq!(
        DreamseqConfig::load_from_path(&path).unwrap().groq_api_key,
        "test-key"
    );
    match previous {
        Some(value) => unsafe { std::env::set_var("GROQ_API_KEY", value) },
        None => unsafe { std::env::remove_var("GROQ_API_KEY") },
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovery_finds_existing_harness_sources() {
    let config = DreamseqConfig::discover();
    assert!(config.harnesses.iter().any(|h| h.name == "grok"));
    assert!(config.harnesses.iter().any(|h| h.name == "kimi"));
}

#[test]
fn test_pattern_extraction_empty_analysis_yields_empty_patterns() {
    use dreamseq::groq::Analysis;
    use dreamseq::patterns::PatternExtractor;

    let extractor = PatternExtractor::new();
    let analysis = Analysis {
        model_failures: vec![],
        harness_friction: vec![],
        missing_tooling: vec![],
        workflow_bottlenecks: vec![],
        repeated_commands: vec![],
        repeated_prompts: vec![],
        context_loss: vec![],
        automation_opportunities: vec![],
    };

    let patterns = extractor.extract(&analysis).unwrap();
    assert!(patterns.is_empty());
}

#[test]
fn test_pattern_extraction_populates_patterns() {
    use dreamseq::groq::{Analysis, ModelFailure};
    use dreamseq::patterns::{PatternExtractor, PatternType};

    let extractor = PatternExtractor::new();
    let analysis = Analysis {
        model_failures: vec![ModelFailure {
            model: "gpt".to_string(),
            issue: "hallucinated API".to_string(),
            frequency: 3,
            example: "foo()".to_string(),
        }],
        harness_friction: vec![],
        missing_tooling: vec![],
        workflow_bottlenecks: vec![],
        repeated_commands: vec![],
        repeated_prompts: vec![],
        context_loss: vec![],
        automation_opportunities: vec![],
    };

    let patterns = extractor.extract(&analysis).unwrap();
    assert_eq!(patterns.len(), 1);
    assert!(matches!(
        patterns[0].pattern_type,
        PatternType::ModelFailure
    ));
    assert_eq!(patterns[0].frequency, 3);
}

#[test]
fn test_steering_detection() {
    let detector = SteeringDetector::new();
    let events = detector
        .detect(&[segment_with_content("I wish I had a tool for this")])
        .unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].category,
        dreamseq::steering::SteeringCategory::MissingTool
    ));
}

#[test]
fn greetings_are_not_missing_tool_events() {
    let detector = SteeringDetector::new();
    for greeting in ["hello", "hi", "hey", "good morning"] {
        let events = detector.detect(&[segment_with_content(greeting)]).unwrap();
        assert!(
            events.is_empty(),
            "greeting '{}' should not produce a steering event",
            greeting
        );
    }
}

#[test]
fn telemetry_again_is_not_manual_repetition() {
    let detector = SteeringDetector::new();
    let events = detector
        .detect(&[segment_with_content(
            "HttpMetricsClient.ExportFailed: try again later",
        )])
        .unwrap();
    assert!(
        events.is_empty(),
        "loose 'again' in telemetry should not be flagged as manual repetition"
    );
}

#[test]
fn normalization_preserves_repeated_evidence_after_whitespace_cleanup() {
    let entries = vec![log_entry("  Hello   world  "), log_entry("hello world")];
    let normalized = Normalizer.normalize(entries).unwrap();
    assert_eq!(normalized.len(), 2);
    assert_eq!(normalized[0].content, "Hello world");
    assert_eq!(normalized[1].content, "hello world");
}

#[test]
fn normalization_removes_empty_entries_but_retains_duplicates() {
    let entries = vec![
        log_entry("first"),
        log_entry("first"),
        log_entry("   "),
        log_entry("second"),
    ];
    let normalized = Normalizer.normalize(entries.clone()).unwrap();
    assert_eq!(normalized.len(), 3);
    assert_eq!(
        normalized.iter().filter(|e| e.content == "first").count(),
        2
    );
    assert!(normalized.iter().any(|e| e.content == "second"));
}

#[tokio::test]
async fn empty_analysis_does_not_require_api_key() {
    let analysis = GroqClient::new("").unwrap().analyze(&[]).await.unwrap();
    assert!(analysis.model_failures.is_empty());
}

#[tokio::test]
async fn groq_client_hits_custom_endpoint() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let body = serde_json::json!({
        "choices": [{"message": {"content": "{\"model_failures\":[],\"harness_friction\":[],\"missing_tooling\":[],\"workflow_bottlenecks\":[],\"repeated_commands\":[],\"repeated_prompts\":[],\"context_loss\":[],\"automation_opportunities\":[]}"}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    });
    let body_bytes = body.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body_bytes.len(),
        body_bytes
    );

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let client = GroqClient::new_with_url("test-key", &format!("http://{}", addr)).unwrap();
    let analysis = client
        .analyze(&[segment_with_content("some log content")])
        .await
        .unwrap();

    assert!(analysis.model_failures.is_empty());
    assert!(analysis.missing_tooling.is_empty());

    server.abort();
    let _ = server.await;
}

#[test]
fn analysis_parser_accepts_numeric_descriptions() {
    let client = GroqClient::new("test").unwrap();
    let text = r#"{"model_failures":[{"model":"gpt","issue":6,"frequency":1,"example":true}],"harness_friction":[],"missing_tooling":[],"workflow_bottlenecks":[],"repeated_commands":[],"repeated_prompts":[],"context_loss":[],"automation_opportunities":[]}"#;
    let parsed = client.parse_analysis_for_test(text).unwrap();
    assert_eq!(parsed.model_failures[0].issue, "6");
    assert_eq!(parsed.model_failures[0].example, "true");
}

#[test]
fn analysis_parser_accepts_numeric_segment_evidence() {
    let client = GroqClient::new("test").unwrap();
    let text = r#"{"model_failures":[],"harness_friction":[],"missing_tooling":[],"workflow_bottlenecks":[],"repeated_commands":[],"repeated_prompts":[],"context_loss":[{"description":"context","affected_segments":[0,1]}],"automation_opportunities":[]}"#;
    assert!(client.parse_analysis_for_test(text).is_ok());
}

#[test]
fn groq_prompt_includes_segments_and_schema() {
    let client = GroqClient::new("test").unwrap();
    let prompt = client.build_analysis_prompt_for_test(&[segment_with_content("debug the build")]);
    assert!(prompt.contains("model_failures"));
    assert!(prompt.contains("debug the build"));
    assert!(prompt.contains("Log segments:"));
    assert!(prompt.contains("untrusted data"));
}

#[test]
fn groq_prompt_covers_all_segments_and_redacts_credentials() {
    let client = GroqClient::new("test").unwrap();
    let segments: Vec<Segment> = (0..25)
        .map(|index| {
            segment_with_content(&format!(
                "segment-{index} api_key=secret-value-{index} authorization: Bearer token-{index} user@example.com /home/alice/project"
            ))
        })
        .collect();
    let prompts = client.build_analysis_prompts_for_test(&segments);
    let combined = prompts.join("\n");
    assert!(combined.contains("segment-24"));
    assert!(!combined.contains("secret-value"));
    assert!(!combined.contains("Bearer token"));
    assert!(!combined.contains("user@example.com"));
    assert!(!combined.contains("/home/alice"));
    assert!(prompts.iter().all(|prompt| prompt.len() <= 48_100));
}

#[test]
fn groq_prompt_batches_oversized_entries_without_dropping_tail() {
    let client = GroqClient::new("test").unwrap();
    let content = format!("{}TAIL-MARKER", "a".repeat(100_000));
    let prompts = client.build_analysis_prompts_for_test(&[segment_with_content(&content)]);
    assert!(prompts.len() >= 3);
    assert!(prompts.iter().any(|prompt| prompt.contains("TAIL-MARKER")));
    assert!(prompts.iter().all(|prompt| prompt.len() <= 48_100));
}

#[tokio::test]
async fn aggregator_accepts_json_arrays() {
    let root = std::env::temp_dir().join(format!("dreamseq-logs-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("logs.json"),
        r#"[{"timestamp":"2026-08-07T10:00:00Z","content":"first"},{"timestamp":"2026-08-07T10:01:00Z","content":"second"}]"#,
    )
    .unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "test".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Json,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn aggregator_accepts_json_lines() {
    let root = std::env::temp_dir().join(format!("dreamseq-logs-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("logs.jsonl"),
        "{\"timestamp\":\"2026-08-07T10:00:00Z\",\"content\":\"first\"}\n{\"timestamp\":\"2026-08-07T10:01:00Z\",\"content\":\"second\"}\n",
    )
    .unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "test".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Json,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn ingestion_report_records_malformed_files() {
    let root = std::env::temp_dir().join(format!("dreamseq-invalid-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("broken.jsonl"), "{not json}\n").unwrap();
    let (entries, report) = dreamseq::LogAggregator::new()
        .aggregate_with_report(&[HarnessConfig {
            name: "broken".into(),
            log_path: root.clone(),
            log_format: dreamseq::config::LogFormat::Json,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert!(entries.is_empty());
    assert_eq!(report.files_seen, 1);
    assert_eq!(report.files_failed, 1);
    assert!(!report.harnesses[0].warnings.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn aggregation_does_not_follow_symbolic_links() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("dreamseq-links-{}", uuid::Uuid::new_v4()));
    let outside = std::env::temp_dir().join(format!("dreamseq-outside-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("private.log"), "must not be ingested").unwrap();
    symlink(&outside, root.join("outside-link")).unwrap();

    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "links".into(),
            log_path: root.clone(),
            log_format: dreamseq::config::LogFormat::Plain,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert!(entries.is_empty());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[tokio::test]
async fn aggregator_parses_codex_jsonl_fields() {
    let root = std::env::temp_dir().join(format!("dreamseq-codex-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("history.jsonl"),
        r#"{"session_id":"019e89bf-e974-7423-a6fd-c6c10218a0e1","ts":1780427521,"text":"Hello"}
{"session_id":"019e89bf-e974-7423-a6fd-c6c10218a0e1","ts":1780468096,"text":"Plan the migration"}
"#,
    )
    .unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "codex".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Json,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].content, "Hello");
    assert_eq!(entries[0].timestamp.timestamp(), 1_780_427_521);
    assert_eq!(entries[1].content, "Plan the migration");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn aggregator_parses_markdown_lines() {
    let root = std::env::temp_dir().join(format!("dreamseq-md-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("log.md"),
        "# Session\n- first action\n- second action\n",
    )
    .unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "md".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Markdown,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2, "markdown headings should be skipped");
    assert!(entries.iter().any(|e| e.content == "- first action"));
    assert!(!entries.iter().any(|e| e.content.starts_with('#')));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn aggregator_parses_json_tool_calls() {
    let root = std::env::temp_dir().join(format!("dreamseq-tools-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("logs.json"),
        r#"[{"timestamp":"2026-08-07T10:00:00Z","content":"run check","tool_calls":[{"tool_name":"cargo_check","parameters":{"target":"debug"},"duration_ms":1200}],"model":"gpt"}]"#,
    )
    .unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "test".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Json,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].metadata.tool_calls.len(), 1);
    assert_eq!(entries[0].metadata.tool_calls[0].tool_name, "cargo_check");
    assert_eq!(
        entries[0].metadata.tool_calls[0].parameters["target"],
        "debug"
    );
    assert_eq!(entries[0].metadata.model, Some("gpt".to_string()));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn aggregator_parses_numeric_timestamp_seconds() {
    let root = std::env::temp_dir().join(format!("dreamseq-epoch-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("logs.json"),
        r#"[{"timestamp":1786092899,"content":"epoch entry"}]"#,
    )
    .unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "test".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Json,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "epoch entry");
    assert_eq!(entries[0].timestamp.timestamp(), 1_786_092_899);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn aggregator_extracts_timestamp_from_plain_lines() {
    let root = std::env::temp_dir().join(format!("dreamseq-ts-plain-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("log.txt"),
        "2026-08-07T10:00:00Z first event\n2026-08-07T10:01:00Z second event\n",
    )
    .unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "plain".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Plain,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].timestamp.timestamp(), 1_786_096_800);
    assert_eq!(entries[0].content, "first event");
    assert_eq!(entries[1].content, "second event");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn aggregator_parses_plain_lines() {
    let root = std::env::temp_dir().join(format!("dreamseq-plain-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("log.txt"), "line one\nline two\n").unwrap();
    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "plain".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Plain,
            bound_filter: None,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn segmentation_splits_on_time_gap() {
    use chrono::{Duration, Utc};
    use dreamseq::segmentation::SemanticSegmenter;

    let now = Utc::now();
    let entries = vec![
        LogEntry {
            id: "a".into(),
            harness: "test".into(),
            timestamp: now,
            content: "first task".into(),
            metadata: LogMetadata {
                model: None,
                provider: None,
                tool_calls: vec![],
                user_messages: 0,
                assistant_messages: 0,
            },
        },
        LogEntry {
            id: "b".into(),
            harness: "test".into(),
            timestamp: now + Duration::minutes(45),
            content: "second task".into(),
            metadata: LogMetadata {
                model: None,
                provider: None,
                tool_calls: vec![],
                user_messages: 0,
                assistant_messages: 0,
            },
        },
    ];

    let segments = SemanticSegmenter::new().segment(entries).unwrap();
    assert_eq!(segments.len(), 2);
}

#[test]
fn segmentation_groups_by_shared_keywords() {
    use dreamseq::segmentation::SemanticSegmenter;

    let now = Utc::now();
    let entries = vec![
        log_entry_with_ts("debug the rust build", now),
        log_entry_with_ts("the rust build failed", now + Duration::seconds(10)),
        log_entry_with_ts(
            "completely unrelated topic here",
            now + Duration::seconds(20),
        ),
    ];

    let segments = SemanticSegmenter::new().segment(entries).unwrap();
    assert_eq!(segments.len(), 2);
    let topic = &segments[0].topic;
    assert!(
        topic.contains("build") || topic.contains("rust") || topic.contains("failing"),
        "expected topic to reflect shared keywords, got {}",
        topic
    );
}

#[tokio::test]
async fn full_pipeline_runs_without_api_key_when_no_logs() {
    use dreamseq::{Dreamseq, DreamseqConfig};

    let mut config = DreamseqConfig::default();
    config.groq_api_key.clear();
    config.harnesses.clear();
    config.anthologies_dir =
        std::env::temp_dir().join(format!("dreamseq-anthologies-{}", uuid::Uuid::new_v4()));
    config.output_dir =
        std::env::temp_dir().join(format!("dreamseq-output-{}", uuid::Uuid::new_v4()));

    let dreamseq = Dreamseq::new(config).unwrap();
    let anthology = dreamseq.run().await.unwrap();

    assert_eq!(anthology.pipeline.raw_entries, 0);
    assert_eq!(anthology.pipeline.normalized_entries, 0);
    assert_eq!(anthology.pipeline.segments, 0);
    assert!(anthology.patterns.is_empty());
    assert!(anthology.steering_events.is_empty());
    assert!(anthology.save().is_ok());
    assert_eq!(
        fs::read_dir(&anthology.config.output_dir)
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        1,
        "each run should write one ingestion report"
    );

    fs::remove_dir_all(&anthology.config.anthologies_dir).ok();
    fs::remove_dir_all(&anthology.config.output_dir).ok();
}

#[tokio::test]
async fn nonempty_pipeline_requires_explicit_remote_consent() {
    use dreamseq::{Dreamseq, DreamseqConfig};

    let root = std::env::temp_dir().join(format!("dreamseq-consent-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("log.txt"), "analyze this log").unwrap();
    let mut config = DreamseqConfig::default();
    config.enable_kaptaind = false;
    config.harnesses = vec![HarnessConfig {
        name: "fixture".into(),
        log_path: root.clone(),
        log_format: dreamseq::config::LogFormat::Plain,
        bound_filter: None,
    }];
    config.output_dir = root.join("output");

    let error = Dreamseq::new(config)
        .unwrap()
        .run()
        .await
        .expect_err("remote analysis should require explicit consent");
    assert!(error.to_string().contains("remote analysis is disabled"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn anthology_saves_do_not_overwrite_same_day_runs() {
    use dreamseq::report::Anthology;

    let directory = std::env::temp_dir().join(format!("dreamseq-save-{}", uuid::Uuid::new_v4()));
    let first_config = DreamseqConfig {
        anthologies_dir: directory.clone(),
        ..DreamseqConfig::default()
    };
    let second_config = first_config.clone();
    let first = Anthology::new(Vec::new(), Vec::new(), first_config);
    let second = Anthology::new(Vec::new(), Vec::new(), second_config);
    let first_path = first.save().unwrap();
    let second_path = second.save().unwrap();
    assert_ne!(first_path, second_path);
    assert!(first_path.exists());
    assert!(second_path.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn full_pipeline_with_fixture_data() {
    use dreamseq::{Dreamseq, DreamseqConfig};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = manifest_dir.join("tests").join("fixtures");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let analysis = serde_json::json!({
        "model_failures": [{"model": "gpt-4", "issue": "hallucinated API", "frequency": 2, "example": "foo()"}],
        "harness_friction": [{"harness": "chatgpt", "issue": "slow responses", "severity": 0.7}],
        "missing_tooling": [{"tool_name": "test-runner", "purpose": "rerun failed tests", "estimated_value": 0.8}],
        "workflow_bottlenecks": [{"description": "repeated builds", "frequency": 3, "time_impact_minutes": 15.0}],
        "repeated_commands": [{"command": "cargo test", "frequency": 3, "context": "rust build"}],
        "repeated_prompts": [],
        "context_loss": [],
        "automation_opportunities": [{"description": "automate test rerun", "estimated_time_saved": 10.0, "confidence": 0.9}]
    });
    let analysis_text = serde_json::to_string(&analysis).unwrap();
    let body = serde_json::json!({
        "choices": [{"message": {"content": analysis_text}}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150}
    });
    let body_bytes = body.to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body_bytes.len(),
        body_bytes
    );

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 32768];
        let _ = socket.read(&mut buf).await.unwrap();
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let mut config = DreamseqConfig::default();
    config.groq_api_key = "test-key".to_string();
    config.allow_remote_analysis = true;
    config.groq_base_url = Some(format!("http://{}", addr));
    config.enable_kaptaind = false;
    config.harnesses = vec![
        HarnessConfig {
            name: "chatgpt".into(),
            log_path: fixtures.join("chatgpt"),
            log_format: dreamseq::config::LogFormat::Json,
            bound_filter: None,
        },
        HarnessConfig {
            name: "kimi".into(),
            log_path: fixtures.join("kimi"),
            log_format: dreamseq::config::LogFormat::Plain,
            bound_filter: None,
        },
        HarnessConfig {
            name: "claude".into(),
            log_path: fixtures.join("claude"),
            log_format: dreamseq::config::LogFormat::Markdown,
            bound_filter: None,
        },
    ];
    config.anthologies_dir = std::env::temp_dir().join(format!(
        "dreamseq-fixture-anthologies-{}",
        uuid::Uuid::new_v4()
    ));
    config.output_dir =
        std::env::temp_dir().join(format!("dreamseq-fixture-output-{}", uuid::Uuid::new_v4()));

    let dreamseq = Dreamseq::new(config).unwrap();
    let mut anthology = dreamseq.run().await.unwrap();

    assert!(
        anthology.pipeline.raw_entries >= 8,
        "expected at least 8 raw entries, got {}",
        anthology.pipeline.raw_entries
    );
    assert!(anthology.pipeline.normalized_entries > 0);
    assert!(anthology.pipeline.segments > 0);
    assert!(!anthology.patterns.is_empty());
    assert!(!anthology.steering_events.is_empty());
    assert!(anthology.save().is_ok());

    anthology.generate().unwrap();
    assert!(!anthology.generate_directives().is_empty());

    server.abort();
    let _ = server.await;

    fs::remove_dir_all(&anthology.config.anthologies_dir).ok();
    fs::remove_dir_all(&anthology.config.output_dir).ok();
}

fn bound_available() -> bool {
    std::process::Command::new("bound")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn bound_aggregates_files_as_log_entries() {
    if !bound_available() {
        return;
    }

    let root = std::env::temp_dir().join(format!("dreamseq-bound-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "first bound entry").unwrap();
    fs::write(root.join("b.txt"), "second bound entry").unwrap();

    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "bound".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Bound,
            bound_filter: None,
        }])
        .await
        .unwrap();

    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .any(|e| e.content.contains("first bound entry"))
    );
    assert!(
        entries
            .iter()
            .any(|e| e.content.contains("second bound entry"))
    );
    assert!(entries.iter().all(|e| {
        e.metadata
            .provider
            .as_ref()
            .map(|p| p.starts_with("bound:"))
            .unwrap_or(false)
    }));

    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn bound_filter_limits_scanned_files() {
    if !bound_available() {
        return;
    }

    let root = std::env::temp_dir().join(format!("dreamseq-bound-filter-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("keep.json"), r#"{"msg":"json log"}"#).unwrap();
    fs::write(root.join("ignore.txt"), "plain log").unwrap();

    let entries = dreamseq::LogAggregator::new()
        .aggregate(&[HarnessConfig {
            name: "bound".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Bound,
            bound_filter: Some("[.json]".to_string()),
        }])
        .await
        .unwrap();

    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("json log"));
    assert!(!entries[0].content.contains("plain log"));

    fs::remove_dir_all(root).unwrap();
}
