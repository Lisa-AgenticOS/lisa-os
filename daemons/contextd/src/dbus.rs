//! D-Bus surface: `dev.lisaos.Context1` (`docs/PLAN.md` §5.3).
//!
//! The context fabric as seen by shell surfaces (the assistant
//! overlay's [my stuff] toggle) and scripts. Rich results are JSON
//! strings — one serialization, and `busctl`/scripts read them
//! directly, matching `dev.lisaos.Agent1`.
//!
//! Shape:
//!
//! ```text
//! Ping() → s
//! Search(s query, a{sv} options) → (s hits_json)
//!     options: "limit" (u, default 3), "hybrid" (b),
//!              "scopes" (as — portal scopes; present ⇒ ACL-scoped
//!              retrieval, deny-by-default on empty/unknown scopes)
//!     hits_json: [{source, provenance, snippet, score}]
//! MemoryGet(s app, s key) → s        (missing key → error)
//! MemorySet(s app, s key, s value)
//! MemoryList(s app) → s              (JSON object, key → value)
//! MemoryWipe(s app)
//! ```
//!
//! Every Search appends a `context.search[.hybrid|.scoped]` ledger
//! entry BEFORE the store is queried — if the append fails, the
//! retrieval does not happen (dataflow rule 4, same gate as
//! inference).
//!
//! # Who is asking (issues #101, #100)
//!
//! Both halves of this interface used to take the caller's word for
//! everything.
//!
//! **Memory** took the namespace as an argument and checked nothing, so
//! any peer on the session bus could read another app's durable memory
//! — `MemoryList("app.lisaos.Assistant")` returns whole session
//! transcripts — or `MemoryWipe` it. The module said identity "attaches
//! with the portal (M2)"; M2 shipped with no Context portal, so nothing
//! was ever going to supply it, while the interface was reachable the
//! whole time. The namespace was a naming convention, not a boundary.
//!
//! **Search** picked its retrieval path from a caller-supplied option:
//! `scopes` present meant the ACL-enforced path, absent meant *every
//! provenance, no filter*. So the ACL was a request rather than a
//! boundary — omit one dictionary key and read mail, screen captures,
//! web and calendar chunks. The one shipping consumer, the overlay's
//! [my stuff] retrieval, omitted it.
//!
//! Now: the app id comes from the caller (`lisa_peer::app`), an `app`
//! argument may only ever *match* it — except from an allowlisted
//! manager, which is how `lisa memory --app X` stays possible — and a
//! Search with no scopes returns nothing rather than everything.
//!
//! Tested over zbus p2p connections (no bus daemon needed → runs on
//! macOS dev hosts and CI alike); session-bus registration is used on
//! real systems.

use crate::store::{ContextStore, StoreError};
use lisa_ledger::{Event, Ledger};
use lisa_peer::app::{AppIdentity, IdentityResolver, ProcResolver};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use zbus::zvariant::{OwnedValue, Value};

pub struct Context1 {
    store: Arc<ContextStore>,
    ledger: Arc<Ledger>,
    identity: Arc<dyn IdentityResolver>,
    /// Programs that may act for another app — `lisa memory --app X`,
    /// and the Settings surfaces. Same allowlist the portal and
    /// `remoted` use.
    managers: Vec<PathBuf>,
}

impl Context1 {
    pub fn new(store: Arc<ContextStore>, ledger: Arc<Ledger>) -> Self {
        Self {
            store,
            ledger,
            identity: Arc::new(ProcResolver::new()),
            managers: lisa_peer::manager::default_managers(),
        }
    }

    /// Test/dev constructor: an explicit resolver and allowlist.
    pub fn with_identity(
        store: Arc<ContextStore>,
        ledger: Arc<Ledger>,
        identity: Arc<dyn IdentityResolver>,
        managers: Vec<PathBuf>,
    ) -> Self {
        Self {
            store,
            ledger,
            identity,
            managers,
        }
    }

