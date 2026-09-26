// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! The shared `lucid.session/v1` fixture (also checked by Lucid against its
//! own types and OpenAPI schema) ingests through the real aggregator.

use dreamseq::aggregator::LogAggregator;
use dreamseq::config::{HarnessConfig, LogFormat};
use std::path::PathBuf;

#[tokio::test]
async fn the_lucid_contract_fixture_ingests_with_provenance() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lucid");
    let harness = HarnessConfig {
        name: "lucid".into(),
        log_path: dir,
        log_format: LogFormat::LucidSession,
        bound_filter: None,
    };
    let (entries, report) = LogAggregator::new()
        .aggregate_with_report(std::slice::from_ref(&harness))
        .await
        .unwrap();
    assert_eq!(report.files_failed, 0);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "evt_1", "evt_2", "evt_3", "evt_4", "evt_5", "evt_6", "evt_7"
        ],
        "the redelivered evt_4 collapses; the genuine repeat evt_3/evt_4 stays"
    );
    let rejected: Vec<_> = entries
        .iter()
        .flat_map(|e| e.metadata.tool_calls.iter())
        .filter(|c| c.result.as_ref().is_some_and(|r| r["status"] == "rejected"))
        .collect();
    assert_eq!(rejected.len(), 2);
    assert!(
        entries
            .iter()
            .all(|e| e.metadata.model.as_deref() == Some("claude-sonnet-5"))
    );
    assert_eq!(entries[1].metadata.assistant_messages, 1);
    assert_eq!(entries[5].metadata.user_messages, 1);
}
