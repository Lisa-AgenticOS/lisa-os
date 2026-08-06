//! The dialog child: where it comes from, and what it is not allowed to
//! reach.
//!
//! `lisa-consentd` draws nothing itself. It spawns `gjs` on a GJS file
//! and talks to it over a pipe (`protocol.rs`). The child is the window;
//! the parent is the peer.
//!
//! # Why the path is pinned to `/usr`, and not resolved through the app
//! channel
//!
//! Every other Lisa surface launches through `lisa-app`, which asks
//! `lisa apps path shell` and therefore prefers whatever
//! `lisa apps update` last unpacked under `/var/lib/lisa-apps` — a
//! directory the user, and so anything running as the user, can write
//! (ADR-0020, and the point of it: a design iterates in seconds).
//!
//! A guardrail cannot be updated that way. ADR-0030 §2's first test is
//! *is it reachable from inside?*, and a dialog file the model's own
//! host could overwrite is a dialog that clicks its own Allow button.
//! So this one surface gives up the app channel and ships with the
//! package, root-owned, at a path spelled here.
//!
//! That is a real cost, stated rather than hidden: changing the
//! confirmation dialog now needs a package update, where changing the
//! dock does not.

use std::path::{Path, PathBuf};

/// Where the packaged dialog lives. Root-owned, replaced only by pacman.
pub const RENDERER: &str = "/usr/share/lisa/shell/consent/lisa-consentd.js";

/// The interpreter. Named absolutely for the same reason the dialog is:
/// resolving `gjs` through `PATH` would let the environment the daemon
/// was activated in choose the program.
pub const GJS: &str = "/usr/bin/gjs";

/// Environment variables the renderer must not inherit.
///
/// `DBUS_SESSION_BUS_ADDRESS` is the load-bearing one and the reason
/// this list exists: with it removed the child **cannot** connect to the
/// broker, so it cannot own `dev.lisaos.Consent1`, cannot call
/// `dev.lisaos.Agent1.Confirm`, and cannot be mistaken for either. The
/// separation stops being a convention about who calls what and becomes
/// a property of the process's environment.
///
/// `DBUS_STARTER_*` go with it: they are set on a D-Bus-activated
/// process, they name the same socket, and GIO will use them.
///
/// GTK4 and libadwaita render without a session bus — verified on the
/// reference device, where the dialog draws and exits 0 with all three
/// removed. What degrades is the settings portal, so the dialog follows
/// the fallback theme rather than the desktop's, and **accessibility**,
/// which is not acceptable and is why [`A11Y_ADDRESS`] exists.
pub const STRIPPED_ENV: [&str; 3] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "DBUS_STARTER_ADDRESS",
    "DBUS_STARTER_BUS_TYPE",
];

/// The one bus address the renderer *is* given.
///
/// GTK reaches a screen reader over the accessibility bus, and it
/// ordinarily finds that bus by asking `org.a11y.Bus.GetAddress` on the
/// **session** bus — which the renderer no longer has. Left at that, the
/// single most important dialog on the machine would be the one dialog
/// Orca cannot read.
///
/// That is a guardrail pointed at the wrong side of ADR-0030 §1: this
/// one sits between the model and the machine, never between a person
/// and their own machine, and a blind owner who cannot hear what they
/// are approving has been fenced out of their own consent decision.
///
/// So the parent resolves the address on the session bus it holds and
/// hands the answer over directly. `at-spi` is a separate bus with no
/// `dev.lisaos.*` name on it and no `dev.lisaos.Agent1` to call, so the
/// child still cannot reach anything it must not.
pub const A11Y_ADDRESS: &str = "AT_SPI_BUS_ADDRESS";

/// Which GJS file to draw with.
///
/// The override exists so the daemon is testable without installing it,
/// and is compiled to a constant `None` in a release build — which is
/// what `os/packages/lisa/PKGBUILD` produces. An env var that survived
/// into the shipped binary would be exactly the hole this module's
/// header refuses: anything that can set the daemon's environment could
/// name its own dialog, and a dialog you wrote approves what you like.
pub fn renderer_path() -> PathBuf {
    if cfg!(debug_assertions)
        && let Some(p) = std::env::var_os("LISA_CONSENT_RENDERER_DEV")
    {
        return PathBuf::from(p);
    }
    PathBuf::from(RENDERER)
}

