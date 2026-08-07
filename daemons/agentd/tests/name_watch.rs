//! #347, witnessed: the daemon must NOTICE losing its bus, not idle.
//!
//! The incident this pins: the session bus dropped agentd's socket,
//! zbus's internal name monitor logged one WARN, and the process sat
//! `active (running)` for hours — `Restart=on-failure` never fired,
//! `dev.lisaos.Agent1` was unowned (the #306 squat window), and the
//! Assistant silently ran without its prompt-class tools.
//!
//! So this test runs the real thing: a private `dbus-daemon`, a real
//! connection owning the real name, the watcher armed — and then the
//! bus is killed out from under it, exactly as on the device. The
//! watcher must resolve; `main` turns that resolution into `exit(1)`.
//!
//! **Where it runs.** Linux, and CI is the proof: the `bus-identity`
//! job installs dbus and sets `LISA_REQUIRE_BUS_TESTS=1`, which turns a
//! skip into a failure, so this cannot quietly stop running. On macOS
//! dev hosts the homebrew daemon prints no address under a test runner
//! (its stock config listens on a `launchd:` socket), so the fixture
//! skips there LOUDLY rather than pretending. Same fixture, same
//! contract, same reasons as `daemons/remoted/tests/bus.rs`.

const BUS_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const LOSS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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

/// Byte-for-byte the fixture `daemons/remoted/tests/bus.rs` uses, and
/// deliberately so: that one demonstrably works in the CI job this test
/// runs in. A config-file variant of this fixture looked equivalent,
/// listened on its socket — and never accepted a connection, on Linux
/// as well as macOS. It failed CI for the fixture's reason rather than
/// the test's, which is the shape of a proof that proves nothing.
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
        use std::io::BufRead;
        let mut line = String::new();
        let got = std::io::BufReader::new(stdout).read_line(&mut line);
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

#[tokio::test]
async fn killing_the_bus_resolves_the_watcher_instead_of_idling() {
    let Some(bus) = start_bus() else { return };

    // A daemon that listens but never accepts leaves this pending
    // forever (observed on macOS), and a test that hangs is worse than
    // one that skips: bound it, and route the timeout through the same
    // skip contract — so it is a loud skip on a dev host and a hard
    // failure in CI, where LISA_REQUIRE_BUS_TESTS makes it fatal.
    let connect = zbus::connection::Builder::address(bus.address.as_str())
        .expect("bus address parses")
        .name(lisa_agentd::dbus::BUS_NAME)
        .expect("name request is well-formed")
        .build();
    let conn = match tokio::time::timeout(BUS_START_TIMEOUT, connect).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(e)) => {
            require_or_skip(&format!("cannot connect to the private bus: {e}"));
            return;
        }
        Err(_) => {
            require_or_skip("the private bus never accepted a connection");
            return;
        }
    };

    let watcher = lisa_agentd::dbus::name_lost(&conn);
    tokio::pin!(watcher);

    // Positive control first: with the bus alive and the name held, the
    // watcher must NOT resolve — a watcher that fires on a healthy bus
    // would restart-loop the daemon forever.
    tokio::select! {
        why = &mut watcher => panic!("the watcher fired on a healthy bus: {why}"),
        _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
    }

    // The incident: the bus dies under the daemon.
    drop(bus);

    let why = tokio::time::timeout(LOSS_TIMEOUT, watcher)
        .await
        .expect("the watcher noticed the dead bus — before this fix the process idled here");
    assert!(
        why.contains("gone") || why.contains("NameLost"),
        "the watcher resolved for an unexpected reason: {why}"
    );
}
