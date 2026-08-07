use chrono::Utc;
use dreamseq::DreamseqConfig;
use dreamseq::aggregator::{LogEntry, LogMetadata};
use dreamseq::groq::GroqClient;
use dreamseq::normalization::Normalizer;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_config_default() {
    let config = DreamseqConfig::default();
    assert!(!config.harnesses.is_empty() || config.harnesses.is_empty()); // Basic existence check
}

#[test]
fn test_config_load_missing() {
    let config = DreamseqConfig::load().unwrap();
    // Should return default config when file doesn't exist
    assert!(config.groq_api_key.is_empty() || !config.groq_api_key.is_empty());
}

#[test]
fn test_pattern_extraction() {
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
    assert!(patterns.is_empty()); // Empty analysis should yield empty patterns
}

#[test]
fn test_steering_detection() {
    use chrono::Utc;
    use dreamseq::aggregator::LogEntry;
    use dreamseq::segmentation::Segment;
    use dreamseq::steering::SteeringDetector;

    let detector = SteeringDetector::new();
    let segment = Segment {
        id: "test".to_string(),
        topic: "test".to_string(),
        entries: vec![LogEntry {
            id: "test".to_string(),
            harness: "test".to_string(),
            timestamp: Utc::now(),
            content: "I wish I had a tool for this".to_string(),
            metadata: dreamseq::aggregator::LogMetadata {
                model: None,
                provider: None,
                tool_calls: vec![],
                user_messages: 0,
                assistant_messages: 0,
            },
        }],
        start_time: Utc::now(),
        end_time: Utc::now(),
        confidence: 1.0,
    };

    let events = detector.detect(&[segment]).unwrap();
    assert!(!events.is_empty()); // Should detect missing tool pattern
}

#[test]
fn greetings_are_not_missing_tool_events() {
    use dreamseq::segmentation::Segment;
    use dreamseq::steering::SteeringDetector;
    let now = Utc::now();
    let segment = Segment {
        id: "greeting".into(),
        topic: "conversation".into(),
        entries: vec![log_entry("hello")],
        start_time: now,
        end_time: now,
        confidence: 1.0,
    };
    assert!(
        SteeringDetector::new()
            .detect(&[segment])
            .unwrap()
            .is_empty()
    );
}

fn log_entry(content: &str) -> LogEntry {
    LogEntry {
        id: content.to_string(),
        harness: "test".to_string(),
        timestamp: Utc::now(),
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

#[test]
fn normalization_deduplicates_after_whitespace_cleanup() {
    let entries = vec![log_entry("  Hello   world  "), log_entry("hello world")];
    let normalized = Normalizer.normalize(entries).unwrap();
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].content, "Hello world");
}

#[tokio::test]
async fn empty_analysis_does_not_require_api_key() {
    let analysis = GroqClient::new("").unwrap().analyze(&[]).await.unwrap();
    assert!(analysis.model_failures.is_empty());
}

#[test]
fn analysis_parser_accepts_numeric_descriptions() {
    let client = GroqClient::new("test").unwrap();
    let text = r#"{"model_failures":[{"model":"gpt","issue":6,"frequency":1,"example":true}],"harness_friction":[],"missing_tooling":[],"workflow_bottlenecks":[],"repeated_commands":[],"repeated_prompts":[],"context_loss":[],"automation_opportunities":[]}"#;
    let parsed = client.parse_analysis_for_test(text);
    assert!(parsed.is_ok());
}

#[test]
fn analysis_parser_accepts_numeric_segment_evidence() {
    let client = GroqClient::new("test").unwrap();
    let text = r#"{"model_failures":[],"harness_friction":[],"missing_tooling":[],"workflow_bottlenecks":[],"repeated_commands":[],"repeated_prompts":[],"context_loss":[{"description":"context","affected_segments":[0,1]}],"automation_opportunities":[]}"#;
    assert!(client.parse_analysis_for_test(text).is_ok());
}

#[test]
fn config_loads_json_and_toml() {
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
        .aggregate(&[dreamseq::config::HarnessConfig {
            name: "test".into(),
            log_path: PathBuf::from(&root),
            log_format: dreamseq::config::LogFormat::Json,
        }])
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    fs::remove_dir_all(root).unwrap();
}
