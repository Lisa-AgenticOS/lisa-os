//! Integration tests for the broker surface (ADR-0008): consent gates
//! egress, every remote request is ledgered with the `remote.` marking
//! before it leaves, and the proxy path works end-to-end against a mock
//! provider (network paths mockable — no real egress in tests) — both
//! non-streaming and TRUE streaming (SSE proxied over the socket).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Json;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use http_body_util::BodyExt;
use lisa_remoted::{api, service::Broker};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    _dir: tempfile::TempDir,
    broker: Arc<Broker>,
    ledger: Arc<lisa_ledger::Ledger>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.path().join("ledger.db")).unwrap());
    let broker = Broker::open(&dir.path().join("state"), Arc::clone(&ledger)).unwrap();
    Fixture {
        _dir: dir,
        broker,
        ledger,
    }
}

async fn body_json(res: axum::response::Response) -> Value {
    serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn chat_request_body(provider: &str, scopes: &str, body: Value) -> Request<Body> {
    Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-lisa-provider", provider)
        .header("x-lisa-scopes", scopes)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn chat_request(provider: &str, scopes: &str) -> Request<Body> {
    chat_request_body(
        provider,
        scopes,
        json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "leave my machine"}],
        }),
    )
}

fn stream_request(provider: &str, scopes: &str) -> Request<Body> {
    chat_request_body(
        provider,
        scopes,
        json!({
            "model": "test-model",
            "stream": true,
            "messages": [{"role": "user", "content": "leave my machine"}],
        }),
    )
}

/// A fake OpenAI-compatible provider on loopback.
async fn mock_provider() -> String {
    async fn completions(Json(body): Json<Value>) -> Json<Value> {
        Json(json!({
            "id": "cmpl-mock",
            "object": "chat.completion",
            "model": body["model"],
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "mock says hi"},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 4, "completion_tokens": 3, "total_tokens": 7},
        }))
    }
    let app = axum::Router::new().route("/v1/chat/completions", post(completions));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/v1")
}