    /// Who is calling, and may they speak for `asked_for`?
    ///
    /// The rule in one line: **you get your own namespace, unless you
    /// are one of the user's own tools.** A caller that cannot be
    /// identified at all gets `host:unknown`, which is a real namespace
    /// — shared by every unidentifiable caller — rather than a pass.
    async fn namespace_for(
        &self,
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
        asked_for: &str,
    ) -> zbus::fdo::Result<String> {
        let peer = lisa_peer::resolve(conn, header).await.map_err(|_| {
            zbus::fdo::Error::AccessDenied("the caller could not be identified".into())
        })?;
        let me = self.identity.identify(&peer);
        if asked_for.is_empty() || asked_for == me.app_id {
            return Ok(me.app_id);
        }
        // Asking for somebody else's namespace: only the user's own
        // tooling may, and only because `lisa memory --app X` is a
        // thing a person does on purpose.
        #[cfg(unix)]
        let exe = lisa_peer::exe_of_peer(&peer).ok();
        #[cfg(not(unix))]
        let exe: Option<PathBuf> = None;
        let managers = lisa_peer::manager::resolve_managers(&self.managers);
        match lisa_peer::manager::may_manage(peer.is_same_user_as_us(), exe.as_deref(), &managers) {
            Ok(()) => Ok(asked_for.to_string()),
            Err(_) => Err(zbus::fdo::Error::AccessDenied(format!(
                "{} may only use its own memory namespace",
                me.app_id
            ))),
        }
    }

