// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Azusa drift integration.
//!
//! [Azusa](https://github.com/elci-group/azusa) is repository drift
//! intelligence: it compares repository states — the worktree against an
//! upstream or remote branch, any two revisions, or plain directory trees —
//! and produces a quantified, explainable drift report. Dreamseq uses it to
//! compare remotes (GitHub, GitLab, Bitbucket, …) against local clones, and
//! keeps a per-user cache of observed drift scores so every device a user
//! works on contributes to — and can see — how far each repository has
//! drifted, with the age of both sides of the comparison spelled out.
//!
//! Azusa never fetches and never touches the network; it reads the local
//! object database only. Dreamseq therefore runs `git fetch` itself before
//! asking azusa for a report. The azusa CLI is invoked as a subprocess
//! (override with `AZUSA_BINARY`), mirroring the `bound` integration.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::color::Colorize;
use crate::config::{AccountTier, DreamseqConfig};

/// Environment variable overriding the azusa binary location.
pub const AZUSA_BINARY_ENV: &str = "AZUSA_BINARY";

/// Schema version of the drift cache written by this module.
pub const DRIFT_CACHE_SCHEMA_VERSION: u32 = 1;

/// Upper bound on local snapshots retained per repo in the cache. Display
/// limits come from the account tier; this only bounds file size.
const MAX_RETAINED_LOCALS: usize = 64;

// ---------------------------------------------------------------------------
// Subprocess client
// ---------------------------------------------------------------------------

/// Subprocess client for the `azusa` CLI.
#[derive(Debug, Clone)]
pub struct AzusaClient {
    binary: PathBuf,
}

impl AzusaClient {
    /// Resolve the azusa binary: explicit env override first, then a PATH
    /// probe. Returns `None` when azusa is not installed.
    pub fn detect() -> Option<Self> {
        if let Ok(override_path) = std::env::var(AZUSA_BINARY_ENV) {
            let binary = PathBuf::from(&override_path);
            if binary.exists() {
                return Some(Self { binary });
            }
        }
        let probe = std::process::Command::new("azusa")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match probe {
            Ok(status) if status.success() => Some(Self {
                binary: PathBuf::from("azusa"),
            }),
            _ => None,
        }
    }

    /// Run `azusa drift --json` in `repo` and parse the canonical report.
    ///
    /// Azusa's exit codes 1 and 2 mean drift was *found* — a successful
    /// analysis — so only 4 (analysis failure) and 5 (invalid invocation)
    /// are treated as errors here.
    pub fn drift(&self, repo: &Path) -> Result<AzusaDriftReport> {
        let output = std::process::Command::new(&self.binary)
            .args(["drift", "--json"])
            .current_dir(repo)
            .stdin(std::process::Stdio::null())
            .output()
            .with_context(|| format!("running azusa drift in {}", repo.display()))?;

        match output.status.code() {
            Some(4) | Some(5) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!(
                    "azusa could not analyze {}: {}",
                    repo.display(),
                    stderr.trim()
                );
            }
            Some(_) => {}
            None => anyhow::bail!("azusa was terminated while analyzing {}", repo.display()),
        }

        let report: AzusaDriftReport = serde_json::from_slice(&output.stdout)
            .with_context(|| format!("parsing azusa drift report for {}", repo.display()))?;
        if report.schema_version != 1 {
            anyhow::bail!(
                "unsupported azusa report schema_version {} (expected 1); upgrade dreamseq or pin AZUSA_BINARY to a matching azusa build",
                report.schema_version
            );
        }
        Ok(report)
    }
}

