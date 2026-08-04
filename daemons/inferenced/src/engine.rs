//! Engine abstraction. `lisa-inferenced` is a *supervisor, not an engine*
//! (`docs/PLAN.md` §5.1): real inference happens in child processes
//! (llama-server et al.); this trait is the seam between the scheduler/API
//! side and those children.

use crate::openai::ChatMessage;
use futures::Stream;
use futures::future::BoxFuture;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine unavailable: {0}")]
    Unavailable(String),
    #[error("preempted by an interactive request")]
    Preempted,
}

pub type TokenStream = Pin<Box<dyn Stream<Item = Result<String, EngineError>> + Send>>;

/// A stream of OpenAI-compat SSE `data:` PAYLOADS (the JSON, without the
/// `data: ` prefix and without the closing `[DONE]`), as produced by the
/// tools/passthrough lane. Same item type as [`TokenStream`] so the
/// scheduler admits it unchanged; a different name because an item here
/// is a whole chunk object, not a token.
pub type RawFrameStream = TokenStream;

/// Everything an engine needs for one generation.
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub messages: Vec<ChatMessage>,
    /// GBNF grammar constraining sampling (guided generation, §5.1/§5.6).
    pub grammar: Option<String>,
    pub max_tokens: Option<u32>,
}

pub trait Engine: Send + Sync {
    fn name(&self) -> &'static str;
    fn generate(&self, req: GenerateRequest) -> TokenStream;
    /// Embed texts into vectors (§5.1 `Session.Embed` / /v1/embeddings).
    fn embed(&self, texts: Vec<String>) -> BoxFuture<'static, Result<Vec<Vec<f32>>, EngineError>>;
    /// One non-streaming OpenAI-compat chat round-trip with the request
    /// body passed through verbatim. The typed token lane cannot carry
    /// `tools`/`tool_calls` (tool turns have null content and extra
    /// roles), so agent surfaces need the engine's own wire format.
    /// Engines without a native OpenAI endpoint reject.
    fn raw_chat(
        &self,
        _body: serde_json::Value,
    ) -> BoxFuture<'static, Result<serde_json::Value, EngineError>> {
        Box::pin(async {
            Err(EngineError::Unavailable(
                "tool calling is not supported by this engine".into(),
            ))
        })
    }
    /// The same passthrough round-trip, STREAMED.
    ///
    /// Issue #225. The tools lane used to answer `stream: true` with a
    /// 400, and the one client this daemon has — forge-harness, behind
    /// every Assistant window — always streams and always sends tools.
    /// Two halves of one system disagreeing about a request shape, with
    /// nothing that could notice.
    ///
    /// So the capability is unconditional here, at the trait, and there
    /// is no "unsupported" branch for an engine to fall into: the
    /// default runs `raw_chat` and re-frames the completion as a single
    /// chunk. An engine with no native SSE loses incremental deltas —
    /// the words arrive at once instead of one at a time — and loses
    /// nothing else, which is a far better trade than a 400 for a
    /// request the API said it accepted.
    ///
    /// Lazy like `generate`: the work starts on first poll and errors
    /// arrive as stream items, so the scheduler admits it the same way.
    fn raw_chat_stream(&self, body: serde_json::Value) -> RawFrameStream {
        let fut = self.raw_chat(body);
        Box::pin(async_stream::stream! {
            match fut.await {
                Ok(completion) => {
                    for frame in completion_as_chunks(&completion) {
                        yield Ok(frame);
                    }
                }
                Err(e) => yield Err(e),
            }
        })
    }

    /// Release resources (kill children). Pool eviction calls this.
    fn shutdown(&self) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}

