//! Deterministic Goblin boundaries for Dreamseq's probabilistic pipeline.

use goblin::{CapabilityId, ResponseStatus, run_capability};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayDecision {
    Accept,
    Escalate(String),
    Reject(String),
}

impl fmt::Display for GatewayDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept => f.write_str("accept"),
            Self::Escalate(reason) => write!(f, "escalate: {reason}"),
            Self::Reject(reason) => write!(f, "reject: {reason}"),
        }
    }
}

/// Validate a Dreamseq run envelope before it is persisted or uploaded.
/// Observe-only callers can record the decision without blocking the pipeline.
pub fn validate_run_envelope(envelope: &Value, threshold: f64) -> GatewayDecision {
    let input = serde_json::json!({
        "data": envelope,
        "required": ["schema_version", "run"]
    });
    let response = run_capability(
        CapabilityId::SchemaValidation,
        &input.to_string(),
        threshold,
    );
    match response.status {
        ResponseStatus::Success => {
            if response.result.get("valid").and_then(Value::as_bool) == Some(true) {
                GatewayDecision::Accept
            } else {
                GatewayDecision::Reject(response.result.to_string())
            }
        }
        ResponseStatus::Escalate => GatewayDecision::Escalate(response.result.to_string()),
        ResponseStatus::Error => GatewayDecision::Reject(response.result.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_minimum_run_envelope() {
        let value = serde_json::json!({"schema_version": 1, "run": {"id": "r1"}});
        assert_eq!(validate_run_envelope(&value, 0.9), GatewayDecision::Accept);
    }

    #[test]
    fn rejects_missing_run() {
        let value = serde_json::json!({"schema_version": 1});
        assert!(matches!(
            validate_run_envelope(&value, 0.9),
            GatewayDecision::Reject(_)
        ));
    }
}
