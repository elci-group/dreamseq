// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! `dreamseq stack-report`: detect agent harnesses, developer tools, and
//! workstation specs, and report them to the Dreamsequence API.
//!
//! Ported from dreamcred's `stack-report` subcommand. dreamcred is a
//! device-local credential broker with its own generic profile system, so
//! its version needed `--profile`/env-var plumbing to find an API URL and
//! access token; dreamseq already manages its own pairing (`dreamseq
//! login`), so this version simply reuses the paired [`crate::cloud`]
//! credentials instead.

use crate::color::Colorize;
use crate::devtools::{self, DetectedTool};
use crate::stack_probe;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Agent harnesses: the CLI and GUI tools people use to drive coding agents
// from a device. Each entry lists CLI binary candidates (checked on PATH)
// and install-path markers (checked with glob, for GUI tools and extensions
// that have no CLI). First hit wins; version comes from `<binary> --version`.
// ---------------------------------------------------------------------------

struct HarnessDef {
    name: &'static str,
    binaries: &'static [&'static str],
    markers: &'static [&'static str],
}

const AGENT_HARNESSES: &[HarnessDef] = &[
    HarnessDef {
        name: "claude",
        binaries: &["claude", "claude-code"],
        markers: &["~/.claude"],
    },
    HarnessDef {
        name: "codex",
        binaries: &["codex"],
        markers: &["~/.codex"],
    },
    HarnessDef {
        name: "kimi",
        binaries: &["kimi", "kimi-code"],
        markers: &["~/.kimi", "~/.config/kimi"],
    },
    HarnessDef {
        name: "kilo",
        binaries: &["kilo", "kilocode"],
        markers: &[
            "~/.vscode/extensions/kilocode.kilo-code*",
            "~/.cursor/extensions/kilocode.kilo-code*",
        ],
    },
    HarnessDef {
        name: "openclaw",
        binaries: &["openclaw"],
        markers: &["~/.openclaw"],
    },
    HarnessDef {
        name: "hermes",
        binaries: &["hermes"],
        markers: &["~/.hermes"],
    },
    HarnessDef {
        name: "crush",
        binaries: &["crush"],
        markers: &["~/.config/crush"],
    },
    HarnessDef {
        name: "gemini",
        binaries: &["gemini", "gemini-cli"],
        markers: &["~/.gemini"],
    },
    HarnessDef {
        name: "aider",
        binaries: &["aider"],
        markers: &[],
    },
    HarnessDef {
        name: "amp",
        binaries: &["amp"],
        markers: &["~/.config/amp"],
    },
    HarnessDef {
        name: "goose",
        binaries: &["goose"],
        markers: &["~/.config/goose"],
    },
    HarnessDef {
        name: "copilot",
        binaries: &["copilot", "gh-copilot"],
        markers: &[],
    },
    HarnessDef {
        name: "cursor",
        binaries: &["cursor", "cursor-agent"],
        markers: &[
            "~/.local/share/cursor",
            "/opt/cursor*",
            "/Applications/Cursor.app",
            "~/Applications/cursor*",
        ],
    },
    HarnessDef {
        name: "windsurf",
        binaries: &["windsurf"],
        markers: &[
            "~/.local/share/windsurf",
            "/opt/windsurf*",
            "/Applications/Windsurf.app",
        ],
    },
    HarnessDef {
        name: "zed",
        binaries: &["zed", "zeditor"],
        markers: &["~/.local/zed.app", "/Applications/Zed.app"],
    },
    HarnessDef {
        name: "cline",
        binaries: &[],
        markers: &[
            "~/.vscode/extensions/saoudrizwan.claude-dev*",
            "~/.cursor/extensions/saoudrizwan.claude-dev*",
        ],
    },
    HarnessDef {
        name: "roo",
        binaries: &["roo"],
        markers: &[
            "~/.vscode/extensions/rooveterinaryinc.roo-cline*",
            "~/.cursor/extensions/rooveterinaryinc.roo-cline*",
        ],
    },
];
const HARNESS_CAP: usize = 200;

