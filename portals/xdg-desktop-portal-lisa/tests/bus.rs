//! The portal against a real message broker (ADR-0033, issues #107 and
//! #108).
//!
//! `tests/portal.rs` runs over a p2p socketpair, which needs no bus
//! daemon and covers consent, grants, quotas and revocation on any dev
//! host. It cannot cover *this*: a point-to-point link has exactly one
//! peer, so `PeerId` is `Direct` for everybody and "another app must not
//! be able to drive your session" is not a sentence that transport can
//! express. A green p2p suite proved nothing about #108.
//!
//! So these tests start a private `dbus-daemon`, connect **two** clients
//! to it, and let one try to reach the other's session. The broker
//! assigns the unique names; nothing here is self-asserted.
//!
//! # What runs where
//!
//! Session ownership is portable — it needs a broker, not Linux — so it
//! runs on a macOS dev host too, provided `dbus-daemon` is installed.
//! The grant-manager check additionally needs `/proc/<pid>/exe` reached
//! through the broker's **pidfd**, which only Linux supplies; its
//! positive control is therefore Linux-only and marked as such. Its
//! negative control runs everywhere.
//!
//! `LISA_REQUIRE_BUS_TESTS=1` turns "no dbus-daemon, skip" into a
//! failure. CI sets it. Without that, a runner that quietly lost its bus
//! would report a green suite for tests that never executed — which is
//! the exact way this repo has been fooled before.
//!
//! # The pidfd, and a broker old enough not to have one
//!
//! `GetConnectionCredentials` gained `ProcessFD` in **dbus 1.16**.
//! Without it `lisa_peer::exe_of_peer` refuses to name a program — by
//! design, since the alternative is a bare pid that can be recycled
//! (#136) — and the portal's consequences are concrete: no caller can
//! manage grants, and every host app resolves to the shared
//! `host:unknown` bucket instead of its own identity.
//!
//! That is a property of the *system*, not of the tests, so it is
//! asserted in both directions rather than skipped: with a pidfd, an
//! allowlisted program can manage grants; without one, nobody can.
//! Lisa OS ships on Arch (dbus 1.16), so the first branch is the one
//! that matters — `LISA_REQUIRE_PIDFD=1` makes its absence a failure,
//! and CI runs one job on a base new enough to hold it.

use lisa_portal::consent::StaticConsent;
use lisa_portal::grants::{Effective, GrantStore};
use lisa_portal::identity::{AppIdentity, StaticIdentity};
use lisa_portal::portal::{PORTAL_BUS_NAME, PORTAL_PATH, PortalState, serve_on_builder};
use lisa_portal::quota::QuotaConfig;
use lisa_portal::upstream::stub::StubUpstream;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// A private session bus, torn down with the test.
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

/// How long to wait for `dbus-daemon` to name its address.
///
/// It is a raw `write()` on Linux, so the real wait is milliseconds. The
/// timeout exists for hosts where it is not: Homebrew's macOS build
/// buffers `--print-address` into a pipe and does not honour a
/// `--config-file` listener either, so the address surfaces minutes
/// later or never. The first version of this file had no timeout and
/// took 157 seconds to run four assertions. A bounded wait turns that
/// into an explicit skip — and `LISA_REQUIRE_BUS_TESTS` turns the skip
/// into a failure everywhere it matters.
const BUS_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Start a throwaway `dbus-daemon`, or explain why we cannot.
fn start_bus() -> Option<Bus> {
    let dir = tempfile::tempdir().ok()?;
    let mut child = match std::process::Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return require_or_skip("dbus-daemon is not installed"),
    };

    // Read on a thread so a daemon that never answers costs five
    // seconds rather than the whole test run.
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
            _dir: dir,
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

struct Fixture {
    _bus: Bus,
    _portal: zbus::Connection,
    grants: Arc<GrantStore>,
    _ledger_dir: tempfile::TempDir,
    address: String,
}

async fn fixture(bus: Bus, managers: Vec<PathBuf>) -> Fixture {
    let grants = Arc::new(GrantStore::open_in_memory().unwrap());
    let ledger_dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(ledger_dir.path().join("l.db")).unwrap());
    let state = PortalState::with_policy(
        Arc::new(StaticIdentity(AppIdentity::host("host:demo"))),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(StubUpstream),
        Arc::clone(&grants),
        ledger,
        QuotaConfig::default(),
        lisa_portal::consent::PromptPolicy::default(),
        managers,
    );
    let builder = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name(PORTAL_BUS_NAME)
        .unwrap();
    let portal = serve_on_builder(builder, state)
        .unwrap()
        .build()
        .await
        .unwrap();
    let address = bus.address.clone();
    Fixture {
        _bus: bus,
        _portal: portal,
        grants,
        _ledger_dir: ledger_dir,
        address,
    }
}

