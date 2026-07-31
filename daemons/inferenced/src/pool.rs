//! Multi-model residency (`docs/PLAN.md` §5.1): one supervised engine
//! child per resident model, spawned lazily on first request and evicted
//! LRU when the residency cap is reached. Eviction kills the child —
//! mmap-backed weights make a later respawn cheap. PSI-driven pressure
//! eviction refines the cap on Linux later.

use crate::engine::{Engine, EngineError};
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Resolves a request's model name to a live engine.
pub trait EngineProvider: Send + Sync {
    fn engine_for(
        &self,
        model: Option<&str>,
    ) -> BoxFuture<'_, Result<Arc<dyn Engine>, EngineError>>;
    /// Model names this provider can currently serve (for /v1/models).
    fn known_models(&self) -> Vec<String>;
}

/// Single fixed engine (the stub, or a single-model deployment).
pub struct SingleEngine {
    pub engine: Arc<dyn Engine>,
    pub name: String,
}

impl EngineProvider for SingleEngine {
    fn engine_for(
        &self,
        _model: Option<&str>,
    ) -> BoxFuture<'_, Result<Arc<dyn Engine>, EngineError>> {
        let engine = Arc::clone(&self.engine);
        Box::pin(async move { Ok(engine) })
    }

    fn known_models(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
}

type EngineFactory =
    Box<dyn Fn(&str, PathBuf, u16) -> Result<Arc<dyn Engine>, EngineError> + Send + Sync>;

/// Pool of per-model engines, keyed by store ref name.
pub struct ModelPool {
    default_model: String,
    refs_dir: PathBuf,
    max_resident: usize,
    factory: EngineFactory,
    inner: Mutex<PoolState>,
}

#[derive(Default)]
struct PoolState {
    engines: HashMap<String, Arc<dyn Engine>>,
    /// Least-recently-used order, most recent last.
    lru: Vec<String>,
}

