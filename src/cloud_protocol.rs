// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

const MAX_API_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Serialize)]
pub(super) struct DevicePoll<'a> {
    pub(super) device_code: &'a str,
}

#[derive(Debug, Deserialize)]
pub(super) struct DeviceToken {
    pub(super) access_token: String,
    pub(super) account_id: String,
    pub(super) device_id: String,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: Option<String>,
    message: Option<String>,
}

pub(super) async fn parse_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    trace_id: &str,
) -> Result<T> {
    tracing::debug!(trace_id, status = %response.status(), "decoding Dreamsequence API response");
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_API_RESPONSE_BYTES as u64)
    {
        anyhow::bail!("Dreamsequence API response exceeds the 4 MiB limit");
    }
    let bytes = response.bytes().await?;
    decode_response(status, &bytes)
}

fn decode_response<T: for<'de> Deserialize<'de>>(status: StatusCode, bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_API_RESPONSE_BYTES {
        anyhow::bail!("Dreamsequence API response exceeds the 4 MiB limit");
    }
    if !status.is_success() {
        let error = serde_json::from_slice::<ApiError>(bytes).unwrap_or(ApiError {
            error: None,
            message: None,
        });
        anyhow::bail!(
            "Dreamsequence API returned {}: {}",
            status,
            error
                .message
                .or(error.error)
                .unwrap_or_else(|| "request failed".to_string())
        );
    }
    serde_json::from_slice(bytes).context("Dreamsequence API returned invalid JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_success_and_structured_api_errors() {
        let value: serde_json::Value = decode_response(StatusCode::OK, br#"{"ok":true}"#).unwrap();
        assert_eq!(value["ok"], true);

        let error = decode_response::<serde_json::Value>(
            StatusCode::UNAUTHORIZED,
            br#"{"error":"invalid_token","message":"Pair this device again"}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("401 Unauthorized"));
        assert!(error.to_string().contains("Pair this device again"));
    }

    #[test]
    fn rejects_invalid_and_oversized_success_bodies() {
        let invalid =
            decode_response::<serde_json::Value>(StatusCode::OK, b"not-json").unwrap_err();
        assert!(invalid.to_string().contains("invalid JSON"));

        let oversized = vec![b'x'; MAX_API_RESPONSE_BYTES + 1];
        let error = decode_response::<serde_json::Value>(StatusCode::OK, &oversized).unwrap_err();
        assert!(error.to_string().contains("4 MiB"));
    }

    #[test]
    fn falls_back_when_an_api_error_has_no_json_body() {
        let error =
            decode_response::<serde_json::Value>(StatusCode::BAD_GATEWAY, b"upstream").unwrap_err();
        assert!(error.to_string().contains("request failed"));
    }
}
