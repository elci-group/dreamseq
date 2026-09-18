// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Per-harness capability probes for `dreamseq stack probe`.
//!
//! Where [`crate::stack_report`] answers "which harnesses and tools are
//! installed?" (inventory), this module answers "what can they actually do?"
//! (capability evidence). Each probe reads config *presence* and non-secret
//! values — provider ids, model names, file existence — and never reads,
//! prints, or transmits API key material. Key *presence* in an environment
//! variable or config file is evidence of configuration; the value itself is
//! only ever checked for non-emptiness.
//!
//! Every probe is best-effort with graceful degradation: an unreadable file,
//! a malformed config, or an unreachable host yields `status: "unknown"` or a
//! field omission, never an error. Probes must never fail the command that
//! runs them.

use crate::color::Colorize;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};

/// Cap on each provider list in [`ProviderMatrix`]: truncate rather than
/// error, so a pathological config can never blow up the payload.
pub const PROVIDER_LIST_CAP: usize = 16;

/// Timeout for one TCP reachability check. Only a socket open is attempted —
/// no TLS handshake, no HTTP request, nothing that could spend money.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// The four-state provider matrix for one harness. All lists are normalized
/// lowercase, deduplicated, and capped at [`PROVIDER_LIST_CAP`].
///
/// - `supported`: providers this harness can talk to in principle (static
///   knowledge from [`HARNESS_PROVIDERS`]).
/// - `configured`: providers with local evidence of usable configuration (an
///   env key set, a credentials file, or an enabled provider with a key in
///   dreamseq's own config).
/// - `subscribed`: providers with evidence of an actual account login (a
///   stored OAuth/credentials file), a stronger claim than `configured`.
/// - `available`: `configured ∩ reachable` — the providers this harness could
///   realistically call right now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMatrix {
    #[serde(default)]
    pub supported: Vec<String>,
    #[serde(default)]
    pub configured: Vec<String>,
    #[serde(default)]
    pub subscribed: Vec<String>,
    #[serde(default)]
    pub available: Vec<String>,
}

/// Capability evidence for one agent harness. `status` is always present:
/// `"configured"` when any provider/model/auth evidence was found, otherwise
/// `"detected"` (the caller knows the binary or install marker exists).
/// Everything else is optional and omitted from JSON when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessProbe {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connectivity: Option<bool>,
    #[serde(default)]
    pub providers: ProviderMatrix,
}

/// Static supported-provider knowledge per harness, from each harness's own
/// documentation of the providers it can drive. Harnesses not listed (or
/// unknown names) support nothing we can claim.
const HARNESS_PROVIDERS: &[(&str, &[&str])] = &[
    ("claude", &["anthropic", "openrouter", "bedrock", "vertex"]),
    ("codex", &["openai", "azure"]),
    ("kimi", &["moonshot"]),
    ("gemini", &["google"]),
    (
        "aider",
        &["anthropic", "openai", "openrouter", "deepseek", "gemini"],
    ),
];

/// Canonical endpoint per provider for the reachability check. Mirrors the
/// hosts in `inference_providers::NAMED_PROVIDERS` (which is `pub(super)` and
/// deliberately not widened for this) plus the per-harness endpoints the
/// harnesses themselves talk to. Only the host is used, on port 443.
const PROVIDER_BASE_URLS: &[(&str, &str)] = &[
    ("anthropic", "https://api.anthropic.com"),
    ("openai", "https://api.openai.com"),
    ("openrouter", "https://openrouter.ai"),
    ("moonshot", "https://api.moonshot.ai"),
    ("google", "https://generativelanguage.googleapis.com"),
    ("gemini", "https://generativelanguage.googleapis.com"),
    ("azure", "https://management.azure.com"),
    ("bedrock", "https://bedrock.amazonaws.com"),
    ("vertex", "https://aiplatform.googleapis.com"),
    ("grok", "https://api.x.ai"),
    ("zai", "https://api.z.ai"),
    ("mistral", "https://api.mistral.ai"),
    ("deepseek", "https://api.deepseek.com"),
    ("cerebras", "https://api.cerebras.ai"),
];

