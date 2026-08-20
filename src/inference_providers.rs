// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
//! Builds the list of BYOK routes a `GroqClient` will use: explicit
//! `DREAMSEQ_BYOK_ROUTES` JSON, the single-route `DREAMSEQ_BYOK_*` shorthand,
//! and auto-detected named providers (OpenAI, Anthropic, Gemini, Grok,
//! z.ai, Mistral, DeepSeek, OpenRouter, Cerebras, and local Ollama/LM
//! Studio/vLLM) that activate the moment their API key appears in the
//! environment — mirroring the legacy `GROQ_API_KEY` auto-detection.

use super::{InferenceRoute, Protocol};
use anyhow::Result;

#[derive(Debug, serde::Deserialize)]
struct ByokRouteConfig {
    name: String,
    base_url: String,
    model: String,
    api_key_env: String,
    #[serde(default)]
    light_model: Option<String>,
    #[serde(default)]
    protocol: Protocol,
}

pub(super) fn configured_byok_routes(legacy_groq_key: &str) -> Result<Vec<InferenceRoute>> {
    let mut routes = Vec::new();
    if let Ok(encoded) = std::env::var("DREAMSEQ_BYOK_ROUTES")
        && !encoded.trim().is_empty()
    {
        let configured: Vec<ByokRouteConfig> = serde_json::from_str(&encoded)
            .map_err(|error| anyhow::anyhow!("DREAMSEQ_BYOK_ROUTES is invalid JSON: {error}"))?;
        for route in configured {
            validate_provider_url(&route.base_url)?;
            match std::env::var(&route.api_key_env) {
                Ok(api_key) if !api_key.trim().is_empty() => routes.push(InferenceRoute {
                    name: route.name,
                    base_url: route.base_url.trim_end_matches('/').to_string(),
                    model: route.model,
                    api_key,
                    light_model: route.light_model,
                    protocol: route.protocol,
                }),
                _ => tracing::warn!(
                    provider = %route.name,
                    key_environment = %route.api_key_env,
                    "skipping BYOK route because its key is unavailable"
                ),
            }
        }
    }

    let generic_key = std::env::var("DREAMSEQ_BYOK_API_KEY").unwrap_or_default();
    let generic_url = std::env::var("DREAMSEQ_BYOK_BASE_URL").unwrap_or_default();
    let generic_model = std::env::var("DREAMSEQ_BYOK_MODEL").unwrap_or_default();
    if !generic_key.trim().is_empty()
        || !generic_url.trim().is_empty()
        || !generic_model.trim().is_empty()
    {
        if generic_key.trim().is_empty()
            || generic_url.trim().is_empty()
            || generic_model.trim().is_empty()
        {
            anyhow::bail!(
                "DREAMSEQ_BYOK_API_KEY, DREAMSEQ_BYOK_BASE_URL, and DREAMSEQ_BYOK_MODEL must be set together"
            );
        }
        validate_provider_url(&generic_url)?;
        let generic_light_model = std::env::var("DREAMSEQ_BYOK_MODEL_LIGHT")
            // traci: allow -- absence of this optional environment override is expected control flow.
            .ok()
            .filter(|value| !value.trim().is_empty());
        let generic_protocol = match std::env::var("DREAMSEQ_BYOK_PROTOCOL") {
            // traci: allow -- absence of this optional environment override is expected control flow; it defaults to OpenAI-compatible below.
            Ok(value) if value.eq_ignore_ascii_case("anthropic") => Protocol::Anthropic,
            _ => Protocol::OpenAiCompatible,
        };
        routes.push(InferenceRoute {
            name: "byok".to_string(),
            base_url: generic_url.trim_end_matches('/').to_string(),
            model: generic_model,
            api_key: generic_key,
            light_model: generic_light_model,
            protocol: generic_protocol,
        });
    }

    add_named_provider_routes(&mut routes);

    if !legacy_groq_key.trim().is_empty()
        && !routes
            .iter()
            .any(|route| route.base_url == "https://api.groq.com/openai/v1")
    {
        routes.push(InferenceRoute {
            name: "groq".to_string(),
            base_url: "https://api.groq.com/openai/v1".to_string(),
            model: std::env::var("GROQ_MODEL")
                .unwrap_or_else(|_| "openai/gpt-oss-120b".to_string()),
            api_key: legacy_groq_key.to_string(),
            // Same variable name and default as dreamsequence-api's server-side
            // tier routing, so the two stay in sync without separate config.
            light_model: Some(
                std::env::var("GROQ_MODEL_LIGHT")
                    .unwrap_or_else(|_| "openai/gpt-oss-20b".to_string()),
            )
            .filter(|value| !value.trim().is_empty()),
            protocol: Protocol::OpenAiCompatible,
        });
    }
    Ok(routes)
}

