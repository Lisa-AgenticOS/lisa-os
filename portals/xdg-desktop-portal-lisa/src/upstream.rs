//! The seam to `dev.lisaos.Inference1` (`daemons/inferenced`). The portal
//! proxies portal sessions onto real daemon sessions; this trait keeps
//! the D-Bus surface testable with a stub upstream on any dev host, and
//! `ZbusUpstream` is exercised against the real `Inference1` interface
//! over zbus p2p in `tests/portal.rs`.

use futures::future::BoxFuture;
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    #[error("inference daemon unavailable: {0}")]
    Unavailable(String),
}

/// One live daemon-side session the portal controls on the app's behalf.
pub trait UpstreamSession: Send + Sync {
    fn generate(
        &self,
        prompt: String,
        params: HashMap<String, OwnedValue>,
    ) -> BoxFuture<'_, Result<(), UpstreamError>>;
    fn embed(&self, texts: Vec<String>) -> BoxFuture<'_, Result<Vec<Vec<f64>>, UpstreamError>>;
    fn cancel(&self) -> BoxFuture<'_, Result<(), UpstreamError>>;
    /// Close the daemon session. Revocation calls this: the daemon drops
    /// its pipe writer, so the app's fd sees EOF immediately.
    fn close(&self) -> BoxFuture<'_, Result<(), UpstreamError>>;
}

pub trait InferenceUpstream: Send + Sync {
    /// Open a daemon session; returns the control handle and the read
    /// end of the token pipe (passed through to the app untouched).
    #[allow(clippy::type_complexity)]
    fn open_session(
        &self,
        options: HashMap<String, OwnedValue>,
    ) -> BoxFuture<'_, Result<(Box<dyn UpstreamSession>, OwnedFd), UpstreamError>>;
}

fn dbus_err(e: zbus::Error) -> UpstreamError {
    UpstreamError::Unavailable(e.to_string())
}

/// The real upstream: `dev.lisaos.Inference1` over an existing zbus
/// connection (session bus in production, p2p in tests).
pub struct ZbusUpstream {
    conn: zbus::Connection,
}

impl ZbusUpstream {
    pub const BUS_NAME: &'static str = "dev.lisaos.Inference1";
    pub const PATH: &'static str = "/dev/lisaos/Inference1";

    pub fn new(conn: zbus::Connection) -> Self {
        Self { conn }
    }

    async fn session_proxy(
        &self,
        path: OwnedObjectPath,
    ) -> Result<zbus::Proxy<'static>, UpstreamError> {
        zbus::Proxy::new(
            &self.conn,
            Self::BUS_NAME,
            path,
            "dev.lisaos.Inference1.Session",
        )
        .await
        .map_err(dbus_err)
    }
}

impl InferenceUpstream for ZbusUpstream {
    fn open_session(
        &self,
        options: HashMap<String, OwnedValue>,
    ) -> BoxFuture<'_, Result<(Box<dyn UpstreamSession>, OwnedFd), UpstreamError>> {
        Box::pin(async move {
            let root = zbus::Proxy::new(
                &self.conn,
                Self::BUS_NAME,
                Self::PATH,
                "dev.lisaos.Inference1",
            )
            .await
            .map_err(dbus_err)?;
            let reply = root
                .call_method("OpenSession", &(options,))
                .await
                .map_err(dbus_err)?;
            let (path, fd): (OwnedObjectPath, zbus::zvariant::OwnedFd) = reply
                .body()
                .deserialize()
                .map_err(|e| UpstreamError::Unavailable(e.to_string()))?;
            let session = ZbusSession {
                proxy: self.session_proxy(path).await?,
            };
            Ok((Box::new(session) as Box<dyn UpstreamSession>, fd.into()))
        })
    }
}

struct ZbusSession {
    proxy: zbus::Proxy<'static>,
}