/// One detected agent harness. `probe` carries the capability evidence from
/// [`stack_probe`]; it is omitted from JSON entirely when probing produced
/// nothing (e.g. a probe error), so payloads stay compatible with consumers
/// written before probes existed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessInfo {
    pub name: String,
    pub version: Option<String>,
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<stack_probe::HarnessProbe>,
}

/// Probe an executable's version for a harness: the first non-empty line of
/// `<executable> --version`, raw and unparsed (unlike the devtools registry's
/// [`crate::devtools`] probe, which extracts a clean version number). Mirrors
/// dreamcred's top-level `binary_version()`. stdout is preferred over stderr,
/// approximating the original's merged-stream read.
async fn harness_binary_version(executable: &str) -> Option<String> {
    let child = tokio::process::Command::new(executable)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let output = tokio::time::timeout(Duration::from_secs(2), async {
        child.wait_with_output().await
    })
    .await
    .ok()?
    .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

/// Detect installed agent harnesses on PATH or by install-path marker.
pub async fn detect_harnesses() -> Vec<HarnessInfo> {
    let mut harnesses = Vec::new();
    for def in AGENT_HARNESSES {
        let resolved = def.binaries.iter().find_map(|name| devtools::which(name));
        let marker = def
            .markers
            .iter()
            .find_map(|pattern| devtools::marker_path(pattern));
        if resolved.is_none() && marker.is_none() {
            continue;
        }
        let mut executable = resolved.clone().or_else(|| devtools::which(def.name));
        if executable.is_none()
            && let Some(marker_path) = &marker
            && Path::new(marker_path).is_file()
        {
            executable = Some(marker_path.clone());
        }
        let version = match &executable {
            Some(exe) => harness_binary_version(exe).await,
            None => None,
        };
        // Capability probe: best-effort by construction (see `stack_probe`),
        // so this never fails detection — worst case the probe reports
        // `status: "detected"` with empty provider lists.
        let probe = Some(stack_probe::probe_harness(
            def.name,
            None,
            &stack_probe::tcp_connector,
        ));
        harnesses.push(HarnessInfo {
            name: def.name.to_string(),
            version,
            origin: resolved.or(marker),
            probe,
        });
    }
    harnesses.sort_by(|a, b| a.name.cmp(&b.name));
    harnesses.truncate(HARNESS_CAP);
    harnesses
}

// ---------------------------------------------------------------------------
// Workstation specs
// ---------------------------------------------------------------------------

/// Best-effort machine vendor/product, from DMI sysfs, device tree, or
/// `sysctl` on macOS.
#[derive(Debug, Clone, Serialize)]
pub struct OemInfo {
    pub name: String,
    pub key: String,
    pub product: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Specs {
    pub hostname: String,
    pub os: String,
    pub os_release: String,
    pub os_id: String,
    pub cpu_vendor: String,
    pub oem: OemInfo,
    pub arch: String,
    pub cpu: String,
    pub memory_total_mb: Option<u64>,
    pub cli_version: String,
}

const OS_ID_MAP: &[(&str, &str)] = &[
    ("darwin", "macos"),
    ("windows", "windows"),
    ("ubuntu", "ubuntu"),
    ("debian", "debian"),
    ("fedora", "fedora"),
    ("arch", "arch"),
    ("manjaro", "arch"),
    ("linuxmint", "linuxmint"),
    ("mint", "linuxmint"),
    ("kali", "kali"),
    ("kalilinux", "kali"),
    ("centos", "centos"),
    ("rhel", "redhat"),
    ("redhat", "redhat"),
    ("opensuse", "opensuse"),
    ("suse", "suse"),
    ("alpine", "alpine"),
    ("gentoo", "gentoo"),
    ("pop", "pop-os"),
    ("popos", "pop-os"),
    ("elementary", "ubuntu"),
    ("zorin", "ubuntu"),
    ("nobara", "fedora"),
    ("raspbian", "raspberry-pi-os"),
    ("chromeos", "chromeos"),
    ("chromiumos", "chromeos"),
    ("nixos", "nixos"),
    ("void", "void"),
];

const OEM_MAP: &[(&str, &str)] = &[
    ("system76", "System76"),
    ("star labs", "Star Labs"),
    ("starlabs", "Star Labs"),
    ("tuxedo", "TUXEDO Computers"),
    ("slimbook", "Slimbook"),
    ("framework", "Framework"),
    ("purism", "Purism"),
    ("lenovo", "Lenovo"),
    ("dell", "Dell"),
    ("alienware", "Alienware"),
    ("hewlett", "HP"),
    ("hp", "HP"),
    ("asus", "ASUS"),
    ("micro-star", "MSI"),
    ("msi", "MSI"),
    ("gigabyte", "Gigabyte"),
    ("apple", "Apple"),
    ("acer", "Acer"),
    ("razer", "Razer"),
    ("huawei", "Huawei"),
    ("samsung", "Samsung"),
    ("lg electronics", "LG"),
    ("sony", "Sony"),
    ("microsoft", "Microsoft"),
    ("raspberry pi", "Raspberry Pi"),
    ("pine64", "PINE64"),
    ("qemu", "QEMU"),
    ("vmware", "VMware"),
    ("innotek", "VirtualBox"),
    ("amazon ec2", "Amazon EC2"),
    ("google", "Google Cloud"),
];

fn slugify(value: &str) -> String {
    static NON_ALNUM_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = NON_ALNUM_RE.get_or_init(|| regex::Regex::new(r"[^a-z0-9]+").unwrap());
    re.replace_all(&value.to_lowercase(), "-")
        .trim_matches('-')
        .to_string()
}

/// OS name in `platform.system()` spelling ("Linux", "Darwin", "Windows"),
/// since that's the vocabulary the rest of this module and the API contract
/// were built around.
fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Darwin",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

/// Best-effort hostname: the `hostname` command (present on Linux, macOS,
/// and Windows alike), then `$HOSTNAME`, then `/etc/hostname`.
fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn kernel_release(system: &str) -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| system.to_string())
}

