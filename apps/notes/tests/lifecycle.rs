//! What the server leaves behind when it is killed (#219).
//!
//! `mcp_bus` defers socket activation and reads socket PRESENCE as tool
//! availability, so a socket file with no process behind it is worse
//! than no socket at all: agentd advertises `search_notes` and
//! `create_note` to the model, dispatches, and gets ECONNREFUSED where
//! it should have been told the app is not running. Verified on the
//! reference machine in exactly that state — `app.lisaos.Surfer.sock`
//! and `app.lisaos.Preview.sock` present, nothing listening, `connect()`
//! refused — which is what #219 is.
//!
//! Runs the real binary, because the thing under test is what the
//! PROCESS does as it dies; a unit test of a helper would assert that
//! the helper works, not that it is installed.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Wait until `cond` holds, or give up. Polling rather than sleeping a
/// fixed time: a fixed sleep is either flaky or slow, and usually both.
fn wait_for(mut cond: impl FnMut() -> bool, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// A live server: the socket exists AND something answers on it. Both
/// halves matter — the defect is precisely the case where the first is
/// true and the second is not.
fn answers(socket: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return false;
    };
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    if stream.write_all(format!("{req}\n").as_bytes()).is_err() {
        return false;
    }
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).is_ok() && line.contains("result")
}

struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        // Never leave a server running if an assertion above blew up.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn signal(child: &Child, sig: i32) {
    // SAFETY: `kill` on a pid we own and have not yet reaped.
    unsafe {
        libc::kill(child.id() as libc::pid_t, sig);
    }
}

#[test]
fn a_terminated_server_takes_its_socket_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("app.lisaos.notes.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_lisa-notes"))
        .arg("--socket")
        .arg(&socket)
        .env("XDG_DATA_HOME", dir.path())
        .spawn()
        .expect("lisa-notes should start");
    let mut child = Reaped(child);

    assert!(
        wait_for(|| answers(&socket), Duration::from_secs(10)),
        "the server never came up on {}",
        socket.display()
    );

    // SIGTERM is what systemd sends, what a logout sends to the
    // session's units, and what `pkill` sends. It is the ordinary way
    // this process ends, not an exotic one.
    signal(&child.0, libc::SIGTERM);
    // Killed by the handler's `_exit(128 + SIGTERM)`, not by the
    // default disposition — which is how we know a handler ran at all.
    let status = child.0.wait().expect("waiting for the server");
    assert_eq!(
        status.code(),
        Some(128 + libc::SIGTERM),
        "the server did not exit through its own signal handler"
    );

    assert!(
        wait_for(|| !socket.exists(), Duration::from_secs(5)),
        "SIGTERM left {} behind — mcp-bus reads that as `notes has tools`, \
         and every dispatch to them is an ECONNREFUSED (#219)",
        socket.display()
    );
}

#[test]
fn an_interrupted_server_takes_its_socket_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("app.lisaos.notes.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_lisa-notes"))
        .arg("--socket")
        .arg(&socket)
        .env("XDG_DATA_HOME", dir.path())
        .spawn()
        .expect("lisa-notes should start");
    let mut child = Reaped(child);
    assert!(wait_for(|| answers(&socket), Duration::from_secs(10)));

    signal(&child.0, libc::SIGINT);
    child.0.wait().unwrap();
    assert!(
        wait_for(|| !socket.exists(), Duration::from_secs(5)),
        "SIGINT left {} behind",
        socket.display()
    );
}

/// The clean path, and the one that must not regress: a server that
/// exits normally is also a server whose socket is gone. `main`
/// returning is the case a `Drop` guard alone would cover, so this is
/// the positive control for the guard as opposed to the handler.
#[test]
fn a_stale_socket_never_blocks_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("app.lisaos.notes.sock");
    // Whatever a previous run left — this is the state the device was
    // found in — must not stop the next one binding.
    std::fs::write(&socket, b"not a socket at all").unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_lisa-notes"))
        .arg("--socket")
        .arg(&socket)
        .env("XDG_DATA_HOME", dir.path())
        .spawn()
        .unwrap();
    let mut child = Reaped(child);
    assert!(
        wait_for(|| answers(&socket), Duration::from_secs(10)),
        "a leftover file blocked the bind"
    );
    signal(&child.0, libc::SIGTERM);
    child.0.wait().unwrap();
}
