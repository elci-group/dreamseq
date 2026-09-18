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
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;

/// One detected developer tool, ready to serialize into the stack report.
/// `state` carries functional evidence (daemon up, authenticated, reachable,
/// configured) for the small allowlist of tools `probe_tool_state` knows how
/// to exercise; it is omitted entirely when no probe applied or succeeded, so
/// payloads stay compatible with consumers written before state existed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedTool {
    pub key: String,
    pub name: String,
    pub category: String,
    pub version: Option<String>,
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<BTreeMap<String, String>>,
}

/// Default cap on tools returned, matching dreamcred's `detect_devtools(cap=500)`.
pub const DEFAULT_CAP: usize = 500;

const MAX_CONCURRENT_PROBES: usize = 8;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PACKAGE_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Registry keys that get a functional state probe after detection (matched
/// case-insensitively on the key). Each probe is non-interactive, bounded by
/// `STATE_PROBE_TIMEOUT`, and silent on failure — state is evidence, not a
/// verdict.
const STATE_PROBE_KEYS: &[&str] = &["docker", "gh", "postgresql", "git"];
const STATE_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Cap on `state` entries per tool; truncate rather than error.
const STATE_MAX_ENTRIES: usize = 8;

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

/// Like [`run_output`], but merges stderr into the returned output. Tools
/// whose status commands report to stderr (e.g. `gh auth status`) need the
/// combined stream for parsing; callers that need stdout only should use
/// `run_output`.
async fn run_output_merged(
    command: &str,
    args: &[&str],
    timeout: Duration,
) -> Option<(bool, String)> {
    let child = tokio::process::Command::new(command)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let output = tokio::time::timeout(timeout, async { child.wait_with_output().await })
        .await
        .ok()?
        .ok()?;
    let merged = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    Some((
        output.status.success(),
        String::from_utf8_lossy(&merged).into_owned(),
    ))
}

// ---------------------------------------------------------------------------
// Functional state probes
// ---------------------------------------------------------------------------

/// Docker state: `docker info` succeeding means the daemon is up; the
/// SecurityOptions section mentions `rootless` when the daemon runs rootless.
fn docker_state_from_info(info: &str) -> BTreeMap<String, String> {
    let mut state = BTreeMap::new();
    state.insert("daemon".to_string(), "yes".to_string());
    state.insert("running".to_string(), "yes".to_string());
    state.insert(
        "rootless".to_string(),
        if info.to_lowercase().contains("rootless") {
            "yes"
        } else {
            "no"
        }
        .to_string(),
    );
    state
}

async fn probe_docker_state() -> Option<BTreeMap<String, String>> {
    let (ok, stdout) = run_output("docker", &["info"], STATE_PROBE_TIMEOUT).await?;
    ok.then(|| docker_state_from_info(&stdout))
}

/// GitHub CLI state: `gh auth status` exits zero when authenticated and names
/// the account; anything else means no usable login.
fn gh_state_from_status(success: bool, output: &str) -> BTreeMap<String, String> {
    let mut state = BTreeMap::new();
    if success {
        state.insert("authenticated".to_string(), "yes".to_string());
        static ACCOUNT_RE: OnceLock<Regex> = OnceLock::new();
        let account_re =
            ACCOUNT_RE.get_or_init(|| Regex::new(r"Logged in to github\.com as (\S+)").unwrap());
        if let Some(captures) = account_re.captures(output) {
            state.insert("account".to_string(), captures[1].to_string());
        }
    } else {
        state.insert("authenticated".to_string(), "no".to_string());
    }
    state
}

async fn probe_gh_state() -> Option<BTreeMap<String, String>> {
    let (ok, merged) = run_output_merged("gh", &["auth", "status"], STATE_PROBE_TIMEOUT).await?;
    Some(gh_state_from_status(ok, &merged))
}

/// PostgreSQL state: `pg_isready` exit 0 means a server is accepting
/// connections; 1 (rejecting) and 2 (no response) both mean a server exists
/// but is not reachable right now. A spawn failure means we cannot tell and
/// contributes nothing.
fn postgres_state_from_exit(success: bool) -> BTreeMap<String, String> {
    let mut state = BTreeMap::new();
    state.insert("server".to_string(), "yes".to_string());
    state.insert(
        "reachable".to_string(),
        if success { "yes" } else { "no" }.to_string(),
    );
    state
}

async fn probe_postgres_state() -> Option<BTreeMap<String, String>> {
    let (ok, _) = run_output("pg_isready", &[], STATE_PROBE_TIMEOUT).await?;
    Some(postgres_state_from_exit(ok))
}

/// Git state: `git config --get user.email` exits zero when an identity is
/// configured and one when none is.
async fn probe_git_state() -> Option<BTreeMap<String, String>> {
    let (ok, _) = run_output(
        "git",
        &["config", "--get", "user.email"],
        STATE_PROBE_TIMEOUT,
    )
    .await?;
    Some(git_state_from_exit(ok))
}

/// Run the state probe for one registry key. Returns `None` for keys outside
/// the allowlist and for any probe that could not produce evidence.
async fn probe_tool_state(key: &str) -> Option<BTreeMap<String, String>> {
    match key.to_lowercase().as_str() {
        "docker" => probe_docker_state().await,
        "gh" => probe_gh_state().await,
        "postgresql" => probe_postgres_state().await,
        "git" => probe_git_state().await,
        _ => None,
    }
}

/// Attach functional state to the detected tools that have a probe, running
/// all probes concurrently. Probes are silent on failure: a tool keeps
/// `state: None` when its probe yields nothing.
async fn attach_tool_state(found: &mut [DetectedTool]) {
    let mut set = tokio::task::JoinSet::new();
    for tool in found.iter() {
        if STATE_PROBE_KEYS.contains(&tool.key.to_lowercase().as_str()) {
            let key = tool.key.clone();
            set.spawn(async move { (key.clone(), probe_tool_state(&key).await) });
        }
    }
    let mut states = HashMap::new();
    while let Some(joined) = set.join_next().await {
        if let Ok((key, state)) = joined {
            states.insert(key, state);
        }
    }
    for tool in found.iter_mut() {
        tool.state = states.remove(&tool.key).and_then(|state| {
            state.and_then(|map| {
                // BTreeMap has no truncate: keep the first STATE_MAX_ENTRIES
                // (lexicographic key order — deterministic) and drop the rest.
                let capped: BTreeMap<String, String> =
                    map.into_iter().take(STATE_MAX_ENTRIES).collect();
                (!capped.is_empty()).then_some(capped)
            })
        });
    }
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
                state: None,
            }
        })
        .collect();
    attach_tool_state(&mut found).await;
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found.truncate(cap);
    found
}

