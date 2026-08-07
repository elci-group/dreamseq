use crate::segmentation::Segment;
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize)]
struct GroqRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: usize,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct GroqResponse {
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    content: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[allow(dead_code)]
    prompt_tokens: usize,
    #[allow(dead_code)]
    completion_tokens: usize,
    total_tokens: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub model_failures: Vec<ModelFailure>,
    pub harness_friction: Vec<HarnessFriction>,
    pub missing_tooling: Vec<MissingTool>,
    pub workflow_bottlenecks: Vec<WorkflowBottleneck>,
    pub repeated_commands: Vec<RepeatedCommand>,
    pub repeated_prompts: Vec<RepeatedPrompt>,
    pub context_loss: Vec<ContextLoss>,
    pub automation_opportunities: Vec<AutomationOpportunity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFailure {
    pub model: String,
    pub issue: String,
    pub frequency: usize,
    pub example: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessFriction {
    pub harness: String,
    pub issue: String,
    pub severity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingTool {
    pub tool_name: String,
    pub purpose: String,
    pub estimated_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowBottleneck {
    pub description: String,
    pub frequency: usize,
    pub time_impact_minutes: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatedCommand {
    pub command: String,
    pub frequency: usize,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatedPrompt {
    pub prompt_pattern: String,
    pub frequency: usize,
    pub suggested_improvement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextLoss {
    pub description: String,
    pub affected_segments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationOpportunity {
    pub description: String,
    pub estimated_time_saved: f64,
    pub confidence: f64,
}

pub struct GroqClient {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl GroqClient {
    pub fn new(api_key: &str) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            api_key: api_key.to_string(),
            model: "openai/gpt-oss-120b".to_string(),
            base_url: "https://api.groq.com/openai/v1".to_string(),
        })
    }

    /// Create a client pointed at a custom base URL for testing.
    #[doc(hidden)]
    pub fn new_with_url(api_key: &str, base_url: &str) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            api_key: api_key.to_string(),
            model: "openai/gpt-oss-120b".to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    pub async fn analyze(&self, segments: &[Segment]) -> Result<Analysis> {
        if segments.is_empty() {
            return Ok(Analysis::default());
        }
        if self.api_key.trim().is_empty() {
            anyhow::bail!("GROQ_API_KEY is required when log segments are available");
        }
        let prompt = self.build_analysis_prompt(segments);

        let request = GroqRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You are an expert at analyzing AI agent interactions and identifying patterns, failures, and improvement opportunities. Analyze the provided log segments and extract actionable insights.".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt,
                },
            ],
            temperature: 0.3,
            max_tokens: 4000,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            anyhow::bail!("Groq API error: {} - {}", status, error_text);
        }

        let groq_response: GroqResponse = response.json().await?;
        let analysis_text = groq_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        tracing::info!(
            "Groq analysis completed. Tokens used: {}",
            groq_response.usage.total_tokens
        );

        self.parse_analysis(&analysis_text)
    }

    fn build_analysis_prompt(&self, segments: &[Segment]) -> String {
        let mut prompt = String::from(
            "Analyze these AI agent log segments and identify patterns. Return a JSON response with this structure:\n\
            {\n\
              \"model_failures\": [{\"model\": \"\", \"issue\": \"\", \"frequency\": 0, \"example\": \"\"}],\n\
              \"harness_friction\": [{\"harness\": \"\", \"issue\": \"\", \"severity\": 0.0}],\n\
              \"missing_tooling\": [{\"tool_name\": \"\", \"purpose\": \"\", \"estimated_value\": 0.0}],\n\
              \"workflow_bottlenecks\": [{\"description\": \"\", \"frequency\": 0, \"time_impact_minutes\": 0.0}],\n\
              \"repeated_commands\": [{\"command\": \"\", \"frequency\": 0, \"context\": \"\"}],\n\
              \"repeated_prompts\": [{\"prompt_pattern\": \"\", \"frequency\": 0, \"suggested_improvement\": \"\"}],\n\
              \"context_loss\": [{\"description\": \"\", \"affected_segments\": []}],\n\
              \"automation_opportunities\": [{\"description\": \"\", \"estimated_time_saved\": 0.0, \"confidence\": 0.0}]\n\
            }\n\n\
            Log segments:\n",
        );

        for (i, segment) in segments.iter().take(10).enumerate() {
            // Limit to prevent token overflow
            prompt.push_str(&format!(
                "\n--- Segment {} (Topic: {}, Confidence: {:.2}) ---\n",
                i, segment.topic, segment.confidence
            ));
            for entry in &segment.entries {
                prompt.push_str(&format!(
                    "[{}] {}: {}\n",
                    entry.harness,
                    entry.timestamp.format("%H:%M:%S"),
                    entry.content
                ));
            }
        }

        prompt
    }

    fn parse_analysis(&self, text: &str) -> Result<Analysis> {
        // Try to extract JSON from the response
        let json_start = text
            .find('{')
            .ok_or_else(|| anyhow::anyhow!("Groq response did not contain a JSON object"))?;
        let json_end = text.rfind('}').ok_or_else(|| {
            anyhow::anyhow!("Groq response did not contain a complete JSON object")
        })?;
        if json_end < json_start {
            anyhow::bail!("Groq response contained malformed JSON boundaries");
        }
        let json_str = &text[json_start..=json_end];

        let mut value: serde_json::Value = serde_json::from_str(json_str)?;
        coerce_analysis_scalars(&mut value);
        let analysis: Analysis = serde_json::from_value(value)?;
        Ok(analysis)
    }

    #[doc(hidden)]
    pub fn parse_analysis_for_test(&self, text: &str) -> Result<Analysis> {
        self.parse_analysis(text)
    }

    #[doc(hidden)]
    pub fn build_analysis_prompt_for_test(&self, segments: &[Segment]) -> String {
        self.build_analysis_prompt(segments)
    }
}

/// GPT OSS occasionally emits a number or boolean in a descriptive field.
/// Preserve metric fields, but stringify scalar values where our public
/// analysis schema intentionally uses human-readable text.
fn coerce_analysis_scalars(value: &mut serde_json::Value) {
    const TEXT_FIELDS: &[&str] = &[
        "model",
        "issue",
        "example",
        "harness",
        "tool_name",
        "purpose",
        "description",
        "command",
        "context",
        "prompt_pattern",
        "suggested_improvement",
    ];

    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if TEXT_FIELDS.contains(&key.as_str()) && !child.is_string() && !child.is_null() {
                    let replacement = match &*child {
                        serde_json::Value::Number(number) => number.to_string(),
                        serde_json::Value::Bool(boolean) => boolean.to_string(),
                        other => other.to_string(),
                    };
                    *child = serde_json::Value::String(replacement);
                } else if key == "affected_segments" {
                    if let serde_json::Value::Array(items) = child {
                        for item in items {
                            if !item.is_string() && !item.is_null() {
                                *item = serde_json::Value::String(match &*item {
                                    serde_json::Value::Number(number) => number.to_string(),
                                    serde_json::Value::Bool(boolean) => boolean.to_string(),
                                    other => other.to_string(),
                                });
                            }
                        }
                    }
                } else {
                    coerce_analysis_scalars(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                coerce_analysis_scalars(item);
            }
        }
        _ => {}
    }
}