impl UpstreamSession for ZbusSession {
    fn generate(
        &self,
        prompt: String,
        params: HashMap<String, OwnedValue>,
    ) -> BoxFuture<'_, Result<(), UpstreamError>> {
        Box::pin(async move {
            self.proxy
                .call_method("Generate", &(prompt, params))
                .await
                .map(|_| ())
                .map_err(dbus_err)
        })
    }

    fn embed(&self, texts: Vec<String>) -> BoxFuture<'_, Result<Vec<Vec<f64>>, UpstreamError>> {
        Box::pin(async move {
            let reply = self
                .proxy
                .call_method("Embed", &(texts,))
                .await
                .map_err(dbus_err)?;
            let (vectors,): (Vec<Vec<f64>>,) = reply
                .body()
                .deserialize()
                .map_err(|e| UpstreamError::Unavailable(e.to_string()))?;
            Ok(vectors)
        })
    }

    fn cancel(&self) -> BoxFuture<'_, Result<(), UpstreamError>> {
        Box::pin(async move {
            self.proxy
                .call_method("Cancel", &())
                .await
                .map(|_| ())
                .map_err(dbus_err)
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<(), UpstreamError>> {
        Box::pin(async move {
            self.proxy
                .call_method("Close", &())
                .await
                .map(|_| ())
                .map_err(dbus_err)
        })
    }
}

/// Deterministic in-process upstream: proves the portal plumbing without
/// a daemon (unit tests; `--upstream stub` dev runs, mirroring
/// inferenced's own StubEngine pattern).
pub mod stub {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::AsyncWriteExt;
    use tokio::sync::Mutex;

    #[derive(Default)]
    pub struct StubUpstream;

    pub struct StubSession {
        writer: Arc<Mutex<Option<tokio::net::unix::pipe::Sender>>>,
        closed: Arc<AtomicBool>,
    }

    impl InferenceUpstream for StubUpstream {
        fn open_session(
            &self,
            _options: HashMap<String, OwnedValue>,
        ) -> BoxFuture<'_, Result<(Box<dyn UpstreamSession>, OwnedFd), UpstreamError>> {
            Box::pin(async move {
                let (reader, writer) =
                    std::io::pipe().map_err(|e| UpstreamError::Unavailable(e.to_string()))?;
                let writer = tokio::net::unix::pipe::Sender::from_owned_fd(writer.into())
                    .map_err(|e| UpstreamError::Unavailable(e.to_string()))?;
                let session = StubSession {
                    writer: Arc::new(Mutex::new(Some(writer))),
                    closed: Arc::new(AtomicBool::new(false)),
                };
                Ok((Box::new(session) as Box<dyn UpstreamSession>, reader.into()))
            })
        }
    }

    impl UpstreamSession for StubSession {
        fn generate(
            &self,
            prompt: String,
            _params: HashMap<String, OwnedValue>,
        ) -> BoxFuture<'_, Result<(), UpstreamError>> {
            Box::pin(async move {
                if self.closed.load(Ordering::SeqCst) {
                    return Err(UpstreamError::Unavailable("session closed".into()));
                }
                // Echo like inferenced's StubEngine, then EOF.
                if let Some(mut writer) = self.writer.lock().await.take() {
                    let _ = writer
                        .write_all(format!("[stub upstream] {prompt}").as_bytes())
                        .await;
                }
                Ok(())
            })
        }

        fn embed(&self, texts: Vec<String>) -> BoxFuture<'_, Result<Vec<Vec<f64>>, UpstreamError>> {
            Box::pin(async move {
                if self.closed.load(Ordering::SeqCst) {
                    return Err(UpstreamError::Unavailable("session closed".into()));
                }
                Ok(texts.iter().map(|t| vec![t.len() as f64, 1.0]).collect())
            })
        }

        fn cancel(&self) -> BoxFuture<'_, Result<(), UpstreamError>> {
            Box::pin(async move { Ok(()) })
        }

        fn close(&self) -> BoxFuture<'_, Result<(), UpstreamError>> {
            Box::pin(async move {
                self.closed.store(true, Ordering::SeqCst);
                // Dropping the writer closes the app's fd: EOF now.
                self.writer.lock().await.take();
                Ok(())
            })
        }
    }
}