    /// Whether this caller may read the index unscoped.
    ///
    /// Only the user's own tooling — `lisa context search` is a person
    /// looking at their own index, and a boundary between a person and
    /// their own machine is the wrong boundary (ADR-0030). An app gets
    /// exactly the provenance its scopes allow.
    async fn caller_may_read_everything(
        &self,
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> bool {
        let Ok(peer) = lisa_peer::resolve(conn, header).await else {
            return false;
        };
        #[cfg(unix)]
        let exe = lisa_peer::exe_of_peer(&peer).ok();
        #[cfg(not(unix))]
        let exe: Option<PathBuf> = None;
        let managers = lisa_peer::manager::resolve_managers(&self.managers);
        lisa_peer::manager::may_manage(peer.is_same_user_as_us(), exe.as_deref(), &managers).is_ok()
    }

    /// The app this caller is, for attribution.
    async fn caller_app(
        &self,
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> AppIdentity {
        match lisa_peer::resolve(conn, header).await {
            Ok(peer) => self.identity.identify(&peer),
            Err(_) => AppIdentity::unknown(),
        }
    }
}

fn store_err(e: StoreError) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

#[zbus::interface(name = "dev.lisaos.Context1")]
impl Context1 {
    /// Liveness probe.
    fn ping(&self) -> String {
        format!("lisa-contextd {}", env!("CARGO_PKG_VERSION"))
    }

    /// Retrieval over the user's index. Options: "limit" (u, default
    /// 3), "hybrid" (b, BM25×cosine blend), "scopes" (as — when
    /// present, the ACL-scoped path: only provenance the granted
    /// scopes permit is ever returned, deny-by-default). Returns a
    /// JSON array of hits. The ledger entry is appended before the
    /// store is touched; append failure refuses the search.
    async fn search(
        &self,
        query: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        let limit = options
            .get("limit")
            .and_then(|v| u32::try_from(Value::from(v.clone())).ok())
            .unwrap_or(3) as usize;
        let hybrid = options
            .get("hybrid")
            .and_then(|v| v.downcast_ref::<bool>().ok())
            .unwrap_or(false);
        // Scopes are no longer optional (issue #100). Their ABSENCE used
        // to mean "every provenance, no filter", so the ACL was a
        // request rather than a boundary: omit one dictionary key and
        // read mail, screen, web and calendar. Absence now means the
        // same as an empty list — nothing — and an unscoped read is
        // available only to the user's own tooling, below.
        let scopes: Vec<String> = options
            .get("scopes")
            .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
            .unwrap_or_default();

        let app = self.caller_app(conn, &header).await;
        // `lisa context search` with no --scope is a person looking at
        // their own index, and refusing them would be the boundary
        // pointed the wrong way (ADR-0030: guardrails sit between the
        // model and the machine, never between a person and their own
        // machine). Everything else is scoped.
        let unscoped_allowed = self.caller_may_read_everything(conn, &header).await;

        // Every retrieval is ledgered BEFORE it runs (PLAN §5.3,
        // dataflow rule 4) — query hash, not text. No append, no search.
        let kind = if !unscoped_allowed && hybrid {
            "context.search.scoped.hybrid"
        } else if !unscoped_allowed {
            "context.search.scoped"
        } else if hybrid {
            "context.search.hybrid"
        } else {
            "context.search"
        };
        // Pick the embedder before ledgering, so the entry can say which
        // one ranked the results. A hybrid search backed by HashEmbedder
        // is lexical with extra steps; "you can read exactly what it
        // did" has to include that (#163).
        let chosen = hybrid.then(crate::embed::resolve);
        let embedder_kind = chosen.as_ref().map_or("none", |c| c.kind);
        // The model matters as much as the kind: "inferenced" covers both
        // the pinned embedding model and whatever chat model the daemon
        // defaults to, and those rank differently (#163).
        let embedder_model = chosen
            .as_ref()
            .and_then(|c| c.model.clone())
            .unwrap_or_else(|| "(daemon default)".into());
        // §5.3 asks for (app, scope, query hash, doc-ids). It was
        // `app_id: "host"` with no scopes, so a retrieval could not be
        // attributed to the app that made it.
        self.ledger
            .append(&Event {
                kind: kind.into(),
                app_id: app.app_id.clone(),
                input_hash: blake3::hash(query.as_bytes()).to_hex().to_string(),
                status: "ok".into(),
                detail: serde_json::json!({
                    "scopes": scopes,
                    "unscoped": unscoped_allowed,
                    "identity": app.kind.as_str(),
                    "embedder": embedder_kind,
                    "embedder_model": embedder_model,
                })
                .to_string(),
                ..Default::default()
            })
            .map_err(|e| {
                zbus::fdo::Error::Failed(format!("ledger append failed — refusing to search: {e}"))
            })?;

        let hits = if !unscoped_allowed {
            let scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
            if hybrid {
                // Both, rather than silently dropping one: an app that
                // asked for the better ranking and got the worse one
                // would have no way to tell.
                self.store.search_hybrid_scoped(
                    &query,
                    &scopes,
                    chosen
                        .as_ref()
                        .map(|c| c.embedder.as_ref())
                        .expect("hybrid implies an embedder"),
                    limit,
                )
            } else {
                self.store.search_scoped(&query, &scopes, limit)
            }
        } else if hybrid {
            self.store.search_hybrid(
                &query,
                chosen
                    .as_ref()
                    .map(|c| c.embedder.as_ref())
                    .expect("hybrid implies an embedder"),
                limit,
            )
        } else {
            self.store.search(&query, limit)
        }
        .map_err(store_err)?;
        serde_json::to_string(&hits)
            .map_err(|e| zbus::fdo::Error::Failed(format!("serializing hits: {e}")))
    }

    /// Read one key from an app's memory namespace. A missing key is
    /// an error (mirrors `lisa memory get`).
    async fn memory_get(
        &self,
        app: String,
        key: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        let app = self.namespace_for(conn, &header, &app).await?;
        match self.store.memory_get(&app, &key).map_err(store_err)? {
            Some(v) => Ok(v),
            None => Err(zbus::fdo::Error::Failed(format!(
                "no value for `{key}` in namespace `{app}`"
            ))),
        }
    }

    /// Upsert one key in an app's memory namespace.
    async fn memory_set(
        &self,
        app: String,
        key: String,
        value: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let app = self.namespace_for(conn, &header, &app).await?;
        self.store.memory_set(&app, &key, &value).map_err(store_err)
    }

    /// All keys in an app's namespace as a JSON object (key → value).
    async fn memory_list(
        &self,
        app: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        let app = self.namespace_for(conn, &header, &app).await?;
        let pairs = self.store.memory_list(&app).map_err(store_err)?;
        let map: serde_json::Map<String, serde_json::Value> = pairs
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect();
        Ok(serde_json::Value::Object(map).to_string())
    }

    /// Wipe an app's namespace entirely (zero residual rows, §5.3).
    async fn memory_wipe(
        &self,
        app: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let app = self.namespace_for(conn, &header, &app).await?;
        self.store.memory_wipe(&app).map(|_| ()).map_err(store_err)
    }
}

/// Register on the session bus (real systems; tests use p2p).
pub async fn serve(
    store: Arc<ContextStore>,
    ledger: Arc<Ledger>,
) -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .name("dev.lisaos.Context1")?
        .serve_at("/dev/lisaos/Context1", Context1::new(store, ledger))?
        .build()
        .await
}
