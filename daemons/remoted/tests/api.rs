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

/// A `Manager` for tests that need to set state up.
///
/// Not a backdoor: it goes through the real `authorize`, with the test
/// binary in its own allowlist — the same shape as Settings being in
/// the shipped one. There is deliberately no `#[cfg(test)]` constructor
/// in the daemon, because that is a door somebody eventually walks
/// through in production.
fn as_manager() -> lisa_peer::manager::Manager {
    let me = std::env::current_exe().unwrap().canonicalize().unwrap();
    lisa_peer::manager::Manager::authorize(true, Some(&me), std::slice::from_ref(&me)).unwrap()
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

    // Registering is manager-only (#99); the listing is what this test
    // is about, and reading it stays open.
    f.broker
        .add_provider(
            &as_manager(),
            "homelab",
            "Homelab",
            "http://10.0.0.2:8080/v1",
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    let res = router
        .clone()
        .oneshot(Request::get("/v1/providers").body(Body::empty()).unwrap())
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

    // Built-ins cannot be removed — a rule about the row, not about the
    // caller, so it is asserted where a caller exists.
    assert!(
        f.broker.remove_provider(&as_manager(), "openai").is_err(),
        "a built-in was removed"
    );
}

#[tokio::test]
async fn keys_are_write_only_presence_is_reported() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    // Written through the broker: storing a key is manager-only now
    // (#99), and what this test is about is that the value never comes
    // back out — not who may put it in.
    f.broker
        .set_key(&as_manager(), "tinker", "tk-secret")
        .unwrap();

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

/// Issue #226, one hop further out. An attached image only reaches a
/// model that can see one, and that model is always a remote — so the
/// picture crosses THIS socket too. Raising inferenced's limit alone
/// would have moved the 413 here and changed nothing a person could see.
///
/// Size is checked before consent: a refusal must be about egress, not
/// about buffering. So the assertion is only that the answer is not
/// `413` — 403 (no consent yet) is the correct answer to this request.
#[tokio::test]
async fn a_request_bigger_than_axums_default_is_not_refused_for_its_size() {
    let f = fixture();
    let filler = "x".repeat(3 * 1024 * 1024);
    let res = api::router(Arc::clone(&f.broker))
        .oneshot(chat_request_body(
            "openai",
            "prompt",
            json!({"model": "test-model", "messages": [{"role": "user", "content": filler}]}),
        ))
        .await
        .unwrap();
    assert_ne!(
        res.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a 3 MiB request died on the broker's body limit"
    );
}

/// And it is still a bound: past the limit the body is refused rather
/// than buffered.
#[tokio::test]
async fn a_request_past_the_limit_is_refused() {
    let f = fixture();
    let filler = "x".repeat(api::MAX_REQUEST_BYTES + 1024);
    let res = api::router(Arc::clone(&f.broker))
        .oneshot(chat_request_body(
            "openai",
            "prompt",
            json!({"model": "test-model", "messages": [{"role": "user", "content": filler}]}),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn consented_request_proxies_and_is_ledgered_before_and_after() {
    let f = fixture();
    let base = mock_provider().await;
    f.broker
        .add_provider(
            &as_manager(),
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key(&as_manager(), "mock", "mk-1").unwrap();
    f.broker.set_consent(&as_manager(), "prompt", true).unwrap();
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
            &as_manager(),
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key(&as_manager(), "mock", "mk-1").unwrap();
    f.broker.set_consent(&as_manager(), "prompt", true).unwrap();
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
            &as_manager(),
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key(&as_manager(), "mock", "mk-1").unwrap();
    f.broker.set_consent(&as_manager(), "prompt", true).unwrap();

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
            &as_manager(),
            "mock",
            "Mock",
            &base,
            lisa_remoted::net::Locality::LocalAllowed,
        )
        .unwrap();
    f.broker.set_key(&as_manager(), "mock", "mk-1").unwrap();
    f.broker.set_consent(&as_manager(), "prompt", true).unwrap();

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
    f.broker.set_consent(&as_manager(), "prompt", true).unwrap();
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
    // A key-only provider has no OAuth flow (and binds no callback
    // port). Driven through the broker because starting a login is
    // manager-only (#99); the refusal for everyone else is asserted in
    // `management_routes_refuse_a_caller_with_no_credentials`.
    let err = f
        .broker
        .begin_login(&as_manager(), "tinker")
        .await
        .expect_err("a key-only provider has no OAuth flow");
    assert!(err.to_string().contains("does not support OAuth"), "{err}");
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

    // Logout without a session is a clean no-op.
    f.broker.logout(&as_manager(), "anthropic").unwrap();
}

#[tokio::test]
async fn consent_toggle_is_reflected_and_ledgered() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    f.broker.set_consent(&as_manager(), "screen", true).unwrap();

    // Reading consent stays open — the Settings page and the CLI both
    // render it, and knowing what is switched on is not the risk.
    let res = router
        .oneshot(Request::get("/v1/consent").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["may_offload"]["screen"], true);
    assert_eq!(body["may_offload"]["prompt"], false);

    let entries = f.ledger.tail(5).unwrap();
    assert_eq!(entries[0].kind, "remote.consent");
    // And it names the program that did it, not a fixed "settings"
    // label that would blame the panel for somebody else's action.
    assert!(
        entries[0].app_id.contains("api-"),
        "the consent entry should name the caller, got {:?}",
        entries[0].app_id
    );
}

/// Issue #92 over the socket, which is the surface the CLI uses: the
/// same LAN endpoint without `allow_local` is refused, and nothing is
/// written. Silence is not consent.
#[tokio::test]
async fn a_local_endpoint_needs_the_caller_to_say_so() {
    let f = fixture();
    for url in [
        "http://10.0.0.2:8080/v1",
        "http://127.0.0.1:11434/v1",
        "http://169.254.169.254/latest/meta-data",
    ] {
        assert!(
            f.broker
                .add_provider(
                    &as_manager(),
                    "sneaky",
                    "x",
                    url,
                    lisa_remoted::net::Locality::PublicOnly
                )
                .is_err(),
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

/// Issue #109: a credential in the URL is refused outright, whatever the
/// caller says about locality — it would otherwise be persisted and
/// appended to an append-only Ledger.
#[tokio::test]
async fn a_credential_in_the_url_is_refused() {
    let f = fixture();
    for locality in [
        lisa_remoted::net::Locality::PublicOnly,
        lisa_remoted::net::Locality::LocalAllowed,
    ] {
        assert!(
            f.broker
                .add_provider(
                    &as_manager(),
                    "corp",
                    "Corp",
                    "https://alice:hunter2@llm.corp.example/v1",
                    locality
                )
                .is_err(),
            "userinfo accepted for {locality:?}"
        );
    }
    let shown = f.broker.providers_json().to_string();
    assert!(
        !shown.contains("hunter2"),
        "the credential is readable: {shown}"
    );
}

/// Issue #99, on the socket plane. The filed exploit was six unauthenticated
/// `PUT /v1/consent` calls turning on every offload scope, after which
/// `screen`, `mail`, `files` and `memory` content could be proxied out
/// through the broker — with the only trace a `remote.consent` row
/// blaming Settings.
///
/// A router built without connect info is a caller nobody vouched for,
/// which is the same answer an unauthorized program gets.
#[tokio::test]
async fn management_routes_refuse_a_caller_with_no_credentials() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));

    let json_req = |method: &str, path: &str, body: Value| {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let empty_req = |method: &str, path: &str| {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap()
    };

    let attempts = vec![
        json_req(
            "PUT",
            "/v1/consent",
            json!({"scope": "mail", "allowed": true}),
        ),
        json_req(
            "PUT",
            "/v1/consent",
            json!({"scope": "screen", "allowed": true}),
        ),
        json_req(
            "POST",
            "/v1/providers",
            json!({"id": "sink", "display_name": "Sink", "base_url": "https://attacker.example/v1"}),
        ),
        json_req(
            "PUT",
            "/v1/providers/openai/key",
            json!({"key": "sk-attacker"}),
        ),
        empty_req("DELETE", "/v1/providers/openai/key"),
        empty_req("DELETE", "/v1/providers/openai"),
        empty_req("POST", "/v1/oauth/anthropic/begin"),
        empty_req("DELETE", "/v1/oauth/anthropic"),
    ];
    for req in attempts {
        let (method, path) = (req.method().clone(), req.uri().clone());
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::FORBIDDEN,
            "{method} {path} was allowed to an unidentified caller"
        );
    }

    // Nothing moved. This is the half that matters: a refusal that
    // still wrote would be worse than no refusal, because it would look
    // safe.
    let consent = f.broker.consent_json();
    for scope in ["prompt", "files", "mail", "calendar", "screen", "memory"] {
        assert_eq!(
            consent["may_offload"][scope], false,
            "{scope} was turned on by an unidentified caller"
        );
    }
    assert!(
        f.broker.providers_json()["providers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["id"] != "sink"),
        "an egress endpoint was registered by an unidentified caller"
    );
}

/// Reads stay open, and must: the Settings page and `lisa remote list`
/// both render them, and the data plane's caller needs none of this. A
/// fix that locked everything would be found by someone noticing their
/// panel had gone blank, which is not how a boundary should announce
/// itself.
#[tokio::test]
async fn reads_stay_open_to_an_unidentified_caller() {
    let f = fixture();
    let router = api::router(Arc::clone(&f.broker));
    for path in ["/health", "/v1/providers", "/v1/consent"] {
        let res = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path} was refused");
    }
}
