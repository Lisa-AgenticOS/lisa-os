//! Unix-socket HTTP surface (ADR-0008 §1). Data plane:
//! `POST /v1/chat/completions` with `x-lisa-provider` + `x-lisa-scopes`
//! headers; management plane shares the socket.
//!
//! # The socket mode is not the access control (issue #99)
//!
//! It used to say it was. The socket is 0600, which keeps out *other
//! users* — and the threat here is another **process of the same user**:
//! an app you installed, a Flatpak with session access, something the
//! agent built. All of them can `connect()` and, before this, could
//! `PUT /v1/consent {"scope":"mail","allowed":true}` six times and then
//! proxy your mail out through the broker.
//!
//! So the management routes ask the kernel who connected
//! (`lisa_peer::unix`, `SO_PEERCRED` + `SO_PEERPIDFD`) and require an
//! allowlisted program, exactly as the D-Bus plane does. The data plane
//! stays open: `inferenced` is its caller, and what it may send is
//! governed by the offload scopes — which are now the thing that cannot
//! be flipped from outside.

use crate::consent::ConsentError;
use crate::oauth::OauthError;
use crate::proxy::ProxyError;
use crate::registry::RegistryError;
use crate::secrets::SecretsError;
use crate::service::{Broker, BrokerError};
use axum::Router;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post, put};
use futures::StreamExt;
use lisa_peer::manager::Manager;
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

fn error_response(e: BrokerError) -> Response {
    let status = match &e {
        BrokerError::Consent(ConsentError::Denied(_)) => StatusCode::FORBIDDEN,
        BrokerError::Consent(ConsentError::UnknownScope(_)) => StatusCode::BAD_REQUEST,
        BrokerError::Registry(RegistryError::Unknown(_)) => StatusCode::NOT_FOUND,
        BrokerError::Registry(_) => StatusCode::BAD_REQUEST,
        BrokerError::Secrets(SecretsError::Missing(_)) => StatusCode::PRECONDITION_FAILED,
        BrokerError::Oauth(OauthError::NotCapable(_)) => StatusCode::BAD_REQUEST,
        BrokerError::Oauth(OauthError::ReauthRequired(_)) => StatusCode::UNAUTHORIZED,
        BrokerError::Oauth(OauthError::InProgress(_)) => StatusCode::CONFLICT,
        BrokerError::Oauth(_) => StatusCode::BAD_GATEWAY,
        BrokerError::Proxy(ProxyError::BadRequest) => StatusCode::BAD_REQUEST,
        BrokerError::Proxy(ProxyError::Upstream { .. }) => StatusCode::BAD_GATEWAY,
        BrokerError::Ledger(_) => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({"error": {"message": e.to_string()}}))).into_response()
}

/// Who connected, attached per connection by [`crate::serve_socket`].
///
/// An `Option` because a router can be built without one — in a test,
/// or by a future caller that forgets. Absent means unidentified, which
/// every management route reads as a refusal: the failure mode of
/// forgetting to wire this up is "management stops working", never
/// "management is open".
#[derive(Clone, Debug, Default)]
pub struct PeerInfo(pub Option<lisa_peer::Peer>);

impl
    axum::extract::connect_info::Connected<
        axum::serve::IncomingStream<'_, tokio::net::UnixListener>,
    > for PeerInfo
{
    /// Ask the kernel who connected, once per connection.
    ///
    /// This is the moment the answer is cheapest and most trustworthy:
    /// the socket is freshly accepted, so `SO_PEERPIDFD` pins the peer's
    /// pid for as long as we hold the descriptor — the request handler
    /// can take as long as it likes without the identity going stale.
    fn connect_info(stream: axum::serve::IncomingStream<'_, tokio::net::UnixListener>) -> Self {
        PeerInfo(Some(lisa_peer::unix::peer_of_socket(stream.io())))
    }
}

/// Programs allowed to operate the management plane over the socket.
#[derive(Clone, Debug)]
pub struct Managers(pub Arc<Vec<PathBuf>>);

impl Default for Managers {
    fn default() -> Self {
        Managers(Arc::new(lisa_peer::manager::default_managers()))
    }
}

