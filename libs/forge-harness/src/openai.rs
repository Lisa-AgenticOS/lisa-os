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
///
/// `url` is either an `http(s)://` endpoint or `unix:<path>` — the same
/// API served over a unix socket, which is the only shape a caller
/// confined to `RestrictAddressFamilies=AF_UNIX` can use (#288, and see
/// [`crate::unix_http`] for why that is the only confinement a user unit
/// gets).
pub struct OpenAiBackend {
    pub url: String,
    pub model: Option<String>,
}

impl OpenAiBackend {
    /// `(agent, endpoint)` for one request against this backend.
    ///
    /// Built per call, exactly as the previous `ureq::post()` was —
    /// that free function makes a fresh agent every time — so moving to
    /// an explicit agent changes the transport and nothing else.
    fn chat_endpoint(&self) -> (ureq::Agent, String) {
        let (agent, base) = crate::unix_http::agent_for(&self.url);
        (agent, format!("{base}/v1/chat/completions"))
    }
}

impl Backend for OpenAiBackend {
    fn next_action(
        &mut self,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Result<AgentAction, ForgeError> {
        let body = request_body(self.model.as_deref(), messages, tools);
        let (agent, endpoint) = self.chat_endpoint();
        let mut response = agent
            .post(&endpoint)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(&body)
            .map_err(|e| ForgeError::Backend(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.body_mut().read_to_string().unwrap_or_default();
            return Err(ForgeError::Backend(backend_refusal(status.as_u16(), &text)));
        }
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
        cancel: &crate::Cancel,
    ) -> Result<AgentAction, ForgeError> {
        let body = streaming_request_body(self.model.as_deref(), messages, tools);
        let (agent, endpoint) = self.chat_endpoint();
        let mut response = agent
            .post(&endpoint)
            .config()
            // Read the refusal instead of throwing it away (#225). ureq's
            // default turns a 4xx into `http status: 400` and drops the
            // body — which is where the daemon put the sentence that
            // said what was actually wrong.
            .http_status_as_error(false)
            .build()
            .send_json(&body)
            .map_err(|e| ForgeError::Backend(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.body_mut().read_to_string().unwrap_or_default();
            return Err(ForgeError::Backend(backend_refusal(status.as_u16(), &text)));
        }

        let mut acc = Accumulated::default();
        let reader = BufReader::new(response.into_body().into_reader());
        for line in reader.lines() {
            // Stop, honoured mid-answer (#227). Between frames, so the
            // socket is dropped at a clean point; abandoning a
            // half-generated sentence costs nothing, which is exactly
            // why this is the one place the loop interrupts itself.
            if cancel.is_cancelled() {
                return Err(ForgeError::Cancelled);
            }
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

/// A refused request, in words a person can act on.
///
/// Issue #225 reached the Assistant window as `backend: http status:
/// 400` — a sentence that tells its reader nothing except that
/// something is broken, and nothing at all about what. The daemon had in
/// fact said exactly what was wrong; ureq's default turns a non-2xx into
/// a status-only error and discards the body it came with.
///
/// So the body is read, and its `error.message` — the OpenAI shape every
/// backend here speaks — becomes the message. A body in some other shape
/// is shown as-is rather than dropped: unreadable is still more than a
/// number.
pub fn backend_refusal(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.trim().chars().take(300).collect());
    if detail.is_empty() {
        return format!("the model service refused the request ({status})");
    }
    format!("the model service refused the request ({status}): {detail}")
}

/// The EXACT body this backend puts on the wire for a streaming turn —
/// tools attached, `stream: true`.
///
/// Public, and public for one reason (issue #225). This backend always
/// streams and always sends `tools`; lisa-inferenced routed a non-empty
/// `tools` array to a lane whose first act was to refuse `stream: true`
/// with a 400. Two halves of one system disagreeing about the request
/// shape, with nothing that could notice — every local-model run in the
/// Assistant died as `backend: http status: 400`.
///
/// So the producer is exported and lisa-inferenced's own test suite
/// feeds it to its own router (`daemons/inferenced/tests/api.rs`). The
/// two sides cannot drift apart silently again: changing what this
/// function emits changes what that test sends.
pub fn streaming_request_body(
    model: Option<&str>,
    messages: &[Message],
    tools: &[ToolSpec],
) -> Value {
    let mut body = request_body(model, messages, tools);
    body["stream"] = json!(true);
    body
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
    /// An error the SERVER reported inside the stream.
    ///
    /// lisa-inferenced signals a mid-stream failure as a frame carrying
    /// `{"error": {"message": …}}` rather than by breaking the HTTP
    /// response, which is the only thing it can do once the 200 and the
    /// first chunks have gone out. Nothing here read that field, so an
    /// engine that failed halfway arrived as an empty `Done("")` — the
    /// assistant printing nothing at all and calling it an answer.
    pub error: Option<String>,
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
    // A failure the server could only report in-band, because the 200
    // already went out. Kept, not swallowed: the alternative is an empty
    // answer that looks like the model had nothing to say.
    if let Some(msg) = v["error"]["message"].as_str() {
        acc.error = Some(msg.to_string());
        return true;
    }
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
    if let Some(msg) = acc.error {
        return Err(ForgeError::Backend(msg));
    }
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

    /// A failure the server could only report in-band.
    ///
    /// Once the 200 and the first chunks have gone out there is no
    /// status code left to send, so lisa-inferenced puts the failure in
    /// a frame. Nothing here read that field, so a run whose engine died
    /// halfway came back as `Done("")` — the Assistant printing an empty
    /// bubble and calling it an answer (#225).
    #[test]
    fn an_error_frame_ends_the_stream_as_an_error_not_as_an_empty_answer() {
        let (acc, seen) = fold_all(&[
            r#"data: {"choices":[{"delta":{"content":"parti"}}]}"#,
            r#"data: {"error":{"message":"llama-server 500: out of memory"}}"#,
            r#"data: {"choices":[{"delta":{"content":"AFTER"}}]}"#,
        ]);
        assert_eq!(seen, "parti", "what arrived before the failure is kept");
        assert_eq!(
            acc.error.as_deref(),
            Some("llama-server 500: out of memory")
        );
        match action_from(acc) {
            Err(ForgeError::Backend(msg)) => assert!(
                msg.contains("out of memory"),
                "the reason has to survive: {msg}"
            ),
            other => panic!("an error frame must not become an answer: {other:?}"),
        }
    }

    /// The message a person actually reads when a request is refused.
    ///
    /// It said `backend: http status: 400` for a week — true, and it
    /// tells its reader nothing. The daemon's own sentence was in the
    /// body ureq threw away.
    #[test]
    fn a_refusal_carries_the_daemons_own_words_not_just_a_number() {
        let msg = backend_refusal(
            400,
            r#"{"error":{"message":"tools must be an array of tool definitions","type":"invalid_request_error"}}"#,
        );
        assert!(msg.contains("tools must be an array"), "{msg}");
        assert!(
            msg.contains("400"),
            "the status is still worth having: {msg}"
        );

        // A body in some other shape is shown, not dropped: unreadable
        // is still more than a number.
        let plain = backend_refusal(
            413,
            "Failed to buffer the request body: length limit exceeded",
        );
        assert!(plain.contains("length limit exceeded"), "{plain}");

        // And an empty body still produces a sentence.
        let bare = backend_refusal(502, "");
        assert!(bare.contains("502"), "{bare}");
        assert!(bare.len() > 10, "a bare status is not a message: {bare}");
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

/// Stop, against a socket that never stops talking (issue #227).
///
/// The claim under test is the one a person makes with the button: that
/// pressing Stop stops the answer arriving, not that it stops the NEXT
/// answer. Everything else in this file folds frames from a string; this
/// needs a real connection, because "the read loop is still blocked in
/// `lines()`" is precisely the failure a string cannot reproduce.
#[cfg(test)]
mod cancel_tests {
    use super::*;
    use crate::Backend;
    use std::io::{BufRead, BufReader as StdBufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// How long the fake model keeps talking. Long enough that reading
    /// it to the end is unmistakably different from stopping at frame
    /// three; bounded, so a regression FAILS rather than hanging the
    /// suite. (Unbounded was tried: without the cancellation check the
    /// call never returns, which is a true red and a terrible test.)
    const FRAMES: usize = 400;
    const FRAME_GAP: Duration = Duration::from_millis(5);

    /// An SSE endpoint that dribbles chunks the way a model does, and
    /// keeps going long past the moment somebody presses Stop.
    fn endless_sse_server(sent: Arc<AtomicUsize>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            // Drain the request head so the client's write completes.
            let mut reader = StdBufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            let mut len = 0usize;
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" {
                    break;
                }
                line.clear();
            }
            let mut body = vec![0u8; len];
            let _ = std::io::Read::read_exact(&mut reader, &mut body);

            let mut stream = stream;
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                  Connection: close\r\n\r\n",
            );
            for _ in 0..FRAMES {
                let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"tick \"}}]}\n\n";
                if stream.write_all(frame.as_bytes()).is_err() || stream.flush().is_err() {
                    return; // the client hung up — which is the point
                }
                sent.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(FRAME_GAP);
            }
            let _ = stream.write_all(b"data: [DONE]\n\n");
        });
        format!("http://{addr}")
    }

    #[test]
    fn stop_interrupts_an_answer_that_is_still_arriving() {
        let sent = Arc::new(AtomicUsize::new(0));
        let url = endless_sse_server(Arc::clone(&sent));
        let mut backend = OpenAiBackend {
            url,
            model: Some("m".into()),
        };
        let cancel = crate::Cancel::default();

        // Stop, from the other side, the moment words start arriving —
        // the same thing the button does from the D-Bus thread.
        let flag = cancel.clone();
        let mut seen = 0usize;
        let mut sink = |_: &str| {
            seen += 1;
            if seen == 3 {
                flag.cancel();
            }
        };

        let started = Instant::now();
        let outcome =
            backend.next_action_streaming(&[Message::user("hello")], &[], &mut sink, &cancel);
        let elapsed = started.elapsed();

        assert!(
            matches!(outcome, Err(ForgeError::Cancelled)),
            "a stopped stream must report that it stopped: {outcome:?}"
        );
        // Reading to the end takes FRAMES × FRAME_GAP; stopping at the
        // third frame takes about three gaps. Half the full run still
        // fails any implementation that only notices Stop once the
        // stream has finished.
        assert!(
            elapsed < FRAME_GAP * (FRAMES as u32) / 2,
            "Stop did not interrupt the read: {elapsed:?}"
        );
        assert!(seen >= 3, "the words before Stop still arrived: {seen}");
        assert!(
            sent.load(Ordering::SeqCst) < FRAMES,
            "the whole answer arrived anyway, so nothing was interrupted"
        );
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

/// The whole backend against a unix socket, not just the transport
/// (#288).
///
/// `unix_http`'s own tests prove bytes move; this proves the thing
/// `lisa-harnessd` actually does — POST a tools+stream turn and fold the
/// SSE frames into deltas — works when the process is forbidden to open
/// an IP socket at all. The server answers with `Transfer-Encoding:
/// chunked`, which is what a streaming lane really sends and what a
/// hand-rolled `Connection: close` client would have got wrong.
#[cfg(test)]
mod unix_socket_tests {
    use super::*;
    use crate::Backend;
    use std::io::Write;
    use std::os::unix::net::UnixListener;

    #[test]
    fn streams_a_turn_over_the_inferenced_unix_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("inferenced.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; len];
            std::io::Read::read_exact(&mut reader, &mut body).unwrap();

            let mut stream = stream;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            for frame in [
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ] {
                write!(stream, "{:x}\r\n{frame}\r\n", frame.len()).unwrap();
                stream.flush().unwrap();
            }
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
            (head, String::from_utf8(body).unwrap())
        });

        let mut backend = OpenAiBackend {
            url: format!("unix:{}", sock.display()),
            model: Some("coder".into()),
        };
        let mut deltas = String::new();
        let mut sink = |d: &str| deltas.push_str(d);
        let action = backend
            .next_action_streaming(
                &[Message::user("hi")],
                &crate::tools::tool_specs(),
                &mut sink,
                &crate::Cancel::default(),
            )
            .expect("a unix-socket turn must complete");

        let (head, body) = server.join().unwrap();
        assert!(
            head.starts_with("POST /v1/chat/completions HTTP/1.1"),
            "{head}"
        );
        let sent: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["model"], "coder");
        assert_eq!(deltas, "Hello there");
        match action {
            AgentAction::Done(text) => assert_eq!(text, "Hello there"),
            other => panic!("unexpected action: {other:?}"),
        }
    }
}
