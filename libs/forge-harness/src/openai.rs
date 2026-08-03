//! OpenAI-compat backend (lisa-inferenced or any compatible endpoint),
//! speaking function/tool calling. The model's file operations are
//! constrained by the tools' JSON schemas — grammar-valid by
//! construction, never free-form text the harness has to parse.

use crate::agent::{AgentAction, Message, Role};
use crate::tools::{ToolCall, ToolSpec};
use crate::{Backend, ForgeError};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};

/// OpenAI-compat backend (lisa-inferenced or any compatible endpoint).
pub struct OpenAiBackend {
    pub url: String,
    pub model: Option<String>,
}

impl Backend for OpenAiBackend {
    fn next_action(
        &mut self,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Result<AgentAction, ForgeError> {
        let body = request_body(self.model.as_deref(), messages, tools);
        let endpoint = format!("{}/v1/chat/completions", self.url.trim_end_matches('/'));
        let mut response = ureq::post(&endpoint)
            .send_json(&body)
            .map_err(|e| ForgeError::Backend(e.to_string()))?;
        let json: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| ForgeError::Backend(e.to_string()))?;
        parse_response(&json)
    }

    fn next_action_streaming(
        &mut self,
        messages: &[Message],
        tools: &[ToolSpec],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<AgentAction, ForgeError> {
        let mut body = request_body(self.model.as_deref(), messages, tools);
        body["stream"] = json!(true);
        let endpoint = format!("{}/v1/chat/completions", self.url.trim_end_matches('/'));
        let response = ureq::post(&endpoint)
            .send_json(&body)
            .map_err(|e| ForgeError::Backend(e.to_string()))?;

        let mut acc = Accumulated::default();
        let reader = BufReader::new(response.into_body().into_reader());
        for line in reader.lines() {
            let line = line.map_err(|e| ForgeError::Backend(format!("reading stream: {e}")))?;
            if line.trim().is_empty() {
                continue;
            }
            if fold_frame(&mut acc, &line, on_delta) {
                break;
            }
        }
        action_from(acc)
    }
}

/// The chat-completions request: history plus tool declarations, one
/// tool call at a time.
fn request_body(model: Option<&str>, messages: &[Message], tools: &[ToolSpec]) -> Value {
    json!({
        "model": model,
        "messages": wire_messages(messages),
        "tools": tools.iter().map(ToolSpec::wire).collect::<Vec<_>>(),
        "tool_choice": "auto",
        "parallel_tool_calls": false,
    })
}

/// What a stream added up to.
#[derive(Debug, Default, PartialEq)]
pub struct Accumulated {
    pub content: String,
    pub call_id: Option<String>,
    pub call_name: Option<String>,
    /// Arguments arrive as fragments across many frames and are only
    /// valid JSON once concatenated — parsing early is the classic
    /// streaming tool-call bug.
    pub call_args: String,
}

/// Fold one SSE `data:` payload into the running result.
///
/// Pure, because everything that can go wrong with streaming tool calls
/// lives here: fragments split mid-token, a name in one frame and its
/// arguments in twenty, `[DONE]` arriving mid-object.
pub fn fold_frame(acc: &mut Accumulated, frame: &str, on_delta: &mut dyn FnMut(&str)) -> bool {
    let payload = match frame.strip_prefix("data:") {
        Some(p) => p.trim(),
        None => return false,
    };
    if payload == "[DONE]" {
        return true;
    }
    let Ok(v) = serde_json::from_str::<Value>(payload) else {
        // A partial or malformed frame is skipped, not fatal: the
        // stream is still running and the next frame may complete it.
        return false;
    };
    let delta = &v["choices"][0]["delta"];
    if let Some(text) = delta["content"].as_str()
        && !text.is_empty()
    {
        acc.content.push_str(text);
        on_delta(text);
    }
    if let Some(calls) = delta["tool_calls"].as_array() {
        for c in calls {
            if let Some(id) = c["id"].as_str() {
                acc.call_id = Some(id.to_string());
            }
            if let Some(name) = c["function"]["name"].as_str()
                && !name.is_empty()
            {
                acc.call_name = Some(name.to_string());
            }
            if let Some(args) = c["function"]["arguments"].as_str() {
                acc.call_args.push_str(args);
            }
        }
    }
    false
}

/// Turn an accumulated stream into the loop's decision.
pub fn action_from(acc: Accumulated) -> Result<AgentAction, ForgeError> {
    match acc.call_name {
        Some(name) => {
            let args = if acc.call_args.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&acc.call_args).map_err(|e| {
                    ForgeError::Backend(format!("tool call arguments were not JSON: {e}"))
                })?
            };
            Ok(AgentAction::Call(ToolCall {
                id: acc.call_id.unwrap_or_else(|| "1".into()),
                name,
                args,
            }))
        }
        None => Ok(AgentAction::Done(acc.content)),
    }
}

