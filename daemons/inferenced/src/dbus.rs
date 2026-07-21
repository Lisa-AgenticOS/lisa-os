//! D-Bus surface: org.lisa.Inference1 (`docs/PLAN.md` §5.1, Appendix A).
//!
//! OpenSession(options) → (session object path, read fd). Tokens stream
//! over the fd as raw UTF-8; the daemon closes its write end when the
//! generation completes, so EOF is end-of-message. Guided generation
//! rides in Generate's params ("schema": JSON Schema string → GBNF).
//! M2 attaches portal identity/grants here; signals (TokenUsage,
//! ModelSwapped, Preempted) land with the Ledger wiring.
//!
//! Tested over zbus peer-to-peer connections (no bus daemon needed);
//! session-bus registration is used on real systems.

use crate::engine::{Engine, GenerateRequest};
use crate::openai::ChatMessage;
use crate::scheduler::{Priority, Scheduler};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub struct Inference1 {
    pub engine: Arc<dyn Engine>,
    pub scheduler: Arc<Scheduler>,
    next_session: AtomicU64,
}

impl Inference1 {
    pub fn new(engine: Arc<dyn Engine>, scheduler: Arc<Scheduler>) -> Self {
        Self {
            engine,
            scheduler,
            next_session: AtomicU64::new(1),
        }
    }
}

#[zbus::interface(name = "org.lisa.Inference1")]
impl Inference1 {
    /// Liveness probe.
    fn ping(&self) -> String {
        format!("lisa-inferenced {}", env!("CARGO_PKG_VERSION"))
    }

    /// Open a session. Returns the session object path and the read end
    /// of the token pipe. Options (a{sv}) are reserved for model_hint,
    /// memory_ns, and scopes (M2 portal wiring).
    async fn open_session(
        &self,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<(OwnedObjectPath, zbus::zvariant::OwnedFd)> {
        let id = self.next_session.fetch_add(1, Ordering::Relaxed);
        let path = OwnedObjectPath::try_from(format!("/org/lisa/Inference1/session/{id}"))
            .expect("session path is valid");

        let (reader, writer) =
            std::io::pipe().map_err(|e| zbus::fdo::Error::Failed(format!("pipe: {e}")))?;
        let writer = tokio::net::unix::pipe::Sender::from_owned_fd(writer.into())
            .map_err(|e| zbus::fdo::Error::Failed(format!("pipe writer: {e}")))?;

        let session = Session {
            engine: Arc::clone(&self.engine),
            scheduler: Arc::clone(&self.scheduler),
            writer: Arc::new(Mutex::new(Some(writer))),
            task: Arc::new(Mutex::new(None)),
        };
        server
            .at(&path, session)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("registering session: {e}")))?;

        let fd: std::os::fd::OwnedFd = reader.into();
        Ok((path, fd.into()))
    }
}

pub struct Session {
    engine: Arc<dyn Engine>,
    scheduler: Arc<Scheduler>,
    writer: Arc<Mutex<Option<tokio::net::unix::pipe::Sender>>>,
    task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[zbus::interface(name = "org.lisa.Inference1.Session")]
impl Session {
    /// Generate from `prompt`; tokens stream over the session fd, which
    /// is closed at end-of-message. Params: "schema" (s, JSON Schema →
    /// grammar-constrained output), "max_tokens" (u), "priority"
    /// ("interactive" | "background").
    async fn generate(
        &self,
        prompt: String,
        params: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        let grammar = match params.get("schema") {
            Some(v) => {
                let raw: &str = v
                    .downcast_ref()
                    .map_err(|_| zbus::fdo::Error::InvalidArgs("schema must be a string".into()))?;
                let schema: serde_json::Value = serde_json::from_str(raw)
                    .map_err(|e| zbus::fdo::Error::InvalidArgs(format!("schema: {e}")))?;
                Some(liblisa::grammar::json_schema_to_gbnf(&schema).map_err(|e| {
                    zbus::fdo::Error::InvalidArgs(format!("schema not supported: {e}"))
                })?)
            }
            None => None,
        };
        let max_tokens = params
            .get("max_tokens")
            .and_then(|v| u32::try_from(Value::from(v.clone())).ok());
        let priority = match params.get("priority") {
            Some(v) => Priority::parse(v.downcast_ref::<&str>().ok()),
            None => Priority::Interactive,
        };

        let stream = self.engine.generate(GenerateRequest {
            messages: vec![ChatMessage {
                role: "user".into(),
                content: prompt,
            }],
            grammar,
            max_tokens,
        });
        let mut stream = self.scheduler.admit(priority, stream).await;

        let writer_slot = Arc::clone(&self.writer);
        let handle = tokio::spawn(async move {
            let Some(mut writer) = writer_slot.lock().await.take() else {
                return; // Session already consumed or closed.
            };
            while let Some(item) = stream.next().await {
                match item {
                    Ok(token) => {
                        if writer.write_all(token.as_bytes()).await.is_err() {
                            break; // Client closed its end.
                        }
                    }
                    Err(_) => break, // Error → early EOF (M2: signal).
                }
            }
            // Dropping the writer closes the fd: EOF = end-of-message.
        });
        *self.task.lock().await = Some(handle);
        Ok(())
    }

    /// Embed texts (aad = array of array of double).
    async fn embed(&self, texts: Vec<String>) -> zbus::fdo::Result<Vec<Vec<f64>>> {
        let vectors = self
            .engine
            .embed(texts)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(vectors
            .into_iter()
            .map(|v| v.into_iter().map(f64::from).collect())
            .collect())
    }

    /// Abort the in-flight generation (the fd sees early EOF).
    async fn cancel(&self) -> zbus::fdo::Result<()> {
        if let Some(handle) = self.task.lock().await.take() {
            handle.abort();
        }
        Ok(())
    }

    /// Close the session and release its object path.
    async fn close(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        if let Some(handle) = self.task.lock().await.take() {
            handle.abort();
        }
        if let Some(path) = header.path() {
            let _ = server.remove::<Session, _>(path).await;
        }
        Ok(())
    }
}

/// Register on the session bus (real systems; tests use p2p connections).
pub async fn serve(
    engine: Arc<dyn Engine>,
    scheduler: Arc<Scheduler>,
) -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .name("org.lisa.Inference1")?
        .serve_at("/org/lisa/Inference1", Inference1::new(engine, scheduler))?
        .build()
        .await
}
