//! LLM provider clients for pencode.
//!
//! Supports the Anthropic Messages API and any OpenAI chat-completions
//! compatible endpoint (OpenAI, OpenRouter, Groq, local servers...).
//!
//! Model specs use upstream's `provider/model` format, e.g.
//! `anthropic/claude-sonnet-4-5` or `openai/gpt-4.1`. API keys resolve from
//! config (`provider.<name>.apiKey`) first, then well-known env vars.

use anyhow::{bail, Context, Result};
use pencode_core::config::Config;
use pencode_protocol::Role;
use std::io::{BufRead, BufReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Anthropic,
    OpenAi,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// A conversation snapshot to send to the provider.
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub system: Option<String>,
    /// Ordered (role, text) turns; tool parts are flattened to their text.
    pub messages: Vec<(Role, String)>,
}

impl Prompt {
    pub fn from_session(session: &pencode_protocol::Session) -> Self {
        let messages = session
            .messages
            .iter()
            .map(|m| (m.role, m.text()))
            .filter(|(_, text)| !text.is_empty())
            .collect();
        Prompt { system: None, messages }
    }
}

const DEFAULT_BASE_URLS: &[(&str, &str)] = &[
    ("anthropic", "https://api.anthropic.com"),
    ("openai", "https://api.openai.com/v1"),
    ("openrouter", "https://openrouter.ai/api/v1"),
    ("groq", "https://api.groq.com/openai/v1"),
];

const ENV_KEYS: &[(&str, &str)] = &[
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("openai", "OPENAI_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
    ("groq", "GROQ_API_KEY"),
];

fn protocol_for(provider_name: &str) -> Protocol {
    if provider_name.contains("anthropic") || provider_name.contains("claude") {
        Protocol::Anthropic
    } else {
        Protocol::OpenAi
    }
}

fn default_base_url(provider_name: &str) -> &str {
    DEFAULT_BASE_URLS
        .iter()
        .find(|(name, _)| *name == provider_name)
        .map(|(_, url)| *url)
        .unwrap_or("https://api.openai.com/v1")
}

/// Resolve a `provider/model` spec against config + environment.
pub fn resolve(model_spec: &str, config: &Config) -> Result<Resolved> {
    let Some((provider_name, model)) = model_spec.split_once('/') else {
        bail!(
            "invalid model `{model_spec}` — expected `provider/model`, \
             e.g. `anthropic/claude-sonnet-4-5`"
        )
    };

    let configured = config.provider.get(provider_name);
    let base_url = configured
        .and_then(|p| p.base_url.clone())
        .unwrap_or_else(|| default_base_url(provider_name).to_string());

    let api_key = match configured.and_then(|p| p.api_key.clone()) {
        Some(key) if !key.is_empty() => key,
        _ => env_key(provider_name)?,
    };

    Ok(Resolved {
        protocol: protocol_for(provider_name),
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key,
        model: model.to_string(),
    })
}

fn env_key(provider_name: &str) -> Result<String> {
    for (name, var) in ENV_KEYS {
        if *name == provider_name {
            if let Ok(value) = std::env::var(var) {
                if !value.is_empty() {
                    return Ok(value);
                }
            }
        }
    }
    // Generic fallbacks so any OpenAI-compatible proxy works via config alone.
    for var in ["PENCODE_API_KEY", "OPENCODE_API_KEY"] {
        if let Ok(value) = std::env::var(var) {
            if !value.is_empty() {
                return Ok(value);
            }
        }
    }
    bail!("no API key for provider `{provider_name}` — set it in ~/.config/pencode/config.json or via env")
}

// ---------------------------------------------------------------------------
// request building / response parsing (pure functions, unit-testable)

pub fn anthropic_body(resolved: &Resolved, prompt: &Prompt, stream: bool) -> serde_json::Value {
    serde_json::json!({
        "model": resolved.model,
        "max_tokens": 8192,
        "stream": stream,
        "messages": prompt.messages.iter().map(|(role, text)| serde_json::json!({
            "role": if *role == Role::Assistant { "assistant" } else { "user" },
            "content": text,
        })).collect::<Vec<_>>(),
        "system": prompt.system,
    })
}

pub fn openai_body(resolved: &Resolved, prompt: &Prompt, stream: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": resolved.model,
        "stream": stream,
        "messages": prompt.messages.iter().map(|(role, text)| serde_json::json!({
            "role": if *role == Role::Assistant { "assistant" } else { "user" },
            "content": text,
        })).collect::<Vec<_>>(),
    });
    if let Some(system) = &prompt.system {
        body["messages"]
            .as_array_mut()
            .unwrap()
            .insert(0, serde_json::json!({ "role": "system", "content": system }));
    }
    body
}

/// Extract incremental text from one Anthropic SSE `data:` payload.
pub fn parse_anthropic_event(data: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    match value["type"].as_str()? {
        "content_block_delta" => value["delta"]["text"].as_str().map(String::from),
        _ => None,
    }
}