/// A user turn's `content`: a bare string, or — when the person
/// attached something (issue #209) — OpenAI content parts with THEIR
/// WORDS FIRST.
///
/// Order is load-bearing. A model handed the image before the question
/// answers the question it invented for the image. And with no
/// attachments the result must be byte-identical to a plain string:
/// every text-only engine on the far side understands that shape and
/// only that shape.
fn user_content(m: &Message) -> Value {
    if m.attachments.is_empty() {
        return json!(m.content);
    }
    let mut parts = vec![json!({"type": "text", "text": m.content})];
    // Forwarded verbatim — the harness does not re-model a provider's
    // part schema, it passes it through (see inferenced `Content::Parts`).
    parts.extend(m.attachments.iter().cloned());
    Value::Array(parts)
}

/// Map the internal history onto the OpenAI message shapes.
fn wire_messages(messages: &[Message]) -> Value {
    messages
        .iter()
        .map(|m| match m.role {
            Role::System => json!({"role": "system", "content": m.content}),
            Role::User => json!({"role": "user", "content": user_content(m)}),
            Role::Assistant => match &m.tool_call {
                Some(call) => json!({
                    "role": "assistant",
                    "content": if m.content.is_empty() { Value::Null } else { json!(m.content) },
                    "tool_calls": [{
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.args.to_string(),
                        }
                    }],
                }),
                None => json!({"role": "assistant", "content": m.content}),
            },
            Role::Tool => json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id,
                "content": m.content,
            }),
        })
        .collect()
}