/// Well-known providers that Dreamseq will use automatically when their API
/// key is present in the environment, mirroring the legacy `GROQ_API_KEY`
/// auto-detection. Each becomes one more route the load balancer can
/// round-robin across (see [`super::inference_health::RouteHealth`]); none
/// is required, and any of them can be overridden or replaced with
/// `DREAMSEQ_BYOK_ROUTES`.
struct NamedProvider {
    name: &'static str,
    base_url: &'static str,
    default_model: &'static str,
    api_key_env: &'static [&'static str],
    model_env: &'static str,
    protocol: Protocol,
}

const NAMED_PROVIDERS: &[NamedProvider] = &[
    NamedProvider {
        name: "openai",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-5-mini",
        api_key_env: &["OPENAI_API_KEY"],
        model_env: "OPENAI_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "anthropic",
        // Anthropic's Messages API is not OpenAI-shaped: `x-api-key` instead
        // of a bearer token, `/v1/messages` instead of `/chat/completions`,
        // and a typed content-block response. See request_anthropic.
        base_url: "https://api.anthropic.com",
        default_model: "claude-sonnet-4-5",
        api_key_env: &["ANTHROPIC_API_KEY"],
        model_env: "ANTHROPIC_MODEL",
        protocol: Protocol::Anthropic,
    },
    NamedProvider {
        name: "gemini",
        // Google's OpenAI-compatibility layer: https://ai.google.dev/gemini-api/docs/openai
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        default_model: "gemini-2.5-flash",
        api_key_env: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        model_env: "GEMINI_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "grok",
        base_url: "https://api.x.ai/v1",
        default_model: "grok-4-fast",
        api_key_env: &["XAI_API_KEY", "GROK_API_KEY"],
        model_env: "XAI_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "zai",
        // Zhipu/z.ai's OpenAI-compatible endpoint for the GLM model family.
        base_url: "https://api.z.ai/api/paas/v4",
        default_model: "glm-5.2",
        api_key_env: &["ZAI_API_KEY", "GLM_API_KEY"],
        model_env: "ZAI_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "mistral",
        base_url: "https://api.mistral.ai/v1",
        default_model: "mistral-large-latest",
        api_key_env: &["MISTRAL_API_KEY"],
        model_env: "MISTRAL_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "deepseek",
        base_url: "https://api.deepseek.com",
        default_model: "deepseek-chat",
        api_key_env: &["DEEPSEEK_API_KEY"],
        model_env: "DEEPSEEK_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "openrouter",
        // A meta-router across dozens of upstream backends in its own
        // right, so this one route buys several providers' worth of
        // redundancy for a single integration.
        base_url: "https://openrouter.ai/api/v1",
        default_model: "openai/gpt-4o-mini",
        api_key_env: &["OPENROUTER_API_KEY"],
        model_env: "OPENROUTER_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "cerebras",
        base_url: "https://api.cerebras.ai/v1",
        default_model: "llama3.1-8b",
        api_key_env: &["CEREBRAS_API_KEY"],
        model_env: "CEREBRAS_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "ollama",
        // Local, OpenAI-compatible, no cloud egress at all. Opt-in requires
        // a non-empty OLLAMA_API_KEY even though a local Ollama server
        // ignores its value — consistent with every other route in this
        // table being enabled by an explicit non-empty environment variable,
        // so dreamseq never silently starts talking to a local model server
        // a user happens to have running for something unrelated.
        base_url: "http://localhost:11434/v1",
        default_model: "llama3.1",
        api_key_env: &["OLLAMA_API_KEY"],
        model_env: "OLLAMA_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "lmstudio",
        base_url: "http://localhost:1234/v1",
        default_model: "local-model",
        api_key_env: &["LMSTUDIO_API_KEY"],
        model_env: "LMSTUDIO_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
    NamedProvider {
        name: "vllm",
        base_url: "http://localhost:8000/v1",
        default_model: "local-model",
        api_key_env: &["VLLM_API_KEY"],
        model_env: "VLLM_MODEL",
        protocol: Protocol::OpenAiCompatible,
    },
];

fn first_nonempty_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            // traci: allow -- most candidate key variables are unset for any given user; that's expected control flow, not a failure.
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

fn add_named_provider_routes(routes: &mut Vec<InferenceRoute>) {
    for provider in NAMED_PROVIDERS {
        if routes
            .iter()
            .any(|route| route.base_url == provider.base_url)
        {
            continue;
        }
        let Some(api_key) = first_nonempty_env(provider.api_key_env) else {
            continue;
        };
        let model = std::env::var(provider.model_env)
            // traci: allow -- absence of this optional model override is expected control flow.
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| provider.default_model.to_string());
        tracing::info!(provider = provider.name, model = %model, "detected BYOK route from environment");
        routes.push(InferenceRoute {
            name: provider.name.to_string(),
            base_url: provider.base_url.to_string(),
            model,
            api_key,
            // No known light-tier variant for these providers by default;
            // DREAMSEQ_BYOK_ROUTES can configure one explicitly if desired.
            light_model: None,
            protocol: provider.protocol,
        });
    }
}

