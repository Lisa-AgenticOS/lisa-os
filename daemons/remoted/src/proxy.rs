//! Request proxying (ADR-0008 §2): render one OpenAI-compatible chat
//! request into the provider's dialect, and normalize the response back.
//! The build/translate steps are pure functions (unit-testable with no
//! network); `send` performs the single upstream HTTPS call. Mockable:
//! tests point a custom provider's base_url at a local server.

use crate::oauth;
use crate::registry::{AuthStyle, Dialect, ProviderSpec};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("provider {0} has no endpoint configured")]
    NoEndpoint(String),
    #[error("request body must contain a messages array")]
    BadRequest,
    #[error("upstream error {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
}

/// A fully-rendered upstream request.
#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

/// Render the OpenAI-shaped `body` for `spec`, authenticated with
/// `credential`.
pub fn build_upstream(
    spec: &ProviderSpec,
    credential: &str,
    body: &Value,
) -> Result<UpstreamRequest, ProxyError> {
    let base = spec
        .base_url
        .as_deref()
        .ok_or_else(|| ProxyError::NoEndpoint(spec.id.clone()))?;
    if !body.get("messages").is_some_and(Value::is_array) {
        return Err(ProxyError::BadRequest);
    }
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    match spec.auth {
        AuthStyle::Bearer => {
            headers.push(("authorization".into(), format!("Bearer {credential}")));
        }
        AuthStyle::AnthropicApiKey => {
            headers.push(("x-api-key".into(), credential.to_string()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        }
        AuthStyle::AnthropicOauth => {
            // Claude subscription (OAuth) auth: Bearer + the documented
            // beta header — NOT x-api-key (Construct brain/provider/anthropic.go).
            headers.push(("authorization".into(), format!("Bearer {credential}")));
            headers.push(("anthropic-beta".into(), oauth::ANTHROPIC_OAUTH_BETA.into()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
        }
    }
    let oauth_anthropic = matches!(spec.auth, AuthStyle::AnthropicOauth);
    match spec.dialect {
        Dialect::OpenaiCompat => {
            // Pass the body through untouched (minus Lisa extensions);
            // the provider speaks the same shape.
            let mut b = body.clone();
            if let Some(obj) = b.as_object_mut() {
                obj.remove("lisa_priority");
            }
            Ok(UpstreamRequest {
                url: format!("{}/chat/completions", base.trim_end_matches('/')),
                headers,
                body: b,
            })
        }
        Dialect::AnthropicMessages => {
            // Native Messages API: hoist system/developer messages into
            // the single `system` string (mirroring Anthropic's own
            // documented compat behavior), keep user/assistant turns.
            let messages = body["messages"].as_array().expect("checked above");
            let mut system_parts: Vec<String> = Vec::new();
            // OAuth (Claude Pro/Max) traffic must present as Claude Code
            // or the Messages API returns 429 — prepend the marker system
            // prompt (Construct brain/provider/anthropic.go).
            if oauth_anthropic {
                system_parts.push(oauth::ANTHROPIC_CLAUDE_CODE_SYSTEM.to_string());
            }
            let mut turns: Vec<Value> = Vec::new();
            for m in messages {
                let role = m["role"].as_str().unwrap_or("user");
                let content = m["content"].as_str().unwrap_or_default().to_string();
                match role {
                    "system" | "developer" => system_parts.push(content),
                    "assistant" => turns.push(json!({"role": "assistant", "content": content})),
                    _ => turns.push(json!({"role": "user", "content": content})),
                }
            }
            let mut out = json!({
                "model": body.get("model").cloned().unwrap_or(Value::Null),
                "max_tokens": body.get("max_tokens").and_then(Value::as_u64).unwrap_or(1024),
                "messages": turns,
            });
            if !system_parts.is_empty() {
                out["system"] = Value::String(system_parts.join("\n"));
            }
            // OAuth requires the ?beta=true query param alongside the
            // beta header (Construct brain/provider/anthropic.go).
            let url = if oauth_anthropic {
                format!("{}/v1/messages?beta=true", base.trim_end_matches('/'))
            } else {
                format!("{}/v1/messages", base.trim_end_matches('/'))
            };
            Ok(UpstreamRequest {
                url,
                headers,
                body: out,
            })
        }
    }
}

/// Normalize the upstream response to the OpenAI-compatible shape the
/// rest of the OS speaks (§5.1). OpenAI-compat responses pass through.
pub fn translate_response(dialect: Dialect, upstream: &Value) -> Value {
    match dialect {
        Dialect::OpenaiCompat => upstream.clone(),
        Dialect::AnthropicMessages => {
            let text = upstream["content"]
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b["type"] == "text")
                        .filter_map(|b| b["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            let input = upstream["usage"]["input_tokens"].as_u64().unwrap_or(0);
            let output = upstream["usage"]["output_tokens"].as_u64().unwrap_or(0);
            json!({
                "id": upstream.get("id").cloned().unwrap_or(Value::Null),
                "object": "chat.completion",
                "model": upstream.get("model").cloned().unwrap_or(Value::Null),
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": text},
                    "finish_reason":
                        if upstream["stop_reason"] == "max_tokens" { "length" } else { "stop" },
                }],
                "usage": {
                    "prompt_tokens": input,
                    "completion_tokens": output,
                    "total_tokens": input + output,
                },
            })
        }
    }
}

/// Output tokens reported by a normalized response (for the Ledger).
pub fn output_tokens(normalized: &Value) -> i64 {
    normalized["usage"]["completion_tokens"]
        .as_i64()
        .unwrap_or(0)
}

/// Perform the upstream call. This is the only network touchpoint in
/// the crate (and in the OS, outside modeld).
pub async fn send(client: &reqwest::Client, req: &UpstreamRequest) -> Result<Value, ProxyError> {
    let mut builder = client.post(&req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    let resp = builder.json(&req.body).send().await?;
    let status = resp.status();
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) if status.is_success() => return Err(ProxyError::Http(e)),
        Err(_) => Value::Null,
    };
    if !status.is_success() {
        return Err(ProxyError::Upstream {
            status: status.as_u16(),
            body: body.to_string(),
        });
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::builtin_providers;

    fn spec(id: &str) -> ProviderSpec {
        builtin_providers()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
    }

    #[test]
    fn openai_compat_passes_the_body_through_with_bearer_auth() {
        let body = json!({
            "model": "gpt-x", "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 5, "lisa_priority": "interactive",
        });
        let up = build_upstream(&spec("openai"), "sk-1", &body).unwrap();
        assert_eq!(up.url, "https://api.openai.com/v1/chat/completions");
        assert!(
            up.headers
                .contains(&("authorization".into(), "Bearer sk-1".into()))
        );
        assert_eq!(up.body["model"], "gpt-x");
        assert!(
            up.body.get("lisa_priority").is_none(),
            "Lisa extensions stripped"
        );
    }

    #[test]
    fn tinker_together_fireworks_route_to_their_verified_bases() {
        let body = json!({"messages": [{"role":"user","content":"x"}]});
        for (id, url) in [
            (
                "tinker",
                "https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1/chat/completions",
            ),
            ("together", "https://api.together.ai/v1/chat/completions"),
            (
                "fireworks",
                "https://api.fireworks.ai/inference/v1/chat/completions",
            ),
        ] {
            let up = build_upstream(&spec(id), "k", &body).unwrap();
            assert_eq!(up.url, url, "{id}");
        }
    }

    #[test]
    fn anthropic_renders_the_native_messages_api_with_system_hoisting() {
        let body = json!({
            "model": "claude-x",
            "messages": [
                {"role": "system", "content": "be terse"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "system", "content": "stay terse"},
                {"role": "user", "content": "bye"},
            ],
        });
        let up = build_upstream(&spec("anthropic"), "sk-ant", &body).unwrap();
        assert_eq!(up.url, "https://api.anthropic.com/v1/messages");
        assert!(up.headers.contains(&("x-api-key".into(), "sk-ant".into())));
        assert!(
            up.headers
                .contains(&("anthropic-version".into(), "2023-06-01".into()))
        );
        assert_eq!(up.body["system"], "be terse\nstay terse");
        assert_eq!(up.body["messages"].as_array().unwrap().len(), 3);
        assert_eq!(
            up.body["max_tokens"], 1024,
            "Messages API requires max_tokens"
        );
    }

    #[test]
    fn anthropic_response_normalizes_to_the_openai_shape() {
        let upstream = json!({
            "id": "msg_1", "model": "claude-x", "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hel"}, {"type": "text", "text": "lo"}],
            "usage": {"input_tokens": 7, "output_tokens": 3},
        });
        let out = translate_response(Dialect::AnthropicMessages, &upstream);
        assert_eq!(out["choices"][0]["message"]["content"], "hello");
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
        assert_eq!(out["usage"]["total_tokens"], 10);
        assert_eq!(output_tokens(&out), 3);
    }

    #[test]
    fn missing_messages_and_missing_endpoint_are_refused() {
        assert!(matches!(
            build_upstream(&spec("openai"), "k", &json!({})),
            Err(ProxyError::BadRequest)
        ));
        let mut s = spec("openai");
        s.base_url = None;
        assert!(matches!(
            build_upstream(&s, "k", &json!({"messages": []})),
            Err(ProxyError::NoEndpoint(_))
        ));
    }
}
