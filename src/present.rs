use crate::color::Colorize;
use crate::report::{Anthology, CandidateTool, InterventionCategory, Priority};
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A sanitized, presentation-ready view of a completed Dreamseq run.
#[derive(Debug, Clone, Serialize)]
pub struct CompletionReport {
    pub identity: IdentityStatus,
    pub analysis: AnalysisSummary,
    pub findings: Vec<Intervention>,
    pub artifacts: ArtifactSummary,
    pub completed_at: String,
}

impl CompletionReport {
    pub fn from_anthology(
        anthology: &Anthology,
        repository: &Path,
        dreams_roots: &[PathBuf],
    ) -> Self {
        Self {
            identity: IdentityStatus::infer(repository, anthology),
            analysis: AnalysisSummary::from_anthology(anthology),
            findings: Intervention::from_anthology(anthology),
            artifacts: ArtifactSummary {
                anthology_path: repository
                    .join("anthologies")
                    .join(format!("dreamseq-{}.json", anthology.date)),
                dreams_roots: dreams_roots.to_vec(),
                dreams_paths: Vec::new(),
            },
            completed_at: anthology
                .generated_at
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentityStatus {
    pub repository: PathBuf,
    pub version: String,
    pub daemon_state: DaemonState,
    pub score: f64,
    pub projection: VersionProjection,
}

impl IdentityStatus {
    pub fn infer(repository: &Path, anthology: &Anthology) -> Self {
        let version = read_version(repository);
        let daemon_state = kaptaind_status(repository);
        let score = compute_score(anthology);
        let projection = project_version(&version, score);
        Self {
            repository: repository.to_path_buf(),
            version,
            daemon_state,
            score,
            projection,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DaemonState {
    Running,
    Stopped,
    NotInstalled,
    Unknown,
}

impl std::fmt::Display for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonState::Running => write!(f, "running"),
            DaemonState::Stopped => write!(f, "stopped"),
            DaemonState::NotInstalled => write!(f, "not installed"),
            DaemonState::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum VersionProjection {
    NoChange,
    Patch(String),
    Minor(String),
}

impl VersionProjection {
    pub fn label(&self) -> String {
        match self {
            VersionProjection::NoChange => "none".to_string(),
            VersionProjection::Patch(v) => format!("patch → v{}", v),
            VersionProjection::Minor(v) => format!("minor → v{}", v),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisSummary {
    pub raw_entries: usize,
    pub normalized_entries: usize,
    pub segments: usize,
    pub estimated_tokens: usize,
    pub steering_events: usize,
    pub patterns: usize,
    pub interventions: usize,
    pub high_impact: usize,
}

impl AnalysisSummary {
    pub fn from_anthology(anthology: &Anthology) -> Self {
        let high_impact = anthology
            .patterns
            .iter()
            .filter(|p| p.impact_score > 0.7)
            .count();
        Self {
            raw_entries: anthology.pipeline.raw_entries,
            normalized_entries: anthology.pipeline.normalized_entries,
            segments: anthology.pipeline.segments,
            estimated_tokens: anthology.pipeline.estimated_input_tokens,
            steering_events: anthology.steering_events.len(),
            patterns: anthology.patterns.len(),
            interventions: anthology.candidate_tools.len(),
            high_impact,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Intervention {
    pub id: String,
    pub priority: Priority,
    pub category: InterventionCategory,
    pub title: String,
    pub time_saved: String,
    pub confidence: f64,
    pub action: String,
    pub rationale: String,
    pub projects: Vec<String>,
}

impl Intervention {
    pub fn from_tool(tool: &CandidateTool) -> Self {
        let action = if tool.existing_matches.is_empty() {
            "create new capability".to_string()
        } else {
            format!("extend {}", tool.existing_matches.join(", "))
        };

        let projects = if !tool.existing_matches.is_empty() {
            tool.existing_matches.clone()
        } else if !tool.affected_projects.is_empty() {
            tool.affected_projects.clone()
        } else {
            Vec::new()
        };

        Self {
            id: tool.id.clone(),
            priority: tool.priority.clone(),
            category: tool.category,
            title: tool.name.clone(),
            time_saved: tool.estimated_time_saved.clone(),
            confidence: tool.confidence,
            action,
            rationale: tool.reason.clone(),
            projects,
        }
    }

    pub fn from_anthology(anthology: &Anthology) -> Vec<Self> {
        anthology
            .candidate_tools
            .iter()
            .map(Self::from_tool)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactSummary {
    pub anthology_path: PathBuf,
    pub dreams_roots: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dreams_paths: Vec<PathBuf>,
}

/// Renders a CompletionReport for human consumption in a terminal.
pub struct HumanRenderer {
    width: usize,
    verbose: bool,
}

impl HumanRenderer {
    pub fn new(width: usize, verbose: bool) -> Self {
        Self {
            width: width.max(40),
            verbose,
        }
    }

    pub fn from_env(verbose: bool) -> Self {
        let width = match std::env::var("COLUMNS") {
            Ok(value) => match value.parse() {
                Ok(width) => width,
                Err(error) => {
                    tracing::debug!(value, error = %error, "ignoring invalid COLUMNS value");
                    100
                }
            },
            Err(_) => 100,
        };
        Self::new(width, verbose)
    }

    pub fn render(&self, report: &CompletionReport) -> String {
        let mut lines: Vec<String> = Vec::new();

        self.render_identity(&mut lines, report);
        lines.push(String::new());
        self.render_analysis(&mut lines, report);
        lines.push(String::new());
        self.render_findings(&mut lines, report);
        lines.push(String::new());
        self.render_artifacts(&mut lines, report);
        lines.push(String::new());
        self.render_complete(&mut lines, report);

        if self.verbose {
            lines.push(String::new());
            self.render_diagnostics(&mut lines, report);
        }

        lines.join("\n")
    }

    fn render_identity(&self, lines: &mut Vec<String>, report: &CompletionReport) {
        let id = &report.identity;
        let repo = id.repository.display().to_string();
        let label_width = 12;

        if self.width >= 60 {
            let box_width = (self.width - 2).min(78);
            let header = " DREAMSEQ ";
            let pad = box_width.saturating_sub(header.len());
            let top = format!("╭{}{}╮", header, "─".repeat(pad));
            lines.push(top);
            lines.push(self.box_line(label_width, "Repository", &repo, box_width));
            lines.push(self.box_line(
                label_width,
                "Version",
                &format!("v{}", id.version),
                box_width,
            ));
            lines.push(self.box_line(
                label_width,
                "Daemon",
                &id.daemon_state.to_string(),
                box_width,
            ));
            lines.push(self.box_line(label_width, "Score", &format!("{:.3}", id.score), box_width));
            lines.push(self.box_line(label_width, "Projection", &id.projection.label(), box_width));
            lines.push(format!("╰{}╯", "─".repeat(box_width)));
        } else {
            lines.push("DREAMSEQ".bold().to_string());
            lines.push(format!("  Repository {}", repo));
            lines.push(format!("  Version    v{}", id.version));
            lines.push(format!("  Daemon     {}", id.daemon_state));
            lines.push(format!("  Score      {:.3}", id.score));
            lines.push(format!("  Projection {}", id.projection.label()));
        }
    }

    fn box_line(&self, label_width: usize, label: &str, value: &str, box_width: usize) -> String {
        let visible = format!("{:>label_width$}  {}", label.dimmed(), value);
        let pad = box_width.saturating_sub(strip_ansi(&visible).chars().count());
        format!("│{}{}│", visible, " ".repeat(pad))
    }

    fn render_analysis(&self, lines: &mut Vec<String>, report: &CompletionReport) {
        lines.push("ANALYSIS".bold().to_string());
        let a = &report.analysis;
        lines.push(format!(
            "  {:>15} raw entries",
            format_number(a.raw_entries).dimmed()
        ));
        lines.push(format!(
            "  {:>15} normalized events",
            format_number(a.normalized_entries).dimmed()
        ));
        lines.push(format!(
            "  {:>15} segments",
            format_number(a.segments).dimmed()
        ));
        lines.push(format!(
            "  {:>15} estimated tokens",
            format_number(a.estimated_tokens).dimmed()
        ));
        lines.push(format!(
            "  {:>15} steering events",
            format_number(a.steering_events).dimmed()
        ));
        lines.push(format!(
            "  {:>15} patterns",
            format_number(a.patterns).dimmed()
        ));
        lines.push(format!(
            "  {:>15} interventions",
            format_number(a.interventions).dimmed()
        ));
    }

    fn render_findings(&self, lines: &mut Vec<String>, report: &CompletionReport) {
        lines.push("RESULT".bold().to_string());
        lines.push(format!(
            "  {} high-impact opportunities identified",
            report.analysis.high_impact
        ));
        lines.push(format!(
            "  {} candidate interventions",
            report.findings.len()
        ));

        if report.findings.is_empty() {
            lines.push(String::new());
            lines.push("  No interventions identified.".dimmed().to_string());
            return;
        }

        for finding in &report.findings {
            lines.push(String::new());
            let priority = match finding.priority {
                Priority::High => "HIGH".red().bold(),
                Priority::Medium => "MED".yellow(),
                Priority::Low => "LOW".green(),
            };
            lines.push(format!(
                "  {}  {}\n        {}\n        {} · {:.0}% confidence",
                priority,
                finding.category.to_string().white(),
                finding.title.cyan().bold(),
                finding.time_saved.white(),
                finding.confidence * 100.0
            ));

            let indented = wrap_text(&finding.rationale, self.width.saturating_sub(14), 8);
            lines.push(indented);

            if !finding.projects.is_empty() {
                let project_list = finding.projects.join(", ");
                lines.push(format!(
                    "        {} {}",
                    "Projects:".dimmed(),
                    project_list.dimmed()
                ));
            }
        }
    }

    fn render_artifacts(&self, lines: &mut Vec<String>, report: &CompletionReport) {
        lines.push("ARTIFACTS".bold().to_string());
        let anth = report.artifacts.anthology_path.display().to_string();
        lines.push(format!("  Anthology      {}", anth.dimmed()));
        lines.push(format!(
            "  Dreams         {} project roots updated",
            report.artifacts.dreams_roots.len()
        ));

        if let Some(sample) = sample_projects(&report.artifacts.dreams_roots, 5) {
            lines.push(format!("  Sample         {}", sample.dimmed()));
        }

        if self.verbose {
            for path in &report.artifacts.dreams_paths {
                lines.push(format!("    • {}", path.display().to_string().dimmed()));
            }
        }
    }

    fn render_complete(&self, lines: &mut Vec<String>, report: &CompletionReport) {
        lines.push("COMPLETE".bold().to_string());
        lines.push(format!(
            "  Analysis complete · {}",
            report.completed_at.dimmed()
        ));
    }

    fn render_diagnostics(&self, lines: &mut Vec<String>, report: &CompletionReport) {
        lines.push("DIAGNOSTICS".bold().to_string());
        lines.push(format!(
            "  Repository: {}",
            report.identity.repository.display()
        ));
        lines.push(format!("  Score: {:.3}", report.identity.score));
        lines.push(format!("  Daemon: {} (raw)", report.identity.daemon_state));
        lines.push(format!(
            "  Dreams roots: {}",
            report.artifacts.dreams_roots.len()
        ));
        for root in &report.artifacts.dreams_roots {
            lines.push(format!("    • {}", root.display()));
        }
    }
}

/// Renders a CompletionReport as JSON for automation.
pub struct JsonRenderer;

impl JsonRenderer {
    pub fn render(report: &CompletionReport) -> Result<String> {
        Ok(serde_json::to_string_pretty(report)?)
    }
}

fn format_number(n: usize) -> String {
    n.to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(std::str::from_utf8)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default()
        .join(",")
}

fn wrap_text(text: &str, width: usize, indent: usize) -> String {
    let indent_str = " ".repeat(indent);
    let content_width = width.saturating_sub(indent);
    let mut result = Vec::new();

    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if line.is_empty() {
                line.push_str(word);
            } else if line.len() + 1 + word.len() > content_width {
                result.push(format!("{}{}", indent_str, line));
                line = word.to_string();
            } else {
                line.push(' ');
                line.push_str(word);
            }
        }
        if !line.is_empty() {
            result.push(format!("{}{}", indent_str, line));
        }
    }

    if result.is_empty() {
        indent_str
    } else {
        result.join("\n")
    }
}

fn sample_projects(roots: &[PathBuf], max: usize) -> Option<String> {
    if roots.is_empty() {
        return None;
    }
    let names: Vec<String> = roots
        .iter()
        .take(max)
        .filter_map(|p| p.file_name()?.to_str().map(|s| s.to_string()))
        .collect();
    if names.is_empty() {
        return None;
    }
    let mut out = names.join(", ");
    if roots.len() > max {
        out.push_str(", ...");
    }
    Some(out)
}

fn read_version(repository: &Path) -> String {
    let mut dir = Some(repository);
    while let Some(d) = dir {
        let path = d.join("VERSION");
        if let Ok(content) = std::fs::read_to_string(&path) {
            let v = content.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
        dir = d.parent();
    }
    "unknown".to_string()
}

fn kaptaind_status(repository: &Path) -> DaemonState {
    let output = std::process::Command::new("kaptaind-cli")
        .arg("status")
        .current_dir(repository)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
            if text.contains("running") {
                DaemonState::Running
            } else if text.contains("stopped") {
                DaemonState::Stopped
            } else {
                DaemonState::Unknown
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
            if err.contains("not found") || err.contains("no such file") {
                DaemonState::NotInstalled
            } else {
                DaemonState::Stopped
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "could not query Kaptaind daemon state");
            if e.kind() == std::io::ErrorKind::NotFound {
                DaemonState::NotInstalled
            } else {
                DaemonState::Unknown
            }
        }
    }
}

fn compute_score(anthology: &Anthology) -> f64 {
    let high_impact = anthology
        .patterns
        .iter()
        .filter(|p| p.impact_score > 0.7)
        .count() as f64;
    let pattern_score = high_impact / (high_impact + 5.0);

    let steering = anthology.steering_events.len() as f64;
    let steering_score = steering / (steering + 20.0);

    ((pattern_score + steering_score) / 2.0).clamp(0.0, 1.0)
}

fn project_version(current: &str, score: f64) -> VersionProjection {
    let thresholds = read_kaptaind_thresholds();
    let parts: Vec<&str> = current.split('.').collect();
    if parts.len() < 2 {
        return VersionProjection::NoChange;
    }
    let (Ok(major), Ok(mut minor)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) else {
        tracing::warn!(
            version = current,
            "cannot project a malformed semantic version"
        );
        return VersionProjection::NoChange;
    };
    let mut patch = match parts.get(2) {
        Some(value) => match value.parse::<u32>() {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(version = current, error = %error, "cannot project a malformed semantic version");
                return VersionProjection::NoChange;
            }
        },
        None => 0,
    };

    if score >= thresholds.minor {
        minor += 1;
        patch = 0;
        VersionProjection::Minor(format!("{}.{}.{}", major, minor, patch))
    } else if score >= thresholds.patch {
        patch += 1;
        VersionProjection::Patch(format!("{}.{}.{}", major, minor, patch))
    } else {
        VersionProjection::NoChange
    }
}

#[derive(Debug, Clone, Copy)]
struct Thresholds {
    minor: f64,
    patch: f64,
}

fn read_kaptaind_thresholds() -> Thresholds {
    let default = Thresholds {
        minor: 0.6,
        patch: 0.1,
    };

    if let Ok(content) = std::fs::read_to_string("kaptaind.toml")
        && let Ok(value) = content.parse::<toml::Value>()
    {
        let minor = value
            .get("version_thresholds")
            .and_then(|v| v.get("minor"))
            .and_then(|v| v.as_float())
            .unwrap_or(default.minor);
        let patch = value
            .get("version_thresholds")
            .and_then(|v| v.get("patch"))
            .and_then(|v| v.as_float())
            .unwrap_or(default.patch);
        return Thresholds { minor, patch };
    }

    default
}

fn strip_ansi(s: &str) -> String {
    // A minimal ANSI stripper sufficient for our own colored output.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_number_adds_commas() {
        assert_eq!(format_number(108763), "108,763");
        assert_eq!(format_number(1000), "1,000");
        assert_eq!(format_number(0), "0");
    }

    #[test]
    fn wrap_text_respects_width() {
        let text = "one two three four five six seven eight";
        let wrapped = wrap_text(text, 14, 2);
        for line in wrapped.lines() {
            assert!(strip_ansi(line).len() <= 14); // total width
        }
    }

    #[test]
    fn sample_projects_truncates() {
        let roots: Vec<PathBuf> = (0..10)
            .map(|i| PathBuf::from(format!("/p/{}", i)))
            .collect();
        let s = sample_projects(&roots, 3).unwrap();
        assert!(s.contains("0, 1, 2"));
        assert!(s.contains("..."));
    }

    #[test]
    fn version_projection_works() {
        assert!(matches!(
            project_version("0.1.1", 0.05),
            VersionProjection::NoChange
        ));
        assert!(
            matches!(project_version("0.1.1", 0.15), VersionProjection::Patch(v) if v == "0.1.2")
        );
        assert!(
            matches!(project_version("0.1.1", 0.65), VersionProjection::Minor(v) if v == "0.2.0")
        );
    }
}