/// Well-known API-key environment variables mapped to the provider whose key
/// they hold. Presence (non-empty) counts as configuration evidence; the
/// value is never read beyond the emptiness check.
const ENV_KEY_PROVIDERS: &[(&str, &str)] = &[
    ("OPENAI_API_KEY", "openai"),
    ("ANTHROPIC_API_KEY", "anthropic"),
    ("OPENROUTER_API_KEY", "openrouter"),
    ("GROQ_API_KEY", "groq"),
    ("MISTRAL_API_KEY", "mistral"),
    ("MOONSHOT_API_KEY", "moonshot"),
    ("GEMINI_API_KEY", "google"),
    ("GOOGLE_API_KEY", "google"),
    ("DEEPSEEK_API_KEY", "deepseek"),
    ("CEREBRAS_API_KEY", "cerebras"),
    ("XAI_API_KEY", "grok"),
    ("ZAI_API_KEY", "zai"),
];

/// Evidence gathered by one harness adapter. Adapters only ever report
/// provider ids, model names, and file/env *presence* — never secrets.
#[derive(Default)]
struct AdapterEvidence {
    provider: Option<String>,
    model: Option<String>,
    /// `"configured"` for key/config presence, `"subscribed"` for a stored
    /// account login (OAuth credentials file, auth.json, ...).
    auth: Option<String>,
    configured: BTreeSet<String>,
    subscribed: BTreeSet<String>,
}

impl AdapterEvidence {
    fn note_configured(&mut self, provider: &str) {
        self.configured.insert(provider.to_string());
        if self.auth.is_none() {
            self.auth = Some("configured".to_string());
        }
    }

    fn note_subscribed(&mut self, provider: &str) {
        self.subscribed.insert(provider.to_string());
        self.auth = Some("subscribed".to_string());
    }
}

/// Supported providers for `name` from [`HARNESS_PROVIDERS`]; empty for
/// harnesses we have no static knowledge about.
fn supported_providers(name: &str) -> Vec<String> {
    HARNESS_PROVIDERS
        .iter()
        .find(|(harness, _)| *harness == name)
        .map(|(_, providers)| providers.iter().map(|s| (*s).to_string()).collect())
        .unwrap_or_default()
}

/// Normalize one provider list: lowercase, trim, dedupe (preserving first-seen
/// order), truncate to [`PROVIDER_LIST_CAP`]. Never fails.
fn normalize_providers(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        let normalized = value.trim().to_lowercase();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        out.push(normalized);
        if out.len() >= PROVIDER_LIST_CAP {
            break;
        }
    }
    out
}

/// Resolve a provider id to its canonical endpoint host, if known.
fn provider_base_url(provider: &str) -> Option<&'static str> {
    PROVIDER_BASE_URLS
        .iter()
        .find(|(name, _)| *name == provider)
        .map(|(_, url)| *url)
}

/// Host part of a `http(s)://host[/...]` URL, without the scheme or path.
fn host_of(url: &str) -> Option<&str> {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .filter(|host| !host.is_empty())
}

/// Production connectivity seam: open a bare TCP connection to the provider's
/// host on port 443. No TLS handshake, no HTTP request — the socket open is
/// enough to distinguish "egress to this provider works" from "it does not",
/// and can never authenticate or spend quota. DNS resolution and each connect
/// attempt are bounded by [`CONNECT_TIMEOUT`].
pub fn tcp_connector(provider: &str) -> bool {
    let Some(url) = provider_base_url(provider) else {
        return false;
    };
    let Some(host) = host_of(url) else {
        return false;
    };
    let host = host.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Resolve with the same deadline mindset: try every returned address
        // and stop at the first socket that opens.
        let reachable = match (host.as_str(), 443).to_socket_addrs() {
            Ok(addrs) => addrs
                .filter_map(|addr| {
                    std::net::TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()
                })
                .next()
                .is_some(),
            Err(_) => false,
        };
        // The receiver may already have timed out; a dead send is fine.
        let _ = tx.send(reachable);
    });
    matches!(
        rx.recv_timeout(CONNECT_TIMEOUT + std::time::Duration::from_secs(2)),
        Ok(true)
    )
}

/// True when `DREAMSEQ_STACK_PROBE_OFFLINE` is set: connectivity checks are
/// skipped entirely and `available` degrades to `configured`.
fn offline_mode() -> bool {
    std::env::var_os("DREAMSEQ_STACK_PROBE_OFFLINE").is_some()
}

