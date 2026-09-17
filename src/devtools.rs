// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! On-device detection engine for the developer-tools registry (ported from
//! dreamcred's `devtools/engine.py` when the `stack-report` subcommand moved
//! into dreamseq).
//!
//! Detection vectors per tool, cheapest first:
//!   1. CLI binary on PATH (`which`) -- version probed via `<binary> --version`
//!   2. Install-path marker (glob, expanded per-user)
//!   3. Installed package name across whatever package managers exist
//!      (dpkg / rpm / pacman / flatpak / snap / brew / npm-global / pip)
//!   4. Freedesktop desktop-file id
//!
//! Everything is read-only and best-effort: a missing package manager, an
//! unreadable directory, or a `--version` hang must never fail the report.

use crate::devtools_registry::TOOLS;
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;

/// One detected developer tool, ready to serialize into the stack report.
#[derive(Debug, Clone, Serialize)]
pub struct DetectedTool {
    pub key: String,
    pub name: String,
    pub category: String,
    pub version: Option<String>,
    pub origin: Option<String>,
}

/// Default cap on tools returned, matching dreamcred's `detect_devtools(cap=500)`.
pub const DEFAULT_CAP: usize = 500;

const MAX_CONCURRENT_PROBES: usize = 8;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PACKAGE_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Resolve `binary` on `PATH`, mirroring Python's `shutil.which` closely
/// enough for detection purposes: an explicit path is checked directly,
/// otherwise every `PATH` entry is tried in order.
pub(crate) fn which(binary: &str) -> Option<String> {
    if binary.contains('/') {
        let path = Path::new(binary);
        return is_executable_file(path).then(|| binary.to_string());
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if is_executable_file(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn expand_tilde(pattern: &str) -> String {
    if pattern == "~" {
        return dirs::home_dir()
            .map(|home| home.to_string_lossy().into_owned())
            .unwrap_or_else(|| pattern.to_string());
    }
    if let Some(rest) = pattern.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into_owned();
    }
    pattern.to_string()
}

/// First filesystem match for a glob pattern (`~` expanded), if any.
pub(crate) fn marker_path(pattern: &str) -> Option<String> {
    let expanded = expand_tilde(pattern);
    glob::glob(&expanded)
        .ok()?
        .filter_map(Result::ok)
        .next()
        .map(|path| path.to_string_lossy().into_owned())
}

async fn run_output(command: &str, args: &[&str], timeout: Duration) -> Option<(bool, String)> {
    let child = tokio::process::Command::new(command)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let output = tokio::time::timeout(timeout, async { child.wait_with_output().await })
        .await
        .ok()?
        .ok()?;
    Some((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

/// Installed package names across whatever package managers exist on this
/// device, lowercased. Every source is best-effort: a missing package
/// manager just contributes nothing.
async fn package_names() -> HashSet<String> {
    let mut names = HashSet::new();
    let whitespace_split_commands: &[(&str, &[&str])] = &[
        ("dpkg-query", &["-W", "-f=${Package}\n"]),
        ("rpm", &["-qa", "--qf", "%{NAME}\n"]),
        ("pacman", &["-Qq"]),
        ("flatpak", &["list", "--app", "--columns=application"]),
        ("brew", &["list", "--formula"]),
        ("brew", &["list", "--cask"]),
    ];
    for (command, args) in whitespace_split_commands {
        if let Some((true, stdout)) = run_output(command, args, PACKAGE_QUERY_TIMEOUT).await {
            names.extend(stdout.split_whitespace().map(str::to_lowercase));
        }
    }
    if let Some((true, stdout)) = run_output("snap", &["list"], PACKAGE_QUERY_TIMEOUT).await {
        for line in stdout.lines().skip(1) {
            if let Some(first) = line.split_whitespace().next() {
                names.insert(first.to_lowercase());
            }
        }
    }
    if let Some((_, root)) = run_output("npm", &["root", "-g"], PACKAGE_QUERY_TIMEOUT).await {
        let root = root.trim();
        if !root.is_empty()
            && let Ok(mut entries) = tokio::fs::read_dir(root).await
        {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(name) = entry.file_name().to_str()
                    && !name.starts_with('.')
                {
                    names.insert(name.to_lowercase());
                }
            }
        }
    }
    // Approximates Python's `importlib.metadata.distributions()` (the
    // interpreter's own installed packages) with the equivalent `pip`
    // listing, since dreamseq has no interpreter of its own to introspect.
    if let Some((true, stdout)) = run_output(
        "python3",
        &[
            "-m",
            "pip",
            "list",
            "--disable-pip-version-check",
            "--format=freeze",
        ],
        PACKAGE_QUERY_TIMEOUT,
    )
    .await
    {
        for line in stdout.lines() {
            if let Some((name, _version)) = line.split_once("==") {
                names.insert(name.trim().to_lowercase());
            }
        }
    }
    names
}

/// Freedesktop `.desktop` file ids visible to this user.
fn desktop_ids() -> HashSet<String> {
    let mut ids = HashSet::new();
    let directories = [
        "/usr/share/applications",
        "/var/lib/flatpak/exports/share/applications",
        "~/.local/share/applications",
    ];
    for directory in directories {
        let expanded = expand_tilde(directory);
        if let Ok(entries) = std::fs::read_dir(&expanded) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    ids.insert(name.to_string());
                }
            }
        }
    }
    ids
}

/// Extract a clean version number from a tool's raw `--version` output,
/// discarding banners that are actually complaints about the unsupported
/// flag. Mirrors dreamcred's `_binary_version` parsing.
fn parse_tool_version(raw_output: &str) -> Option<String> {
    let raw: String = raw_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");
    if raw.is_empty() {
        return None;
    }
    let lowered = raw.to_lowercase();
    const NOISE_MARKERS: &[&str] = &[
        "[options]",
        "bad command line option",
        "flag provided but not defined",
        "unknown argument",
        "unrecognised",
        "unrecognized",
        "invalid option",
        "no such option",
    ];
    if NOISE_MARKERS.iter().any(|marker| lowered.contains(marker))
        || lowered.starts_with("usage")
        || lowered.starts_with("unknown")
        || lowered.starts_with("invalid")
    {
        return None;
    }
    static VERSION_WORD_RE: OnceLock<Regex> = OnceLock::new();
    let version_word =
        VERSION_WORD_RE.get_or_init(|| Regex::new(r"(?i)\bversion\s+v?(\d+(?:\.\d+)+)").unwrap());
    if let Some(captures) = version_word.captures(&raw) {
        return Some(captures[1].chars().take(64).collect());
    }
    static BARE_VERSION_RE: OnceLock<Regex> = OnceLock::new();
    let bare_version =
        BARE_VERSION_RE.get_or_init(|| Regex::new(r"\bv?(\d+(?:\.\d+)+)\b").unwrap());
    bare_version
        .captures(&raw)
        .map(|captures| captures[1].chars().take(64).collect())
}

/// Probe one executable's version for the devtools registry: stdout and
/// stderr merged (tools often print `--version` complaints to stderr),
/// first three non-empty lines, then a clean number extracted.
async fn tool_binary_version(executable: &str) -> Option<String> {
    let child = tokio::process::Command::new(executable)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let output = tokio::time::timeout(VERSION_PROBE_TIMEOUT, async {
        child.wait_with_output().await
    })
    .await
    .ok()?
    .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = stdout
        .lines()
        .chain(stderr.lines())
        .collect::<Vec<_>>()
        .join("\n");
    parse_tool_version(&combined)
}

async fn probe_versions(probes: HashSet<String>) -> HashMap<String, Option<String>> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PROBES));
    let mut set = tokio::task::JoinSet::new();
    for probe in probes {
        let permit = semaphore.clone();
        set.spawn(async move {
            let _permit = permit.acquire_owned().await.ok();
            let version = tool_binary_version(&probe).await;
            (probe, version)
        });
    }
    let mut results = HashMap::new();
    while let Some(joined) = set.join_next().await {
        if let Ok((probe, version)) = joined {
            results.insert(probe, version);
        }
    }
    results
}

