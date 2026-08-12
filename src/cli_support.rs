// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
use anyhow::Result;
use dreamseq::cloud::{CloudClient, CredentialStore};
use dreamseq::color::Colorize;
use dreamseq::present::{CompletionReport, HumanRenderer, JsonRenderer};
use dreamseq::{Anthology, TrendAnalysis};

pub(crate) async fn sync_if_paired(anthology: &Anthology) -> Result<()> {
    let store = CredentialStore::discover()?;
    let Some(credentials) = store.load_optional()? else {
        return Ok(());
    };
    CloudClient::new(Some(&credentials.api_url))?
        .upload(&credentials, anthology)
        .await
}

pub(crate) fn make_relative_to(
    path: &std::path::Path,
    base: &std::path::Path,
) -> std::path::PathBuf {
    path.strip_prefix(base)
        .map_or_else(|_| path.to_path_buf(), |p| p.to_path_buf())
}

pub(crate) fn save_dreams_to_speck_projects(
    anthology: &Anthology,
) -> Result<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let speck_path = home.join(".speckrc");
    let mut roots = Vec::new();
    match std::fs::read_to_string(&speck_path) {
        Ok(content) => {
            let document: toml::Value = toml::from_str(&content)?;
            if let Some(tools) = document.get("tools").and_then(toml::Value::as_table) {
                for tool in tools.values() {
                    if let Some(path) = tool.get("path").and_then(toml::Value::as_str) {
                        let root = std::path::PathBuf::from(path);
                        if root.is_dir() && !roots.contains(&root) {
                            roots.push(root);
                        }
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(path = %speck_path.display(), "Speck registry not found; publishing only to the current repository");
        }
        Err(error) => {
            return Err(anyhow::anyhow!(
                "Cannot read {}: {}",
                speck_path.display(),
                error
            ));
        }
    }
    if roots.is_empty() {
        roots.push(std::env::current_dir()?);
    }
    roots.sort();
    let mut paths = Vec::with_capacity(roots.len());
    for root in &roots {
        paths.push(anthology.save_dreams(root)?);
    }
    Ok((roots, paths))
}

pub(crate) fn find_anthology_for_date(
    directory: &std::path::Path,
    date: &str,
) -> Result<Option<std::path::PathBuf>> {
    if !directory.exists() {
        return Ok(None);
    }
    let prefix = format!("dreamseq-{date}");
    let mut newest: Option<(chrono::DateTime<chrono::Utc>, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) || path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not read anthology candidate");
                continue;
            }
        };
        let anthology = match serde_json::from_str::<Anthology>(&content) {
            Ok(anthology) => anthology,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not parse anthology candidate");
                continue;
            }
        };
        if newest
            .as_ref()
            .is_none_or(|(generated_at, _)| anthology.generated_at > *generated_at)
        {
            newest = Some((anthology.generated_at, path));
        }
    }
    Ok(newest.map(|(_, path)| path))
}

pub(crate) fn find_latest_anthology(directory: &std::path::Path) -> Result<Option<Anthology>> {
    if !directory.exists() {
        return Ok(None);
    }
    let mut newest: Option<Anthology> = None;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not read anthology candidate");
                continue;
            }
        };
        let anthology = match serde_json::from_str::<Anthology>(&content) {
            Ok(anthology) => anthology,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not parse anthology candidate");
                continue;
            }
        };
        if newest
            .as_ref()
            .is_none_or(|current| anthology.generated_at > current.generated_at)
        {
            newest = Some(anthology);
        }
    }
    Ok(newest)
}

pub(crate) fn notify_completion(anthology: &Anthology) {
    let summary = format!(
        "Dreamseq analyzed {} entries and found {} intervention candidates.",
        anthology.pipeline.raw_entries,
        anthology.candidate_tools.len()
    );
    match std::process::Command::new("voxd-cli")
        .args(["speak", "--system", &summary])
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            tracing::warn!(status = %output.status, error = %String::from_utf8_lossy(&output.stderr), "TTS notification failed")
        }
        Err(error) => {
            tracing::warn!(error = %error, "voxd-cli is unavailable; TTS notification skipped")
        }
    }
}

pub(crate) fn print_trends(trends: &TrendAnalysis) {
    println!("{} {}", "📅 Period:".blue(), trends.period);
    println!();
    if trends.trends.is_empty() {
        println!("   {}", "No trend data available.".dimmed());
        return;
    }
    for trend_data in trends.trends.values() {
        let direction = match trend_data.trend_direction {
            dreamseq::trends::TrendDirection::Increasing => "📈 Increasing".red(),
            dreamseq::trends::TrendDirection::Decreasing => "📉 Decreasing".green(),
            dreamseq::trends::TrendDirection::Stable => "➡️ Stable".white(),
        };
        println!(
            "   {} {} — {}",
            "▸".cyan(),
            trend_data.metric_name.cyan().bold(),
            direction
        );
        println!("     Current:  {:.2}", trend_data.current_value);
        println!("     Previous: {:.2}", trend_data.previous_value);
        println!();
        println!("{}", trend_data.visualization);
        println!();
    }
}

pub(crate) fn render_report(report: &CompletionReport, json: bool, verbose: bool) -> Result<()> {
    if json {
        println!("{}", JsonRenderer::render(report)?);
    } else {
        println!("{}", HumanRenderer::from_env(verbose).render(report));
    }
    Ok(())
}