/// The caller of a request, as an extractor that cannot fail.
///
/// `ConnectInfo<PeerInfo>` on its own rejects with a 500 when the server
/// was built without connect info — a wiring mistake would then look
/// like a server fault rather than a refusal, and the tests that assert
/// "an unidentified caller is refused" would be asserting the wrong
/// status. Reading it out of the request extensions makes absence an
/// ordinary value: `None`, which every management route treats as
/// unidentified.
pub struct CallerPeer(pub Option<lisa_peer::Peer>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for CallerPeer {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Infallible> {
        Ok(CallerPeer(
            parts
                .extensions
                .get::<ConnectInfo<PeerInfo>>()
                .and_then(|c| c.0.0.clone()),
        ))
    }
}

/// Resolve the caller of a management request, or refuse.
///
/// `peer` is `None` when the server was built without connect info at
/// all — a wiring mistake, or a test calling the router directly. That
/// is a refusal, and deliberately the *same* refusal an unauthorized
/// program gets: the failure mode of forgetting to attach identity must
/// be "nothing can be managed", never "anything can".
fn manager_of(
    peer: Option<&lisa_peer::Peer>,
    managers: &Managers,
) -> Result<Manager, Box<Response>> {
    let refuse = || -> Box<Response> {
        Box::new(
            (
                StatusCode::FORBIDDEN,
                Json(json!({"error": {"message":
                "only Settings and the lisa CLI can change remote-provider settings"}})),
            )
                .into_response(),
        )
    };
    let Some(peer) = peer else {
        return Err(refuse());
    };
    // The identification error is carried, not discarded (#161). An
    // empty `exe` cannot tell "the bus gave us no pidfd" from "we have a
    // pidfd but could not read /proc/<pid>/exe" — and those have
    // completely different fixes. Every caller on this machine was
    // refused with exe="" and the log said nothing more, so the reason
    // had to be reconstructed by reading code.
    #[cfg(unix)]
    let identified = lisa_peer::exe_of_peer(peer);
    #[cfg(not(unix))]
    let identified: Result<PathBuf, lisa_peer::IdentityError> =
        Err(lisa_peer::IdentityError::Unsupported);
    let exe = identified.as_ref().ok().cloned();
    let allowed = lisa_peer::manager::resolve_managers(&managers.0);
    Manager::authorize(peer.is_same_user_as_us(), exe.as_deref(), &allowed).map_err(|why| {
        tracing::warn!(
            exe = exe.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            identity = identified.as_ref().err().map(|e| e.to_string()).unwrap_or_default(),
            pid = peer.pid.unwrap_or(0),
            same_user = peer.is_same_user_as_us(),
            %why,
            "refused a management call over the socket"
        );
        refuse()
    })
}

pub fn router(broker: Arc<Broker>) -> Router {
    router_with_managers(broker, Managers::default())
}

pub fn router_with_managers(broker: Arc<Broker>, managers: Managers) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/providers", get(providers).post(add_provider))
        .route("/v1/providers/{id}", delete(remove_provider))
        .route("/v1/providers/{id}/key", put(set_key).delete(clear_key))
        .route("/v1/consent", get(consent).put(set_consent))
        .route("/v1/oauth/{provider}/begin", post(oauth_begin))
        .route("/v1/oauth/{provider}", delete(oauth_logout))
        .layer(axum::Extension(managers))
        .with_state(broker)
}

async fn health(State(broker): State<Arc<Broker>>) -> Json<Value> {
    let providers = broker.providers_json()["providers"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    Json(json!({
        "status": "ok",
        "daemon": "lisa-remoted",
        "egress": "remote",
        "providers": providers,
    }))
}

async fn chat(
    State(broker): State<Arc<Broker>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(provider) = headers
        .get("x-lisa-provider")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "missing x-lisa-provider header"}})),
        )
            .into_response();
    };
    let scopes: Vec<String> = headers
        .get("x-lisa-scopes")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // stream:true answers as SSE over the socket (ADR-0010 update):
    // `data:` frames in the OpenAI chunk shape, a `{"error":...}` frame
    // on mid-stream failure, `data: [DONE]` last. Pre-flight failures
    // (consent, credentials, upstream refusal) still return plain JSON
    // errors with the proper status. stream:false is unchanged.
    if body["stream"].as_bool().unwrap_or(false) {
        return match broker.chat_stream(&provider, &scopes, &body).await {
            Ok(stream) => {
                let events = stream.map(|d| Ok::<_, Infallible>(Event::default().data(d)));
                Sse::new(events).into_response()
            }
            Err(e) => error_response(e),
        };
    }
    match broker.chat(&provider, &scopes, &body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_response(e),
    }
}

