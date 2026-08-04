//! Integration tests for the OpenAI-compat surface. These are the M0
//! forerunners of the §5.1 acceptance block (which additionally requires a
//! real model, latency budgets, and the egress packet counter in CI).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use lisa_inferenced::{api, engine, scheduler};
use std::sync::Arc;

fn test_state() -> (tempfile::TempDir, api::AppState) {
    let dir = tempfile::tempdir().unwrap();
    let ledger = lisa_ledger::Ledger::open(dir.path().join("ledger.db")).unwrap();
    (
        dir,
        api::AppState {
            engines: Arc::new(lisa_inferenced::pool::SingleEngine {
                engine: Arc::new(engine::StubEngine),
                name: "lisa-system-stub".to_string(),
            }),
            scheduler: Arc::new(scheduler::Scheduler::new(1)),
            engine_kind: "stub".to_string(),
            model_name: "lisa-system-stub".to_string(),
            ledger: Arc::new(ledger),
        },
    )
}

fn test_router() -> axum::Router {
    let (dir, state) = test_state();
    std::mem::forget(dir); // keep the temp ledger alive for the test
    api::router(state)
}

#[tokio::test]
async fn every_inference_is_ledgered_before_and_after() {
    let (_dir, state) = test_state();
    let ledger = Arc::clone(&state.ledger);
    let router = api::router(state);
    let req = Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [{"role": "user", "content": "audit me"}]
            })
            .to_string(),
        ))
        .unwrap();
    let res = router.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let entries = ledger.tail(10).unwrap();
    assert_eq!(entries.len(), 2, "start + completion entries: {entries:?}");
    assert_eq!(entries[1].kind, "inference.generate");
    assert_eq!(entries[1].status, "started");
    assert!(entries[1].preview.contains("audit me"));
    assert_eq!(entries[0].kind, "inference.complete");
    assert_eq!(entries[0].status, "ok");
    assert_eq!(entries[0].ref_id, Some(entries[1].id));
    assert!(entries[0].output_tokens > 0);
}

/// Issue #225: the request forge-harness actually sends must be one this
/// daemon actually accepts.
///
/// The harness always streams and always attaches tools; this daemon
/// routed a non-empty `tools` array to a lane whose FIRST act was to
/// refuse `stream: true` with a 400. Every local-model run in the
/// Assistant came back as `backend: http status: 400`, and nothing in
/// either half could see the other.
///
/// The body here is not a hand-written copy of what the harness sends —
/// it IS what the harness sends, from `forge_harness::openai::
/// streaming_request_body`, the function `next_action_streaming` calls.
/// That is the mechanism: a change to the harness's request shape shows
/// up as a failure in this daemon's suite, which is what a comment
/// saying "these must match" could never do.
#[tokio::test]
async fn the_body_forge_harness_streams_is_one_this_daemon_accepts() {
    use forge_harness::{Message, tool_specs};

    let body = forge_harness::openai::streaming_request_body(
        Some("lisa-system-stub"),
        &[Message::system("policy"), Message::user("Task: hello")],
        &tool_specs(),
    );
    // Guard the premise: if these two ever stop being true the test
    // stops testing #225 and would pass for the wrong reason.
    assert_eq!(body["stream"], serde_json::json!(true));
    assert!(!body["tools"].as_array().unwrap().is_empty());

    let req = Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = test_router().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "inferenced refuses the request its own harness sends"
    );

    // And it is a real stream the harness can fold, not a 200 with an
    // apology in it: SSE chunks, ending in [DONE].
    let ct = res.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.starts_with("text/event-stream"), "content-type: {ct}");
    let text =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(text.trim_end().ends_with("data: [DONE]"), "body: {text}");

    // Folded by the harness's own frame reader — the consumer that
    // failed for real.
    let mut acc = forge_harness::openai::Accumulated::default();
    let mut sink = |_: &str| {};
    for line in text.lines() {
        if forge_harness::openai::fold_frame(&mut acc, line, &mut sink) {
            break;
        }
    }
    match forge_harness::openai::action_from(acc).unwrap() {
        forge_harness::AgentAction::Done(text) => {
            assert!(
                text.contains("hello"),
                "the stub's reply did not arrive: {text}"
            );
        }
        other => panic!("expected a plain reply from the stub, got {other:?}"),
    }
}

