// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use chrono::Utc;
use dreamseq::present::{
    AnalysisSummary, CompletionReport, DaemonState, HumanRenderer, Intervention, JsonRenderer,
};
use dreamseq::report::{Anthology, CandidateTool, InterventionCategory, PipelineStats, Priority};
use dreamseq::steering::{SteeringCategory, SteeringEvent};
use std::path::PathBuf;

fn empty_anthology() -> Anthology {
    Anthology::new(Vec::new(), Vec::new(), dreamseq::DreamseqConfig::default())
}

fn anthology_with_interventions(tools: Vec<CandidateTool>) -> Anthology {
    let mut anthology = empty_anthology();
    anthology.candidate_tools = tools;
    anthology.patterns = vec![
        dreamseq::patterns::Pattern {
            id: "p1".into(),
            pattern_type: dreamseq::patterns::PatternType::AutomationOpportunity,
            description: "automate test rerun".into(),
            frequency: 1,
            confidence: 0.9,
            impact_score: 0.85,
            affected_harnesses: vec![],
            estimated_minutes_per_day: Some(15.0),
            manifestation_count: 1,
        },
        dreamseq::patterns::Pattern {
            id: "p2".into(),
            pattern_type: dreamseq::patterns::PatternType::ModelFailure,
            description: "gpt-4: hallucinated API".into(),
            frequency: 2,
            confidence: 0.8,
            impact_score: 0.75,
            affected_harnesses: vec!["gpt-4".into()],
            estimated_minutes_per_day: None,
            manifestation_count: 1,
        },
    ];
    anthology.pipeline = PipelineStats {
        raw_entries: 108763,
        normalized_entries: 53027,
        segments: 42632,
        estimated_input_tokens: 1070791,
        remote_analysis_consent: Some(dreamseq::RemoteAnalysisConsent::AutoApproved),
    };
    anthology
}

fn sample_tool(id: &str, priority: Priority, category: InterventionCategory) -> CandidateTool {
    CandidateTool {
        id: id.into(),
        name: format!("tool-{id}"),
        priority,
        category,
        reason: "Detect payment failures before provider degradation.".into(),
        estimated_time_saved: "90 min/day".into(),
        confidence: 0.9,
        affected_projects: vec![],
        existing_matches: vec!["goblin".into(), "deckhand".into()],
        mutation_fitness: 0.92,
        capability_overlap: 0.67,
        implementation_cost: "medium".into(),
    }
}

#[test]
fn normal_output_contains_all_tiers() {
    let tools = vec![
        sample_tool(
            "DS-0001",
            Priority::High,
            InterventionCategory::MissingCapability,
        ),
        sample_tool(
            "DS-0002",
            Priority::Medium,
            InterventionCategory::WorkflowAcceleration,
        ),
    ];
    let mut anthology = anthology_with_interventions(tools);
    anthology.steering_events = vec![SteeringEvent {
        id: "s1".into(),
        category: SteeringCategory::MissingTool,
        description: "missing tool".into(),
        entry_id: "e1".into(),
        timestamp: Utc::now(),
        context: "I wish I had a tool for this".into(),
        severity: 0.8,
    }];

    let report = CompletionReport::from_anthology(
        &anthology,
        &PathBuf::from("/home/sal/dreamseq"),
        &[
            PathBuf::from("/home/sal/amber"),
            PathBuf::from("/home/sal/goblin"),
        ],
    );

    let renderer = HumanRenderer::new(100, false);
    let output = renderer.render(&report);

    assert!(output.contains("DREAMSEQ"), "missing identity tier");
    assert!(output.contains("ANALYSIS"), "missing analysis tier");
    assert!(output.contains("RESULT"), "missing result tier");
    assert!(output.contains("ARTIFACTS"), "missing artifacts tier");
    assert!(output.contains("COMPLETE"), "missing complete tier");
    assert!(
        output.contains("tool-DS-0001"),
        "missing intervention title"
    );
    assert!(
        output.contains("Missing capability"),
        "missing intervention category"
    );
    assert!(output.contains("goblin, deckhand"), "missing project list");
}

