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
    assert!(config.enable_kaptaind);
    assert_eq!(
        config.groq_api_key,
        std::env::var("GROQ_API_KEY").unwrap_or_default()
    );
    assert_eq!(config.output_dir, home.join("dreamseq").join("output"));
}

#[test]
fn test_config_load_missing_uses_discovered_defaults() {
    let config = DreamseqConfig::load().unwrap();
    let default = DreamseqConfig::default();

    // When the config file is absent, load() falls back to discovery(), which
    // keeps the same default directories and flags.
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
fn normalization_deduplicates_after_whitespace_cleanup() {
    let entries = vec![log_entry("  Hello   world  "), log_entry("hello world")];
    let normalized = Normalizer.normalize(entries).unwrap();
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].content, "Hello world");
}

#[test]
fn normalization_counts_empty_and_duplicate_entries() {
    let entries = vec![
        log_entry("first"),
        log_entry("first"),
        log_entry("   "),
        log_entry("second"),
    ];
    let normalized = Normalizer.normalize(entries.clone()).unwrap();
    assert_eq!(normalized.len(), 2);
    assert!(normalized.iter().any(|e| e.content == "first"));
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
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
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

    fs::remove_dir_all(&anthology.config.anthologies_dir).ok();
    fs::remove_dir_all(&anthology.config.output_dir).ok();
}