/// Extract incremental text from one OpenAI SSE `data:` payload.
pub fn parse_openai_event(data: &str) -> Option<String> {
    if data.trim() == "[DONE]" {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    value["choices"][0]["delta"]["content"]
        .as_str()
        .map(String::from)
}

fn anthropic_url(resolved: &Resolved) -> String {
    let base = resolved.base_url.trim_end_matches("/v1");
    format!("{base}/v1/messages")
}

fn openai_url(resolved: &Resolved) -> String {
    format!("{}/chat/completions", resolved.base_url)
}

fn send(resolved: &Resolved, body: &serde_json::Value, stream: bool) -> Result<reqwest::blocking::Response> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let mut request = client
        .post(match resolved.protocol {
            Protocol::Anthropic => anthropic_url(resolved),
            Protocol::OpenAi => openai_url(resolved),
        })
        .header("content-type", "application/json");

    request = match resolved.protocol {
        Protocol::Anthropic => request
            .header("x-api-key", &resolved.api_key)
            .header("anthropic-version", "2023-06-01"),
        Protocol::OpenAi => request.header("authorization", format!("Bearer {}", resolved.api_key)),
    };
    if stream {
        request = request.header("accept", "text/event-stream");
    }

    let response = request.body(body.to_string()).send()?;
    let status = response.status();
    if !status.is_success() {
        let snippet = response.text().unwrap_or_default();
        let snippet: String = snippet.chars().take(300).collect();
        bail!("provider returned {status}: {snippet}");
    }
    Ok(response)
}

fn sse_data_lines(reader: impl BufRead) -> impl Iterator<Item = String> {
    reader.lines().filter_map(|line| {
        let line = line.ok()?;
        line.strip_prefix("data: ").map(String::from).or_else(|| {
            if line == "data:" {
                Some(String::new())
            } else {
                None
            }
        })
    })
}

/// One-shot completion (no streaming).
pub fn complete(resolved: &Resolved, prompt: &Prompt) -> Result<String> {
    let body = match resolved.protocol {
        Protocol::Anthropic => anthropic_body(resolved, prompt, false),
        Protocol::OpenAi => openai_body(resolved, prompt, false),
    };
    let response = send(resolved, &body, false)?;
    let value: serde_json::Value =
        response.json().context("decoding provider response")?;

    let text = match resolved.protocol {
        Protocol::Anthropic => value["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        Protocol::OpenAi => value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    };
    if text.is_empty() {
        bail!("provider returned an empty response");
    }
    Ok(text)
}

/// Streaming completion; invokes `on_delta` per text chunk and returns the
/// full concatenated response when finished.
pub fn stream(
    resolved: &Resolved,
    prompt: &Prompt,
    on_delta: &mut dyn FnMut(&str),
) -> Result<String> {
    let body = match resolved.protocol {
        Protocol::Anthropic => anthropic_body(resolved, prompt, true),
        Protocol::OpenAi => openai_body(resolved, prompt, true),
    };
    let response = send(resolved, &body, true)?;
    let reader = BufReader::new(response);

    let parse = |data: &str| match resolved.protocol {
        Protocol::Anthropic => parse_anthropic_event(data),
        Protocol::OpenAi => parse_openai_event(data),
    };

    let mut full = String::new();
    for data in sse_data_lines(reader) {
        if let Some(delta) = parse(&data) {
            on_delta(&delta);
            full.push_str(&delta);
        }
    }
    if full.is_empty() {
        bail!("provider stream ended without any content");
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_provider_and_model_from_spec() {
        let mut config = Config::default();
        config
            .provider
            .insert("anthropic".into(), test_provider_config("sk-test"));
        let resolved = resolve("anthropic/claude-sonnet-4-5", &config).unwrap();
        assert_eq!(resolved.protocol, Protocol::Anthropic);
        assert_eq!(resolved.model, "claude-sonnet-4-5");
        assert_eq!(resolved.base_url, "https://api.anthropic.com");
        assert_eq!(resolved.api_key, "sk-test");
    }

    #[test]
    fn rejects_specs_without_provider_prefix() {
        assert!(resolve("just-a-model", &Config::default()).is_err());
    }

    #[test]
    fn parses_anthropic_deltas_only() {
        let delta = parse_anthropic_event(
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}"#,
        );
        assert_eq!(delta.as_deref(), Some("hi"));
        assert!(parse_anthropic_event(r#"{"type":"ping"}"#).is_none());
    }

    #[test]
    fn parses_openai_deltas_and_done_sentinel() {
        let delta = parse_openai_event(
            r#"{"choices":[{"delta":{"content":"he"}}"{"ignored":1}]}"#,
        );
        assert!(delta.is_none()); // malformed json -> None

        let delta = parse_openai_event(r#"{"choices":[{"delta":{"content":"he"}}]}"#);
        assert_eq!(delta.as_deref(), Some("he"));
        assert!(parse_openai_event("[DONE]").is_none());
    }

    #[test]
    fn bodies_carry_roles_and_system() {
        let resolved = Resolved {
            protocol: Protocol::OpenAi,
            base_url: "http://localhost".into(),
            api_key: "k".into(),
            model: "m".into(),
        };
        let prompt = Prompt {
            system: Some("be brief".into()),
            messages: vec![(Role::User, "hello".into())],
        };
        let body = openai_body(&resolved, &prompt, false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hello");

        let resolved_anthropic = Resolved { protocol: Protocol::Anthropic, ..resolved };
        let body = anthropic_body(&resolved_anthropic, &prompt, false);
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["max_tokens"], 8192);
    }

    fn test_provider_config(key: &str) -> pencode_core::config::ProviderConfig {
        pencode_core::config::ProviderConfig {
            api_key: Some(key.into()),
            base_url: None,
        }
    }
}
