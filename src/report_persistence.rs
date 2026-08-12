use crate::report::{Anthology, Priority};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn save(anthology: &Anthology) -> Result<PathBuf> {
    fs::create_dir_all(&anthology.config.anthologies_dir)?;
    let filename = format!("dreamseq-{}-{}.json", anthology.date, anthology.id);
    let path = anthology.config.anthologies_dir.join(filename);
    let content = serde_json::to_string_pretty(anthology)?;
    crate::fs_security::write_private_atomic(&path, content.as_bytes())?;
    tracing::info!("Saved anthology to {:?}", path);
    Ok(path)
}

pub(crate) fn save_dreams(anthology: &Anthology, repository: &Path) -> Result<PathBuf> {
    let dreams_dir = repository.join(".dreams");
    fs::create_dir_all(&dreams_dir)?;
    fs::create_dir_all(dreams_dir.join("history"))?;
    for lifecycle in ["completed.dreams", "rejected.dreams"] {
        let path = dreams_dir.join(lifecycle);
        if !path.exists() {
            match crate::fs_security::create_private(&path, b"version: 1\ndreams: []\n") {
                Ok(()) => {}
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) => {
                }
                Err(error) => return Err(error),
            }
        }
    }

    let path = dreams_dir.join("active.dreams");
    let output = render_dreams(anthology, repository);
    crate::fs_security::write_private_atomic(&path, output.as_bytes())?;
    crate::fs_security::write_private_atomic(
        &dreams_dir
            .join("history")
            .join(format!("{}-{}.dreams", anthology.date, anthology.id)),
        output.as_bytes(),
    )?;
    Ok(path)
}

fn render_dreams(anthology: &Anthology, repository: &Path) -> String {
    let mut output = String::new();
    let project = repository
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    output.push_str("version: 1\n");
    output.push_str(&format!("project: {}\n", yaml_scalar(project)));
    output.push_str("generated:\n  by: dreamseq\n");
    output.push_str(&format!(
        "  date: {}\n  confidence: {:.2}\n",
        anthology.date,
        anthology.average_confidence()
    ));
    output.push_str("source:\n");
    output.push_str(&format!("    logs: {}\n", anthology.pipeline.raw_entries));
    output.push_str(&format!(
        "    normalized_events: {}\n",
        anthology.pipeline.normalized_entries
    ));
    output.push_str(&format!("    segments: {}\n", anthology.pipeline.segments));
    output.push_str(&format!(
        "    steering_events: {}\n",
        anthology.steering_events.len()
    ));
    output.push_str("\ndreams:\n");
    for (index, tool) in anthology.candidate_tools.iter().enumerate() {
        let priority = match tool.priority {
            Priority::High => "HIGH",
            Priority::Medium => "MEDIUM",
            Priority::Low => "LOW",
        };
        let action = if tool.existing_matches.is_empty() {
            "create capability"
        } else {
            "extend existing capability"
        };
        let affected = if tool.existing_matches.is_empty() {
            "      - none identified\n".to_string()
        } else {
            tool.existing_matches
                .iter()
                .map(|name| format!("      - {}\n", yaml_scalar(name)))
                .collect()
        };
        output.push_str(&format!(
            "\n  - id: dream-{:03}\n    priority: {}\n    category: workflow-friction\n    title: {}\n    problem: {}\n    evidence:\n      reason: {}\n      confidence: {:.2}\n      mutation_fitness: {:.2}\n    affected:\n{}    proposed_mutation:\n      action: {}\n      capability: {}\n    acceptance:\n      - Evidence-backed intervention reduces recurrence of this friction\n      - Outcome is verified by tests or deterministic operational checks\n    expected_value: {}\n",
            index + 1,
            priority,
            yaml_scalar(&tool.name),
            yaml_scalar(&tool.reason),
            yaml_scalar(&tool.reason),
            tool.confidence,
            tool.mutation_fitness,
            affected,
            action,
            yaml_scalar(&tool.name),
            yaml_scalar(&tool.estimated_time_saved)
        ));
    }
    output
}

fn yaml_scalar(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
