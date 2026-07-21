//! org.lisa.Inference1 over zbus peer-to-peer connections (PLAN Appendix
//! A). P2P over a socketpair needs no bus daemon, so this runs on macOS
//! dev hosts and CI alike; real session-bus registration is exercised on
//! Linux systems.

use lisa_inferenced::dbus::Inference1;
use lisa_inferenced::engine::StubEngine;
use lisa_inferenced::scheduler::Scheduler;
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

async fn p2p_pair() -> (zbus::Connection, zbus::Connection) {
    let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
    let guid = zbus::Guid::generate();
    // Build both ends concurrently: each build() awaits the handshake,
    // which needs the peer to be handshaking too.
    let server_fut = zbus::connection::Builder::unix_stream(server_sock)
        .server(guid)
        .unwrap()
        .p2p()
        .serve_at(
            "/org/lisa/Inference1",
            Inference1::new(Arc::new(StubEngine), Arc::new(Scheduler::new(1))),
        )
        .unwrap()
        .build();
    let client_fut = zbus::connection::Builder::unix_stream(client_sock)
        .p2p()
        .build();
    let (server, client) = tokio::try_join!(server_fut, client_fut).unwrap();
    (server, client)
}

async fn open_session(client: &zbus::Connection) -> (OwnedObjectPath, std::os::fd::OwnedFd) {
    let proxy = zbus::Proxy::new(
        client,
        "org.lisa.Inference1",
        "/org/lisa/Inference1",
        "org.lisa.Inference1",
    )
    .await
    .unwrap();
    let reply = proxy
        .call_method("OpenSession", &(HashMap::<String, OwnedValue>::new(),))
        .await
        .unwrap();
    let (path, fd): (OwnedObjectPath, zbus::zvariant::OwnedFd) =
        reply.body().deserialize().unwrap();
    (path, fd.into())
}

#[tokio::test]
async fn generate_streams_tokens_over_the_fd_until_eof() {
    let (_server, client) = p2p_pair().await;
    let (path, fd) = open_session(&client).await;

    let session = zbus::Proxy::new(
        &client,
        "org.lisa.Inference1",
        path.clone(),
        "org.lisa.Inference1.Session",
    )
    .await
    .unwrap();
    session
        .call_method(
            "Generate",
            &("hello over dbus", HashMap::<String, OwnedValue>::new()),
        )
        .await
        .unwrap();

    // Read the pipe to EOF off the async runtime.
    let text = tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::from(fd);
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        s
    })
    .await
    .unwrap();
    assert!(text.contains("hello over dbus"), "streamed: {text}");
}

#[tokio::test]
async fn embed_returns_deterministic_vectors() {
    let (_server, client) = p2p_pair().await;
    let (path, _fd) = open_session(&client).await;

    let session = zbus::Proxy::new(
        &client,
        "org.lisa.Inference1",
        path,
        "org.lisa.Inference1.Session",
    )
    .await
    .unwrap();
    let reply = session
        .call_method("Embed", &(vec!["alpha", "beta", "alpha"],))
        .await
        .unwrap();
    let (vectors,): (Vec<Vec<f64>>,) = reply.body().deserialize().unwrap();
    assert_eq!(vectors.len(), 3);
    assert_eq!(vectors[0].len(), 8);
    assert_eq!(vectors[0], vectors[2], "same text must embed identically");
    assert_ne!(vectors[0], vectors[1]);
}

#[tokio::test]
async fn close_removes_the_session_object() {
    let (_server, client) = p2p_pair().await;
    let (path, _fd) = open_session(&client).await;

    let session = zbus::Proxy::new(
        &client,
        "org.lisa.Inference1",
        path.clone(),
        "org.lisa.Inference1.Session",
    )
    .await
    .unwrap();
    session.call_method("Close", &()).await.unwrap();
    // A second call must fail: the object is gone.
    let err = session.call_method("Cancel", &()).await;
    assert!(err.is_err(), "session should be unregistered after Close");
}