/// The command that draws dialogs, ready to spawn.
///
/// Split out from spawning so the environment discipline above has a
/// test that does not need a display, a bus, or gjs.
pub fn renderer_command(gjs: &Path, script: &Path, a11y: Option<&str>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(gjs);
    cmd.arg("-m").arg(script);
    for var in STRIPPED_ENV {
        cmd.env_remove(var);
    }
    // Order matters: the strip loop runs first, so an a11y address is
    // never removed by a future addition to `STRIPPED_ENV`, and a
    // session address inherited under that name cannot survive.
    match a11y {
        Some(addr) => cmd.env(A11Y_ADDRESS, addr),
        // Nothing resolved the accessibility bus. Remove the variable
        // rather than let one arrive from the daemon's own environment,
        // where it would be whatever activated us.
        None => cmd.env_remove(A11Y_ADDRESS),
    };
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    // stderr is inherited on purpose: the renderer's `logError` lines
    // belong in the daemon's journal, where an operator looks when the
    // dialog did not appear.
    cmd.stderr(std::process::Stdio::inherit());
    cmd.kill_on_drop(true);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole redesign rests on: the child has no route
    /// to the broker. If this list ever loses
    /// `DBUS_SESSION_BUS_ADDRESS`, the renderer can open a connection,
    /// and "the consent surface is a GJS process" is true again (#289).
    #[test]
    fn the_renderer_cannot_reach_the_session_bus() {
        assert!(
            STRIPPED_ENV.contains(&"DBUS_SESSION_BUS_ADDRESS"),
            "the renderer would inherit a session bus address"
        );
        let cmd = renderer_command(Path::new("/usr/bin/gjs"), Path::new("/tmp/x.js"), None);
        let removed: Vec<_> = cmd
            .as_std()
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        for var in STRIPPED_ENV {
            assert!(
                removed.iter().any(|r| r == var),
                "{var} survives into the renderer"
            );
        }
        // …and an unresolved accessibility bus is removed too, never
        // left to whatever activated the daemon.
        assert!(removed.iter().any(|r| r == A11Y_ADDRESS));
    }

    /// The dialog a screen reader can read (ADR-0030 §1: a guardrail
    /// sits between the model and the machine, never between a person
    /// and their own machine).
    #[test]
    fn the_accessibility_bus_is_handed_over_and_is_not_the_session_bus() {
        let cmd = renderer_command(
            Path::new("/usr/bin/gjs"),
            Path::new("/tmp/x.js"),
            Some("unix:path=/run/user/1000/at-spi/bus"),
        );
        let env: Vec<(String, Option<String>)> = cmd
            .as_std()
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(
            env.iter().any(|(k, v)| k == A11Y_ADDRESS
                && v.as_deref() == Some("unix:path=/run/user/1000/at-spi/bus")),
            "the renderer was not given the accessibility bus: {env:?}"
        );
        // The session bus is still gone. If a future edit resolved the
        // a11y address to the session socket, this is what catches it.
        assert!(
            env.iter()
                .any(|(k, v)| k == "DBUS_SESSION_BUS_ADDRESS" && v.is_none()),
            "the session bus came back with the accessibility bus"
        );
    }

    /// A release build has no way to be pointed at another dialog.
    #[test]
    fn the_shipped_binary_has_no_renderer_override() {
        // SAFETY: single-threaded test, and the variable is read only
        // by `renderer_path` below.
        unsafe { std::env::set_var("LISA_CONSENT_RENDERER_DEV", "/tmp/mallory.js") };
        let p = renderer_path();
        unsafe { std::env::remove_var("LISA_CONSENT_RENDERER_DEV") };
        if cfg!(debug_assertions) {
            assert_eq!(
                p,
                PathBuf::from("/tmp/mallory.js"),
                "dev override is broken"
            );
        } else {
            assert_eq!(
                p,
                PathBuf::from(RENDERER),
                "a release build honoured an environment variable for the dialog"
            );
        }
    }

    /// The packaged path is under `/usr`, which `lisa apps update` never
    /// writes to. Written as an assertion because the tempting change —
    /// "make the dialog update through the app channel like every other
    /// surface" — is a one-word edit that silently removes the guardrail.
    #[test]
    fn the_dialog_ships_with_the_package_not_the_app_channel() {
        assert!(RENDERER.starts_with("/usr/share/lisa/"), "{RENDERER}");
        assert!(!RENDERER.contains("/var/"), "{RENDERER}");
    }
}
