// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT

/// Keep useful text findings when a provider emits shorthand strings instead
/// of the typed analysis objects. Defaults are conservative and never invent
/// evidence.
pub(super) fn normalize_analysis_items(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let schemas: &[(&str, &[(&str, serde_json::Value)])] = &[
        (
            "model_failures",
            &[
                ("model", serde_json::json!("unknown")),
                ("issue", serde_json::json!("")),
                ("frequency", serde_json::json!(1)),
                ("example", serde_json::json!("")),
            ],
        ),
        (
            "harness_friction",
            &[
                ("harness", serde_json::json!("unknown")),
                ("issue", serde_json::json!("")),
                ("severity", serde_json::json!(0.5)),
            ],
        ),
        (
            "missing_tooling",
            &[
                ("tool_name", serde_json::json!("candidate-capability")),
                ("purpose", serde_json::json!("")),
                ("estimated_value", serde_json::json!(0.5)),
            ],
        ),
        (
            "workflow_bottlenecks",
            &[
                ("description", serde_json::json!("")),
                ("frequency", serde_json::json!(1)),
                ("time_impact_minutes", serde_json::json!(0.0)),
            ],
        ),
        (
            "repeated_commands",
            &[
                ("command", serde_json::json!("")),
                ("frequency", serde_json::json!(1)),
                ("context", serde_json::json!("")),
            ],
        ),
        (
            "repeated_prompts",
            &[
                ("prompt_pattern", serde_json::json!("")),
                ("frequency", serde_json::json!(1)),
                ("suggested_improvement", serde_json::json!("")),
            ],
        ),
        (
            "context_loss",
            &[
                ("description", serde_json::json!("")),
                ("affected_segments", serde_json::json!([])),
            ],
        ),
        (
            "automation_opportunities",
            &[
                ("description", serde_json::json!("")),
                ("estimated_time_saved", serde_json::json!(0.0)),
                ("confidence", serde_json::json!(0.5)),
            ],
        ),
    ];
    for (key, fields) in schemas {
        let Some(items) = object
            .get_mut(*key)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for item in items.iter_mut() {
            let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) else {
                continue;
            };
            let mut normalized = serde_json::Map::new();
            for (field, default) in *fields {
                normalized.insert((*field).to_string(), default.clone());
            }
            let primary = match *key {
                "model_failures" | "harness_friction" => "issue",
                "missing_tooling" => "purpose",
                "workflow_bottlenecks" | "context_loss" | "automation_opportunities" => {
                    "description"
                }
                "repeated_commands" => "command",
                "repeated_prompts" => "prompt_pattern",
                _ => "description",
            };
            normalized.insert(
                primary.to_string(),
                serde_json::Value::String(text.to_string()),
            );
            *item = serde_json::Value::Object(normalized);
        }
    }
}

pub(super) fn redact_sensitive(text: &str) -> String {
    static ASSIGNMENT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?i)((?:api[_-]?key|access[_-]?token|auth[_-]?token|secret|password|authorization)\s*[:=]\s*(?:bearer\s+)?)[^\s,;\"']+"#,
        )
        .unwrap_or_else(|error| invalid_builtin_regex("credential_assignment", error))
    });
    static JWT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\b[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b")
            .unwrap_or_else(|error| invalid_builtin_regex("jwt", error))
    });
    static PROVIDER_TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\b(?:AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{20,})\b",
        )
        .unwrap_or_else(|error| invalid_builtin_regex("provider_token", error))
    });
    static EMAIL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
            .unwrap_or_else(|error| invalid_builtin_regex("email", error))
    });
    static HOME_PATH: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"/(?:home|Users)/[^/\s]+")
            .unwrap_or_else(|error| invalid_builtin_regex("home_path", error))
    });

    let redacted = ASSIGNMENT.replace_all(text, "${1}[REDACTED]");
    let redacted = JWT.replace_all(&redacted, "[REDACTED_JWT]");
    let redacted = PROVIDER_TOKEN.replace_all(&redacted, "[REDACTED_TOKEN]");
    let redacted = EMAIL.replace_all(&redacted, "[REDACTED_EMAIL]");
    HOME_PATH
        .replace_all(&redacted, "/home/[REDACTED_USER]")
        .into_owned()
}

fn invalid_builtin_regex(name: &'static str, error: regex::Error) -> ! {
    tracing::error!(name, error = %error, "built-in redaction regex compilation failed");
    std::panic::panic_any("invalid built-in redaction regex")
}

