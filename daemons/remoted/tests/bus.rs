//! The management plane against real transports (issue #99).
//!
//! `tests/dbus.rs` and `tests/api.rs` cover the negative half — a caller
//! with no credentials changes nothing — because that is what a p2p
//! socket and a bare router can express. They cannot express the other
//! half: that an allowlisted program *can* still manage. Without it,
//! "everything is refused" would be satisfied by a plane that is simply
//! broken, and the day Settings stopped working nobody would know which
//! commit did it.
//!
//! So this file runs both planes for real:
//!
//! - **D-Bus**, against a private `dbus-daemon`, so the caller has a
//!   unique name and the broker supplies credentials.
//! - **The unix socket**, accepted through `axum::serve` with connect
//!   info, so the kernel answers `SO_PEERCRED`/`SO_PEERPIDFD` about a
//!   genuine connection.
//!
//! In both, the allowlist contains this test binary — the same shape as
//! Settings being in the shipped one.
//!
//! # Where each half can run
//!
//! Naming the caller's program needs `/proc/<pid>/exe` through a pidfd,
//! which is Linux — and, for D-Bus, a broker new enough to pass one
//! (`ProcessFD`, dbus 1.16). Elsewhere the calls are refused, which is
//! the documented degradation and is asserted as such rather than
//! skipped. `LISA_REQUIRE_BUS_TESTS=1` makes a missing `dbus-daemon`
//! fatal; `LISA_REQUIRE_PIDFD=1` makes a missing pidfd fatal. CI sets
//! both in the job that runs on a base new enough to hold them.

use lisa_remoted::api;
use lisa_remoted::dbus::Remote1;
use lisa_remoted::service::Broker;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;

const BUS_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

struct Bus {
    child: std::process::Child,
    address: String,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_bus() -> Option<Bus> {
    let mut child = match std::process::Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return require_or_skip("dbus-daemon is not installed"),
    };
    let stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let got = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(got.ok().map(|_| line));
    });
    match rx.recv_timeout(BUS_START_TIMEOUT) {
        Ok(Some(line)) if !line.trim().is_empty() => Some(Bus {
            child,
            address: line.trim().to_string(),
        }),
        _ => {
            let _ = child.kill();
            require_or_skip("dbus-daemon did not print an address in time")
        }
    }
}

fn require_or_skip(why: &str) -> Option<Bus> {
    if std::env::var_os("LISA_REQUIRE_BUS_TESTS").is_some() {
        panic!("LISA_REQUIRE_BUS_TESTS is set but the bus tests cannot run: {why}");
    }
    eprintln!("skipping bus test: {why} (set LISA_REQUIRE_BUS_TESTS=1 to make this fatal)");
    None
}

/// Whether identity is establishable here at all. When it is not, the
/// documented behaviour is that management is refused — which is what
/// the caller then asserts.
fn pidfd_expected(has_pidfd: bool) -> bool {
    if has_pidfd {
        return true;
    }
    if std::env::var_os("LISA_REQUIRE_PIDFD").is_some() {
        panic!(
            "LISA_REQUIRE_PIDFD is set but no pidfd is available here — \
             the management plane cannot identify anyone on this machine"
        );
    }
    eprintln!("no pidfd available: asserting the refused branch instead");
    false
}

fn broker() -> (tempfile::TempDir, Arc<Broker>, Arc<lisa_ledger::Ledger>) {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.path().join("ledger.db")).unwrap());
    let broker = Broker::open(&dir.path().join("state"), Arc::clone(&ledger)).unwrap();
    (dir, broker, ledger)
}

fn me() -> PathBuf {
    std::env::current_exe().unwrap().canonicalize().unwrap()
}