/// One more app on the bus. The broker gives it its own unique name;
/// that is the whole mechanism under test.
async fn client(f: &Fixture) -> zbus::Connection {
    zbus::connection::Builder::address(f.address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap()
}

async fn open_session(conn: &zbus::Connection) -> zbus::Result<OwnedObjectPath> {
    let proxy = zbus::Proxy::new(
        conn,
        PORTAL_BUS_NAME,
        PORTAL_PATH,
        "dev.lisaos.portal.Inference",
    )
    .await?;
    let reply = proxy
        .call_method("OpenSession", &(HashMap::<String, OwnedValue>::new(),))
        .await?;
    let (path, _fd): (OwnedObjectPath, zbus::zvariant::OwnedFd) =
        reply.body().deserialize().unwrap();
    Ok(path)
}

/// Whether this broker hands out a pidfd for its peers (dbus >= 1.16).
///
/// Linux-only because its only caller is: a pidfd is a Linux concept,
/// and on any other host the answer is already "no".
///
/// Asked of the broker rather than assumed from a version string: what
/// matters is the field arriving in the reply, which is exactly what
/// `lisa_peer::resolve` keys off.
#[cfg(target_os = "linux")]
async fn broker_supplies_pidfd(conn: &zbus::Connection) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let Some(me) = conn.unique_name() else {
        return false;
    };
    match dbus.get_connection_credentials(me.to_owned().into()).await {
        Ok(creds) => creds.process_fd().is_some(),
        Err(_) => false,
    }
}

/// `Ok` to run pidfd-dependent assertions, or a reason not to.
#[cfg(target_os = "linux")]
fn pidfd_required_or_skipped(has_pidfd: bool) -> bool {
    if has_pidfd {
        return true;
    }
    if std::env::var_os("LISA_REQUIRE_PIDFD").is_some() {
        panic!(
            "LISA_REQUIRE_PIDFD is set but this broker supplies no ProcessFD \
             (dbus < 1.16) — host identity would collapse to host:unknown here"
        );
    }
    eprintln!(
        "broker supplies no ProcessFD (dbus < 1.16): asserting the \
         no-pidfd branch instead (set LISA_REQUIRE_PIDFD=1 to make this fatal)"
    );
    false
}

async fn session_proxy(conn: &zbus::Connection, path: &OwnedObjectPath) -> zbus::Proxy<'static> {
    zbus::Proxy::new(
        conn,
        PORTAL_BUS_NAME,
        path.clone(),
        "dev.lisaos.portal.Session",
    )
    .await
    .unwrap()
}

/// Issue #108, the whole of it: session objects were not bound to the
/// app that opened them, so any app holding (or guessing) a path could
/// cancel a neighbour's generation, close it, or run one billed to the
/// neighbour's quota and written into the Ledger under their name.
#[tokio::test]
async fn one_app_cannot_touch_another_apps_session() {
    let Some(bus) = start_bus() else { return };
    let f = fixture(bus, Vec::new()).await;

    let alice = client(&f).await;
    let mallory = client(&f).await;
    assert_ne!(
        alice.unique_name(),
        mallory.unique_name(),
        "the broker must give the two clients different names"
    );

    let path = open_session(&alice).await.unwrap();
    let theirs = session_proxy(&mallory, &path).await;

    for (verb, ok) in [
        ("Cancel", theirs.call_method("Cancel", &()).await.is_ok()),
        ("Close", theirs.call_method("Close", &()).await.is_ok()),
    ] {
        assert!(!ok, "a foreign peer was allowed to {verb} the session");
    }
    assert!(
        theirs
            .call_method("Embed", &(vec!["steal some quota"],))
            .await
            .is_err(),
        "a foreign peer ran inference billed to the session's owner"
    );

    // Positive control: the same path, the same calls, the real owner.
    // Without this the test would pass just as well if the path were
    // broken for everybody.
    let mine = session_proxy(&alice, &path).await;
    mine.call_method("Embed", &(vec!["mine"],))
        .await
        .expect("the owner must still be able to use its own session");
    mine.call_method("Cancel", &())
        .await
        .expect("the owner must still be able to cancel");
}

/// The refusal must not double as a directory of live sessions
/// (ADR-0033 §4): being refused and not existing look the same.
#[tokio::test]
async fn a_foreign_session_is_indistinguishable_from_a_missing_one() {
    let Some(bus) = start_bus() else { return };
    let f = fixture(bus, Vec::new()).await;
    let alice = client(&f).await;
    let mallory = client(&f).await;

    let real = open_session(&alice).await.unwrap();
    let imaginary =
        OwnedObjectPath::try_from("/dev/lisaos/portal/session/0123456789abcdef").unwrap();

    let refused = session_proxy(&mallory, &real)
        .await
        .call_method("Cancel", &())
        .await
        .unwrap_err();
    let missing = session_proxy(&mallory, &imaginary)
        .await
        .call_method("Cancel", &())
        .await
        .unwrap_err();

    let name = |e: &zbus::Error| match e {
        zbus::Error::MethodError(n, _, _) => n.to_string(),
        other => panic!("unexpected error shape: {other}"),
    };
    assert_eq!(
        name(&refused),
        name(&missing),
        "a live session answers differently from an imaginary one"
    );
}