#[test]
fn empty_output_is_friendly() {
    let anthology = empty_anthology();
    let report = CompletionReport::from_anthology(&anthology, &PathBuf::from("/repo"), &[]);
    let renderer = HumanRenderer::new(100, false);
    let output = renderer.render(&report);

    assert!(output.contains("No interventions identified"));
    assert!(!output.contains("HIGH"));
}

#[test]
fn many_interventions_render_without_panic() {
    let tools: Vec<CandidateTool> = (0..50)
        .map(|i| {
            sample_tool(
                &format!("DS-{i:04}"),
                Priority::Low,
                InterventionCategory::Other,
            )
        })
        .collect();
    let anthology = anthology_with_interventions(tools);
    let report = CompletionReport::from_anthology(&anthology, &PathBuf::from("/repo"), &[]);
    let renderer = HumanRenderer::new(100, false);
    let output = renderer.render(&report);

    assert!(output.len() > 1000);
    assert!(output.contains("DS-0049"));
}

#[test]
fn long_rationale_wraps() {
    let long_reason = "word ".repeat(200);
    let tool = CandidateTool {
        reason: long_reason.clone(),
        ..sample_tool(
            "DS-0003",
            Priority::High,
            InterventionCategory::MissingCapability,
        )
    };
    let anthology = anthology_with_interventions(vec![tool]);
    let report = CompletionReport::from_anthology(&anthology, &PathBuf::from("/repo"), &[]);
    let renderer = HumanRenderer::new(80, false);
    let output = renderer.render(&report);

    for line in output.lines() {
        let width = strip_ansi(line).chars().count();
        assert!(width <= 100, "line too long ({} cols): {}", width, line);
    }
}

#[test]
fn narrow_terminal_renders_cleanly() {
    let tools = vec![sample_tool(
        "DS-0004",
        Priority::High,
        InterventionCategory::MissingCapability,
    )];
    let anthology = anthology_with_interventions(tools);
    let report = CompletionReport::from_anthology(&anthology, &PathBuf::from("/repo"), &[]);
    let renderer = HumanRenderer::new(40, false);
    let output = renderer.render(&report);

    assert!(output.contains("DREAMSEQ"));
    assert!(output.contains("ANALYSIS"));
    assert!(output.contains("RESULT"));
}

#[test]
fn verbose_includes_full_paths() {
    let tools = vec![sample_tool(
        "DS-0005",
        Priority::High,
        InterventionCategory::MissingCapability,
    )];
    let anthology = anthology_with_interventions(tools);
    let roots = vec![
        PathBuf::from("/home/sal/amber"),
        PathBuf::from("/home/sal/goblin"),
    ];
    let paths = vec![
        PathBuf::from("/home/sal/amber/.dreams/active.dreams"),
        PathBuf::from("/home/sal/goblin/.dreams/active.dreams"),
    ];
    let mut report = CompletionReport::from_anthology(&anthology, &PathBuf::from("/repo"), &roots);
    report.artifacts.dreams_paths = paths;
    let renderer = HumanRenderer::new(100, true);
    let output = renderer.render(&report);

    assert!(output.contains("DIAGNOSTICS"));
    assert!(output.contains("/home/sal/amber/.dreams/active.dreams"));
    assert!(output.contains("/home/sal/goblin/.dreams/active.dreams"));
}

#[test]
fn json_output_is_valid() {
    let tools = vec![sample_tool(
        "DS-0006",
        Priority::Medium,
        InterventionCategory::WorkflowAcceleration,
    )];
    let anthology = anthology_with_interventions(tools);
    let report = CompletionReport::from_anthology(
        &anthology,
        &PathBuf::from("/repo"),
        &[PathBuf::from("/home/sal/project")],
    );
    let json = JsonRenderer::render(&report).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["analysis"]["interventions"].is_number());
    assert!(parsed["findings"].as_array().unwrap().len() == 1);
}

