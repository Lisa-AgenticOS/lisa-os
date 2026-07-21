//! llama-server supervision + streaming proxy (`docs/PLAN.md` §5.1).
//!
//! Supervisor, not engine: llama-server runs as a child process; a crash
//! is a restart, not a system-AI outage. Every request goes through
//! `ensure_running`, so a kill -9'd child is respawned on the next call
//! (§5.1 acceptance: service restored < 5 s). M1 remainder: one child per
//! resident model, LoRA hot-swap, VRAM budget arbitration, QoS scheduler.

use crate::config::LlamaConfig;
use crate::engine::{Engine, EngineError, TokenStream};
use crate::openai::ChatMessage;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

pub struct LlamaEngine {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: LlamaConfig,
    child: Mutex<Option<Child>>,
    client: reqwest::Client,
}

impl LlamaEngine {
    pub fn new(cfg: LlamaConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                child: Mutex::new(None),
                client: reqwest::Client::new(),
            }),
        }
    }

    pub async fn ensure_running(&self) -> Result<(), EngineError> {
        self.inner.ensure_running().await
    }

    pub async fn shutdown(&self) {
        if let Some(mut child) = self.inner.child.lock().await.take() {
            let _ = child.kill().await;
        }
    }
}

impl Inner {
    fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", self.cfg.port)
    }

    /// Spawn llama-server if absent or dead, then wait until it reports
    /// healthy. llama-server binds its port immediately and serves 503
    /// while the model loads, so readiness means /health == 200, never a
    /// bare TCP connect.
    async fn ensure_running(&self) -> Result<(), EngineError> {
        let mut guard = self.child.lock().await;
        if let Some(child) = guard.as_mut()
            && child
                .try_wait()
                .map_err(|e| EngineError::Unavailable(e.to_string()))?
                .is_none()
        {
            drop(guard);
            return self.wait_healthy().await;
        }

        let model =
            self.cfg.model_path.as_ref().ok_or_else(|| {
                EngineError::Unavailable("llama.model_path not configured".into())
            })?;
        info!(model = %model.display(), port = self.cfg.port, "spawning llama-server");
        let child = Command::new(&self.cfg.server_bin)
            .arg("--model")
            .arg(model)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(self.cfg.port.to_string())
            .args(&self.cfg.extra_args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                EngineError::Unavailable(format!("spawning {}: {e}", self.cfg.server_bin.display()))
            })?;
        *guard = Some(child);
        drop(guard);
        self.wait_healthy().await
    }

    /// Poll llama-server's /health until 200 (model loaded). ~60 s budget
    /// to cover cold model loads.
    async fn wait_healthy(&self) -> Result<(), EngineError> {
        let url = format!("http://127.0.0.1:{}/health", self.cfg.port);
        for _ in 0..600 {
            if let Ok(r) = self.client.get(&url).send().await
                && r.status().is_success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(EngineError::Unavailable(format!(
            "llama-server not healthy on {url}"
        )))
    }

    /// One streaming completion request against the child; yields token
    /// deltas parsed from its OpenAI-compat SSE stream.
    async fn open_stream(
        &self,
        messages: &[ChatMessage],
    ) -> Result<reqwest::Response, EngineError> {
        let response = self
            .client
            .post(self.endpoint())
            .json(&serde_json::json!({
                "messages": messages,
                "stream": true,
            }))
            .send()
            .await
            .map_err(|e| EngineError::Unavailable(format!("llama-server request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(EngineError::Unavailable(format!(
                "llama-server returned {status}: {}",
                body.chars().take(200).collect::<String>()
            )));
        }
        Ok(response)
    }
}

impl Engine for LlamaEngine {
    fn name(&self) -> &'static str {
        "llama"
    }

    fn generate(&self, messages: Vec<ChatMessage>) -> TokenStream {
        let inner = Arc::clone(&self.inner);
        Box::pin(async_stream::stream! {
            if let Err(e) = inner.ensure_running().await {
                yield Err(e);
                return;
            }
            // One retry: the child may have died since the health check
            // (crash-restore path).
            let response = match inner.open_stream(&messages).await {
                Ok(r) => Ok(r),
                Err(first) => {
                    warn!("llama-server request failed ({first}); respawning and retrying once");
                    match inner.ensure_running().await {
                        Ok(()) => inner.open_stream(&messages).await,
                        Err(e) => Err(e),
                    }
                }
            };
            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };

            // SSE: reassemble lines across chunk boundaries, yield deltas.
            use futures::StreamExt;
            let mut bytes = response.bytes_stream();
            let mut buf = Vec::new();
            'outer: while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(EngineError::Unavailable(format!("stream error: {e}")));
                        return;
                    }
                };
                buf.extend_from_slice(&chunk);
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line);
                    let Some(data) = line.trim().strip_prefix("data: ") else {
                        continue;
                    };
                    if data == "[DONE]" {
                        break 'outer;
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(token) = v["choices"][0]["delta"]["content"].as_str()
                        && !token.is_empty()
                    {
                        yield Ok(token.to_string());
                    }
                }
            }
        })
    }
}
