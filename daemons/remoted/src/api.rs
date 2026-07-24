//! Unix-socket HTTP surface (ADR-0008 §1). Data plane:
//! `POST /v1/chat/completions` with `x-lisa-provider` + `x-lisa-scopes`
//! headers. Management plane shares the socket; socket permissions are
//! the access control (M2 attaches per-app identity via the portal).

use crate::consent::ConsentError;
use crate::oauth::OauthError;
use crate::proxy::ProxyError;
use crate::registry::RegistryError;
use crate::secrets::SecretsError;
use crate::service::{Broker, BrokerError};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post, put};
use serde::Deserialize;
use serde_json::{Value, json};
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

pub fn router(broker: Arc<Broker>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/providers", get(providers).post(add_provider))
        .route("/v1/providers/{id}", delete(remove_provider))
        .route("/v1/providers/{id}/key", put(set_key).delete(clear_key))
        .route("/v1/consent", get(consent).put(set_consent))
        .route("/v1/oauth/{provider}/begin", post(oauth_begin))
        .route("/v1/oauth/{provider}", delete(oauth_logout))
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
}

async fn add_provider(State(broker): State<Arc<Broker>>, Json(req): Json<AddProvider>) -> Response {
    match broker.add_provider(&req.id, &req.display_name, &req.base_url) {
        Ok(()) => Json(broker.providers_json()).into_response(),
        Err(e) => error_response(e),
    }
}

async fn remove_provider(State(broker): State<Arc<Broker>>, Path(id): Path<String>) -> Response {
    match broker.remove_provider(&id) {
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
    Path(id): Path<String>,
    Json(req): Json<SetKey>,
) -> Response {
    match broker.set_key(&id, &req.key) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(e),
    }
}

async fn clear_key(State(broker): State<Arc<Broker>>, Path(id): Path<String>) -> Response {
    match broker.clear_key(&id) {
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

async fn set_consent(State(broker): State<Arc<Broker>>, Json(req): Json<SetConsent>) -> Response {
    match broker.set_consent(&req.scope, req.allowed) {
        Ok(()) => Json(broker.consent_json()).into_response(),
        Err(e) => error_response(e),
    }
}

/// Begin "Sign in with …" for `provider`; returns the authorize URL the
/// caller opens in a browser. Completion is observed by polling provider
/// state (`connected`) or, over D-Bus, the `LoginCompleted` signal.
async fn oauth_begin(State(broker): State<Arc<Broker>>, Path(provider): Path<String>) -> Response {
    match broker.begin_login(&provider).await {
        Ok(url) => Json(json!({"authorize_url": url})).into_response(),
        Err(e) => error_response(e),
    }
}

/// Forget a stored OAuth session (idempotent).
async fn oauth_logout(State(broker): State<Arc<Broker>>, Path(provider): Path<String>) -> Response {
    match broker.logout(&provider) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => error_response(e),
    }
}