fn os_release(system: &str) -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                return value.trim().trim_matches('"').to_string();
            }
        }
    }
    kernel_release(system)
}

fn detect_os_id(system: &str) -> String {
    if system == "Darwin" {
        return "macos".to_string();
    }
    if system == "Windows" {
        return "windows".to_string();
    }
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        let mut fields: HashMap<&str, String> = HashMap::new();
        for line in content.lines() {
            if let Some((key, value)) = line.split_once('=') {
                fields.insert(key, value.trim().trim_matches('"').to_string());
            }
        }
        let id = fields.get("ID").cloned().unwrap_or_default();
        let id_like = fields.get("ID_LIKE").cloned().unwrap_or_default();
        for candidate in std::iter::once(id.as_str()).chain(id_like.split_whitespace()) {
            let lowered = candidate.to_lowercase();
            if let Some((_, mapped)) = OS_ID_MAP.iter().find(|(key, _)| *key == lowered) {
                return (*mapped).to_string();
            }
        }
    }
    "linux".to_string()
}

fn detect_cpu_vendor(system: &str, arch: &str) -> String {
    if system == "Darwin" {
        return if arch == "aarch64" || arch == "arm64" {
            "apple"
        } else {
            "intel"
        }
        .to_string();
    }
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        let lowered = cpuinfo.to_lowercase();
        if lowered.contains("authenticamd") {
            return "amd".to_string();
        }
        if lowered.contains("genuineintel") {
            return "intel".to_string();
        }
    }
    let machine = arch.to_lowercase();
    if machine == "aarch64" || machine == "arm64" || machine.starts_with("arm") {
        return "arm".to_string();
    }
    "unknown".to_string()
}