fn validate_provider_url(url: &str) -> Result<()> {
    let local = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost");
    if !url.starts_with("https://") && !local {
        anyhow::bail!("BYOK inference endpoints must use HTTPS (localhost is allowed for tests)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serializes the env-mutating tests below; std::env is process-global and
    // cargo runs unit tests in this binary on multiple threads by default.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<T>(pairs: &[(&str, Option<&str>)], body: impl FnOnce() -> T) -> T {
        // Recover rather than propagate: an earlier test panicking mid-body
        // (see the catch_unwind below) must not poison every later test.
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Start every test from a clean slate covering every env var any
        // NAMED_PROVIDERS entry reads, not just the ones this call names —
        // a real key already exported in the host environment for unrelated
        // tooling (observed live: a real OPENROUTER_API_KEY) must not leak
        // into a test that never asked for it.
        let mut all_vars: Vec<&str> = NAMED_PROVIDERS
            .iter()
            .flat_map(|provider| {
                provider
                    .api_key_env
                    .iter()
                    .copied()
                    .chain(std::iter::once(provider.model_env))
            })
            .collect();
        for (name, _) in pairs {
            if !all_vars.contains(name) {
                all_vars.push(name);
            }
        }

        let previous: Vec<(&str, Option<String>)> = all_vars
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect();
        for name in &all_vars {
            unsafe { std::env::remove_var(name) };
        }
        for (name, value) in pairs {
            if let Some(value) = value {
                unsafe { std::env::set_var(name, value) };
            }
        }

        // A panicking assertion inside `body` must not skip restoration —
        // that's exactly what poisoned the lock above for every subsequent
        // test the first time this happened.
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

    #[test]
    fn detects_openai_route_from_environment() {
        with_env(
            &[
                ("OPENAI_API_KEY", Some("test-openai-key")),
                ("OPENAI_MODEL", None),
            ],
            || {
                let mut routes = Vec::new();
                add_named_provider_routes(&mut routes);
                let route = routes
                    .iter()
                    .find(|route| route.name == "openai")
                    .expect("openai route should be detected");
                assert_eq!(route.base_url, "https://api.openai.com/v1");
                assert_eq!(route.model, "gpt-5-mini");
                assert_eq!(route.api_key, "test-openai-key");
            },
        );
    }

    #[test]
    fn detects_the_original_four_openai_compatible_providers_independently() {
        with_env(
            &[
                ("OPENAI_API_KEY", Some("k-openai")),
                ("GEMINI_API_KEY", Some("k-gemini")),
                ("GOOGLE_API_KEY", None),
                ("XAI_API_KEY", Some("k-xai")),
                ("GROK_API_KEY", None),
                ("ZAI_API_KEY", Some("k-zai")),
                ("GLM_API_KEY", None),
            ],
            || {
                let mut routes = Vec::new();
                add_named_provider_routes(&mut routes);
                let names: Vec<&str> = routes.iter().map(|route| route.name.as_str()).collect();
                assert_eq!(names, vec!["openai", "gemini", "grok", "zai"]);
            },
        );
    }

    #[test]
    fn detects_every_named_provider_when_all_keys_are_present() {
        with_env(
            &[
                ("OPENAI_API_KEY", Some("k")),
                ("ANTHROPIC_API_KEY", Some("k")),
                ("GEMINI_API_KEY", Some("k")),
                ("XAI_API_KEY", Some("k")),
                ("ZAI_API_KEY", Some("k")),
                ("MISTRAL_API_KEY", Some("k")),
                ("DEEPSEEK_API_KEY", Some("k")),
                ("OPENROUTER_API_KEY", Some("k")),
                ("CEREBRAS_API_KEY", Some("k")),
                ("OLLAMA_API_KEY", Some("k")),
                ("LMSTUDIO_API_KEY", Some("k")),
                ("VLLM_API_KEY", Some("k")),
            ],
            || {
                let mut routes = Vec::new();
                add_named_provider_routes(&mut routes);
                let names: Vec<&str> = routes.iter().map(|route| route.name.as_str()).collect();
                assert_eq!(
                    names,
                    vec![
                        "openai",
                        "anthropic",
                        "gemini",
                        "grok",
                        "zai",
                        "mistral",
                        "deepseek",
                        "openrouter",
                        "cerebras",
                        "ollama",
                        "lmstudio",
                        "vllm"
                    ]
                );
            },
        );
    }

    #[test]
    fn anthropic_route_gets_the_anthropic_protocol_not_openai_compatible() {
        with_env(&[("ANTHROPIC_API_KEY", Some("k-anthropic"))], || {
            let mut routes = Vec::new();
            add_named_provider_routes(&mut routes);
            let anthropic = routes
                .iter()
                .find(|route| route.name == "anthropic")
                .expect("anthropic route should be detected");
            assert_eq!(anthropic.protocol, Protocol::Anthropic);

            let openai = routes.iter().find(|route| route.name == "openai");
            assert!(openai.is_none(), "no OPENAI_API_KEY was set");
        });
    }

    #[test]
    fn local_providers_are_openai_compatible_and_require_explicit_opt_in() {
        // Local servers need no real credential, but dreamseq still requires
        // a non-empty env var to enable them — nothing here should be
        // detected just because a local Ollama/vLLM/LM Studio instance
        // happens to be running.
        with_env(&[], || {
            let mut routes = Vec::new();
            add_named_provider_routes(&mut routes);
            assert!(
                !routes
                    .iter()
                    .any(|route| ["ollama", "lmstudio", "vllm"].contains(&route.name.as_str()))
            );
        });

        with_env(&[("OLLAMA_API_KEY", Some("local"))], || {
            let mut routes = Vec::new();
            add_named_provider_routes(&mut routes);
            let ollama = routes
                .iter()
                .find(|route| route.name == "ollama")
                .expect("ollama route should be detected once opted in");
            assert_eq!(ollama.protocol, Protocol::OpenAiCompatible);
            assert_eq!(ollama.base_url, "http://localhost:11434/v1");
        });
    }

    #[test]
    fn respects_model_override_env_var() {
        with_env(
            &[
                ("XAI_API_KEY", Some("test-xai-key")),
                ("GROK_API_KEY", None),
                ("XAI_MODEL", Some("grok-custom")),
            ],
            || {
                let mut routes = Vec::new();
                add_named_provider_routes(&mut routes);
                let route = routes
                    .iter()
                    .find(|route| route.name == "grok")
                    .expect("grok route should be detected");
                assert_eq!(route.model, "grok-custom");
            },
        );
    }

    #[test]
    fn falls_back_to_secondary_key_env_var() {
        with_env(
            &[
                ("GEMINI_API_KEY", None),
                ("GOOGLE_API_KEY", Some("test-google-key")),
            ],
            || {
                let mut routes = Vec::new();
                add_named_provider_routes(&mut routes);
                let route = routes
                    .iter()
                    .find(|route| route.name == "gemini")
                    .expect("gemini route should be detected via GOOGLE_API_KEY");
                assert_eq!(route.api_key, "test-google-key");
            },
        );
    }

    #[test]
    fn skips_provider_when_no_key_is_set() {
        with_env(&[("ZAI_API_KEY", None), ("GLM_API_KEY", None)], || {
            let mut routes = Vec::new();
            add_named_provider_routes(&mut routes);
            assert!(!routes.iter().any(|route| route.name == "zai"));
        });
    }

    #[test]
    fn does_not_duplicate_a_route_already_configured_for_the_same_base_url() {
        with_env(&[("OPENAI_API_KEY", Some("test-openai-key"))], || {
            let mut routes = vec![InferenceRoute {
                name: "custom-openai".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                model: "gpt-4o".to_string(),
                api_key: "explicit-key".to_string(),
                light_model: None,
                protocol: Protocol::OpenAiCompatible,
            }];
            add_named_provider_routes(&mut routes);
            assert_eq!(routes.len(), 1);
            assert_eq!(routes[0].name, "custom-openai");
        });
    }
}