/// A free loopback port from the OS. Children used to get
/// `base_port + offset`, but with TWO daemons (system API on 7777,
/// per-user companion on 7778) the static scheme self-collided: the
/// companion's first child was assigned the companion's OWN port and the
/// system daemon's first child was assigned the companion's port — either
/// way the child dies binding and every request hangs on a dead engine
/// (found live on the field iMac, 2026-07-25). Ask the kernel instead.
fn free_port() -> Result<u16, EngineError> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| EngineError::Unavailable(format!("no free loopback port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| EngineError::Unavailable(format!("no free loopback port: {e}")))?
        .port();
    drop(listener); // tiny bind race with the child is acceptable in practice
    Ok(port)
}

impl ModelPool {
    pub fn new(
        default_model: String,
        refs_dir: PathBuf,
        max_resident: usize,
        factory: EngineFactory,
    ) -> Self {
        Self {
            default_model,
            refs_dir,
            max_resident: max_resident.max(1),
            factory,
            inner: Mutex::new(PoolState::default()),
        }
    }

    async fn resolve(&self, model: Option<&str>) -> Result<Arc<dyn Engine>, EngineError> {
        // Well-known aliases resolve to the resident model, so callers can
        // just say "lisa" (or omit the field) without knowing the exact
        // model id — the common single-model case just works.
        let requested = model.unwrap_or(&self.default_model);
        let name = if matches!(
            requested,
            "lisa" | "lisa-system" | "lisa-system-stub" | "default" | "auto" | ""
        ) {
            self.default_model.clone()
        } else {
            requested.to_string()
        };
        let mut state = self.inner.lock().await;
        #[allow(unused_mut)]
        let mut name = name;

        if let Some(engine) = state.engines.get(&name) {
            let engine = Arc::clone(engine);
            state.lru.retain(|n| n != &name);
            state.lru.push(name);
            return Ok(engine);
        }

        // The default is no longer validated at startup, because the
        // store may be empty then and full ten minutes later (#143). So
        // it is checked here like any other name, and an empty store
        // gets a sentence a person can act on rather than a
        // llama-server spawned onto a path that does not exist.
        // The default was chosen at startup, and on a machine that booted
        // with an empty store that name is a placeholder (#143). Rather
        // than fail — or spawn llama-server onto a path that does not
        // exist — fall back to whatever IS in the store now. This is the
        // whole fix: a model downloaded after boot becomes servable
        // without anything being restarted.
        let path = if name == self.default_model {
            let candidate = self.refs_dir.join(&name);
            if candidate.exists() {
                candidate
            } else if let Some(found) = first_in_store(&self.refs_dir) {
                info!(
                    stale = %self.default_model,
                    using = %found,
                    "configured default is not in the store; serving what is"
                );
                let p = self.refs_dir.join(&found);
                name = found;
                if let Some(engine) = state.engines.get(&name) {
                    let engine = Arc::clone(engine);
                    state.lru.retain(|n| n != &name);
                    state.lru.push(name);
                    return Ok(engine);
                }
                p
            } else {
                return Err(EngineError::Unavailable(
                    "no model installed yet — download one in Settings, or run \
                     `lisa models pull <name>`. Nothing needs restarting."
                        .to_string(),
                ));
            }
        } else {
            let candidate = self.refs_dir.join(&name);
            if !candidate.exists() {
                return Err(EngineError::Unavailable(format!(
                    "model `{name}` is not in the store (lisa models list)"
                )));
            }
            candidate
        };

        // Evict LRU beyond the residency cap before spawning another.
        while state.lru.len() >= self.max_resident {
            let evicted_name = state.lru.remove(0);
            if let Some(evicted) = state.engines.remove(&evicted_name) {
                info!(model = evicted_name, "evicting LRU resident model");
                evicted.shutdown().await;
            }
        }

        let port = free_port()?;
        info!(model = name, port, "admitting model to the pool");
        let engine = (self.factory)(&name, path, port)?;
        state.engines.insert(name.clone(), Arc::clone(&engine));
        state.lru.push(name);
        Ok(engine)
    }
}

/// The first model actually present in the store, alphabetically.
///
/// Used when the configured default is not there — which is the normal
/// state of a machine that booted with an empty store and has since
/// downloaded something (#143).
fn first_in_store(dir: &std::path::Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.into_iter().next()
}

impl EngineProvider for ModelPool {
    fn engine_for(
        &self,
        model: Option<&str>,
    ) -> BoxFuture<'_, Result<Arc<dyn Engine>, EngineError>> {
        let model = model.map(str::to_string);
        Box::pin(async move { self.resolve(model.as_deref()).await })
    }

    fn known_models(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.refs_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        if !names.contains(&self.default_model) {
            names.push(self.default_model.clone());
        }
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{GenerateRequest, StubEngine, TokenStream};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockEngine {
        alive: Arc<AtomicUsize>,
    }

    impl Engine for MockEngine {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn generate(&self, req: GenerateRequest) -> TokenStream {
            StubEngine.generate(req)
        }
        fn embed(
            &self,
            texts: Vec<String>,
        ) -> BoxFuture<'static, Result<Vec<Vec<f32>>, EngineError>> {
            StubEngine.embed(texts)
        }
        fn shutdown(&self) -> BoxFuture<'static, ()> {
            let alive = Arc::clone(&self.alive);
            Box::pin(async move {
                alive.fetch_sub(1, Ordering::SeqCst);
            })
        }
    }

    fn test_pool(dir: &std::path::Path, cap: usize) -> (Arc<AtomicUsize>, ModelPool) {
        // The default has to be IN the store now: an empty store is no
        // longer a startup-validated special case, because a store that
        // is empty at boot and full after a download is the normal way a
        // fresh machine behaves (#143).
        std::fs::write(dir.join("default-model"), b"gguf").unwrap();
        let alive = Arc::new(AtomicUsize::new(0));
        let spawned = Arc::clone(&alive);
        let pool = ModelPool::new(
            "default-model".into(),
            dir.to_path_buf(),
            cap,
            Box::new(move |_name, _path, _port| {
                spawned.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(MockEngine {
                    alive: Arc::clone(&spawned),
                }))
            }),
        );
        (alive, pool)
    }

    #[tokio::test]
    async fn same_model_reuses_the_engine() {
        let dir = tempfile::tempdir().unwrap();
        let (alive, pool) = test_pool(dir.path(), 2);
        pool.resolve(None).await.unwrap();
        pool.resolve(None).await.unwrap();
        pool.resolve(Some("default-model")).await.unwrap();
        assert_eq!(alive.load(Ordering::SeqCst), 1, "one spawn for one model");
    }

    #[tokio::test]
    async fn lru_eviction_shuts_down_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model-b"), b"x").unwrap();
        std::fs::write(dir.path().join("model-c"), b"x").unwrap();
        let (alive, pool) = test_pool(dir.path(), 2);

        pool.resolve(None).await.unwrap(); // default
        pool.resolve(Some("model-b")).await.unwrap();
        assert_eq!(alive.load(Ordering::SeqCst), 2);

        // Touch default so model-b becomes LRU, then admit model-c.
        pool.resolve(None).await.unwrap();
        pool.resolve(Some("model-c")).await.unwrap();
        assert_eq!(alive.load(Ordering::SeqCst), 2, "cap holds: one evicted");

        // model-b was evicted; asking for it again respawns.
        pool.resolve(Some("model-b")).await.unwrap();
        assert_eq!(alive.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn well_known_aliases_resolve_to_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let (alive, pool) = test_pool(dir.path(), 2);
        for alias in ["lisa", "default", "auto", ""] {
            pool.resolve(Some(alias)).await.unwrap();
        }
        // All aliases hit the one default model — a single spawn.
        assert_eq!(alive.load(Ordering::SeqCst), 1);
    }

    /// Issue #143: a machine booted with an empty store decided, once and
    /// for ever, that there were no models. gemma-3-1b-it-q8 installed
    /// cleanly, `lisa models list` showed it, and /v1/models kept serving
    /// `lisa-system-stub` until somebody ran `systemctl restart
    /// lisa-inferenced` by hand.
    #[tokio::test]
    async fn a_model_downloaded_after_startup_becomes_servable() {
        let dir = tempfile::tempdir().unwrap();
        let alive = Arc::new(AtomicUsize::new(0));
        let spawned = Arc::clone(&alive);
        // The default this pool was built with does not exist — exactly
        // what `llama_refs_and_default` produces on an empty store.
        let pool = ModelPool::new(
            "lisa-system".into(),
            dir.path().to_path_buf(),
            2,
            Box::new(move |_n, _p, _port| {
                spawned.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(MockEngine {
                    alive: Arc::clone(&spawned),
                }))
            }),
        );

        // Before the download: a refusal that tells the person what to
        // do, and explicitly that nothing needs restarting.
        let msg = match pool.resolve(None).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("an empty store must not resolve to an engine"),
        };
        assert!(msg.contains("no model installed"), "{msg}");
        assert!(msg.contains("Nothing needs restarting"), "{msg}");
        assert_eq!(
            alive.load(Ordering::SeqCst),
            0,
            "must not spawn a doomed child"
        );

        // The download lands. Nothing else happens — no restart, no
        // signal, no rescan call.
        std::fs::write(dir.path().join("gemma-3-1b-it-q8"), b"gguf").unwrap();

        // …and the very next request serves it.
        assert!(
            pool.resolve(None).await.is_ok(),
            "a downloaded model must be servable"
        );
        assert_eq!(alive.load(Ordering::SeqCst), 1);
        assert!(
            pool.known_models()
                .contains(&"gemma-3-1b-it-q8".to_string())
        );
    }

    #[tokio::test]
    async fn unknown_model_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (_alive, pool) = test_pool(dir.path(), 2);
        let err = match pool.resolve(Some("no-such-model")).await {
            Err(e) => e,
            Ok(_) => panic!("unknown model must be refused"),
        };
        assert!(err.to_string().contains("not in the store"));
    }
}

#[cfg(test)]
mod port_tests {
    use super::free_port;

    #[test]
    fn free_ports_are_bindable_and_distinct_enough() {
        let a = free_port().unwrap();
        let l = std::net::TcpListener::bind(("127.0.0.1", a));
        assert!(l.is_ok(), "port {a} from free_port() must be bindable");
        // While `a` is held, a second allocation must not return `a`.
        let b = free_port().unwrap();
        assert_ne!(a, b, "kernel must not hand out a port that is in use");
    }
}