#[test]
fn raw_evidence_not_in_human_output() {
    // Clustered steering events used to embed the full raw log context in the
    // intervention reason. Verify the generation path no longer does that.
    let raw_context = "session_loop{thread_id=123}: Submission sub=Submission { id: \"...\", op: UserInput { ... }, thread_settings: ... }";
    let events = vec![
        SteeringEvent {
            id: "s1".into(),
            category: SteeringCategory::MissingTool,
            description: "missing tool".into(),
            entry_id: "e1".into(),
            timestamp: Utc::now(),
            context: raw_context.into(),
            severity: 0.8,
        },
        SteeringEvent {
            id: "s2".into(),
            category: SteeringCategory::MissingTool,
            description: "missing tool".into(),
            entry_id: "e2".into(),
            timestamp: Utc::now(),
            context: raw_context.into(),
            severity: 0.8,
        },
    ];
    let mut anthology = empty_anthology();
    anthology.steering_events = events;
    anthology.generate().unwrap();

    assert!(
        !anthology.candidate_tools.is_empty(),
        "expected clustered intervention"
    );
    let reason = &anthology.candidate_tools[0].reason;
    assert!(
        !reason.contains("session_loop"),
        "raw evidence leaked into CandidateTool.reason: {reason}"
    );
    assert!(
        !reason.contains("Submission sub"),
        "raw evidence leaked into CandidateTool.reason: {reason}"
    );

    let report = CompletionReport::from_anthology(&anthology, &PathBuf::from("/repo"), &[]);
    let renderer = HumanRenderer::new(100, false);
    let output = renderer.render(&report);
    assert!(
        !output.contains("session_loop"),
        "raw evidence leaked into human output"
    );
    assert!(
        !output.contains("Submission sub"),
        "raw evidence leaked into human output"
    );
}

#[test]
fn analysis_summary_counts_interventions() {
    let tools = vec![
        sample_tool(
            "DS-0008",
            Priority::High,
            InterventionCategory::MissingCapability,
        ),
        sample_tool("DS-0009", Priority::Low, InterventionCategory::Other),
    ];
    let anthology = anthology_with_interventions(tools);
    let summary = AnalysisSummary::from_anthology(&anthology);

    assert_eq!(summary.interventions, 2);
    assert_eq!(summary.high_impact, 2); // both sample patterns have impact_score > 0.7
    assert_eq!(
        summary.remote_analysis_consent,
        Some(dreamseq::RemoteAnalysisConsent::AutoApproved)
    );
}

#[test]
fn human_output_shows_remote_analysis_consent() {
    let report = CompletionReport::from_anthology(
        &anthology_with_interventions(vec![]),
        &PathBuf::from("/repo"),
        &[],
    );
    let output = HumanRenderer::new(80, false).render(&report);
    assert!(
        strip_ansi(&output).contains("auto-approved"),
        "human output should disclose how remote analysis consent was obtained"
    );
}

#[test]
fn json_output_includes_remote_analysis_consent() {
    let report = CompletionReport::from_anthology(
        &anthology_with_interventions(vec![]),
        &PathBuf::from("/repo"),
        &[],
    );
    let json = JsonRenderer::render(&report).unwrap();
    assert!(json.contains("remote_analysis_consent"));
    assert!(json.contains("auto_approved"));
}

#[test]
fn intervention_from_tool_prefers_existing_matches() {
    let tool = CandidateTool {
        existing_matches: vec!["goblin".into()],
        affected_projects: vec!["old-project".into()],
        ..sample_tool(
            "DS-0010",
            Priority::High,
            InterventionCategory::MissingCapability,
        )
    };
    let intervention = Intervention::from_tool(&tool);

    assert_eq!(intervention.projects, vec!["goblin"]);
    assert!(intervention.action.contains("extend goblin"));
}

#[test]
fn daemon_state_display_and_version_projection() {
    assert_eq!(DaemonState::Running.to_string(), "running");
    assert_eq!(DaemonState::NotInstalled.to_string(), "not installed");
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            out.push(ch);
        }
    }
    out
}
