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
//! inference). Per-app memory is namespace-isolated exactly as in the
//! library API: every method takes the app id and no call can cross
//! it; caller-identity enforcement attaches with the portal (M2).
//!
//! Tested over zbus p2p connections (no bus daemon needed → runs on
//! macOS dev hosts and CI alike); session-bus registration is used on
//! real systems.

use crate::embed::HashEmbedder;
use crate::store::{ContextStore, StoreError};
use lisa_ledger::{Event, Ledger};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::zvariant::{OwnedValue, Value};

pub struct Context1 {
    store: Arc<ContextStore>,
    ledger: Arc<Ledger>,
}

impl Context1 {
    pub fn new(store: Arc<ContextStore>, ledger: Arc<Ledger>) -> Self {
        Self { store, ledger }
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
    fn search(
        &self,
        query: String,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<String> {
        let limit = options
            .get("limit")
            .and_then(|v| u32::try_from(Value::from(v.clone())).ok())
            .unwrap_or(3) as usize;
        let hybrid = options
            .get("hybrid")
            .and_then(|v| v.downcast_ref::<bool>().ok())
            .unwrap_or(false);
        // A present `scopes` key means the CALLER ASKED for scoped
        // retrieval — honor that even when the list is empty or the variant
        // is malformed (deny-by-default: empty scopes match nothing). The
        // old fallthrough silently widened a failed scoped request into an
        // UNSCOPED search (issue #14).
        let scoped_requested = options.contains_key("scopes");
        let scopes: Vec<String> = options
            .get("scopes")
            .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
            .unwrap_or_default();

        // Every retrieval is ledgered BEFORE it runs (PLAN §5.3,
        // dataflow rule 4) — query hash, not text. No append, no search.
        let kind = if scoped_requested {
            "context.search.scoped"
        } else if hybrid {
            "context.search.hybrid"
        } else {
            "context.search"
        };
        self.ledger
            .append(&Event {
                kind: kind.into(),
                app_id: "host".into(),
                input_hash: blake3::hash(query.as_bytes()).to_hex().to_string(),
                status: "ok".into(),
                ..Default::default()
            })
            .map_err(|e| {
                zbus::fdo::Error::Failed(format!("ledger append failed — refusing to search: {e}"))
            })?;

        let hits = if scoped_requested {
            let scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
            self.store.search_scoped(&query, &scopes, limit)
        } else if hybrid {
            self.store
                .search_hybrid(&query, &HashEmbedder::default(), limit)
        } else {
            self.store.search(&query, limit)
        }
        .map_err(store_err)?;
        serde_json::to_string(&hits)
            .map_err(|e| zbus::fdo::Error::Failed(format!("serializing hits: {e}")))
    }

    /// Read one key from an app's memory namespace. A missing key is
    /// an error (mirrors `lisa memory get`).
    fn memory_get(&self, app: String, key: String) -> zbus::fdo::Result<String> {
        match self.store.memory_get(&app, &key).map_err(store_err)? {
            Some(v) => Ok(v),
            None => Err(zbus::fdo::Error::Failed(format!(
                "no value for `{key}` in namespace `{app}`"
            ))),
        }
    }

    /// Upsert one key in an app's memory namespace.
    fn memory_set(&self, app: String, key: String, value: String) -> zbus::fdo::Result<()> {
        self.store.memory_set(&app, &key, &value).map_err(store_err)
    }

    /// All keys in an app's namespace as a JSON object (key → value).
    fn memory_list(&self, app: String) -> zbus::fdo::Result<String> {
        let pairs = self.store.memory_list(&app).map_err(store_err)?;
        let map: serde_json::Map<String, serde_json::Value> = pairs
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect();
        Ok(serde_json::Value::Object(map).to_string())
    }

    /// Wipe an app's namespace entirely (zero residual rows, §5.3).
    fn memory_wipe(&self, app: String) -> zbus::fdo::Result<()> {
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
