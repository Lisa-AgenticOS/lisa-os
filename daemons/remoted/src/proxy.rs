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
                    // A tool RESULT. Anthropic carries these as a user
                    // turn holding a tool_result block, not as a role of
                    // its own — dropping them (as this did) leaves the
                    // model asking for the same tool forever, because it
                    // never learns what came back.
                    "tool" => turns.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": m["tool_call_id"].as_str().unwrap_or_default(),
                            "content": content,
                        }],
                    })),
                    "assistant" => {
                        // An assistant turn that CALLED a tool must go
                        // back as a tool_use block, or the tool_result
                        // that follows refers to nothing and Anthropic
                        // rejects the conversation.
                        match m["tool_calls"].as_array().and_then(|c| c.first()) {
                            Some(call) => {
                                let args = call["function"]["arguments"].as_str().unwrap_or("{}");
                                turns.push(json!({
                                    "role": "assistant",
                                    "content": [{
                                        "type": "tool_use",
                                        "id": call["id"].as_str().unwrap_or("call_1"),
                                        "name": call["function"]["name"].as_str().unwrap_or(""),
                                        "input": serde_json::from_str::<Value>(args)
                                            .unwrap_or_else(|_| json!({})),
                                    }],
                                }));
                            }
                            None => turns.push(json!({"role": "assistant", "content": content})),
                        }
                    }
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
            // TOOLS. Without this the model is simply never told they
            // exist, and answers — correctly — that it cannot do
            // anything. The shapes differ only in nesting: OpenAI wraps
            // the declaration in `function`, Anthropic does not, and
            // calls the schema `input_schema`.
            if let Some(tools) = body["tools"].as_array()
                && !tools.is_empty()
            {
                out["tools"] = Value::Array(
                    tools
                        .iter()
                        .map(|t| {
                            let f = &t["function"];
                            json!({
                                "name": f["name"].as_str().unwrap_or_default(),
                                "description": f["description"].as_str().unwrap_or_default(),
                                "input_schema": f
                                    .get("parameters")
                                    .cloned()
                                    .unwrap_or_else(|| json!({"type": "object"})),
                            })
                        })
                        .collect(),
                );
                // "auto" is Anthropic's default; only a forced choice
                // needs saying, and OpenAI's "none" has no equivalent
                // beyond simply not offering the tools.
                match body["tool_choice"].as_str() {
                    Some("required") => out["tool_choice"] = json!({"type": "any"}),
                    Some("none") => {
                        out.as_object_mut().map(|o| o.remove("tools"));
                    }
                    _ => {}
                }
            }
            // Streaming requests stream upstream too (ADR-0010 update):
            // the OpenAI-compat dialect passes the flag through verbatim;
            // the Anthropic dialect re-renders the body, so carry it over.
            if body["stream"].as_bool().unwrap_or(false) {
                out["stream"] = Value::Bool(true);
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
            // tool_use blocks become OpenAI tool_calls. Dropping them —
            // which keeping only `text` did — means a model that called
            // a tool looks like one that said nothing, and the loop ends
            // having done nothing while the model believes it acted.
            let tool_calls: Vec<Value> = upstream["content"]
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b["type"] == "tool_use")
                        .map(|b| {
                            json!({
                                "id": b["id"].as_str().unwrap_or("call_1"),
                                "type": "function",
                                "function": {
                                    "name": b["name"].as_str().unwrap_or_default(),
                                    // OpenAI carries arguments as a JSON
                                    // *string*; Anthropic as an object.
                                    "arguments": b["input"].to_string(),
                                },
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut message = json!({"role": "assistant", "content": text});
            if !tool_calls.is_empty() {
                message["tool_calls"] = Value::Array(tool_calls.clone());
            }
            let input = upstream["usage"]["input_tokens"].as_u64().unwrap_or(0);
            let output = upstream["usage"]["output_tokens"].as_u64().unwrap_or(0);
            json!({
                "id": upstream.get("id").cloned().unwrap_or(Value::Null),
                "object": "chat.completion",
                "model": upstream.get("model").cloned().unwrap_or(Value::Null),
                "choices": [{
                    "index": 0,
                    "message": message,
                    "finish_reason": match upstream["stop_reason"].as_str() {
                        Some("max_tokens") => "length",
                        // The loop keys off this to know a tool was
                        // called rather than a turn finished.
                        Some("tool_use") => "tool_calls",
                        _ if !tool_calls.is_empty() => "tool_calls",
                        _ => "stop",
                    },
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

fn post(client: &reqwest::Client, req: &UpstreamRequest) -> reqwest::RequestBuilder {
    let mut builder = client.post(&req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    builder.json(&req.body)
}

/// Perform the upstream call. `send`/`send_stream` are the only network
/// touchpoints in the crate (and in the OS, outside modeld).
pub async fn send(client: &reqwest::Client, req: &UpstreamRequest) -> Result<Value, ProxyError> {
    let resp = post(client, req).send().await?;
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

/// Perform a *streaming* upstream call (ADR-0010 update): the request is
/// sent as-is (callers set `stream:true` in the body) and the provider's
/// raw SSE bytes come back as a stream. A non-2xx status is read to
/// completion and surfaced as `Upstream` before any byte is forwarded.
pub async fn send_stream(
    client: &reqwest::Client,
    req: &UpstreamRequest,
) -> Result<impl futures::Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send + use<>, ProxyError>
{
    use futures::StreamExt;
    let resp = post(client, req).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ProxyError::Upstream {
            status: status.as_u16(),
            body,
        });
    }
    Ok(resp.bytes_stream().map(|r| r.map(|b| b.to_vec())))
}

#[cfg(test)]
mod tool_translation_tests {
    use super::*;

    use crate::registry::builtin_providers;

    fn anthropic(body: Value) -> Value {
        let spec = builtin_providers()
            .into_iter()
            .find(|p| p.id == "anthropic")
            .unwrap();
        build_upstream(&spec, "sk-ant-test", &body).unwrap().body
    }

    /// Without this the model is never told the tools exist and answers,
    /// correctly, that it cannot do anything — which is exactly what the
    /// Assistant reported on the device.
    #[test]
    fn tools_reach_anthropic_in_its_own_shape() {
        let out = anthropic(json!({
            "model": "claude-x",
            "messages": [{"role": "user", "content": "read the page"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "read_page",
                    "description": "Read the open page",
                    "parameters": {"type": "object", "properties": {"q": {"type": "string"}}},
                },
            }],
            "tool_choice": "auto",
        }));
        let t = &out["tools"][0];
        assert_eq!(t["name"], "read_page");
        assert_eq!(t["description"], "Read the open page");
        // Anthropic calls it input_schema and does not nest under
        // `function`.
        assert_eq!(t["input_schema"]["properties"]["q"]["type"], "string");
        assert!(t.get("function").is_none());
        // "auto" is Anthropic's default; saying so is noise.
        assert!(out.get("tool_choice").is_none());
    }

    #[test]
    fn tool_choice_none_removes_the_tools_entirely() {
        let out = anthropic(json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "t", "parameters": {}}}],
            "tool_choice": "none",
        }));
        assert!(out.get("tools").is_none(), "not offering them IS 'none'");
    }

    /// An assistant turn that called a tool must go back as a tool_use
    /// block: the tool_result that follows refers to its id, and without
    /// it Anthropic rejects the whole conversation.
    #[test]
    fn a_tool_call_and_its_result_survive_the_round_trip() {
        let out = anthropic(json!({
            "messages": [
                {"role": "user", "content": "read it"},
                {"role": "assistant", "content": "", "tool_calls": [{
                    "id": "toolu_42",
                    "type": "function",
                    "function": {"name": "read_page", "arguments": "{\"q\":\"x\"}"},
                }]},
                {"role": "tool", "tool_call_id": "toolu_42", "content": "the page says hello"},
            ],
        }));
        let turns = out["messages"].as_array().unwrap();
        assert_eq!(turns.len(), 3);

        let call = &turns[1]["content"][0];
        assert_eq!(call["type"], "tool_use");
        assert_eq!(call["id"], "toolu_42");
        assert_eq!(call["name"], "read_page");
        // Arguments are a JSON *string* on the OpenAI side and an object
        // on Anthropic's.
        assert_eq!(call["input"]["q"], "x");

        let result = &turns[2];
        assert_eq!(result["role"], "user", "Anthropic has no `tool` role");
        assert_eq!(result["content"][0]["type"], "tool_result");
        assert_eq!(result["content"][0]["tool_use_id"], "toolu_42");
        assert_eq!(result["content"][0]["content"], "the page says hello");
    }

    #[test]
    fn malformed_call_arguments_do_not_lose_the_turn() {
        let out = anthropic(json!({
            "messages": [{"role": "assistant", "content": "", "tool_calls": [{
                "id": "c1", "function": {"name": "t", "arguments": "not json"},
            }]}],
        }));
        assert_eq!(out["messages"][0]["content"][0]["input"], json!({}));
    }

    /// Keeping only `text` blocks meant a model that called a tool
    /// looked like one that said nothing: the loop ended having done
    /// nothing while the model believed it had acted.
    #[test]
    fn tool_use_comes_back_as_tool_calls() {
        let norm = translate_response(
            Dialect::AnthropicMessages,
            &json!({
                "id": "msg_1",
                "model": "claude-x",
                "stop_reason": "tool_use",
                "content": [
                    {"type": "text", "text": "Let me look."},
                    {"type": "tool_use", "id": "toolu_9", "name": "read_page",
                     "input": {"q": "x"}},
                ],
                "usage": {"input_tokens": 10, "output_tokens": 5},
            }),
        );
        let msg = &norm["choices"][0]["message"];
        assert_eq!(msg["content"], "Let me look.");
        let call = &msg["tool_calls"][0];
        assert_eq!(call["id"], "toolu_9");
        assert_eq!(call["function"]["name"], "read_page");
        // OpenAI wants arguments as a string.
        assert_eq!(call["function"]["arguments"], r#"{"q":"x"}"#);
        assert_eq!(norm["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn a_plain_answer_carries_no_tool_calls_key_at_all() {
        let norm = translate_response(
            Dialect::AnthropicMessages,
            &json!({
                "stop_reason": "end_turn",
                "content": [{"type": "text", "text": "hello"}],
                "usage": {"input_tokens": 1, "output_tokens": 1},
            }),
        );
        let msg = &norm["choices"][0]["message"];
        assert_eq!(msg["content"], "hello");
        assert!(
            msg.get("tool_calls").is_none(),
            "an absent key, not an empty array"
        );
        assert_eq!(norm["choices"][0]["finish_reason"], "stop");
    }
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
    fn stream_flag_reaches_both_dialects() {
        let body = json!({
            "model": "m", "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
        });
        let openai = build_upstream(&spec("openai"), "k", &body).unwrap();
        assert_eq!(openai.body["stream"], true, "compat passes through");
        let anthropic = build_upstream(&spec("anthropic"), "k", &body).unwrap();
        assert_eq!(anthropic.body["stream"], true, "re-rendered body keeps it");
        // And absent means absent — non-streaming behavior unchanged.
        let body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let anthropic = build_upstream(&spec("anthropic"), "k", &body).unwrap();
        assert!(anthropic.body.get("stream").is_none());
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