/// A failed run is exactly the run you want in the Ledger.
///
/// The 400 above landed BEFORE `ledger_gate`, so the request that broke
/// every Assistant run left no record at all — the audit log showed a
/// quiet day. Whatever this lane answers, it answers after saying so.
#[tokio::test]
async fn a_tools_request_is_ledgered_even_when_it_cannot_be_served() {
    let (_dir, state) = test_state();
    let ledger = Arc::clone(&state.ledger);
    // `tools` as an object is not a tool list: a request-shape refusal,
    // the same class as the streaming 400 that recorded nothing.
    let req = Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [{"role": "user", "content": "audit my failure"}],
                "tools": {"not": "an array"},
            })
            .to_string(),
        ))
        .unwrap();
    let res = api::router(state).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let entries = ledger.tail(10).unwrap();
    assert_eq!(entries.len(), 1, "a refusal left no trace: {entries:?}");
    assert_eq!(entries[0].kind, "inference.generate");
    assert_eq!(entries[0].status, "refused");
    assert!(
        entries[0].detail.contains("tools must be an array"),
        "the ledger does not say why: {:?}",
        entries[0].detail
    );
}

/// Issue #226: an attached image over ~1.5 MB died as `413`.
///
/// Nothing in this repo had ever named a request-size limit, so the one
/// in force was axum's DEFAULT — 2 MiB — and a base64 data: URI is 4/3
/// of the file it carries, which put the ceiling at a ~1.5 MB picture.
/// The image-attachment feature (#209) therefore could not carry a
/// screenshot at all.
///
/// The limit is now `api::MAX_REQUEST_BYTES` and it is written down. This
/// test is the mechanism that keeps it written down: it fails if the
/// explicit layer is ever dropped, because the default would take over
/// and 3 MiB would 413 again.
#[tokio::test]
async fn a_request_bigger_than_axums_default_is_accepted() {
    // 3 MiB: over axum's 2 MiB default, well under ours. A real
    // screenshot attachment lands in exactly this band.
    let filler = "x".repeat(3 * 1024 * 1024);
    let req = Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"messages": [{"role": "user", "content": filler}]}).to_string(),
        ))
        .unwrap();
    let res = test_router().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "a 3 MiB request must not be refused for its size"
    );
}

/// The limit is a limit, not a suggestion: past it the request is
/// refused rather than buffered. An unbounded body is how one attachment
/// takes the daemon's memory with it.
#[tokio::test]
async fn a_request_past_the_limit_is_refused() {
    let filler = "x".repeat(api::MAX_REQUEST_BYTES + 1024);
    let req = Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({"messages": [{"role": "user", "content": filler}]}).to_string(),
        ))
        .unwrap();
    let res = test_router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// The chain this number has to satisfy — the Assistant's composer caps
// a send at 16 MiB of image bytes, base64 makes that 21.4 MiB, harnessd
// refuses an `attachments` option over 24 MiB, and that whole request
// then arrives here — is asserted at COMPILE time next to the constant
// in `api.rs`, not from here. A limit below the hop before it means a
// person is told "attached" and then told "413", which is #226 again one
// layer down, and that is worth failing the build over rather than a
// test run.

#[tokio::test]
async fn health_reports_ok_and_engine() {
    let res = test_router()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["engine"], "stub");
}

#[tokio::test]
async fn models_lists_the_resident_model() {
    let res = test_router()
        .oneshot(Request::get("/v1/models").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"][0]["id"], "lisa-system-stub");
}

#[tokio::test]
async fn chat_completion_non_streaming_echoes_prompt() {
    let req = Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [{"role": "user", "content": "write a haiku about entropy"}]
            })
            .to_string(),
        ))
        .unwrap();
    let res = test_router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let content = body["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(
        content.contains("write a haiku about entropy"),
        "got: {content}"
    );
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert_eq!(body["object"], "chat.completion");
}

#[tokio::test]
async fn chat_completion_streaming_emits_sse_chunks_and_done() {
    let req = Request::post("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [{"role": "user", "content": "stream me"}],
                "stream": true
            })
            .to_string(),
        ))
        .unwrap();
    let res = test_router().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.starts_with("text/event-stream"), "content-type: {ct}");

    let body =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body.contains("chat.completion.chunk"), "body: {body}");
    assert!(body.trim_end().ends_with("data: [DONE]"), "body: {body}");

    // Reassemble the deltas the way a real SSE client would.
    let content: String = body
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter(|d| *d != "[DONE]")
        .filter_map(|d| serde_json::from_str::<serde_json::Value>(d).ok())
        .filter_map(|c| {
            c["choices"][0]["delta"]["content"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    assert!(content.contains("stream me"), "reassembled: {content}");
}
