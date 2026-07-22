//! Remote-provider routing round-trip (§5.11). A mock broker on a unix
//! socket stands in for lisa-remoted: this exercises the hand-rolled
//! HTTP/1.1-over-unix transport end to end, and confirms the request
//! carries the provider + scope headers the broker gates on.

use futures::StreamExt;
use lisa_inferenced::engine::{GenerateRequest, StubEngine};
use lisa_inferenced::pool::{EngineProvider, SingleEngine};
use lisa_inferenced::openai::ChatMessage;
use lisa_inferenced::remote::RemoteRouter;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// Accept one connection, echo back an OpenAI completion whose content
/// reflects the provider + scopes headers the router sent.
async fn spawn_mock_broker(path: std::path::PathBuf) {
    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (mut conn, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = conn.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        let provider = header(&req, "x-lisa-provider");
        let scopes = header(&req, "x-lisa-scopes");
        let content = format!("routed via {provider}, scopes=[{scopes}]");
        let body = serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": content}}]
        })
        .to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        conn.write_all(resp.as_bytes()).await.unwrap();
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

    let text: String = engine
        .generate(GenerateRequest {
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
            grammar: None,
            max_tokens: None,
        })
        .map(|t| t.unwrap())
        .collect::<Vec<_>>()
        .await
        .join("");

    assert!(text.contains("routed via huggingface"), "got: {text}");
    assert!(text.contains("scopes=[prompt]"), "scope header missing: {text}");
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
        results.iter().any(|r| r.as_ref().err().is_some_and(|e| e
            .to_string()
            .contains("not consented"))),
        "denial should surface: {results:?}"
    );
}
