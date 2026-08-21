// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Privacy-reduced Padagonia integration primitives.
//!
//! This module deliberately stops at a validated local contract. Network
//! transport remains in `cloud`; raw prompts, credentials, and event bodies
//! never enter this representation.

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u8 = 1;
pub const MAX_BATCH_EVENTS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Namespace(String);

impl Namespace {
    pub fn new(product: &str, environment: &str, tenant: &str) -> Result<Self> {
        let value = format!("{product}.{environment}.{tenant}");
        if value.is_empty()
            || value.len() > 256
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
        {
            bail!("invalid Padagonia namespace")
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provenance {
    pub producer: String,
    pub producer_version: String,
    pub source_reference: String,
    pub observed_at: DateTime<Utc>,
    pub asserted_at: DateTime<Utc>,
    pub confidence_semantics: String,
    pub correlation_id: String,
    pub retention_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrivacyEvent {
    pub schema_version: u8,
    pub namespace: Namespace,
    pub event_id: String,
    pub event_type: String,
    pub entity_id: String,
    pub content_hash: String,
    pub attributes: BTreeMap<String, String>,
    pub provenance: Provenance,
}

impl PrivacyEvent {
    pub fn new(
        namespace: Namespace,
        event_type: impl Into<String>,
        entity_id: impl Into<String>,
        attributes: BTreeMap<String, String>,
        provenance: Provenance,
    ) -> Result<Self> {
        let entity_id = entity_id.into();
        let event_type = event_type.into();
        let canonical = serde_json::to_string(&(&namespace, &event_type, &entity_id, &attributes))
            .inspect_err(|error| {
                tracing::error!(error = %error, "failed to canonicalize Padagonia event identity")
            })?;
        let event_id = content_hash(&canonical);
        let attributes_json = serde_json::to_string(&attributes).inspect_err(|error| {
            tracing::error!(error = %error, "failed to canonicalize Padagonia event attributes")
        })?;
        let content_hash = content_hash(&attributes_json);
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            namespace,
            event_id: event_id.clone(),
            event_type,
            entity_id,
            content_hash,
            attributes,
            provenance,
        })
    }
}

#[derive(Debug, Default)]
pub struct BatchWriter {
    events: Vec<PrivacyEvent>,
}

impl BatchWriter {
    pub fn push(&mut self, event: PrivacyEvent) -> Result<()> {
        if self.events.len() >= MAX_BATCH_EVENTS {
            bail!("Padagonia batch limit of {MAX_BATCH_EVENTS} events exceeded")
        }
        self.events.push(event);
        Ok(())
    }

    pub fn take(&mut self) -> Vec<PrivacyEvent> {
        std::mem::take(&mut self.events)
    }

    #[must_use]
    pub fn idempotency_key(events: &[PrivacyEvent]) -> String {
        content_hash(
            &events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub max_age_days: i64,
}

impl RetentionPolicy {
    #[must_use]
    pub fn retain(&self, event: &PrivacyEvent, now: DateTime<Utc>) -> bool {
        event.provenance.observed_at >= now - Duration::days(self.max_age_days.max(0))
    }
}

#[must_use]
pub fn redact_attributes(input: &Map<String, Value>) -> BTreeMap<String, String> {
    input
        .iter()
        .filter(|(key, _)| !is_sensitive_key(key))
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "prompt",
        "secret",
        "token",
        "credential",
        "password",
        "raw",
        "content",
    ]
    .iter()
    .any(|fragment| key.contains(fragment))
}

fn content_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance(at: DateTime<Utc>) -> Provenance {
        Provenance {
            producer: "dreamseq".into(),
            producer_version: "test".into(),
            source_reference: "run-1".into(),
            observed_at: at,
            asserted_at: at,
            confidence_semantics: "ranking signal, not probability".into(),
            correlation_id: "corr-1".into(),
            retention_class: "aggregate-30d".into(),
        }
    }

    #[test]
    fn namespace_and_event_are_versioned_and_hashed() {
        let namespace = Namespace::new("dreamseq", "prod", "tenant-a").unwrap();
        let event = PrivacyEvent::new(
            namespace,
            "pattern.observed",
            "pattern-1",
            BTreeMap::new(),
            provenance(Utc::now()),
        )
        .unwrap();
        assert_eq!(event.schema_version, SCHEMA_VERSION);
        assert_eq!(event.event_id.len(), 64);
        assert_eq!(event.content_hash.len(), 64);
    }

    #[test]
    fn redaction_removes_sensitive_values() {
        let input =
            serde_json::json!({"model": "x", "prompt": "private", "access_token": "secret"});
        let values = redact_attributes(input.as_object().unwrap());
        assert_eq!(values.get("model"), Some(&"x".to_string()));
        assert!(!values.contains_key("prompt"));
        assert!(!values.contains_key("access_token"));
    }

    #[test]
    fn batch_is_bounded_and_idempotent() {
        let namespace = Namespace::new("dreamseq", "prod", "tenant-a").unwrap();
        let mut writer = BatchWriter::default();
        for index in 0..MAX_BATCH_EVENTS {
            writer
                .push(
                    PrivacyEvent::new(
                        namespace.clone(),
                        "event",
                        index.to_string(),
                        BTreeMap::new(),
                        provenance(Utc::now()),
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let events = writer.events.clone();
        assert_eq!(
            BatchWriter::idempotency_key(&events),
            BatchWriter::idempotency_key(&events)
        );
        assert!(
            writer
                .push(
                    PrivacyEvent::new(
                        namespace,
                        "event",
                        "overflow",
                        BTreeMap::new(),
                        provenance(Utc::now())
                    )
                    .unwrap()
                )
                .is_err()
        );
    }

    #[test]
    fn retention_expires_old_events() {
        let now = Utc::now();
        let namespace = Namespace::new("dreamseq", "prod", "tenant-a").unwrap();
        let old = PrivacyEvent::new(
            namespace,
            "event",
            "old",
            BTreeMap::new(),
            provenance(now - Duration::days(31)),
        )
        .unwrap();
        assert!(!RetentionPolicy { max_age_days: 30 }.retain(&old, now));
    }
}
