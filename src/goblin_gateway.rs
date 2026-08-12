//! Deterministic Goblin boundaries for Dreamseq's probabilistic pipeline.

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
    let Some(object) = envelope.as_object() else {
        return GatewayDecision::Reject("run envelope must be a JSON object".into());
    };
    if object.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return GatewayDecision::Reject("unsupported or missing schema_version".into());
    }
    if !object.get("run").is_some_and(Value::is_object) {
        return GatewayDecision::Reject("run envelope must contain an object-valued run".into());
    }
    if threshold < 0.5 {
        GatewayDecision::Escalate("validation threshold is below the safe minimum".into())
    } else {
        GatewayDecision::Accept
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