/// Best-effort `git fetch --all` so azusa sees current remote refs. Returns
/// whether the fetch succeeded; failure usually means the network is
/// unreachable, in which case azusa still compares against the last-fetched
/// refs and the cache keeps the previous remote observation time.
pub fn fetch_remotes(repo: &Path) -> bool {
    std::process::Command::new("git")
        .args(["fetch", "--all", "--quiet", "--tags"])
        .current_dir(repo)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Whether `dir` is inside a git work tree. Drift measurement skips
/// directories that are not git checkouts rather than failing on them.
pub fn is_git_worktree(dir: &Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The origin remote URL of `repo`, when it has one. This is the cache key
/// that lets scores from different devices aggregate onto the same repo.
pub fn origin_url(repo: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(repo)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!url.is_empty()).then_some(url)
}

// ---------------------------------------------------------------------------
// azusa report (minimal contract; JSON is the contract, extra fields ignored)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct AzusaDriftReport {
    pub schema_version: u32,
    pub repository: AzusaRepository,
    #[serde(default)]
    pub state_b: Option<AzusaStateRef>,
    #[serde(default)]
    pub direction: String,
    pub score: AzusaScore,
    pub risk: AzusaRisk,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzusaRepository {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzusaStateRef {
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzusaScore {
    /// Composite drift index, 0.0–10.0 (0 = in sync).
    pub overall: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzusaRisk {
    #[serde(default)]
    pub level: String,
}

// ---------------------------------------------------------------------------
// Drift cache
// ---------------------------------------------------------------------------

/// Per-user cache of observed drift scores, keyed by repository identity
/// (origin URL when known, else the canonical repo path). One observation per
/// device per repo; the file aggregates cleanly across a user's machines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftCache {
    pub schema_version: u32,
    #[serde(default)]
    pub repos: BTreeMap<String, RepoDriftEntry>,
}

impl Default for DriftCache {
    fn default() -> Self {
        Self {
            schema_version: DRIFT_CACHE_SCHEMA_VERSION,
            repos: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDriftEntry {
    /// Stable identity (origin URL or canonical path); the cache key.
    pub identity: String,
    /// Display name: directory or remote-derived, from azusa's report.
    pub name: String,
    /// Where this repo lives on the device that recorded the observation, so
    /// later runs can rescan it. Devices disagree on paths for URL-keyed
    /// repos; each device overwrites this with its own location.
    #[serde(default)]
    pub local_path: Option<PathBuf>,
    /// When the remote side was last observed (fetch + analysis both ran).
    pub remote_captured_at: Option<DateTime<Utc>>,
    /// Per-device local observations, newest first. One per device: a
    /// device's current state is singular, so a rescan replaces its entry.
    #[serde(default)]
    pub locals: Vec<LocalDriftSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalDriftSnapshot {
    /// Device that produced this observation (see `device_name`).
    pub device: String,
    /// azusa composite drift index at observation time, 0.0–10.0.
    pub score: f32,
    /// Drift direction ("in-sync", "a-to-b", "b-to-a", …), as reported.
    pub direction: String,
    /// Risk level ("low", "medium", "high", "critical"), as reported.
    pub risk: String,
    /// The remote state selector compared against (e.g. "origin/main").
    pub compared_to: String,
    /// When the local state was captured.
    pub local_captured_at: DateTime<Utc>,
    /// When the remote state compared against was captured.
    pub remote_captured_at: DateTime<Utc>,
}

/// Path of the drift cache: alongside the config file
/// (`~/.config/dreamseq/drift-cache.json`).
pub fn drift_cache_path() -> Result<PathBuf> {
    let config_path = DreamseqConfig::path()?;
    Ok(config_path.with_file_name("drift-cache.json"))
}

impl DriftCache {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading drift cache {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("parsing drift cache {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        crate::fs_security::write_private_atomic(path, content.as_bytes())?;
        Ok(())
    }
}

/// Fold a fresh azusa report into the cache, replacing any previous snapshot
/// from the same device. `remote_captured_at` is the caller's responsibility:
/// it should be "now" only when a fetch confirmed the remote state.
pub fn record_observation(
    cache: &mut DriftCache,
    identity: String,
    name: String,
    local_path: Option<PathBuf>,
    report: &AzusaDriftReport,
    device: &str,
    local_captured_at: DateTime<Utc>,
    remote_captured_at: DateTime<Utc>,
) {
    let entry = cache
        .repos
        .entry(identity.clone())
        .or_insert_with(|| RepoDriftEntry {
            identity: identity.clone(),
            name: name.clone(),
            local_path: None,
            remote_captured_at: None,
            locals: Vec::new(),
        });
    entry.name = name;
    if let Some(path) = local_path {
        entry.local_path = Some(path);
    }
    entry.remote_captured_at = Some(remote_captured_at);
    entry.locals.retain(|snapshot| snapshot.device != device);
    entry.locals.insert(
        0,
        LocalDriftSnapshot {
            device: device.to_string(),
            score: report.score.overall,
            direction: report.direction.clone(),
            risk: report.risk.level.clone(),
            compared_to: report
                .state_b
                .as_ref()
                .map(|state| state.label.clone())
                .unwrap_or_default(),
            local_captured_at,
            remote_captured_at,
        },
    );
    entry.locals.truncate(MAX_RETAINED_LOCALS);
}

// ---------------------------------------------------------------------------
// Measurement & upload
// ---------------------------------------------------------------------------

/// One fresh observation, ready to fold into the cache and upload.
#[derive(Debug, Clone, Serialize)]
pub struct RecordedObservation {
    /// Cache key (origin URL when known, else canonical path).
    pub identity: String,
    /// Display name from azusa's report.
    pub name: String,
    /// Where the repo lives on the observing device.
    pub local_path: PathBuf,
    /// When the remote side of this observation was captured.
    pub remote_captured_at: DateTime<Utc>,
    /// The device's local snapshot, including score and timestamps.
    pub snapshot: LocalDriftSnapshot,
}

/// Outcome of a measurement pass over all locally available repos.
#[derive(Debug, Default)]
pub struct DriftMeasurement {
    /// Observations recorded during this pass (upload candidates).
    pub observations: Vec<RecordedObservation>,
    /// Non-fatal problems, one per repo that could not be measured.
    pub warnings: Vec<String>,
}

/// Fetch (best effort), score one repo with azusa, and fold the result into
/// `cache`. Returns the recorded observation on success.
fn scan_repo(
    client: &AzusaClient,
    cache: &mut DriftCache,
    config: &DreamseqConfig,
    repo: &Path,
) -> Result<RecordedObservation> {
    let fetched = fetch_remotes(repo);
    let report = client.drift(repo)?;
    let now = Utc::now();
    let local_path = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let identity = origin_url(repo).unwrap_or_else(|| local_path.display().to_string());
    // Without a successful fetch we cannot claim the remote state is
    // current; carry the previous observation time forward instead.
    let previous_remote_at = cache
        .repos
        .get(&identity)
        .and_then(|entry| entry.remote_captured_at);
    let remote_captured_at = if fetched {
        now
    } else {
        previous_remote_at.unwrap_or(now)
    };
    let device = device_name(config);
    let snapshot = LocalDriftSnapshot {
        device: device.clone(),
        score: report.score.overall,
        direction: report.direction.clone(),
        risk: report.risk.level.clone(),
        compared_to: report
            .state_b
            .as_ref()
            .map(|state| state.label.clone())
            .unwrap_or_default(),
        local_captured_at: now,
        remote_captured_at,
    };
    record_observation(
        cache,
        identity.clone(),
        report.repository.name.clone(),
        Some(local_path.clone()),
        &report,
        &device,
        now,
        remote_captured_at,
    );
    Ok(RecordedObservation {
        identity,
        name: report.repository.name.clone(),
        local_path,
        remote_captured_at,
        snapshot,
    })
}

/// Directories worth scanning this run: the extra dirs first (typically the
/// current working directory), then every cached repo path that still exists
/// on this device, de-duplicated. Non-git directories are dropped so a stray
/// cache entry cannot break the pass.
pub fn local_scan_candidates(cache: &DriftCache, extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for dir in extra_dirs.iter().chain(
        cache
            .repos
            .values()
            .filter_map(|entry| entry.local_path.as_ref()),
    ) {
        let Ok(canonical) = dir.canonicalize() else {
            continue;
        };
        if !canonical.is_dir() || !seen.insert(canonical.clone()) {
            continue;
        }
        if !is_git_worktree(&canonical) {
            continue;
        }
        candidates.push(canonical);
    }
    candidates
}

/// Measure drift for every available repo: `extra_dirs` first, then each
/// cached repo that exists locally. Updates and saves the drift cache;
/// per-repo failures degrade to warnings rather than failing the pass.
pub fn measure_repos(config: &DreamseqConfig, extra_dirs: &[PathBuf]) -> Result<DriftMeasurement> {
    let mut measurement = DriftMeasurement::default();
    let Some(client) = AzusaClient::detect() else {
        measurement.warnings.push(format!(
            "azusa is not installed or not on PATH; skipping drift measurement \
             (set {AZUSA_BINARY_ENV} or install azusa)"
        ));
        return Ok(measurement);
    };

    let cache_path = drift_cache_path()?;
    let mut cache = DriftCache::load(&cache_path)?;
    for repo in local_scan_candidates(&cache, extra_dirs) {
        match scan_repo(&client, &mut cache, config, &repo) {
            Ok(observation) => measurement.observations.push(observation),
            Err(error) => measurement.warnings.push(format!(
                "could not measure drift in {}: {error}",
                repo.display()
            )),
        }
    }
    if !measurement.observations.is_empty() {
        if let Err(error) = cache.save(&cache_path) {
            measurement
                .warnings
                .push(format!("could not save drift cache: {error}"));
        }
    }
    Ok(measurement)
}

/// Upload fresh observations to the paired Dreamsequence account so the
/// website's drift scores update. Returns `Ok(None)` when this device is not
/// paired (nothing to do), `Ok(Some(n))` with the observations uploaded, or
/// the transport error.
pub async fn upload_if_paired(
    observations: &[RecordedObservation],
    trace_id: &str,
) -> Result<Option<usize>> {
    if observations.is_empty() {
        return Ok(Some(0));
    }
    let Some(credentials) =
        crate::cloud::CredentialStore::discover().and_then(|store| store.load_optional())?
    else {
        return Ok(None);
    };
    crate::cloud::CloudClient::new(Some(&credentials.api_url))?
        .upload_drift_with_trace_id(&credentials, observations, trace_id)
        .await?;
    Ok(Some(observations.len()))
}

// ---------------------------------------------------------------------------
// Device identity
// ---------------------------------------------------------------------------

/// Resolve the device name for drift observations: the configured name wins,
/// then the OS hostname from the environment, then a placeholder.
pub fn resolve_device_name(configured: Option<&str>, hostname: Option<&str>) -> String {
    configured
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            hostname
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown-device".to_string())
}

/// Device name used for observations on this machine.
pub fn device_name(config: &DreamseqConfig) -> String {
    if let Some(name) = config
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return name.to_string();
    }
    if let Ok(name) = std::env::var("HOSTNAME")
        && let name = name.trim()
        && !name.is_empty()
    {
        return name.to_string();
    }
    hostname()
        .map(str::to_string)
        .unwrap_or_else(|| "unknown-device".to_string())
}

/// Fallback for environments where `HOSTNAME` is not exported into child
/// processes (the common default in bash): read the kernel hostname file.
fn hostname() -> Option<&'static str> {
    use std::sync::OnceLock;
    static HOSTNAME: OnceLock<Option<String>> = OnceLock::new();
    HOSTNAME
        .get_or_init(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|content| content.trim().to_string())
                .filter(|content| !content.is_empty())
        })
        .as_deref()
}

// ---------------------------------------------------------------------------
// Tier-limited view model
// ---------------------------------------------------------------------------

/// How the enterprise average was computed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterpriseAverage {
    /// Mean of the included local scores, rounded like azusa (2 decimals).
    pub score: f32,
    /// Number of device observations included (at most 16).
    pub devices: usize,
    /// Newest local observation included in the average.
    pub local_captured_at: DateTime<Utc>,
    /// Newest remote observation included in the average.
    pub remote_captured_at: Option<DateTime<Utc>>,
}

/// What a tier is allowed to see for one repo. Pro shows individual local
/// scores (up to 4); enterprise collapses up to 16 device scores into a
/// single collaboration-friendly average instead of a wall of numbers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDriftView {
    pub identity: String,
    pub name: String,
    pub remote_captured_at: Option<DateTime<Utc>>,
    /// Individual local snapshots, at most `tier.max_local_scores()`.
    pub locals: Vec<LocalDriftSnapshot>,
    /// Present only for the enterprise tier.
    pub average: Option<EnterpriseAverage>,
}

/// Build the tier-limited view of one cache entry. Local snapshots are
/// already newest-first; the newest `max_local_scores()` are shown, and the
/// enterprise average is taken over those same snapshots.
pub fn view_entry(entry: &RepoDriftEntry, tier: AccountTier) -> RepoDriftView {
    let limited: Vec<LocalDriftSnapshot> = entry
        .locals
        .iter()
        .take(tier.max_local_scores())
        .cloned()
        .collect();

    let average = (tier == AccountTier::Enterprise && !limited.is_empty()).then(|| {
        let sum: f32 = limited.iter().map(|snapshot| snapshot.score).sum();
        EnterpriseAverage {
            score: (sum / limited.len() as f32 * 100.0).round() / 100.0,
            devices: limited.len(),
            local_captured_at: limited[0].local_captured_at,
            remote_captured_at: Some(limited[0].remote_captured_at),
        }
    });

    RepoDriftView {
        identity: entry.identity.clone(),
        name: entry.name.clone(),
        remote_captured_at: entry.remote_captured_at,
        locals: if tier == AccountTier::Enterprise {
            Vec::new()
        } else {
            limited
        },
        average,
    }
}

/// Tier-limited view of the whole cache.
pub fn view_cache(cache: &DriftCache, tier: AccountTier) -> Vec<RepoDriftView> {
    cache
        .repos
        .values()
        .map(|entry| view_entry(entry, tier))
        .collect()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn fmt_time(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn score_style(score: f32) -> String {
    let text = format!("{score:.2}");
    if score >= 7.0 {
        text.red()
    } else if score >= 4.0 {
        text.yellow()
    } else {
        text.green()
    }
}

fn risk_style(risk: &str) -> String {
    match risk {
        "critical" => risk.red(),
        "high" => risk.red(),
        "medium" => risk.yellow(),
        _ => risk.dimmed(),
    }
}

/// Human-readable rendering of the tier-limited drift view.
pub fn render_human(views: &[RepoDriftView], tier: AccountTier) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}", "🧭 Azusa drift scores".bold()));
    out.push_str(&format!(" {}\n", format!("(tier: {tier})").dimmed()));

    if views.is_empty() {
        out.push_str(&format!(
            "\n{} No cached drift knowledge yet. Run {} inside a git repository to record one.\n",
            "ℹ️".blue(),
            "dreamseq drift".cyan()
        ));
        return out;
    }

    for view in views {
        out.push_str(&format!("\n{}\n", view.name.cyan().bold()));
        out.push_str(&format!(
            "  remote (as of {})\n",
            view.remote_captured_at
                .map(fmt_time)
                .unwrap_or_else(|| "unknown".dimmed())
        ));
        if let Some(average) = &view.average {
            out.push_str(&format!(
                "  local across {} {} (as of {}) · drift score {} · {}\n",
                average.devices,
                if average.devices == 1 {
                    "device"
                } else {
                    "devices"
                },
                fmt_time(average.local_captured_at),
                score_style(average.score),
                "enterprise average".dimmed()
            ));
        }
        for snapshot in &view.locals {
            out.push_str(&format!(
                "  local on {} (as of {}) · {} · drift score {} {} {}\n",
                snapshot.device.white(),
                fmt_time(snapshot.local_captured_at),
                if snapshot.compared_to.is_empty() {
                    "remote".dimmed()
                } else {
                    snapshot.compared_to.dimmed()
                },
                score_style(snapshot.score),
                risk_style(&snapshot.risk),
                snapshot.direction.dimmed()
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Command orchestration
// ---------------------------------------------------------------------------

/// Run `dreamseq drift`: refresh the current repo's observation when azusa is
/// available (unless `--cached`), upload fresh scores when paired, then
/// render every cached repo's scores through the tier-limited view.
pub async fn run_drift(config: &DreamseqConfig, cached: bool, json: bool) -> Result<()> {
    let cache_path = drift_cache_path()?;
    let mut cache = DriftCache::load(&cache_path)?;
    let mut warnings: Vec<String> = Vec::new();
    let mut uploaded: Option<usize> = None;

    if !cached {
        let repo = std::env::current_dir()?;
        let mut fresh: Vec<RecordedObservation> = Vec::new();
        match AzusaClient::detect() {
            Some(client) => match scan_repo(&client, &mut cache, config, &repo) {
                Ok(observation) => {
                    fresh.push(observation);
                    if let Err(error) = cache.save(&cache_path) {
                        warnings.push(format!("could not save drift cache: {error}"));
                    }
                }
                Err(error) => warnings.push(format!("azusa scan failed: {error}")),
            },
            None => warnings.push(format!(
                "azusa is not installed or not on PATH; showing cached scores only \
                 (set {AZUSA_BINARY_ENV} or install azusa, e.g. \
                 `cargo install --path ../azusa/crates/azusa-cli`)"
            )),
        }
        match upload_if_paired(&fresh, &crate::telemetry::new_trace_id()).await {
            Ok(count) => uploaded = count,
            Err(error) => warnings.push(format!("could not upload drift scores: {error}")),
        }
    }

    let tier = config.tier;
    let views = view_cache(&cache, tier);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "tier": tier,
                "repos": views,
                "warnings": warnings,
                "uploaded": uploaded.unwrap_or(0),
            }))?
        );
    } else {
        for warning in &warnings {
            eprintln!("{} {}", "⚠️".yellow(), warning.yellow());
        }
        if let Some(count) = uploaded
            && count > 0
        {
            println!(
                "{}",
                format!("☁️ Uploaded {count} drift score(s) to Dreamsequence").dimmed()
            );
        }
        print!("{}", render_human(&views, tier));
    }
    Ok(())
}
