pub mod aggregator;
pub mod config;
pub mod groq;
pub mod kaptaind;
pub mod normalization;
pub mod patterns;
pub mod report;
pub mod segmentation;
pub mod steering;
pub mod trends;

pub use aggregator::LogAggregator;
pub use config::DreamseqConfig;
pub use groq::GroqClient;
pub use kaptaind::KaptaindMonitor;
pub use normalization::Normalizer;
pub use patterns::PatternExtractor;
pub use report::{Anthology, Directive, PipelineStats, Priority};
pub use segmentation::SemanticSegmenter;
pub use steering::SteeringDetector;
pub use trends::{TrendAnalysis, TrendAnalyzer};

use anyhow::Result;

/// Main Dreamseq engine
pub struct Dreamseq {
    config: DreamseqConfig,
    aggregator: LogAggregator,
    normalizer: Normalizer,
    segmenter: SemanticSegmenter,
    groq_client: GroqClient,
    pattern_extractor: PatternExtractor,
    steering_detector: SteeringDetector,
    trend_analyzer: TrendAnalyzer,
    kaptaind_monitor: Option<KaptaindMonitor>,
}

impl Dreamseq {
    pub fn new(config: DreamseqConfig) -> Result<Self> {
        let groq_client = GroqClient::new(&config.groq_api_key)?;
        let anthologies_dir = config.anthologies_dir.clone();

        let kaptaind_monitor = if config.enable_kaptaind {
            Some(KaptaindMonitor::new(std::env::current_dir()?))
        } else {
            None
        };

        Ok(Self {
            config,
            aggregator: LogAggregator::new(),
            normalizer: Normalizer::new(),
            segmenter: SemanticSegmenter::new(),
            groq_client,
            pattern_extractor: PatternExtractor::new(),
            steering_detector: SteeringDetector::new(),
            trend_analyzer: TrendAnalyzer::with_directory(anthologies_dir),
            kaptaind_monitor,
        })
    }

    /// Run the complete Dreamseq pipeline
    pub async fn run(&self) -> Result<Anthology> {
        tracing::info!("Starting Dreamseq pipeline");

        // Step 0: Check kaptaind status if enabled
        if let Some(monitor) = &self.kaptaind_monitor
            && let Ok(status) = monitor.status()
        {
            tracing::info!("Kaptaind status: {}", status);
        }

        // Step 1: Aggregate logs from all harnesses
        let raw_logs = self.aggregator.aggregate(&self.config.harnesses).await?;
        let raw_count = raw_logs.len();
        tracing::info!("Aggregated {} log entries", raw_count);

        // Step 2: Normalize logs
        let normalized_logs = self.normalizer.normalize(raw_logs)?;
        tracing::info!("Normalized to {} unique entries", normalized_logs.len());

        // Step 3: Semantic segmentation
        let normalized_count = normalized_logs.len();
        let estimated_input_tokens = normalized_logs
            .iter()
            .map(|entry| entry.content.split_whitespace().count())
            .sum();
        let segments = self.segmenter.segment(normalized_logs)?;
        tracing::info!("Created {} segments", segments.len());

        // Step 4: Analyze with Groq
        let analysis = self.groq_client.analyze(&segments).await?;
        tracing::info!("Completed Groq analysis");

        // Step 5: Extract patterns
        let patterns = self.pattern_extractor.extract(&analysis)?;
        tracing::info!("Extracted {} patterns", patterns.len());

        // Step 6: Detect user steering
        let steering_events = self.steering_detector.detect(&segments)?;
        tracing::info!("Detected {} steering events", steering_events.len());

        // Step 7: Generate anthology
        let mut anthology = Anthology::new(patterns, steering_events, self.config.clone());
        anthology.set_pipeline_stats(PipelineStats {
            raw_entries: raw_count,
            normalized_entries: normalized_count,
            segments: segments.len(),
            estimated_input_tokens,
        });

        // Step 8: Cross-day trend analysis
        if let Ok(trends) = self.trend_analyzer.analyze(&anthology).await {
            anthology.add_trends(trends);
            tracing::info!("Added trend analysis");
        }

        // Step 9: Run kaptaind analysis if enabled
        if let Some(monitor) = &self.kaptaind_monitor
            && let Ok(analysis) = monitor.analyze()
        {
            tracing::info!("Kaptaind analysis: {}", analysis);
        }

        Ok(anthology)
    }
}
