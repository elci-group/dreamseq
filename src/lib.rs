pub mod aggregator;
pub mod analysis;
pub mod bound;
pub mod cloud;
pub mod color;
pub mod config;
pub(crate) mod fs_security;
pub mod groq;
pub mod kaptaind;
pub mod normalization;
pub mod patterns;
pub mod present;
pub mod report;
mod report_persistence;
mod report_types;
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
pub mod goblin_gateway;

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
        let groq_client = if let Some(base_url) = &config.groq_base_url {
            GroqClient::new_with_url(&config.groq_api_key, base_url)?
        } else {
            let cloud = if std::env::var("DREAMSEQ_DISABLE_CLOUD_INFERENCE").as_deref() == Ok("1") {
                None
            } else {
                match crate::cloud::CredentialStore::discover()
                    .and_then(|store| store.load_optional())
                {
                    Ok(credentials) => credentials,
                    Err(error) => {
                        tracing::warn!(error = %error, "stored cloud credentials are unavailable; continuing with BYOK routes");
                        None
                    }
                }
            };
            GroqClient::new_routed(&config.groq_api_key, cloud)?
        };
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
        tracing::info!(
            harnesses = self.config.harnesses.len(),
            "starting Dreamseq pipeline"
        );

        // Step 0: Check kaptaind status if enabled
        if let Some(monitor) = &self.kaptaind_monitor
            && let Ok(status) = monitor.status()
        {
            tracing::info!(status = %status.trim(), "Kaptaind status checked");
        }

        // Step 1: Aggregate logs from all harnesses
        let (raw_logs, ingestion_report) = self
            .aggregator
            .aggregate_with_report(&self.config.harnesses)
            .await?;
        let raw_count = raw_logs.len();
        self.save_ingestion_report(&ingestion_report).await?;
        tracing::info!(raw_entries = raw_count, "aggregated log entries");

        // Step 2: Normalize logs
        let normalized_logs = self.normalizer.normalize(raw_logs)?;
        tracing::info!(
            normalized_entries = normalized_logs.len(),
            "normalized log entries while retaining repeated evidence"
        );

        // Step 3: Semantic segmentation
        let normalized_count = normalized_logs.len();
        let estimated_input_tokens = normalized_logs
            .iter()
            .map(|entry| entry.content.split_whitespace().count())
            .sum();
        let segments = self.segmenter.segment(normalized_logs)?;
        tracing::info!(segments = segments.len(), "created semantic segments");

        // Step 4: Analyze with Groq
        if !segments.is_empty() && !self.config.allow_remote_analysis {
            anyhow::bail!(
                "remote analysis is disabled; set allow_remote_analysis=true after reviewing the privacy implications"
            );
        }
        let analysis = self.groq_client.analyze(&segments).await?;
        tracing::info!(
            segments = segments.len(),
            "completed routed inference analysis"
        );

        // Step 5: Extract patterns
        let patterns = self.pattern_extractor.extract(&analysis)?;
        tracing::info!(patterns = patterns.len(), "extracted engineering patterns");

        // Step 6: Detect user steering
        let steering_events = self.steering_detector.detect(&segments)?;
        tracing::info!(
            steering_events = steering_events.len(),
            "detected steering events"
        );

        // Step 7: Generate anthology
        let mut anthology = Anthology::new(patterns, steering_events, self.config.clone());
        anthology.set_pipeline_stats(PipelineStats {
            raw_entries: raw_count,
            normalized_entries: normalized_count,
            segments: segments.len(),
            estimated_input_tokens,
        });

        // Step 8: Cross-day trend analysis
        match self.trend_analyzer.analyze(&anthology).await {
            Ok(trends) => {
                anthology.add_trends(trends);
                tracing::info!(anthology_id = %anthology.id, "added trend analysis");
            }
            Err(error) => tracing::warn!(error = %error, "trend analysis was unavailable"),
        }

        // Step 9: Run kaptaind analysis if enabled
        if let Some(monitor) = &self.kaptaind_monitor {
            match monitor.analyze() {
                Ok(analysis) => tracing::info!(analysis, "Kaptaind analysis completed"),
                Err(error) => tracing::warn!(error = %error, "Kaptaind analysis was unavailable"),
            }
        }

        Ok(anthology)
    }

    async fn save_ingestion_report(
        &self,
        report: &crate::aggregator::IngestionReport,
    ) -> Result<()> {
        tokio::fs::create_dir_all(&self.config.output_dir).await?;
        let path = self
            .config
            .output_dir
            .join(format!("ingestion-{}.json", uuid::Uuid::new_v4().simple()));
        let content = serde_json::to_vec_pretty(report)?;
        tokio::fs::write(&path, content).await?;
        tracing::info!(path = %path.display(), "saved ingestion report");
        Ok(())
    }
}