/// A fake *streaming* OpenAI-compatible provider: SSE chunks, a usage
/// frame, `[DONE]`. `truncate` cuts the stream off before `[DONE]`.
async fn mock_stream_provider(truncate: bool) -> String {
    async fn completions(truncate: bool) -> impl axum::response::IntoResponse {
        let frames: Vec<String> = if truncate {
            vec![
                json!({"choices": [{"index": 0, "delta": {"content": "cut "}, "finish_reason": null}]})
                    .to_string(),
            ]
        } else {
            vec![
                json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": null}]})
                    .to_string(),
                json!({"choices": [{"index": 0, "delta": {"content": "mock "}, "finish_reason": null}]})
                    .to_string(),
                json!({"choices": [{"index": 0, "delta": {"content": "streams"}, "finish_reason": "stop"}]})
                    .to_string(),
                json!({"choices": [], "usage": {"completion_tokens": 7}}).to_string(),
                "[DONE]".to_string(),
            ]
        };
        Sse::new(futures::stream::iter(
            frames
                .into_iter()
                .map(|f| Ok::<_, Infallible>(Event::default().data(f))),
        ))
    }
    let app =
        axum::Router::new().route("/v1/chat/completions", post(move || completions(truncate)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/v1")
}

/// Collect the SSE `data:` payloads of a streamed broker response.
async fn sse_payloads(res: axum::response::Response) -> Vec<String> {
    let raw = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&raw)
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn health_identifies_the_egress_broker() {
    let f = fixture();
    let res = api::router(f.broker)
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["daemon"], "lisa-remoted");
    assert_eq!(body["egress"], "remote");
}

#[tokio::test]
async fn providers_list_includes_builtins_and_custom_rows() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));

    let res = router
        .clone()
        .oneshot(Request::get("/v1/providers").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = body_json(res).await;
    let ids: Vec<&str> = body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "openai",
            "anthropic",
            "tinker",
            "together",
            "fireworks",
            "huggingface",
            "moonshot",
            "google",
            "deepseek",
            "groq",
            "mistral",
            "xai",
            "openrouter",
            "perplexity",
        ]
    );

    let res = router
        .clone()
        .oneshot(
            Request::post("/v1/providers")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    // A LAN box now needs the caller to say so (#92);
                    // silence means the public-internet rules.
                    json!({"id": "homelab", "display_name": "Homelab",
                           "base_url": "http://10.0.0.2:8080/v1",
                           "allow_local": true})
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert!(
        body["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "homelab"),
        "custom provider registered"
    );

    // Built-ins cannot be removed.
    let res = router
        .oneshot(
            Request::delete("/v1/providers/openai")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn keys_are_write_only_presence_is_reported() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .clone()
        .oneshot(
            Request::put("/v1/providers/tinker/key")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"key": "tk-secret"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = router
        .oneshot(Request::get("/v1/providers").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = body_json(res).await;
    let raw = body.to_string();
    assert!(
        !raw.contains("tk-secret"),
        "key material must never be readable"
    );
    let tinker = body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "tinker")
        .unwrap();
    assert_eq!(tinker["has_credential"], true);
}

#[tokio::test]
async fn default_consent_refuses_egress_and_ledgers_the_denial() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .oneshot(chat_request("openai", "prompt"))
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "nothing leaves by default"
    );

    let entries = f.ledger.tail(10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "remote.generate");
    assert_eq!(entries[0].status, "denied");
    assert!(entries[0].detail.contains("\"egress\":\"remote\""));
}

#[tokio::test]
async fn consented_request_proxies_and_is_ledgered_before_and_after() {
    let f = fixture();
    let base = mock_provider().await;
    f.broker
        .add_provider(
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key("mock", "mk-1").unwrap();
    f.broker.set_consent("prompt", true).unwrap();
    let consent_rows = f.ledger.tail(10).unwrap().len();

    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .oneshot(chat_request("mock", "prompt"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["choices"][0]["message"]["content"], "mock says hi");

    let entries = f.ledger.tail(10).unwrap();
    assert_eq!(
        entries.len(),
        consent_rows + 2,
        "start + complete: {entries:?}"
    );
    assert_eq!(entries[1].kind, "remote.generate");
    assert_eq!(entries[1].status, "started");
    assert_eq!(entries[1].model, "mock:test-model");
    assert!(entries[1].detail.contains("\"egress\":\"remote\""));
    assert!(entries[1].preview.contains("leave my machine"));
    assert_eq!(entries[0].kind, "remote.complete");
    assert_eq!(entries[0].status, "ok");
    assert_eq!(entries[0].ref_id, Some(entries[1].id));
    assert_eq!(entries[0].output_tokens, 3);
}

#[tokio::test]
async fn streaming_request_proxies_sse_and_is_ledgered_before_and_after() {
    let f = fixture();
    let base = mock_stream_provider(false).await;
    f.broker
        .add_provider(
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key("mock", "mk-1").unwrap();
    f.broker.set_consent("prompt", true).unwrap();
    let consent_rows = f.ledger.tail(10).unwrap().len();

    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .oneshot(stream_request("mock", "prompt"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream"),
        "streamed responses are SSE over the socket"
    );
    let payloads = sse_payloads(res).await;
    assert_eq!(payloads.last().map(String::as_str), Some("[DONE]"));
    let text: String = payloads
        .iter()
        .filter_map(|p| serde_json::from_str::<Value>(p).ok())
        .filter_map(|c| {
            c["choices"][0]["delta"]["content"]
                .as_str()
                .map(String::from)
        })
        .collect();
    assert_eq!(text, "mock streams", "deltas arrive in order");

    let entries = f.ledger.tail(10).unwrap();
    assert_eq!(entries.len(), consent_rows + 2, "start + complete");
    assert_eq!(entries[1].kind, "remote.generate");
    assert_eq!(entries[1].status, "started");
    assert_eq!(entries[0].kind, "remote.complete");
    assert_eq!(entries[0].status, "ok");
    assert_eq!(entries[0].ref_id, Some(entries[1].id));
    assert_eq!(
        entries[0].output_tokens, 7,
        "provider-reported usage wins over chunk count"
    );
    assert!(entries[0].detail.contains("\"streamed\":true"));
    assert!(entries[0].detail.contains("\"output_chars\":12"));
}

#[tokio::test]
async fn truncated_provider_stream_surfaces_and_ledgers_an_error() {
    let f = fixture();
    let base = mock_stream_provider(true).await;
    f.broker
        .add_provider(
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key("mock", "mk-1").unwrap();
    f.broker.set_consent("prompt", true).unwrap();

    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .oneshot(stream_request("mock", "prompt"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "failure arrives mid-stream");
    let payloads = sse_payloads(res).await;
    assert_eq!(payloads.last().map(String::as_str), Some("[DONE]"));
    let error = &payloads[payloads.len() - 2];
    let error: Value = serde_json::from_str(error).unwrap();
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ended before completion"),
        "{error}"
    );

    let entries = f.ledger.tail(2).unwrap();
    assert_eq!(entries[0].kind, "remote.complete");
    assert_eq!(entries[0].status, "error");
    assert!(entries[0].detail.contains("ended before completion"));
}

#[tokio::test]
async fn streaming_never_bypasses_consent() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .oneshot(stream_request("openai", "prompt"))
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "stream:true hits the same consent gate"
    );
    let entries = f.ledger.tail(10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "remote.generate");
    assert_eq!(entries[0].status, "denied");
}

#[tokio::test]
async fn unconsented_scope_is_refused_even_when_prompt_is_allowed() {
    let f = fixture();
    let base = mock_provider().await;
    f.broker
        .add_provider(
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key("mock", "mk-1").unwrap();
    f.broker.set_consent("prompt", true).unwrap();

    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .oneshot(chat_request("mock", "prompt, mail"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body = body_json(res).await;
    assert!(body["error"]["message"].as_str().unwrap().contains("mail"));
}

#[tokio::test]
async fn missing_credential_is_a_precondition_failure_not_egress() {
    let f = fixture();
    f.broker.set_consent("prompt", true).unwrap();
    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .oneshot(chat_request("openai", "prompt"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn unknown_provider_is_404_and_missing_header_is_400() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .clone()
        .oneshot(chat_request("nope", "prompt"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = router
        .oneshot(
            Request::post("/v1/chat/completions")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"messages": []}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oauth_begin_rejects_key_only_providers() {
    let f = fixture();
    let router = api::router(f.broker);
    // A key-only provider has no OAuth flow (and binds no callback port).
    let res = router
        .oneshot(
            Request::post("/v1/oauth/tinker/begin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = body_json(res).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not support OAuth"),
        "{body}"
    );
}

#[tokio::test]
async fn oauth_state_reports_capability_and_logout_is_idempotent() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));

    let res = router
        .clone()
        .oneshot(Request::get("/v1/providers").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = body_json(res).await;
    let anthropic = body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "anthropic")
        .unwrap();
    assert_eq!(anthropic["oauth_capable"], true);
    assert_eq!(anthropic["connected"], false);

    // Logout without a session is a clean 200 no-op.
    let res = router
        .oneshot(
            Request::delete("/v1/oauth/anthropic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn consent_toggle_is_reflected_and_ledgered() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    let res = router
        .clone()
        .oneshot(
            Request::put("/v1/consent")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"scope": "screen", "allowed": true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["may_offload"]["screen"], true);
    assert_eq!(body["may_offload"]["prompt"], false);

    let entries = f.ledger.tail(5).unwrap();
    assert_eq!(entries[0].kind, "remote.consent");
}

/// Issue #92 over the socket, which is the surface the CLI uses: the
/// same LAN endpoint without `allow_local` is refused, and nothing is
/// written. Silence is not consent.
#[tokio::test]
async fn a_local_endpoint_needs_the_caller_to_say_so() {
    let f = fixture();
    let router = lisa_remoted::api::router(Arc::clone(&f.broker));
    for url in [
        "http://10.0.0.2:8080/v1",
        "http://127.0.0.1:11434/v1",
        "http://169.254.169.254/latest/meta-data",
    ] {
        let res = router
            .clone()
            .oneshot(
                Request::post("/v1/providers")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"id": "sneaky", "display_name": "x", "base_url": url}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::BAD_REQUEST,
            "{url} was registered without allow_local"
        );
    }
    assert!(
        f.broker.providers_json()["providers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["id"] != "sneaky"),
        "a refused provider was written anyway"
    );
}

/// Issue #109 over the same surface: a credential in the URL is refused
/// outright, whatever the caller says about locality — it would
/// otherwise be persisted and appended to an append-only Ledger.
#[tokio::test]
async fn a_credential_in_the_url_is_refused_over_the_socket() {
    let f = fixture();
    let router = lisa_remoted::api::router(Arc::clone(&f.broker));
    for allow_local in [false, true] {
        let res = router
            .clone()
            .oneshot(
                Request::post("/v1/providers")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"id": "corp", "display_name": "Corp",
                               "base_url": "https://alice:hunter2@llm.corp.example/v1",
                               "allow_local": allow_local})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
    let shown = f.broker.providers_json().to_string();
    assert!(
        !shown.contains("hunter2"),
        "the credential is readable: {shown}"
    );
}
