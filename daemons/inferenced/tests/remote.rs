//! Remote-provider routing round-trip (§5.11). A mock broker on a unix
//! socket stands in for lisa-remoted: this exercises the hand-rolled
//! HTTP/1.1-over-unix transport end to end — TRUE streaming (ADR-0010
//! update): the broker answers chunked SSE and the router yields real
//! deltas — and confirms the request carries the provider + scope
//! headers (and `stream:true`) the broker gates on.

use futures::StreamExt;
use lisa_inferenced::engine::{GenerateRequest, StubEngine};
use lisa_inferenced::openai::ChatMessage;
use lisa_inferenced::pool::{EngineProvider, SingleEngine};
use lisa_inferenced::remote::RemoteRouter;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// One chunked-transfer SSE frame carrying `data`.
fn sse_chunk(data: &str) -> Vec<u8> {
    let event = format!("data: {data}\n\n");
    format!("{:x}\r\n{}\r\n", event.len(), event).into_bytes()
}

fn delta_chunk(content: &str) -> Vec<u8> {
    sse_chunk(
        &serde_json::json!({
            "choices": [{"index": 0, "delta": {"content": content}, "finish_reason": null}]
        })
        .to_string(),
    )
}

/// Accept one connection, stream back SSE deltas whose content reflects
/// the provider + scopes headers (and stream flag) the router sent.
async fn spawn_mock_broker(path: std::path::PathBuf) {
    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (mut conn, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = conn.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        let provider = header(&req, "x-lisa-provider");
        let scopes = header(&req, "x-lisa-scopes");
        let streamed = req.contains("\"stream\":true");
        let mut resp = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
             Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            .to_vec();
        resp.extend(delta_chunk(&format!("routed via {provider}, ")));
        resp.extend(delta_chunk(&format!("scopes=[{scopes}], ")));
        resp.extend(delta_chunk(&format!("stream={streamed}")));
        resp.extend(sse_chunk("[DONE]"));
        resp.extend_from_slice(b"0\r\n\r\n");
        conn.write_all(&resp).await.unwrap();
        conn.shutdown().await.ok();
    });
}

fn header(req: &str, name: &str) -> String {
    req.lines()
        .find(|l| l.to_ascii_lowercase().starts_with(name))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default()
}

fn router(socket: std::path::PathBuf) -> RemoteRouter {
    let inner = Arc::new(SingleEngine {
        engine: Arc::new(StubEngine),
        name: "lisa-system-stub".into(),
    });
    RemoteRouter::new(inner, socket)
}

#[tokio::test]
async fn remote_model_routes_through_the_broker_socket() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("remoted.sock");
    spawn_mock_broker(sock.clone()).await;

    let engine = router(sock)
        .engine_for(Some("remote:huggingface:openai/gpt-oss-120b"))
        .await
        .unwrap();
    assert_eq!(engine.name(), "remote");

    let tokens: Vec<String> = engine
        .generate(GenerateRequest {
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
            grammar: None,
            max_tokens: None,
        })
        .map(|t| t.unwrap())
        .collect()
        .await;

    // Real deltas: one token per SSE chunk, in order — not re-chunked.
    assert_eq!(tokens.len(), 3, "one token per broker delta: {tokens:?}");
    let text = tokens.join("");
    assert!(text.contains("routed via huggingface"), "got: {text}");
    assert!(
        text.contains("scopes=[prompt]"),
        "scope header missing: {text}"
    );
    assert!(
        text.contains("stream=true"),
        "broker must be asked to stream: {text}"
    );
}

#[tokio::test]
async fn broker_denial_surfaces_as_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("remoted.sock");
    // Mock broker that denies (403 with an error body), like an
    // un-consented scope would.
    let listener = UnixListener::bind(&sock).unwrap();
    tokio::spawn(async move {
        let (mut conn, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = conn.read(&mut buf).await.unwrap();
        let body = serde_json::json!({
            "error": {"message": "scope 'prompt' not consented for offload"}
        })
        .to_string();
        let resp = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        conn.write_all(resp.as_bytes()).await.unwrap();
        conn.shutdown().await.ok();
    });

    let engine = router(sock)
        .engine_for(Some("remote:openai:gpt-4o"))
        .await
        .unwrap();
    let results: Vec<_> = engine
        .generate(GenerateRequest {
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            grammar: None,
            max_tokens: None,
        })
        .collect()
        .await;
    assert!(
        results.iter().any(|r| r
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("not consented"))),
        "denial should surface: {results:?}"
    );
}