/// Non-empty value of an environment variable; only presence/emptiness is
/// ever observed, matching the module's no-secrets rule.
fn env_present(name: &str) -> bool {
    std::env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

/// Read a JSON file into a `serde_json::Value`, tolerating absence and
/// malformed content alike.
fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Parse `key = "value"` pairs from TOML-ish content. Intentionally
/// hand-rolled rather than a full TOML parse: the probe only wants a couple
/// of flat string keys (`model`, `model_provider`, `provider`), and pulling
/// a document model in here would widen this module's failure surface.
/// Section headers are ignored; the first occurrence of a key wins.
fn parse_toml_pairs(content: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // Strip an inline `# comment` and surrounding quotes, best-effort.
        let value = value
            .split('#')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string();
        let key = key.trim().trim_matches('"').to_string();
        if !key.is_empty() && !value.is_empty() {
            pairs.push((key, value));
        }
    }
    pairs
}

/// True when the TOML-ish content declares at least one `[section]` header —
/// for kimi-code, a provider section in the config file is the "configured"
/// signal the adapter reports as auth evidence.
fn toml_has_section(content: &str) -> bool {
    content
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with('[') && line.ends_with(']'))
}

/// Scan a JSON `env` object (as found in Claude Code's settings.json) for
/// well-known API-key variable *names*. Only the keys are inspected — the
/// values are secret material and are never read.
fn providers_from_env_object(env: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    env.keys()
        .filter_map(|key| {
            ENV_KEY_PROVIDERS
                .iter()
                .find(|(var, _)| var == key)
                .map(|(_, provider)| (*provider).to_string())
        })
        .collect()
}