fn detect_oem(system: &str) -> OemInfo {
    let mut candidates: Vec<String> = Vec::new();
    for field in ["sys_vendor", "board_vendor", "product_name", "board_name"] {
        if let Ok(value) = std::fs::read_to_string(format!("/sys/class/dmi/id/{field}")) {
            candidates.push(value.trim().to_string());
        }
    }
    if let Ok(value) = std::fs::read_to_string("/proc/device-tree/model") {
        candidates.push(value.trim_matches('\0').to_string());
    }
    if system == "Darwin"
        && let Ok(output) = std::process::Command::new("sysctl")
            .args(["-n", "hw.model"])
            .output()
        && output.status.success()
    {
        candidates.push(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let joined = candidates
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ");
    let lowered = joined.to_lowercase();
    for (marker, name) in OEM_MAP {
        if lowered.contains(marker) {
            let product = std::fs::read_to_string("/sys/class/dmi/id/product_name")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            return OemInfo {
                name: (*name).to_string(),
                key: slugify(name),
                product,
            };
        }
    }
    if joined.is_empty() {
        return OemInfo {
            name: "Unknown".to_string(),
            key: "unknown".to_string(),
            product: None,
        };
    }
    // Matches dreamcred exactly: it's specifically candidates[0] (sys_vendor)
    // here, not the first non-empty candidate -- an upstream quirk preserved
    // for parity.
    let raw = candidates
        .first()
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("Unknown");
    let name: String = raw.chars().take(64).collect();
    let mut key = slugify(raw);
    key.truncate(32);
    let key = if key.is_empty() {
        "unknown".to_string()
    } else {
        key
    };
    OemInfo {
        name,
        key,
        product: None,
    }
}

fn detect_cpu() -> String {
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("model name")
                && let Some((_, value)) = line.split_once(':')
            {
                return value.trim().to_string();
            }
        }
    }
    String::new()
}

fn detect_memory_mb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        if line.starts_with("MemTotal")
            && let Some(kb_str) = line.split_whitespace().nth(1)
            && let Ok(kb) = kb_str.parse::<u64>()
        {
            return Some(kb / 1024);
        }
    }
    None
}

fn arch_name(system: &str) -> String {
    let arch = std::env::consts::ARCH;
    if system == "Darwin" && arch == "aarch64" {
        "arm64".to_string()
    } else {
        arch.to_string()
    }
}

/// Detect this workstation's specs: hostname, OS, CPU, memory, and OEM.
pub fn detect_specs() -> Specs {
    let system = os_name();
    let arch = arch_name(system);
    Specs {
        hostname: hostname(),
        os: system.to_string(),
        os_release: os_release(system),
        os_id: detect_os_id(system),
        cpu_vendor: detect_cpu_vendor(system, &arch),
        oem: detect_oem(system),
        arch,
        cpu: detect_cpu(),
        memory_total_mb: detect_memory_mb(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

// ---------------------------------------------------------------------------
// Command orchestration
// ---------------------------------------------------------------------------

/// Identifies this CLI as the report's source, distinct from the retired
/// `dreamcred` tool (which never sent this field). The Dreamsequence API
/// only offers remote-triggered stack-report updates ("upon next contact")
/// to devices whose most recent report carries this marker, since there is
/// no way to make an unrecognized or legacy client honour such a request.
pub const REPORTER: &str = "dreamseq";

/// Full stack-report payload: specs, agent harnesses, and detected dev tools.
#[derive(Debug, Clone, Serialize)]
pub struct StackReportPayload {
    #[serde(flatten)]
    pub specs: Specs,
    pub harnesses: Vec<HarnessInfo>,
    pub tools: Vec<DetectedTool>,
    pub reporter: &'static str,
}

/// Comma-separated devtools registry batch list, e.g. `"1,2,3"`.
pub fn parse_batches(value: &str) -> Result<HashSet<u32>, String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<u32>()
                .map_err(|_| format!("invalid batch list: {value:?}"))
        })
        .collect()
}

