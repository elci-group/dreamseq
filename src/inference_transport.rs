// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use super::{
    CloudInferenceResponse, GroqClient, InferenceOutput, InferenceRequest, InferenceRoute,
    MAX_ATTEMPTS, OpenAiResponse,
};
use crate::cloud::Credentials;
use anyhow::Result;
use reqwest::StatusCode;
use std::time::Duration;

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
                    let error = super::truncate(&response.text().await.unwrap_or_default(), 300);
                    if !retryable || attempt == MAX_ATTEMPTS {
                        anyhow::bail!("HTTP {status}: {error}");
                    }
                }
                Err(error) if attempt == MAX_ATTEMPTS => return Err(error.into()),
                Err(error) => {
                    tracing::debug!(attempt, error = %error, "transient cloud inference error, retrying");
                }
            }
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
                    let error = super::truncate(&response.text().await.unwrap_or_default(), 300);
                    if !retryable || attempt == MAX_ATTEMPTS {
                        anyhow::bail!("HTTP {status}: {error}");
                    }
                }
                Err(error) if attempt == MAX_ATTEMPTS => return Err(error.into()),
                Err(error) => {
                    tracing::debug!(attempt, error = %error, "transient provider error, retrying");
                }
            }
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