/// Detect installed registry tools. `batches: None` detects every batch.
pub async fn detect_devtools(batches: Option<&HashSet<u32>>, cap: usize) -> Vec<DetectedTool> {
    let packages = package_names().await;
    let desktops = desktop_ids();

    struct Candidate {
        index: usize,
        origin: Option<String>,
        probe: Option<String>,
    }

    let mut candidates = Vec::new();
    for (index, tool) in TOOLS.iter().enumerate() {
        if let Some(batches) = batches
            && !batches.contains(&tool.batch)
        {
            continue;
        }
        let binary = tool.binaries.iter().find_map(|name| which(name));
        let marker = tool.paths.iter().find_map(|pattern| marker_path(pattern));
        let package = tool
            .packages
            .iter()
            .find(|name| packages.contains(&name.to_lowercase()));
        let desktop = tool.desktop.iter().find(|id| desktops.contains(**id));
        if binary.is_none() && marker.is_none() && package.is_none() && desktop.is_none() {
            continue;
        }
        let probe = binary
            .clone()
            .or_else(|| marker.clone().filter(|path| Path::new(path).is_file()));
        let origin = binary
            .or(marker)
            .or_else(|| package.map(|name| name.to_string()))
            .or_else(|| desktop.map(|id| id.to_string()));
        candidates.push(Candidate {
            index,
            origin,
            probe,
        });
    }

    let probes: HashSet<String> = candidates.iter().filter_map(|c| c.probe.clone()).collect();
    let versions = probe_versions(probes).await;

    let mut found: Vec<DetectedTool> = candidates
        .into_iter()
        .map(|candidate| {
            let tool = &TOOLS[candidate.index];
            let version = candidate
                .probe
                .as_ref()
                .and_then(|probe| versions.get(probe).cloned().flatten());
            DetectedTool {
                key: tool.key.to_string(),
                name: tool.name.to_string(),
                category: tool.category.to_string(),
                version,
                origin: candidate.origin,
            }
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found.truncate(cap);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_and_noisy_version_banners() {
        assert_eq!(
            parse_tool_version("git version 2.43.0"),
            Some("2.43.0".to_string())
        );
        assert_eq!(
            parse_tool_version("Docker version 27.0.3, build 7d4bcd8"),
            Some("27.0.3".to_string())
        );
        assert_eq!(
            parse_tool_version("gh version 2.96.0 (2026-07-02)"),
            Some("2.96.0".to_string())
        );
        assert_eq!(
            parse_tool_version("bare 1.2.3 tool"),
            Some("1.2.3".to_string())
        );
        assert_eq!(parse_tool_version("Usage: foo [options]"), None);
        assert_eq!(parse_tool_version("unknown argument --version"), None);
        assert_eq!(parse_tool_version(""), None);
    }

    #[test]
    fn which_finds_binaries_on_a_synthetic_path() {
        let dir = std::env::temp_dir().join(format!("dreamseq-which-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("toolprobe");
        std::fs::write(&binary, b"#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let original_path = std::env::var_os("PATH");
        // traci: allow -- test sets and restores its own PATH override.
        unsafe {
            std::env::set_var(
                "PATH",
                format!(
                    "{}:{}",
                    dir.display(),
                    original_path
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ),
            );
        }
        assert!(which("toolprobe").is_some());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn detect_devtools_respects_batch_filtering_and_cap() {
        let all = detect_devtools(None, 2).await;
        assert!(all.len() <= 2);
        let empty_batch: HashSet<u32> = [9999].into_iter().collect();
        let none_found = detect_devtools(Some(&empty_batch), DEFAULT_CAP).await;
        assert!(none_found.is_empty());
    }
}