/// The D-Bus plane: an allowlisted program flips a consent scope, and
/// the Ledger names it.
#[tokio::test]
async fn an_allowlisted_program_can_manage_over_dbus() {
    let Some(bus) = start_bus() else { return };
    let (_dir, b, ledger) = broker();

    let _server = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("dev.lisaos.Remoted")
        .unwrap()
        .serve_at(
            "/dev/lisaos/Remote1",
            Remote1::with_managers(Arc::clone(&b), vec![me()]),
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let client = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let p = zbus::Proxy::new(
        &client,
        "dev.lisaos.Remoted",
        "/dev/lisaos/Remote1",
        "dev.lisaos.Remote1",
    )
    .await
    .unwrap();

    let has_pidfd = {
        let dbus = zbus::fdo::DBusProxy::new(&client).await.unwrap();
        let me = client.unique_name().unwrap().to_owned();
        dbus.get_connection_credentials(me.into())
            .await
            .map(|c| c.process_fd().is_some())
            .unwrap_or(false)
    };

    let flipped = p.call_method("SetConsent", &("prompt", true)).await;
    if !pidfd_expected(has_pidfd) {
        assert!(
            flipped.is_err(),
            "a plane that cannot identify anyone must not authorize anyone"
        );
        assert_eq!(b.consent_json()["may_offload"]["prompt"], false);
        return;
    }

    flipped.expect("an allowlisted program must be able to flip a scope");
    assert_eq!(b.consent_json()["may_offload"]["prompt"], true);

    // The Ledger names the program, not a fixed "settings" label. That
    // was the audit half of #99: the one surface that could have shown
    // "something turned on your egress" blamed the panel for it.
    let entry = ledger
        .tail(1)
        .unwrap()
        .into_iter()
        .next()
        .expect("a ledger entry");
    assert_eq!(entry.kind, "remote.consent");
    assert!(
        entry.app_id.contains("bus-"),
        "the consent entry should name the caller, got {:?}",
        entry.app_id
    );
}

/// The socket plane, through a real `accept()` so the kernel answers
/// about a genuine peer.
#[tokio::test]
async fn an_allowlisted_program_can_manage_over_the_socket() {
    let (_dir, b, _ledger) = broker();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();

    let router = api::router_with_managers(Arc::clone(&b), api::Managers(Arc::new(vec![me()])));
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<api::PeerInfo>(),
        )
        .await
    });

    // A plain HTTP/1.1 request over the socket — no client crate needed,
    // and it keeps the test honest about what actually crosses the wire.
    let body = serde_json::json!({"scope": "files", "allowed": true}).to_string();
    let req = format!(
        "PUT /v1/consent HTTP/1.1\r\nHost: lisa\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let status = {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        sock.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).await.unwrap();
        String::from_utf8_lossy(&resp)
            .lines()
            .next()
            .unwrap()
            .to_string()
    };
    server.abort();

    // The peer of this connection is the test binary itself, so a pidfd
    // is available exactly where Linux offers one.
    let has_pidfd = {
        let (a, _b2) = std::os::unix::net::UnixStream::pair().unwrap();
        lisa_peer::unix::peer_of_socket(&a).process_fd.is_some()
    };
    if !pidfd_expected(has_pidfd) {
        assert!(status.contains("403"), "expected a refusal, got {status:?}");
        assert_eq!(b.consent_json()["may_offload"]["files"], false);
        return;
    }

    assert!(status.contains("200"), "expected 200, got {status:?}");
    assert_eq!(
        b.consent_json()["may_offload"]["files"],
        true,
        "an allowlisted program could not flip a scope over the socket"
    );
}

/// The same socket, the same real connection — and a caller that is not
/// on the allowlist. This is the exploit from the issue, over the
/// transport it was demonstrated on.
#[tokio::test]
async fn an_unlisted_program_is_refused_over_the_socket() {
    let (_dir, b, _ledger) = broker();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();

    // An allowlist that exists but does not contain us: the difference
    // between "no policy" and "you are not in it".
    let router = api::router_with_managers(
        Arc::clone(&b),
        api::Managers(Arc::new(vec![dir.path().join("some-other-program")])),
    );
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<api::PeerInfo>(),
        )
        .await
    });

    let body = serde_json::json!({"scope": "mail", "allowed": true}).to_string();
    let req = format!(
        "PUT /v1/consent HTTP/1.1\r\nHost: lisa\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let status = {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        sock.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).await.unwrap();
        String::from_utf8_lossy(&resp)
            .lines()
            .next()
            .unwrap()
            .to_string()
    };
    server.abort();

    assert!(status.contains("403"), "expected a refusal, got {status:?}");
    assert_eq!(
        b.consent_json()["may_offload"]["mail"],
        false,
        "an unlisted program turned on mail offload"
    );
}
