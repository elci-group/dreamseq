// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use super::{
    AnthropicMessage, AnthropicRequest, AnthropicResponse, CloudInferenceResponse, GroqClient,
    InferenceOutput, InferenceRequest, InferenceRoute, MAX_ATTEMPTS, OpenAiResponse,
};
use crate::cloud::Credentials;
use anyhow::Result;
use reqwest::StatusCode;
use std::time::Duration;

/// A failed inference call, carrying the HTTP status (when the provider
/// actually responded) so callers can classify the failure — e.g. 401/403
/// as unrecoverable versus 429/5xx as worth a cooldown — without parsing
/// error text.
#[derive(Debug)]
pub(super) struct InferenceError {
    pub(super) status: Option<u16>,
    message: String,
}

impl std::fmt::Display for InferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(f, "HTTP {status}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for InferenceError {}

impl GroqClient {
    pub(super) async fn request_cloud(
        &self,
        credentials: &Credentials,
        request: &InferenceRequest,
    ) -> Result<InferenceOutput> {
        for attempt in 1..=MAX_ATTEMPTS {
            let response = self
                .client
                .post(format!(
                    "{}/api/v1/inference",
                    crate::cloud::effective_api_url(&credentials.api_url).trim_end_matches('/')
                ))
                .bearer_auth(&credentials.access_token)
                .json(request)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    let response: CloudInferenceResponse = response.json().await?;
                    return Ok(InferenceOutput {
                        content: response.content,
                        tokens_used: response.usage.total_tokens,
                        provider: response.provider,
                        model: response.model,
                    });
                }
                Ok(response) => {
                    let status = response.status();
                    let retryable = retryable_status(status);
                    let message = super::truncate(&response.text().await.unwrap_or_default(), 300);
                    if !retryable || attempt == MAX_ATTEMPTS {
                        return Err(InferenceError {
                            status: Some(status.as_u16()),
                            message,
                        }
                        .into());
                    }
                }
                Err(error) if attempt == MAX_ATTEMPTS => return Err(error.into()),
                Err(error) => {
                    tracing::debug!(attempt, error = %error, "transient cloud inference error, retrying");
                }
            }
            crate::progress::stage(
                "  ⏳",
                &format!("Retrying Dreamsequence cloud inference (attempt {}/{MAX_ATTEMPTS})...", attempt + 1),
            );
            tokio::time::sleep(backoff(attempt)).await;
        }
        anyhow::bail!("cloud inference exhausted retry attempts")
    }

    pub(super) async fn request_openai_compatible(
        &self,
        route: &InferenceRoute,
        request: &InferenceRequest,
    ) -> Result<InferenceOutput> {
        for attempt in 1..=MAX_ATTEMPTS {
            let response = self
                .client
                .post(format!("{}/chat/completions", route.base_url))
                .bearer_auth(&route.api_key)
                .json(request)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    let response: OpenAiResponse = response.json().await?;
                    let content = response
                        .choices
                        .first()
                        .map(|choice| choice.message.content.clone())
                        .ok_or_else(|| anyhow::anyhow!("response contained no choices"))?;
                    return Ok(InferenceOutput {
                        content,
                        tokens_used: response.usage.total_tokens,
                        provider: route.name.clone(),
                        model: route.model.clone(),
                    });
                }
                Ok(response) => {
                    let status = response.status();
                    let retryable = retryable_status(status);
                    let message = super::truncate(&response.text().await.unwrap_or_default(), 300);
                    if !retryable || attempt == MAX_ATTEMPTS {
                        return Err(InferenceError {
                            status: Some(status.as_u16()),
                            message,
                        }
                        .into());
                    }
                }
                Err(error) if attempt == MAX_ATTEMPTS => return Err(error.into()),
                Err(error) => {
                    tracing::debug!(attempt, error = %error, "transient provider error, retrying");
                }
            }
            crate::progress::stage(
                "  ⏳",
                &format!("Retrying '{}' (attempt {}/{MAX_ATTEMPTS})...", route.name, attempt + 1),
            );
            tokio::time::sleep(backoff(attempt)).await;
        }
        anyhow::bail!("provider exhausted retry attempts")
    }

    /// Anthropic's Messages API is not OpenAI-shaped: `x-api-key` instead of
    /// a bearer token, `/v1/messages` instead of `/chat/completions`, a
    /// top-level `system` field instead of a `system`-role message, and a
    /// typed content-block array in the response instead of `choices`.
    pub(super) async fn request_anthropic(
        &self,
        route: &InferenceRoute,
        request: &InferenceRequest,
    ) -> Result<InferenceOutput> {
        let system = request
            .messages
            .iter()
            .find(|message| message.role == "system")
            .map(|message| message.content.as_str());
        let messages: Vec<AnthropicMessage<'_>> = request
            .messages
            .iter()
            .filter(|message| message.role != "system")
            .map(|message| AnthropicMessage {
                role: &message.role,
                content: &message.content,
            })
            .collect();
        let body = AnthropicRequest {
            model: &request.model,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            system,
            messages,
        };

        for attempt in 1..=MAX_ATTEMPTS {
            let response = self
                .client
                .post(format!("{}/v1/messages", route.base_url))
                .header("x-api-key", &route.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    let response: AnthropicResponse = response.json().await?;
                    let content: String = response
                        .content
                        .into_iter()
                        .filter(|block| block.block_type == "text")
                        .map(|block| block.text)
                        .collect();
                    if content.is_empty() {
                        anyhow::bail!("response contained no text content");
                    }
                    return Ok(InferenceOutput {
                        content,
                        tokens_used: response.usage.input_tokens + response.usage.output_tokens,
                        provider: route.name.clone(),
                        model: route.model.clone(),
                    });
                }
                Ok(response) => {
                    let status = response.status();
                    let retryable = retryable_status(status);
                    let message = super::truncate(&response.text().await.unwrap_or_default(), 300);
                    if !retryable || attempt == MAX_ATTEMPTS {
                        return Err(InferenceError {
                            status: Some(status.as_u16()),
                            message,
                        }
                        .into());
                    }
                }
                Err(error) if attempt == MAX_ATTEMPTS => return Err(error.into()),
                Err(error) => {
                    tracing::debug!(attempt, error = %error, "transient anthropic provider error, retrying");
                }
            }
            crate::progress::stage(
                "  ⏳",
                &format!("Retrying '{}' (attempt {}/{MAX_ATTEMPTS})...", route.name, attempt + 1),
            );
            tokio::time::sleep(backoff(attempt)).await;
        }
        anyhow::bail!("provider exhausted retry attempts")
    }
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 409 | 425 | 429) || status.is_server_error()
}

fn backoff(attempt: usize) -> Duration {
    Duration::from_millis(250 * (1_u64 << attempt.saturating_sub(1).min(3)))
}