/// A response is either a tool call (the first one — parallel calls are
/// disabled) or, with no tool calls, the model's done signal.
fn parse_response(json: &Value) -> Result<AgentAction, ForgeError> {
    let message = &json["choices"][0]["message"];
    if message.is_null() {
        return Err(ForgeError::Backend(format!("no message in {json}")));
    }
    if let Some(calls) = message["tool_calls"].as_array()
        && let Some(first) = calls.first()
    {
        let name = first["function"]["name"]
            .as_str()
            .ok_or_else(|| ForgeError::Backend(format!("tool call without a name in {json}")))?;
        // `arguments` is a JSON *string* on the wire; some endpoints
        // hand back an object instead — accept both.
        let args = match &first["function"]["arguments"] {
            Value::String(s) => serde_json::from_str(s)
                .map_err(|e| ForgeError::Backend(format!("bad tool arguments: {e}")))?,
            other if other.is_object() => other.clone(),
            _ => json!({}),
        };
        return Ok(AgentAction::Call(ToolCall {
            id: first["id"].as_str().unwrap_or("call_0").to_string(),
            name: name.to_string(),
            args,
        }));
    }
    let content = message["content"]
        .as_str()
        .ok_or_else(|| ForgeError::Backend(format!("no content in {json}")))?;
    Ok(AgentAction::Done(content.to_string()))
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    fn fold_all(frames: &[&str]) -> (Accumulated, String) {
        let mut acc = Accumulated::default();
        let mut seen = String::new();
        for f in frames {
            let mut sink = |d: &str| seen.push_str(d);
            if fold_frame(&mut acc, f, &mut sink) {
                break;
            }
        }
        (acc, seen)
    }

    #[test]
    fn text_deltas_arrive_in_order_and_accumulate() {
        let (acc, seen) = fold_all(&[
            r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"lo t"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"here"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(acc.content, "Hello there");
        assert_eq!(seen, "Hello there", "the watcher sees every fragment");
    }

    /// The classic streaming tool-call bug: arguments arrive as
    /// fragments and are only valid JSON once concatenated. Parsing a
    /// fragment early gives an error on perfectly good input.
    #[test]
    fn tool_call_arguments_are_only_parsed_once_complete() {
        let (acc, seen) = fold_all(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"c1","function":{"name":"read_page"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{\"q\""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":": \"x\"}"}}]}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(acc.call_name.as_deref(), Some("read_page"));
        assert_eq!(acc.call_args, r#"{"q": "x"}"#);
        assert_eq!(seen, "", "a tool call is not text to show the user");

        let action = action_from(acc).unwrap();
        match action {
            AgentAction::Call(c) => {
                assert_eq!(c.name, "read_page");
                assert_eq!(c.args["q"], "x");
                assert_eq!(c.id, "c1");
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn no_tool_call_means_done_with_the_text() {
        let (acc, _) = fold_all(&[r#"data: {"choices":[{"delta":{"content":"all set"}}]}"#]);
        match action_from(acc).unwrap() {
            AgentAction::Done(text) => assert_eq!(text, "all set"),
            other => panic!("expected done, got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_frame_is_skipped_rather_than_fatal() {
        // A truncated frame mid-stream must not lose the frames around
        // it: the stream is still running.
        let (acc, _) = fold_all(&[
            r#"data: {"choices":[{"delta":{"content":"a"}}]}"#,
            "data: {not json",
            "garbage without the data prefix",
            r#"data: {"choices":[{"delta":{"content":"b"}}]}"#,
        ]);
        assert_eq!(acc.content, "ab");
    }

    #[test]
    fn done_stops_the_fold() {
        let (acc, _) = fold_all(&[
            r#"data: {"choices":[{"delta":{"content":"kept"}}]}"#,
            "data: [DONE]",
            r#"data: {"choices":[{"delta":{"content":"AFTER"}}]}"#,
        ]);
        assert_eq!(acc.content, "kept");
    }

    #[test]
    fn empty_arguments_become_an_empty_object_not_a_parse_error() {
        let acc = Accumulated {
            call_name: Some("run_tests".into()),
            ..Accumulated::default()
        };
        match action_from(acc).unwrap() {
            AgentAction::Call(c) => assert_eq!(c.args, json!({})),
            other => panic!("expected a call, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ToolSpec> {
        crate::tools::tool_specs()
    }

    #[test]
    fn request_carries_tools_and_wire_history() {
        let call = ToolCall {
            id: "c1".into(),
            name: "write_file".into(),
            args: json!({"path": "a.txt", "content": "hi"}),
        };
        let history = vec![
            Message::system("sys"),
            Message::user("task"),
            Message::assistant_call(call),
            Message::tool_result("c1", "wrote a.txt"),
            Message::user("findings"),
        ];
        let body = request_body(Some("coder"), &history, &specs());
        assert_eq!(body["model"], "coder");
        assert_eq!(body["parallel_tool_calls"], false);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 7);
        assert_eq!(tools[0]["type"], "function");
        assert!(tools.iter().any(|t| t["function"]["name"] == "edit_file"));

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "write_file");
        // Arguments cross the wire as a JSON string.
        let arguments: Value = serde_json::from_str(
            msgs[2]["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(arguments, json!({"path": "a.txt", "content": "hi"}));
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "c1");
    }

    /// Issue #209's last mile: a user turn carrying an attachment leaves
    /// as OpenAI CONTENT PARTS, with the person's words FIRST.
    ///
    /// Order is the whole point — a model handed the image before the
    /// question answers the question it invented for the image. And a
    /// message with no attachments must stay a plain string, because
    /// that is what every text-only engine on the far side understands.
    #[test]
    fn attachments_turn_a_user_message_into_parts_text_first() {
        let png = json!({
            "type": "image_url",
            "image_url": {"url": "data:image/png;base64,AAAA"},
        });
        let mut m = Message::user("what is this?");
        m.attachments = vec![png.clone()];
        let wire = wire_messages(&[Message::user("plain"), m]);
        let msgs = wire.as_array().unwrap();

        // Without attachments: byte-identical to before — a bare string.
        assert_eq!(msgs[0]["content"], json!("plain"));

        // With them: parts, the text one first.
        let parts = msgs[1]["content"].as_array().expect("content parts");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], json!({"type": "text", "text": "what is this?"}));
        assert_eq!(parts[1], png, "the part is forwarded verbatim");
        assert_eq!(msgs[1]["role"], "user");
    }

    /// Attachments belong to the person's turn. An assistant or tool
    /// message must never grow parts from them: the loop appends tool
    /// results by the thousand, and a stray image on one of those is an
    /// image the model was never shown by a human.
    #[test]
    fn only_a_user_turn_carries_attachments() {
        let part = json!({"type": "image_url", "image_url": {"url": "data:image/png;base64,AA"}});
        let mut a = Message::assistant_text("hi");
        a.attachments = vec![part.clone()];
        let mut t = Message::tool_result("c1", "read it");
        t.attachments = vec![part];
        let wire = wire_messages(&[a, t]);
        let msgs = wire.as_array().unwrap();
        assert_eq!(msgs[0]["content"], json!("hi"));
        assert_eq!(msgs[1]["content"], json!("read it"));
    }

    #[test]
    fn parses_a_tool_call_response() {
        let response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_9",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\": \"lib/main.dart\"}"
                        }
                    }]
                }
            }]
        });
        let action = parse_response(&response).unwrap();
        assert_eq!(
            action,
            AgentAction::Call(ToolCall {
                id: "call_9".into(),
                name: "read_file".into(),
                args: json!({"path": "lib/main.dart"}),
            })
        );
    }

    #[test]
    fn parses_object_shaped_arguments_too() {
        let response = json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "x",
                        "function": {"name": "run_tests", "arguments": {}}
                    }]
                }
            }]
        });
        match parse_response(&response).unwrap() {
            AgentAction::Call(call) => assert_eq!(call.name, "run_tests"),
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_reply_is_the_done_signal() {
        let response = json!({
            "choices": [{"message": {"role": "assistant", "content": "all done, analyzer clean"}}]
        });
        assert_eq!(
            parse_response(&response).unwrap(),
            AgentAction::Done("all done, analyzer clean".into())
        );
    }
}