/// Paths used to be `/dev/lisaos/portal/session/{1,2,3,…}`, which made
/// every other app's sessions free to enumerate.
#[tokio::test]
async fn session_paths_are_not_enumerable() {
    let Some(bus) = start_bus() else { return };
    let f = fixture(bus, Vec::new()).await;
    let alice = client(&f).await;

    let first = open_session(&alice).await.unwrap();
    let second = open_session(&alice).await.unwrap();
    let tail = |p: &OwnedObjectPath| p.as_str().rsplit('/').next().unwrap().to_string();

    assert_ne!(tail(&first), "1", "session paths are still a counter");
    assert_ne!(tail(&first), tail(&second));
    assert!(
        tail(&first).len() >= 32,
        "a short token is a guessable token: {}",
        tail(&first)
    );
    // Not merely different — unrelated. Consecutive tokens must not be
    // consecutive anything.
    assert!(
        tail(&first).chars().all(|c| c.is_ascii_hexdigit()),
        "unexpected token alphabet"
    );
}

/// Issue #107's negative control, on the transport where the exploit was
/// demonstrated: a real bus, a real unsandboxed caller, and an empty
/// allowlist. Every verb must refuse, and nothing must be written.
#[tokio::test]
async fn a_bus_caller_that_is_not_a_manager_cannot_write_grants() {
    let Some(bus) = start_bus() else { return };
    let f = fixture(bus, Vec::new()).await;
    let mallory = client(&f).await;
    let grants = zbus::Proxy::new(
        &mallory,
        PORTAL_BUS_NAME,
        PORTAL_PATH,
        "dev.lisaos.portal.Grants",
    )
    .await
    .unwrap();

    assert!(
        grants
            .call_method("Grant", &("org.example.Victim", "inference"))
            .await
            .is_err(),
        "an ordinary bus caller minted a grant for another app"
    );
    assert!(
        grants
            .call_method("Deny", &("org.gnome.Calendar", "inference"))
            .await
            .is_err(),
        "an ordinary bus caller locked another app out"
    );
    assert_eq!(
        f.grants
            .effective("org.example.Victim", "inference")
            .unwrap(),
        Effective::Unset
    );
    assert_eq!(
        f.grants
            .effective("org.gnome.Calendar", "inference")
            .unwrap(),
        Effective::Unset
    );
}

/// The positive control for #107: an allowlisted program *can* manage
/// grants. Without it, "grants are refused" would be satisfied by a
/// portal whose Grants interface is simply broken.
///
/// Linux-only, and honestly so: matching the caller's executable needs
/// `/proc/<pid>/exe` reached through the broker's pidfd, and neither
/// exists on the macOS dev host — where this same call is refused, which
/// is what the negative control above asserts. ADR-0033 flagged exactly
/// this gap and asked for a CI assertion rather than a local run; this
/// is it.
///
/// On a broker older than dbus 1.16 there is no pidfd to resolve, and
/// the test asserts *that* branch instead of skipping: nobody can manage
/// grants, which is the fail-closed behaviour and worth pinning too.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn an_allowlisted_program_can_manage_grants() {
    let Some(bus) = start_bus() else { return };
    // The test binary is the manager, so the client below is running the
    // allowlisted executable — the real Settings arrangement, with a
    // path we can name from inside a test.
    let me = std::env::current_exe().unwrap().canonicalize().unwrap();
    let f = fixture(bus, vec![me]).await;
    let settings = client(&f).await;
    let grants = zbus::Proxy::new(
        &settings,
        PORTAL_BUS_NAME,
        PORTAL_PATH,
        "dev.lisaos.portal.Grants",
    )
    .await
    .unwrap();

    let granted = grants
        .call_method("Grant", &("org.example.Demo", "inference"))
        .await;

    if !pidfd_required_or_skipped(broker_supplies_pidfd(&settings).await) {
        // No pidfd: the portal cannot name the caller's program, so it
        // refuses — including the program that would otherwise be
        // allowed. Fail-closed, and a real deployment consequence.
        assert!(
            granted.is_err(),
            "a portal that cannot identify anyone must not authorize anyone"
        );
        assert_eq!(
            f.grants.effective("org.example.Demo", "inference").unwrap(),
            Effective::Unset
        );
        return;
    }

    granted.expect("an allowlisted program must be able to grant");
    assert_eq!(
        f.grants.effective("org.example.Demo", "inference").unwrap(),
        Effective::Allowed
    );

    let reply = grants.call_method("List", &()).await.unwrap();
    let (rows,): (Vec<(String, String, String)>,) = reply.body().deserialize().unwrap();
    assert_eq!(
        rows,
        vec![(
            "org.example.Demo".to_string(),
            "inference".to_string(),
            "allowed".to_string()
        )]
    );
}