fn git_state_from_exit(success: bool) -> BTreeMap<String, String> {
    let mut state = BTreeMap::new();
    state.insert(
        "configured".to_string(),
        if success { "yes" } else { "no" }.to_string(),
    );
    state
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

    #[test]
    fn docker_state_marks_rootless_from_security_options() {
        let rootful = docker_state_from_info(" Security Options: apparmor seccomp cgroupns\n");
        assert_eq!(rootful["daemon"], "yes");
        assert_eq!(rootful["running"], "yes");
        assert_eq!(rootful["rootless"], "no");

        let rootless = docker_state_from_info(" Security Options: seccomp rootless\n");
        assert_eq!(rootless["rootless"], "yes");
    }

    #[test]
    fn gh_state_parses_account_and_distinguishes_anonymous() {
        let logged_in = gh_state_from_status(
            true,
            "github.com\n  ✓ Logged in to github.com as octocool (oauth_token)\n",
        );
        assert_eq!(logged_in["authenticated"], "yes");
        assert_eq!(logged_in["account"], "octocool");

        let anonymous = gh_state_from_status(false, "");
        assert_eq!(anonymous["authenticated"], "no");
        assert!(!anonymous.contains_key("account"));
    }

    #[test]
    fn postgres_state_maps_exit_codes_to_reachability() {
        let accepting = postgres_state_from_exit(true);
        assert_eq!(accepting["server"], "yes");
        assert_eq!(accepting["reachable"], "yes");
        // Exit 1 (rejecting) and 2 (no response) both mean: server present,
        // not reachable right now.
        let rejecting = postgres_state_from_exit(false);
        assert_eq!(rejecting["server"], "yes");
        assert_eq!(rejecting["reachable"], "no");
    }

    #[test]
    fn git_state_maps_exit_code_to_configured() {
        assert_eq!(git_state_from_exit(true)["configured"], "yes");
        assert_eq!(git_state_from_exit(false)["configured"], "no");
    }

    #[test]
    fn old_tool_payloads_without_state_still_round_trip() {
        let legacy = r#"{"key": "git", "name": "Git", "category": "vcs", "version": "2.43.0", "origin": "/usr/bin/git"}"#;
        let tool: DetectedTool = serde_json::from_str(legacy).unwrap();
        assert!(tool.state.is_none());
        let reserialized = serde_json::to_value(&tool).unwrap();
        assert!(reserialized.get("state").is_none());

        // And a probed tool keeps its (sorted, capped) state in JSON.
        let mut state = BTreeMap::new();
        state.insert("authenticated".to_string(), "yes".to_string());
        let probed = DetectedTool {
            key: "gh".to_string(),
            name: "GitHub CLI".to_string(),
            category: "vcs".to_string(),
            version: None,
            origin: None,
            state: Some(state),
        };
        let json = serde_json::to_value(&probed).unwrap();
        assert_eq!(json["state"]["authenticated"], "yes");
    }
}
