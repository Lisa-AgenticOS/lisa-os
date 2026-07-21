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
            .map(|m| m.content.clone())
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