/// Providers configured in dreamseq's own `~/.config/dreamseq/config.json`:
/// entries that are enabled and carry a non-empty `api_key`. The file is
/// read as untyped JSON so a config written by an older/newer CLI version
/// still contributes what it can; a missing file contributes nothing.
fn configured_from_dreamseq_config(home: &Path) -> Vec<String> {
    let path = home.join(".config").join("dreamseq").join("config.json");
    let Some(config) = read_json(&path) else {
        return Vec::new();
    };
    config
        .get("providers")
        .and_then(|providers| providers.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("enabled")
                        .and_then(|enabled| enabled.as_bool())
                        .unwrap_or(false)
                        && entry
                            .get("api_key")
                            .and_then(|key| key.as_str())
                            .map(|key| !key.trim().is_empty())
                            .unwrap_or(false)
                })
                .filter_map(|entry| entry.get("name").and_then(|name| name.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Claude Code adapter: settings.json for provider/model, a credentials file
/// for the subscribed signal, and `ANTHROPIC_API_KEY` presence for configured.
fn probe_claude(home: &Path, evidence: &mut AdapterEvidence) {
    if let Some(settings) = read_json(&home.join(".claude").join("settings.json")) {
        if let Some(model) = settings.get("model").and_then(|m| m.as_str())
            && !model.trim().is_empty()
        {
            evidence.model = Some(model.trim().to_string());
            if evidence.provider.is_none() {
                evidence.provider = Some("anthropic".to_string());
            }
        }
        // The `env` block in settings.json carries provider key material;
        // only the variable names are evidence.
        if let Some(env) = settings.get("env").and_then(|e| e.as_object()) {
            for provider in providers_from_env_object(env) {
                evidence.note_configured(&provider);
            }
        }
    }
    if env_present("ANTHROPIC_API_KEY") {
        evidence.note_configured("anthropic");
    }
    if home.join(".claude").join(".credentials.json").is_file() {
        evidence.note_subscribed("anthropic");
    }
}

/// Codex adapter: `~/.codex/config.json` first, then `config.toml`, for the
/// configured `model`/`model_provider`; `auth.json` existence marks the
/// configured provider (openai when none is configured) as subscribed.
fn probe_codex(home: &Path, evidence: &mut AdapterEvidence) {
    let dir = home.join(".codex");
    if let Some(config) = read_json(&dir.join("config.json")) {
        if let Some(model) = config.get("model").and_then(|m| m.as_str())
            && !model.trim().is_empty()
        {
            evidence.model = Some(model.trim().to_string());
        }
        if let Some(provider) = config.get("model_provider").and_then(|p| p.as_str())
            && !provider.trim().is_empty()
        {
            evidence.provider = Some(provider.trim().to_lowercase());
        }
    }
    // Only consult config.toml when config.json had nothing to say, matching
    // codex's own precedence (the JSON config overrides the TOML one).
    if evidence.model.is_none()
        && evidence.provider.is_none()
        && let Ok(content) = std::fs::read_to_string(dir.join("config.toml"))
    {
        for (key, value) in parse_toml_pairs(&content) {
            match key.as_str() {
                "model" if evidence.model.is_none() => evidence.model = Some(value),
                "model_provider" if evidence.provider.is_none() => {
                    evidence.provider = Some(value.to_lowercase());
                }
                _ => {}
            }
        }
    }
    if evidence.provider.is_none() && evidence.model.is_some() {
        evidence.provider = Some("openai".to_string());
    }
    if dir.join("auth.json").is_file() {
        let provider = evidence
            .provider
            .clone()
            .unwrap_or_else(|| "openai".to_string());
        evidence.note_subscribed(&provider);
    }
}

/// Kimi Code adapter: `~/.kimi-code/config.toml` holds a provider section
/// with `provider` and `model` keys. Any provider section at all counts as
/// auth configured, per the harness's config layout.
fn probe_kimi(home: &Path, evidence: &mut AdapterEvidence) {
    let Ok(content) = std::fs::read_to_string(home.join(".kimi-code").join("config.toml")) else {
        return;
    };
    for (key, value) in parse_toml_pairs(&content) {
        match key.as_str() {
            "model" if evidence.model.is_none() => evidence.model = Some(value),
            "provider" if evidence.provider.is_none() => {
                evidence.provider = Some(value.to_lowercase());
            }
            _ => {}
        }
    }
    if evidence.provider.is_none() && evidence.model.is_some() {
        evidence.provider = Some("moonshot".to_string());
    }
    if toml_has_section(&content) {
        evidence.note_configured(
            &evidence
                .provider
                .clone()
                .unwrap_or_else(|| "moonshot".to_string()),
        );
    }
}

/// Gemini adapter: no local config file worth parsing; the API key env vars
/// are the whole configuration story.
fn probe_gemini(evidence: &mut AdapterEvidence) {
    if env_present("GEMINI_API_KEY") || env_present("GOOGLE_API_KEY") {
        evidence.provider = Some("google".to_string());
        evidence.note_configured("google");
    }
}

/// Generic fallback, run for every harness: well-known API-key env vars imply
/// the corresponding provider is configured for whatever harness is asking.
fn probe_env_keys(evidence: &mut AdapterEvidence) {
    for (var, provider) in ENV_KEY_PROVIDERS {
        if env_present(var) {
            evidence.note_configured(provider);
        }
    }
}

/// Probe one harness for capability evidence. `env_home` overrides the home
/// directory the adapters read fixtures from (tests inject a tempdir;
/// production passes `None` to use the real home). `connector` is the
/// reachability seam: tests inject a stub, production passes
/// [`tcp_connector`]. This function never fails — at worst it returns a
/// probe with `status: "detected"` and empty lists.
pub fn probe_harness(
    name: &str,
    env_home: Option<&Path>,
    connector: &dyn Fn(&str) -> bool,
) -> HarnessProbe {
    let home = env_home
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut evidence = AdapterEvidence::default();
    match name {
        "claude" => probe_claude(&home, &mut evidence),
        "codex" => probe_codex(&home, &mut evidence),
        "kimi" => probe_kimi(&home, &mut evidence),
        "gemini" => probe_gemini(&mut evidence),
        _ => {}
    }
    probe_env_keys(&mut evidence);
    for provider in configured_from_dreamseq_config(&home) {
        evidence.note_configured(&provider);
    }

    let configured = normalize_providers(evidence.configured.iter().cloned());
    let subscribed = normalize_providers(evidence.subscribed.iter().cloned());
    let supported = normalize_providers(supported_providers(name));

    // Connectivity: `available` is `configured ∩ reachable`. In offline mode
    // no packets are sent at all and `available` degrades to `configured`.
    let (connectivity, available) = if offline_mode() {
        (None, configured.clone())
    } else {
        let reachable: Vec<String> = configured
            .iter()
            .filter(|provider| connector(provider))
            .cloned()
            .collect();
        let any_configured = !configured.is_empty();
        let any_reachable = !reachable.is_empty();
        (any_configured.then_some(any_reachable), reachable)
    };

    let status = if !configured.is_empty()
        || evidence.provider.is_some()
        || evidence.model.is_some()
        || evidence.auth.is_some()
    {
        "configured"
    } else {
        "detected"
    };

    HarnessProbe {
        status: status.to_string(),
        provider: evidence.provider,
        model: evidence.model,
        auth: evidence.auth,
        connectivity,
        providers: ProviderMatrix {
            supported,
            configured,
            subscribed,
            available,
        },
    }
}

/// Display name for a harness in human output: the registry name uppercased,
/// except the handful of harnesses whose marketing name differs from their
/// binary name ("claude" is "Claude Code", "kimi" is "Kimi").
fn display_name(name: &str) -> String {
    match name {
        "claude" => "CLAUDE CODE".to_string(),
        other => other.to_uppercase(),
    }
}

/// One row of the `dreamseq stack probe` output: the harness identity (name,
/// version, install path) plus its capability probe.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeEntry {
    pub harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub probe: HarnessProbe,
}

impl ProbeEntry {
    fn from_harness(info: &crate::stack_report::HarnessInfo) -> Self {
        Self {
            harness: info.name.clone(),
            version: info.version.clone(),
            path: info.origin.clone(),
            probe: info.probe.clone().unwrap_or_else(|| HarnessProbe {
                status: "detected".to_string(),
                provider: None,
                model: None,
                auth: None,
                connectivity: None,
                providers: ProviderMatrix::default(),
            }),
        }
    }
}

/// Render one probe entry for humans: display name, version, path, a status
/// line, the provider matrix as comma lists, and the active model.
fn print_probe_entry(entry: &ProbeEntry) {
    let mut heading = display_name(&entry.harness).bold();
    if let Some(version) = &entry.version {
        heading = format!("{heading} {}", version.dimmed());
    }
    if let Some(path) = &entry.path {
        heading = format!("{heading}  {}", path.dimmed());
    }
    println!("{heading}");
    println!("  status: {}", entry.probe.status);
    if let Some(auth) = &entry.probe.auth {
        println!("  auth: {auth}");
    }
    match entry.probe.connectivity {
        Some(true) => println!("  connectivity: ok"),
        Some(false) => println!("  connectivity: unreachable"),
        None => {}
    }
    if let Some(provider) = &entry.probe.provider {
        println!("  provider: {provider}");
    }
    println!("  providers:");
    for (label, values) in [
        ("supported", &entry.probe.providers.supported),
        ("configured", &entry.probe.providers.configured),
        ("subscribed", &entry.probe.providers.subscribed),
        ("available", &entry.probe.providers.available),
    ] {
        let rendered = if values.is_empty() {
            "(none)".dimmed()
        } else {
            values.join(", ").white()
        };
        println!("    {label:<11}{rendered}");
    }
    if let Some(model) = &entry.probe.model {
        println!("  models:");
        println!("    active: {model}");
    }
}

/// Implementation of `dreamseq stack probe [harness]`: probe the named
/// harness, or every detected harness when omitted. `--json` emits one
/// [`ProbeEntry`] per harness (an array when probing all); the default human
/// layout prints the provider matrix and active model per harness. Never
/// fails on probe problems — an unknown harness name still yields a probe,
/// just one with little evidence behind it.
pub async fn run_probe_command(harness: Option<String>, as_json: bool) -> Result<()> {
    let detected = crate::stack_report::detect_harnesses().await;
    let entries: Vec<ProbeEntry> = match &harness {
        Some(name) => {
            match detected
                .iter()
                .find(|info| info.name.eq_ignore_ascii_case(name))
            {
                Some(info) => vec![ProbeEntry::from_harness(info)],
                None => {
                    // Not installed (or not in the registry): still probe, so
                    // env/config evidence for ad-hoc harness names shows up.
                    vec![ProbeEntry {
                        harness: name.clone(),
                        version: None,
                        path: None,
                        probe: probe_harness(name, None, &tcp_connector),
                    }]
                }
            }
        }
        None => {
            if detected.is_empty() {
                println!(
                    "{} No agent harnesses detected on this device.",
                    "⚠️".yellow()
                );
            }
            detected.iter().map(ProbeEntry::from_harness).collect()
        }
    };

    if as_json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                println!();
            }
            print_probe_entry(entry);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serializes the env-mutating tests below; std::env is process-global and
    // cargo runs unit tests in this binary on multiple threads by default.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Every env var any probe reads, so tests can clear and restore the full
    /// set under a lock (a real key exported in the host environment must not
    /// leak into an assertion that never asked for it).
    const PROBE_ENV_VARS: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "GROQ_API_KEY",
        "MISTRAL_API_KEY",
        "MOONSHOT_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "DEEPSEEK_API_KEY",
        "CEREBRAS_API_KEY",
        "XAI_API_KEY",
        "ZAI_API_KEY",
        "DREAMSEQ_STACK_PROBE_OFFLINE",
    ];

    /// Run `body` with the probe-relevant env vars cleared, restoring the
    /// previous values afterwards. Recover rather than propagate on a
    /// poisoned lock: an earlier test panicking mid-body must not block the
    /// rest of the suite.
    fn with_clean_env<T>(body: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous: Vec<(&str, Option<String>)> = PROBE_ENV_VARS
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect();
        for name in PROBE_ENV_VARS {
            unsafe { std::env::remove_var(name) };
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        for (name, value) in previous {
            match value {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// A fresh tempdir acting as a fake $HOME; removed on drop of the test.
    fn fixture_home(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dreamseq-probe-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A connector stub: providers in `reachable` connect, everything else
    /// fails. Never touches the network.
    fn stub_connector<'a>(reachable: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |provider| reachable.iter().any(|entry| *entry == provider)
    }

    #[test]
    fn claude_adapter_reads_settings_model_and_credentials() {
        with_clean_env(|| {
            let home = fixture_home("claude");
            std::fs::create_dir_all(home.join(".claude")).unwrap();
            std::fs::write(
                home.join(".claude/settings.json"),
                r#"{"model": "claude-sonnet-4-5", "env": {"ANTHROPIC_API_KEY": "sk-secret-never-read"}}"#,
            )
            .unwrap();
            let probe = probe_harness("claude", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.status, "configured");
            assert_eq!(probe.model.as_deref(), Some("claude-sonnet-4-5"));
            assert_eq!(probe.provider.as_deref(), Some("anthropic"));
            assert_eq!(probe.auth.as_deref(), Some("configured"));
            assert!(
                probe
                    .providers
                    .configured
                    .contains(&"anthropic".to_string())
            );
            assert!(
                probe
                    .providers
                    .supported
                    .contains(&"openrouter".to_string())
            );

            // A stored credentials file upgrades anthropic to subscribed.
            std::fs::write(home.join(".claude/.credentials.json"), "{}").unwrap();
            let probe = probe_harness("claude", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.auth.as_deref(), Some("subscribed"));
            assert!(
                probe
                    .providers
                    .subscribed
                    .contains(&"anthropic".to_string())
            );
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn claude_adapter_tolerates_missing_and_malformed_settings() {
        with_clean_env(|| {
            let home = fixture_home("claude-broken");
            let probe = probe_harness("claude", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.status, "detected");
            assert!(probe.model.is_none());
            std::fs::write(home.join(".claude"), "not a directory").unwrap();
            let probe = probe_harness("claude", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.status, "detected");
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn codex_adapter_reads_json_then_toml_and_marks_auth_subscribed() {
        with_clean_env(|| {
            let home = fixture_home("codex");
            std::fs::create_dir_all(home.join(".codex")).unwrap();
            // TOML only: model + model_provider, hand-rolled parse.
            std::fs::write(
                home.join(".codex/config.toml"),
                "[model_providers.oss]\nmodel_provider = \"azure\"\nmodel = \"gpt-5-codex\"\n",
            )
            .unwrap();
            let probe = probe_harness("codex", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.model.as_deref(), Some("gpt-5-codex"));
            assert_eq!(probe.provider.as_deref(), Some("azure"));
            assert_eq!(probe.status, "configured");

            // auth.json marks the configured provider subscribed.
            std::fs::write(home.join(".codex/auth.json"), "{}").unwrap();
            let probe = probe_harness("codex", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.auth.as_deref(), Some("subscribed"));
            assert!(probe.providers.subscribed.contains(&"azure".to_string()));

            // config.json wins over config.toml when both exist.
            std::fs::write(
                home.join(".codex/config.json"),
                r#"{"model": "o3", "model_provider": "openai"}"#,
            )
            .unwrap();
            let probe = probe_harness("codex", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.model.as_deref(), Some("o3"));
            assert_eq!(probe.provider.as_deref(), Some("openai"));
            assert!(probe.providers.subscribed.contains(&"openai".to_string()));
            assert!(!probe.providers.subscribed.contains(&"azure".to_string()));
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn codex_adapter_falls_back_to_openai_when_only_a_model_is_set() {
        with_clean_env(|| {
            let home = fixture_home("codex-openai");
            std::fs::create_dir_all(home.join(".codex")).unwrap();
            std::fs::write(home.join(".codex/config.toml"), "model = \"gpt-5\"\n").unwrap();
            std::fs::write(home.join(".codex/auth.json"), "{}").unwrap();
            let probe = probe_harness("codex", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.provider.as_deref(), Some("openai"));
            assert!(probe.providers.subscribed.contains(&"openai".to_string()));
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn kimi_adapter_reads_provider_section() {
        with_clean_env(|| {
            let home = fixture_home("kimi");
            std::fs::create_dir_all(home.join(".kimi-code")).unwrap();
            std::fs::write(
                home.join(".kimi-code/config.toml"),
                "[providers.moonshot]\nprovider = \"moonshot\"\nmodel = \"kimi-k2-0711-preview\"\nbase_url = \"https://api.moonshot.ai\"\n",
            )
            .unwrap();
            let probe = probe_harness("kimi", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.status, "configured");
            assert_eq!(probe.provider.as_deref(), Some("moonshot"));
            assert_eq!(probe.model.as_deref(), Some("kimi-k2-0711-preview"));
            assert_eq!(probe.auth.as_deref(), Some("configured"));
            assert!(probe.providers.configured.contains(&"moonshot".to_string()));
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn gemini_adapter_uses_key_env_presence() {
        with_clean_env(|| {
            let home = fixture_home("gemini");
            unsafe { std::env::set_var("GEMINI_API_KEY", "AIza-not-a-real-key") };
            let probe = probe_harness("gemini", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.status, "configured");
            assert_eq!(probe.provider.as_deref(), Some("google"));
            assert!(probe.providers.configured.contains(&"google".to_string()));
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn env_key_matrix_maps_well_known_vars_to_providers() {
        with_clean_env(|| {
            let home = fixture_home("envkeys");
            unsafe {
                std::env::set_var("OPENAI_API_KEY", "x");
                std::env::set_var("ANTHROPIC_API_KEY", "x");
                std::env::set_var("OPENROUTER_API_KEY", "x");
                std::env::set_var("GROQ_API_KEY", "x");
                std::env::set_var("MISTRAL_API_KEY", "x");
                std::env::set_var("MOONSHOT_API_KEY", "x");
                std::env::set_var("GOOGLE_API_KEY", "x");
                std::env::set_var("DEEPSEEK_API_KEY", "x");
                std::env::set_var("CEREBRAS_API_KEY", "x");
                std::env::set_var("XAI_API_KEY", "x");
                std::env::set_var("ZAI_API_KEY", "x");
            }
            // An unknown harness name exercises the pure generic fallback.
            let probe = probe_harness("kilocode", Some(&home), &stub_connector(&[]));
            let expected = [
                "openai",
                "anthropic",
                "openrouter",
                "groq",
                "mistral",
                "moonshot",
                "google",
                "deepseek",
                "cerebras",
                "grok",
                "zai",
            ];
            for provider in expected {
                assert!(
                    probe.providers.configured.contains(&provider.to_string()),
                    "missing {provider}"
                );
            }
            assert_eq!(probe.providers.configured.len(), expected.len());
            assert_eq!(probe.status, "configured");
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn dreamseq_config_providers_with_keys_count_as_configured() {
        with_clean_env(|| {
            let home = fixture_home("dreamseq-config");
            let config_dir = home.join(".config/dreamseq");
            std::fs::create_dir_all(&config_dir).unwrap();
            std::fs::write(
                config_dir.join("config.json"),
                r#"{"providers": [
                    {"name": "openrouter", "enabled": true, "base_url": "https://openrouter.ai/api/v1", "model": "m", "api_key": "sk-live-key"},
                    {"name": "openai", "enabled": false, "base_url": "https://api.openai.com/v1", "model": "m", "api_key": "sk-live-key"},
                    {"name": "mistral", "enabled": true, "base_url": "https://api.mistral.ai/v1", "model": "m", "api_key": ""}
                ]}"#,
            )
            .unwrap();
            let probe = probe_harness("aider", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.providers.configured, vec!["openrouter".to_string()]);
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn available_is_configured_intersect_reachable() {
        with_clean_env(|| {
            let home = fixture_home("connectivity");
            let reachable = ["anthropic", "openrouter"];
            let connector = stub_connector(&reachable);
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", "x") };
            unsafe { std::env::set_var("OPENROUTER_API_KEY", "x") };
            unsafe { std::env::set_var("MISTRAL_API_KEY", "x") };
            let probe = probe_harness("aider", Some(&home), &connector);
            assert_eq!(
                probe.providers.configured,
                vec![
                    "anthropic".to_string(),
                    "mistral".to_string(),
                    "openrouter".to_string()
                ]
            );
            assert_eq!(
                probe.providers.available,
                vec!["anthropic".to_string(), "openrouter".to_string()]
            );
            assert_eq!(probe.connectivity, Some(true));

            // Nothing reachable: connectivity false, available empty.
            let probe = probe_harness("aider", Some(&home), &stub_connector(&[]));
            assert!(probe.providers.available.is_empty());
            assert_eq!(probe.connectivity, Some(false));
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn offline_mode_skips_connectivity_and_keeps_configured_as_available() {
        with_clean_env(|| {
            let home = fixture_home("offline");
            // Even a connector that claims everything is reachable must not
            // be consulted in offline mode.
            let connector = |_: &str| true;
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", "x") };
            unsafe { std::env::set_var("DREAMSEQ_STACK_PROBE_OFFLINE", "1") };
            let probe = probe_harness("claude", Some(&home), &connector);
            assert_eq!(probe.connectivity, None);
            assert_eq!(probe.providers.available, probe.providers.configured);
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn provider_lists_are_lowercased_deduped_and_capped() {
        let values: Vec<String> = (0..32).map(|i| format!("Provider-{i}")).collect();
        let normalized = normalize_providers(values);
        assert_eq!(normalized.len(), PROVIDER_LIST_CAP);

        let mixed = normalize_providers([
            "Anthropic".to_string(),
            "anthropic".to_string(),
            " ANTHROPIC ".to_string(),
            "OpenAI".to_string(),
            "".to_string(),
            "  ".to_string(),
        ]);
        assert_eq!(mixed, vec!["anthropic".to_string(), "openai".to_string()]);
    }

    #[test]
    fn unknown_harness_without_evidence_reports_detected() {
        with_clean_env(|| {
            let home = fixture_home("bare");
            let probe = probe_harness("windsurf", Some(&home), &stub_connector(&[]));
            assert_eq!(probe.status, "detected");
            assert!(probe.provider.is_none());
            assert!(probe.model.is_none());
            assert!(probe.auth.is_none());
            assert!(probe.providers.supported.is_empty());
            std::fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn parse_toml_pairs_handles_quotes_comments_and_sections() {
        let pairs = parse_toml_pairs(
            "# comment\n[section]\nmodel = \"gpt-5\" # trailing\nprovider = 'moonshot'\nkey_only\n= empty\nempty =\n",
        );
        assert!(pairs.contains(&("model".to_string(), "gpt-5".to_string())));
        assert!(pairs.contains(&("provider".to_string(), "moonshot".to_string())));
        assert!(!pairs.iter().any(|(key, _)| key == "empty"));
        assert!(toml_has_section("[section]\nmodel = \"x\""));
        assert!(!toml_has_section("model = \"x\""));
    }

    #[test]
    fn old_payloads_without_probe_fields_still_round_trip() {
        // Harness payloads written before probes existed carry no `probe`
        // field; the struct must still deserialize them and re-serialize
        // without inventing one.
        let legacy = r#"{"name": "claude", "version": "1.0.0", "origin": "/usr/bin/claude"}"#;
        let info: crate::stack_report::HarnessInfo = serde_json::from_str(legacy).unwrap();
        assert!(info.probe.is_none());
        let reserialized = serde_json::to_value(&info).unwrap();
        assert!(reserialized.get("probe").is_none());
        assert_eq!(reserialized["name"], "claude");

        // Same contract for the probe's own matrix: empty lists survive a
        // round trip and lists never exceed the cap in JSON.
        let probe = HarnessProbe {
            status: "configured".to_string(),
            provider: Some("anthropic".to_string()),
            model: None,
            auth: None,
            connectivity: Some(true),
            providers: ProviderMatrix {
                supported: vec!["anthropic".to_string()],
                configured: vec!["anthropic".to_string()],
                subscribed: vec![],
                available: vec!["anthropic".to_string()],
            },
        };
        let json = serde_json::to_string(&probe).unwrap();
        assert!(!json.contains("\"model\""));
        assert!(!json.contains("\"auth\""));
        let parsed: HarnessProbe = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, probe);
    }
}