/// One whole chat completion → the SSE chunk frames that carry the same
/// answer.
///
/// The message object becomes the `delta` verbatim: an OpenAI chunk's
/// delta and a completion's message have the same fields, `tool_calls`
/// included (with `arguments` as a JSON string in both), so a client
/// that folds streamed frames — forge-harness does — reassembles exactly
/// what the non-streaming caller would have received.
pub fn completion_as_chunks(completion: &serde_json::Value) -> Vec<String> {
    let choice = &completion["choices"][0];
    let delta = match &choice["message"] {
        serde_json::Value::Object(m) => serde_json::Value::Object(m.clone()),
        // No message at all: pass the body's own error through rather
        // than inventing an empty answer.
        _ => serde_json::json!({"content": ""}),
    };
    vec![
        serde_json::json!({
            "id": completion["id"],
            "object": "chat.completion.chunk",
            "created": completion["created"],
            "model": completion["model"],
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": choice["finish_reason"],
            }],
        })
        .to_string(),
    ]
}

/// Deterministic echo engine: proves the full plumbing (HTTP, SSE, CLI)
/// without weights. Streams word-sized tokens with a small delay so
/// streaming consumers actually exercise incremental rendering.
pub struct StubEngine;

impl Engine for StubEngine {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn generate(&self, req: GenerateRequest) -> TokenStream {
        let last_user = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.text())
            .unwrap_or_default();
        let reply = format!(
            "[lisa-inferenced stub] You said: \u{201c}{last_user}\u{201d}. \
             Run a real engine with --model (see `just smoke-real`)."
        );
        let tokens: Vec<String> = reply.split_inclusive(' ').map(str::to_string).collect();
        Box::pin(async_stream::stream! {
            for t in tokens {
                tokio::time::sleep(Duration::from_millis(5)).await;
                yield Ok(t);
            }
        })
    }

    /// The stub serves the tools lane too.
    ///
    /// Not scope creep: without it the passthrough lane was the one
    /// route with no modelless path at all, so nothing could exercise it
    /// on a dev host or in `just smoke` — which is how #225 shipped. It
    /// never calls a tool (there is no model here to decide to), so it
    /// answers the way the token lane does: by echoing.
    fn raw_chat(
        &self,
        body: serde_json::Value,
    ) -> BoxFuture<'static, Result<serde_json::Value, EngineError>> {
        Box::pin(async move {
            let last_user = body["messages"]
                .as_array()
                .and_then(|ms| {
                    ms.iter()
                        .rev()
                        .find(|m| m["role"] == "user")
                        .map(|m| match &m["content"] {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                })
                .unwrap_or_default();
            let reply = format!(
                "[lisa-inferenced stub] You said: \u{201c}{last_user}\u{201d}. \
                 Run a real engine with --model (see `just smoke-real`)."
            );
            let words = reply.split_whitespace().count();
            Ok(serde_json::json!({
                "id": format!("chatcmpl-lisa-stub-{}", crate::openai::unix_now()),
                "object": "chat.completion",
                "created": crate::openai::unix_now(),
                "model": body["model"],
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": reply},
                    "finish_reason": "stop",
                }],
                "usage": {"prompt_tokens": 0, "completion_tokens": words, "total_tokens": words},
            }))
        })
    }

    fn embed(&self, texts: Vec<String>) -> BoxFuture<'static, Result<Vec<Vec<f32>>, EngineError>> {
        // Deterministic 8-dim vectors from an FNV-1a rolling hash: enough
        // to test plumbing, dimensionality, and determinism modelless.
        Box::pin(async move {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut h: u64 = 0xcbf29ce484222325;
                    let mut v = Vec::with_capacity(8);
                    for (i, b) in t.bytes().chain(0u8..8).enumerate() {
                        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
                        if i >= t.len() {
                            v.push((h % 2000) as f32 / 1000.0 - 1.0);
                        }
                    }
                    v
                })
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn stub_echoes_last_user_message() {
        let engine = StubEngine;
        let req = GenerateRequest {
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: "policy".into(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: "hello lisa".into(),
                },
            ],
            grammar: None,
            max_tokens: None,
        };
        let text: String = engine
            .generate(req)
            .map(|t| t.unwrap())
            .collect::<Vec<_>>()
            .await
            .join("");
        assert!(text.contains("hello lisa"), "got: {text}");
    }
}
