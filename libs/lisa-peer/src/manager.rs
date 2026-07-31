//! Who may operate the user's own management surfaces (issues #107,
//! #99; ADR-0033).
//!
//! Two daemons have a plane that is "for the user's own tooling" and was
//! enforced by saying so in a comment:
//!
//! - the portal's `Grant`/`Deny`/`Revoke`, which take an `app_id` with
//!   no relation to the caller — so any unsandboxed process could
//!   pre-grant a scope to a victim app, or write a remembered `Deny` and
//!   lock one out (#107);
//! - `remoted`'s `SetConsent`/`SetKey`/`AddProvider`/`BeginLogin`, where
//!   any session peer could turn on every offload scope and then proxy
//!   `mail`, `files` and `screen` content out through the broker (#99).
//!
//! ADR-0033 rejected "fix each surface locally — forty patches, forty
//! chances to differ", so the rule lives here once.
//!
//! # The rule
//!
//! Grant management is reachable only from a **program we ship for the
//! purpose**, identified by its executable (`lisa_peer::exe_of_peer` —
//! the kernel's answer, through a pidfd), running as **our own user**.
//! Everything else is refused, including anything the portal cannot
//! identify at all.
//!
//! This is an allowlist of files, not of names: `host:*` identities and
//! desktop ids are derived data, and deriving an authorization from
//! derived data is how #106 and #107 became the same bug twice.
//!
//! # Why not polkit
//!
//! `allow_active` needs a local seat, which the reference machine does
//! not have over SSH, and Lisa's own rule is that nothing user-facing
//! asks for `sudo` (CLAUDE.md 7b). An exe allowlist is deterministic,
//! decidable without a live session, and — because the decision here is
//! a pure function — testable on a macOS dev host.
//!
//! # The limit, stated plainly
//!
//! An allowlisted program is trusted completely: if Settings can be made
//! to call `Grant`, the grant is written. This moves the boundary from
//! "any process" to "three files", which is the whole of the fix, and it
//! is not the same as proving those three files never misbehave.

use std::path::{Path, PathBuf};

