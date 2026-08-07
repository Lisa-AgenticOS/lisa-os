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
//! dev hosts the homebrew daemon never becomes usable under a test
//! runner — its stock config listens on a `launchd:` socket, and a
//! hand-written config left the daemon listening but not accepting —
//! so the fixture skips there LOUDLY rather than pretending. Same
//! contract, same reasons, as `daemons/remoted/tests/bus.rs`.

const BUS_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const LOSS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

struct Bus {
    child: std::process::Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_bus() -> Option<Bus> {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => return require_or_skip(&format!("no tempdir: {e}")),
    };
    let sock = dir.path().join("bus.sock");
    let conf = dir.path().join("bus.conf");
    if std::fs::write(
        &conf,
        format!(
            "<busconfig>\n  <type>session</type>\n  <listen>unix:path={}</listen>\n  \
             <policy context=\"default\">\n    <allow send_destination=\"*\"/>\n    \
             <allow own=\"*\"/>\n    <allow user=\"*\"/>\n  </policy>\n</busconfig>\n",
            sock.display()
        ),
    )
    .is_err()
    {
        return require_or_skip("cannot write the bus config");
    }
    // Through `sh` for one reason: `ulimit -n 256`. dbus-daemon closes
    // every descriptor up to RLIMIT_NOFILE at startup, and a test
    // runner can hand it a limit in the millions — minutes of spinning
    // with no socket, no output and no error. Cheap insurance; the
    // daemon needs a handful of descriptors.
    let child = match std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"ulimit -n 256 2>/dev/null; exec "$0" "$@""#)
        .arg("dbus-daemon")
        .arg(format!("--config-file={}", conf.display()))
        .arg("--nofork")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return require_or_skip("dbus-daemon is not installed"),
    };
    let mut bus = Bus {
        child,
        address: format!("unix:path={}", sock.display()),
        _dir: dir,
    };
    // The daemon is up when its socket exists; a daemon that died
    // (bad config, unsupported option) never produces it.
    let deadline = std::time::Instant::now() + BUS_START_TIMEOUT;
    while !sock.exists() {
        if let Ok(Some(status)) = bus.child.try_wait() {
            return require_or_skip(&format!("dbus-daemon exited at startup: {status}"));
        }
        if std::time::Instant::now() > deadline {
            return require_or_skip("dbus-daemon did not create its socket in time");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Some(bus)
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