async fn providers(State(broker): State<Arc<Broker>>) -> Json<Value> {
    Json(broker.providers_json())
}

#[derive(Deserialize)]
struct AddProvider {
    id: String,
    display_name: String,
    base_url: String,
    /// "This endpoint is my own machine / my own network." Absent means
    /// no: a caller that does not mention it gets the public-internet
    /// rules, which is the safe reading of silence (#92).
    #[serde(default)]
    allow_local: bool,
}

async fn add_provider(
    State(broker): State<Arc<Broker>>,
    CallerPeer(peer): CallerPeer,
    axum::Extension(managers): axum::Extension<Managers>,
    Json(req): Json<AddProvider>,
) -> Response {
    let who = match manager_of(peer.as_ref(), &managers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    let locality = if req.allow_local {
        crate::net::Locality::LocalAllowed
    } else {
        crate::net::Locality::PublicOnly
    };
    match broker.add_provider(&who, &req.id, &req.display_name, &req.base_url, locality) {
        Ok(()) => Json(broker.providers_json()).into_response(),
        Err(e) => error_response(e),
    }
}

async fn remove_provider(
    State(broker): State<Arc<Broker>>,
    CallerPeer(peer): CallerPeer,
    axum::Extension(managers): axum::Extension<Managers>,
    Path(id): Path<String>,
) -> Response {
    let who = match manager_of(peer.as_ref(), &managers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    match broker.remove_provider(&who, &id) {
        Ok(()) => Json(broker.providers_json()).into_response(),
        Err(e) => error_response(e),
    }
}

#[derive(Deserialize)]
struct SetKey {
    key: String,
}

async fn set_key(
    State(broker): State<Arc<Broker>>,
    CallerPeer(peer): CallerPeer,
    axum::Extension(managers): axum::Extension<Managers>,
    Path(id): Path<String>,
    Json(req): Json<SetKey>,
) -> Response {
    let who = match manager_of(peer.as_ref(), &managers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    match broker.set_key(&who, &id, &req.key) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(e),
    }
}

async fn clear_key(
    State(broker): State<Arc<Broker>>,
    CallerPeer(peer): CallerPeer,
    axum::Extension(managers): axum::Extension<Managers>,
    Path(id): Path<String>,
) -> Response {
    let who = match manager_of(peer.as_ref(), &managers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    match broker.clear_key(&who, &id) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(e),
    }
}

async fn consent(State(broker): State<Arc<Broker>>) -> Json<Value> {
    Json(broker.consent_json())
}

#[derive(Deserialize)]
struct SetConsent {
    scope: String,
    allowed: bool,
}

async fn set_consent(
    State(broker): State<Arc<Broker>>,
    CallerPeer(peer): CallerPeer,
    axum::Extension(managers): axum::Extension<Managers>,
    Json(req): Json<SetConsent>,
) -> Response {
    let who = match manager_of(peer.as_ref(), &managers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    match broker.set_consent(&who, &req.scope, req.allowed) {
        Ok(()) => Json(broker.consent_json()).into_response(),
        Err(e) => error_response(e),
    }
}

/// Begin "Sign in with …" for `provider`; returns the authorize URL the
/// caller opens in a browser. Completion is observed by polling provider
/// state (`connected`) or, over D-Bus, the `LoginCompleted` signal.
async fn oauth_begin(
    State(broker): State<Arc<Broker>>,
    CallerPeer(peer): CallerPeer,
    axum::Extension(managers): axum::Extension<Managers>,
    Path(provider): Path<String>,
) -> Response {
    let who = match manager_of(peer.as_ref(), &managers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    match broker.begin_login(&who, &provider).await {
        Ok(url) => Json(json!({"authorize_url": url})).into_response(),
        Err(e) => error_response(e),
    }
}

/// Forget a stored OAuth session (idempotent).
async fn oauth_logout(
    State(broker): State<Arc<Broker>>,
    CallerPeer(peer): CallerPeer,
    axum::Extension(managers): axum::Extension<Managers>,
    Path(provider): Path<String>,
) -> Response {
    let who = match manager_of(peer.as_ref(), &managers) {
        Ok(w) => w,
        Err(r) => return *r,
    };
    match broker.logout(&who, &provider) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(e),
    }
}