pub(super) fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Stringify descriptive scalars and parse numeric metrics emitted as prose.
pub(super) fn coerce_analysis_scalars(value: &mut serde_json::Value) {
    const TEXT_FIELDS: &[&str] = &[
        "model",
        "issue",
        "example",
        "harness",
        "tool_name",
        "purpose",
        "description",
        "command",
        "context",
        "prompt_pattern",
        "suggested_improvement",
    ];
    const NUMERIC_FIELDS: &[&str] = &[
        "frequency",
        "severity",
        "estimated_value",
        "time_impact_minutes",
        "estimated_time_saved",
        "confidence",
    ];

    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if NUMERIC_FIELDS.contains(&key.as_str()) && child.is_string() {
                    let number =
                        parse_numeric_string(child.as_str().unwrap_or_default()).unwrap_or(0.0);
                    *child = serde_json::json!(number);
                } else if TEXT_FIELDS.contains(&key.as_str())
                    && !child.is_string()
                    && !child.is_null()
                {
                    let replacement = match &*child {
                        serde_json::Value::Number(number) => number.to_string(),
                        serde_json::Value::Bool(boolean) => boolean.to_string(),
                        other => other.to_string(),
                    };
                    *child = serde_json::Value::String(replacement);
                } else if key == "affected_segments" {
                    if let serde_json::Value::Array(items) = child {
                        for item in items {
                            if !item.is_string() && !item.is_null() {
                                *item = serde_json::Value::String(match &*item {
                                    serde_json::Value::Number(number) => number.to_string(),
                                    serde_json::Value::Bool(boolean) => boolean.to_string(),
                                    other => other.to_string(),
                                });
                            }
                        }
                    }
                } else {
                    coerce_analysis_scalars(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                coerce_analysis_scalars(item);
            }
        }
        _ => {}
    }
}

pub(super) fn normalize_analysis_json(input: &str) -> String {
    static NUMERIC_VALUE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
                r#"(?i)(?:\"(frequency|severity|estimated_value|time_impact_minutes|estimated_time_saved|confidence)\"|(frequency|severity|estimated_value|time_impact_minutes|estimated_time_saved|confidence))\s*:\s*(?:\"[^\"]*\"|'[^']*'|[^,}\n]+)"#,
            )
            .unwrap_or_else(|error| invalid_builtin_regex("numeric_analysis_field", error))
    });
    NUMERIC_VALUE
        .replace_all(input, |captures: &regex::Captures<'_>| {
            let field = captures
                .get(1)
                .or_else(|| captures.get(2))
                .map(|match_| match_.as_str())
                .unwrap_or("frequency");
            let full = captures
                .get(0)
                .map(|match_| match_.as_str())
                .unwrap_or_default();
            let raw_value = full
                .split_once(':')
                .map(|(_, value)| value)
                .unwrap_or_default();
            let number =
                parse_numeric_string(raw_value.trim().trim_matches(['\"', '\''])).unwrap_or(0.0);
            format!("\"{field}\":{number}")
        })
        .into_owned()
}

fn parse_numeric_string(value: &str) -> Option<f64> {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .trim_end_matches("-plus")
        .to_string();
    if let Ok(number) = normalized.parse::<f64>() {
        return Some(number);
    }
    let leading_digits = normalized
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .find(|part| !part.is_empty())
        // traci: allow -- invalid fragments are expected while extracting a number from prose.
        .and_then(|part| part.parse::<f64>().ok());
    if leading_digits.is_some() {
        return leading_digits;
    }
    let leading_word = normalized
        .split(|character: char| !character.is_ascii_alphabetic())
        .find(|part| !part.is_empty())
        .unwrap_or_default();
    match leading_word {
        "zero" => Some(0.0),
        "one" => Some(1.0),
        "two" => Some(2.0),
        "three" => Some(3.0),
        "four" => Some(4.0),
        "five" => Some(5.0),
        "six" => Some(6.0),
        "seven" => Some(7.0),
        "eight" => Some(8.0),
        "nine" => Some(9.0),
        "ten" => Some(10.0),
        "eleven" => Some(11.0),
        "twelve" => Some(12.0),
        "thirteen" => Some(13.0),
        "fourteen" => Some(14.0),
        "fifteen" => Some(15.0),
        "sixteen" => Some(16.0),
        "seventeen" => Some(17.0),
        "eighteen" => Some(18.0),
        "nineteen" => Some(19.0),
        "twenty" => Some(20.0),
        "thirty" => Some(30.0),
        "forty" => Some(40.0),
        "fifty" => Some(50.0),
        "sixty" => Some(60.0),
        "seventy" => Some(70.0),
        "eighty" => Some(80.0),
        "ninety" => Some(90.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_identity_and_home_paths() {
        let input = "api_key=sk-abcdefghijklmnopqrstuvwxyz user@example.com /home/alice/project";
        let redacted = redact_sensitive(input);
        assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(!redacted.contains("user@example.com"));
        assert!(!redacted.contains("/home/alice"));
    }

    #[test]
    fn repairs_shorthand_items_and_scalar_types() {
        let mut value = serde_json::json!({
            "model_failures": ["bad retry"],
            "workflow_bottlenecks": [{"description": 42, "frequency": "three"}],
            "context_loss": [{"description": true, "affected_segments": [1, false]}]
        });
        normalize_analysis_items(&mut value);
        coerce_analysis_scalars(&mut value);
        assert_eq!(value["model_failures"][0]["issue"], "bad retry");
        assert_eq!(value["workflow_bottlenecks"][0]["description"], "42");
        assert_eq!(value["workflow_bottlenecks"][0]["frequency"], 3.0);
        assert_eq!(value["context_loss"][0]["affected_segments"][1], "false");
    }

    #[test]
    fn normalizes_numeric_prose_and_unicode_truncation() {
        let normalized = normalize_analysis_json("{frequency: '12-plus', confidence: 'high'}");
        assert!(normalized.contains("\"frequency\":12"));
        assert!(normalized.contains("\"confidence\":0"));
        assert_eq!(truncate("aé🙂z", 3), "aé🙂");
    }
}