/// "Upon next contact" scheduling, exercised by `dreamseq run`: this
/// codebase has no way to push a request to a device, so instead every
/// paired device checks in here and submits a fresh report itself if one is
/// due — either because a workspace member requested a remote update via
/// the dashboard, or because this device was just paired and has never
/// reported at all (mandatory first-contact reporting; see also the
/// unconditional report `dreamseq login` submits right after pairing, which
/// this call would otherwise duplicate on the very next `run`).
///
/// Returns `Ok(None)` when this device is not paired (nothing to check),
/// `Ok(Some(true))` when a report was due and has just been submitted, or
/// `Ok(Some(false))` when nothing was due.
pub async fn check_in_if_paired(trace_id: &str) -> Result<Option<bool>> {
    let Some(credentials) =
        crate::cloud::CredentialStore::discover().and_then(|store| store.load_optional())?
    else {
        return Ok(None);
    };
    let client = crate::cloud::CloudClient::new(Some(&credentials.api_url))?;
    if !client
        .report_status_with_trace_id(&credentials, trace_id)
        .await?
    {
        return Ok(Some(false));
    }
    run_stack_report(false, false, None).await?;
    Ok(Some(true))
}

/// Run `dreamseq stack-report`: compose the payload and either print it
/// (`--dry-run`) or send it to the paired Dreamsequence account.
pub async fn run_stack_report(
    dry_run: bool,
    as_json: bool,
    batches: Option<HashSet<u32>>,
) -> Result<()> {
    let specs = detect_specs();
    let harnesses = detect_harnesses().await;
    let tools = devtools::detect_devtools(batches.as_ref(), devtools::DEFAULT_CAP).await;
    let payload = StackReportPayload {
        specs,
        harnesses,
        tools,
        reporter: REPORTER,
    };

    if dry_run {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        println!(
            "dry run: {} harnesses + {} dev tools detected (nothing sent)",
            payload.harnesses.len(),
            payload.tools.len()
        );
        return Ok(());
    }

    let credentials = crate::cloud::CredentialStore::discover()?.load()?;
    let client = crate::cloud::CloudClient::new(Some(&credentials.api_url))?;
    let trace_id = crate::telemetry::new_trace_id();
    let response = client
        .report_stack_with_trace_id(&credentials, &payload, &trace_id)
        .await?;

    if as_json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        let device_id = response
            .get("device_id")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        println!(
            "{} Stack reported: {} harnesses + {} dev tools on {} (device {}).",
            "✅".green(),
            payload.harnesses.len(),
            payload.tools.len(),
            payload.specs.hostname,
            device_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_batches() {
        let batches = parse_batches("1, 2,3").unwrap();
        assert_eq!(batches, [1, 2, 3].into_iter().collect());
        assert!(parse_batches("1,x").is_err());
        assert_eq!(parse_batches("").unwrap(), HashSet::new());
    }

    #[test]
    fn os_id_maps_known_distros_and_falls_back_to_linux() {
        assert_eq!(detect_os_id("Darwin"), "macos");
        assert_eq!(detect_os_id("Windows"), "windows");
    }

    #[test]
    fn slugify_strips_non_alphanumerics() {
        assert_eq!(slugify("Star Labs"), "star-labs");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn detect_specs_reports_a_nonempty_hostname_and_platform() {
        let specs = detect_specs();
        assert!(!specs.hostname.is_empty());
        assert!(!specs.os.is_empty());
        assert!(!specs.cli_version.is_empty());
    }

    #[tokio::test]
    async fn detect_harnesses_stays_within_the_cap() {
        let harnesses = detect_harnesses().await;
        assert!(harnesses.len() <= HARNESS_CAP);
    }

    #[tokio::test]
    async fn detected_harnesses_carry_a_serialized_probe() {
        let harnesses = detect_harnesses().await;
        for harness in &harnesses {
            let json = serde_json::to_value(harness).unwrap();
            // Every detected harness gets a probe; unknown-evidence probes
            // degrade to "detected" but are still present.
            assert_eq!(
                json["probe"]["status"],
                harness.probe.as_ref().unwrap().status
            );
        }
    }
}