/// Programs that may operate a management plane on a Lisa OS system.
///
/// - the CLI's two real locations — `/usr/bin/lisa` is a resolver shell
///   script that `exec`s one of these, so neither the script nor the
///   shell is ever the caller's executable;
/// - Settings, which hosts the Intelligence panel in-process.
pub const DEFAULT_MANAGERS: [&str; 4] = [
    "/usr/lib/lisa/bin/lisa",
    // Both payload locations. The runtime channel moved to
    // /var/lib/lisa-apps because the old path sits inside
    // inferenced's DynamicUser StateDirectory, where no user can reach
    // it (see APPS_DIR in cli/lisa/src/apps.rs). /usr/bin/lisa execs
    // whichever it finds, so this list has to name both or the CLI
    // becomes un-identifiable the moment a runtime payload installs —
    // and the failure is a refusal to manage grants, which reads like a
    // security decision rather than a stale path.
    "/var/lib/lisa-apps/payloads/runtime/current/bin/lisa",
    "/var/lib/lisa/apps/payloads/runtime/current/bin/lisa",
    "/usr/bin/gnome-control-center",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ManagerRefusal {
    #[error("grant management is refused to callers from another user")]
    NotOurUser,
    #[error("the caller could not be identified, so it cannot manage grants")]
    Unidentified,
    #[error("only Settings and the lisa CLI can change this")]
    NotAManager,
}

/// May this caller operate a management plane?
///
/// `exe` is the caller's executable as the kernel reports it, already
/// symlink-resolved; `managers` are the allowlisted paths, likewise
/// resolved (see [`resolve_managers`]). Pure, so every adversarial case
/// below is a unit test rather than a claim.
pub fn may_manage(
    same_user: bool,
    exe: Option<&Path>,
    managers: &[PathBuf],
) -> Result<(), ManagerRefusal> {
    if !same_user {
        return Err(ManagerRefusal::NotOurUser);
    }
    let Some(exe) = exe else {
        return Err(ManagerRefusal::Unidentified);
    };
    // Exact file equality. Not a prefix, not a basename, not a parent
    // directory — every weaker comparison is a way back to #106.
    if managers.iter().any(|m| m == exe) {
        Ok(())
    } else {
        Err(ManagerRefusal::NotAManager)
    }
}

/// Resolve configured manager paths to real files, dropping what does
/// not exist.
///
/// Re-resolved at every check rather than cached at startup: the channel
/// CLI lives behind a `current` symlink that `lisa apps update` moves,
/// and a cached answer would keep authorizing the *previous* binary
/// after an update — or stop authorizing anything at all if the portal
/// happened to start before the channel was unpacked.
pub fn resolve_managers(configured: &[PathBuf]) -> Vec<PathBuf> {
    configured
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect()
}

/// The shipped defaults, as paths.
pub fn default_managers() -> Vec<PathBuf> {
    DEFAULT_MANAGERS.iter().map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"#!/bin/sh\n").unwrap();
        p.canonicalize().unwrap()
    }

    /// Issues #107 and #99. The demonstrated exploits were a plain host
    /// process calling `Grant("org.example.Victim", "inference")` and a
    /// plain session peer calling `SetConsent("mail", true)`.
    #[test]
    fn an_ordinary_host_process_cannot_manage_grants() {
        let dir = tempfile::tempdir().unwrap();
        let settings = exe(dir.path(), "gnome-control-center");
        let attacker = exe(dir.path(), "totally-normal-app");
        let managers = vec![settings.clone()];

        assert_eq!(may_manage(true, Some(&settings), &managers), Ok(()));
        assert_eq!(
            may_manage(true, Some(&attacker), &managers),
            Err(ManagerRefusal::NotAManager)
        );
    }

    /// Fail closed on every axis: no allowlist, no identity, wrong user.
    /// An empty allowlist authorizing everybody is the failure mode that
    /// would make a packaging mistake into a silent bypass.
    #[test]
    fn anything_unproven_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let settings = exe(dir.path(), "gnome-control-center");

        assert_eq!(
            may_manage(true, Some(&settings), &[]),
            Err(ManagerRefusal::NotAManager),
            "an empty allowlist must authorize nobody"
        );
        assert_eq!(
            may_manage(true, None, std::slice::from_ref(&settings)),
            Err(ManagerRefusal::Unidentified)
        );
        assert_eq!(
            may_manage(false, Some(&settings), std::slice::from_ref(&settings)),
            Err(ManagerRefusal::NotOurUser)
        );
    }

    /// The comparison is the whole file path. A neighbour, a parent, a
    /// same-named file one directory over: all different programs.
    #[test]
    fn only_the_exact_file_counts() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("usr/lib/lisa/bin");
        let other = dir.path().join("home/mallory/bin");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let managers = vec![exe(&real, "lisa")];

        for impostor in [
            exe(&other, "lisa"),
            exe(&real, "lisa2"),
            real.canonicalize().unwrap(),
        ] {
            assert_eq!(
                may_manage(true, Some(&impostor), &managers),
                Err(ManagerRefusal::NotAManager),
                "`{}` was accepted as a manager",
                impostor.display()
            );
        }
    }

    /// A symlinked manager must still match: `/proc/<pid>/exe` reports
    /// the resolved file, so the allowlist has to be resolved the same
    /// way or the real Settings binary would be refused.
    #[test]
    fn a_symlinked_manager_resolves_to_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let real = exe(dir.path(), "gnome-control-center-real");
        let link = dir.path().join("gnome-control-center");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let managers = resolve_managers(&[link]);
        assert_eq!(managers, vec![real.clone()]);
        assert_eq!(may_manage(true, Some(&real), &managers), Ok(()));
    }

    /// Paths that do not exist are dropped rather than compared: an
    /// absent channel CLI must not become a wildcard.
    #[test]
    fn missing_manager_paths_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let real = exe(dir.path(), "lisa");
        let managers = resolve_managers(&[dir.path().join("not-installed"), real.clone()]);
        assert_eq!(managers, vec![real]);
    }

    /// The proof cannot be minted without passing the check, and it
    /// carries the program's own name rather than a fixed label.
    #[test]
    fn the_manager_proof_names_the_program_that_earned_it() {
        let dir = tempfile::tempdir().unwrap();
        let settings = exe(dir.path(), "gnome-control-center");
        let attacker = exe(dir.path(), "totally-normal-app");
        let managers = std::slice::from_ref(&settings);

        let ok = Manager::authorize(true, Some(&settings), managers).unwrap();
        assert_eq!(ok.label(), "gnome-control-center");

        assert_eq!(
            Manager::authorize(true, Some(&attacker), managers),
            Err(ManagerRefusal::NotAManager)
        );
        assert_eq!(
            Manager::authorize(true, None, managers),
            Err(ManagerRefusal::Unidentified)
        );
        assert_eq!(
            Manager::authorize(false, Some(&settings), managers),
            Err(ManagerRefusal::NotOurUser)
        );
    }

    /// The shipped defaults are the three files documented above — a
    /// fourth appearing without a decision is worth failing over.
    #[test]
    fn the_default_allowlist_is_the_documented_three() {
        assert_eq!(default_managers().len(), 3);
        assert!(default_managers().iter().all(|p| p.is_absolute()));
    }
}

/// Proof that a caller passed [`may_manage`], plus a name for the audit
/// trail.
///
/// The Ledger used to record every management action as `app_id:
/// "settings"` regardless of who made it — so the one surface that could
/// have shown a user "something turned on all your egress scopes"
/// actively blamed Settings for it (#99). Now the entry names the
/// program the kernel says it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manager {
    label: String,
}

impl Manager {
    /// Check `peer` against `managers` and, on success, mint the proof.
    ///
    /// `exe` is the caller's executable as the kernel reports it —
    /// `lisa_peer::exe_of_peer`, whose Err cases all mean "we cannot
    /// name this program", which is a refusal rather than a fallback.
    pub fn authorize(
        same_user: bool,
        exe: Option<&Path>,
        managers: &[PathBuf],
    ) -> Result<Manager, ManagerRefusal> {
        may_manage(same_user, exe, managers)?;
        Ok(Manager {
            label: exe
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                // Unreachable: `may_manage` refuses a caller with no
                // exe. Named rather than unwrapped, because a panic in
                // an authorization path is a denial of service.
                .unwrap_or_else(|| "unknown".into()),
        })
    }

    /// How this caller appears in the Ledger.
    pub fn label(&self) -> &str {
        &self.label
    }
}
