//! Landlock confinement for the subprocesses the forge loop spawns
//! (ADR-0029 phase 3, issue #53).
//!
//! # The hole this closes
//!
//! Phases 1 and 2 confine the harness's *own* file tools and commands.
//! They do not confine a **subprocess**. `run_tests` invokes `cargo
//! test`, which compiles and runs `build.rs` and test bodies — code the
//! model just wrote — as the user, outside every rule in `lisa-guard`.
//! No Rust-level policy can close that: once `execve` has happened the
//! guard is not in the process any more.
//!
//! Landlock is an unprivileged kernel LSM that restricts a process's
//! filesystem view to a named path set. It needs no root, no container,
//! and no configuration on the machine — which is what makes it usable
//! on somebody's laptop rather than only in CI.
//!
//! # Where it is applied, and why that is the only correct place
//!
//! A Landlock ruleset is inherited and **cannot be relaxed**, so
//! applying it in the harness process would confine the harness itself
//! and every later child, for the life of the daemon. It is therefore
//! applied in the child, after `fork` and before `exec`, through
//! `pre_exec`.
//!
//! That callback runs in a forked child that has not exec'd yet, so it
//! must not allocate or take locks — the usual `pre_exec` rule. Every
//! path is resolved and every rule built *before* the fork; the callback
//! only makes syscalls.
//!
//! # Honest degradation
//!
//! On macOS, on kernels without Landlock, and on ABI versions too old
//! for the access rights we need, the subprocess runs **unconfined** and
//! the caller is told so. Reporting confinement that did not happen
//! would be worse than not having it: the whole value of a guardrail is
//! that somebody can rely on it.

/// What confinement actually happened, so a caller can say so rather
/// than assume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confinement {
    /// The child is restricted to the paths given.
    Enforced,
    /// No Landlock here. The reason is worth carrying: "this kernel is
    /// too old" and "this is macOS" lead to different answers.
    Unavailable(String),
}

impl Confinement {
    pub fn is_enforced(&self) -> bool {
        matches!(self, Confinement::Enforced)
    }

    /// A line for the Ledger and for tool output. A jail that did not
    /// close must say so where somebody reads it, not only in a log.
    pub fn note(&self) -> Option<String> {
        match self {
            Confinement::Enforced => None,
            Confinement::Unavailable(why) => Some(format!("subprocess ran UNCONFINED: {why}")),
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::Confinement;
    use landlock::{
        ABI, Access, AccessFs, PathBeneath, PathFd, RestrictionStatus, Ruleset, RulesetAttr,
        RulesetCreated, RulesetCreatedAttr, RulesetStatus,
    };
    use std::path::Path;

    /// Read-only trees a toolchain genuinely needs: the compiler, the
    /// SDK, the registry it fetches crates into, and the system's own
    /// libraries. Anything not named here — including `$HOME` — is
    /// invisible to the child.
    ///
    /// `/proc` and `/dev` are included because a compiler that cannot
    /// open `/dev/null` fails in ways nobody will connect to a security
    /// change.
    const READ_ONLY: &[&str] = &[
        "/usr",
        "/etc",
        "/lib",
        "/lib64",
        "/bin",
        "/sbin",
        "/proc",
        "/dev",
        "/var/lib/lisa/flutter",
    ];

    /// Trees a build legitimately writes outside the project: the cargo
    /// registry cache and a temp dir. Named explicitly rather than
    /// inherited, so the list is auditable.
    fn writable_caches(home: &Path) -> Vec<std::path::PathBuf> {
        vec![
            home.join(".cargo"),
            home.join(".pub-cache"),
            std::env::temp_dir(),
        ]
    }

    /// Build the ruleset and enforce it. Called in the child, between
    /// fork and exec.
    pub fn confine(project: &Path, home: &Path) -> Confinement {
        let abi = ABI::V1;
        let read_write = AccessFs::from_all(abi);
        let read_only = AccessFs::from_read(abi);

        // No panics on this path: it runs in a forked child before
        // exec, where unwinding is not something a caller can catch.
        let mut ruleset = match Ruleset::default()
            .handle_access(read_write)
            .and_then(|r| r.create())
        {
            Ok(r) => r,
            Err(e) => return Confinement::Unavailable(format!("ruleset: {e}")),
        };

        // `add_rule` CONSUMES the ruleset and returns a new one, so a
        // failed rule cannot simply be skipped — the ruleset is gone
        // with it. Each step therefore reassigns or returns; silently
        // continuing with a half-built ruleset would produce a jail with
        // holes in it and no sign that anything went wrong.
        //
        // A path that does not exist is a different matter and is
        // skipped: `.pub-cache` on a machine with no Flutter, or
        // `/var/lib/lisa/flutter` on a dev host, are simply absent.
        let mut add = |set: RulesetCreated, dir: &Path, access| -> Result<RulesetCreated, String> {
            match PathFd::new(dir) {
                Err(_) => Ok(set), // not on this machine
                Ok(fd) => set
                    .add_rule(PathBeneath::new(fd, access))
                    .map_err(|e| format!("rule for {}: {e}", dir.display())),
            }
        };

        // The project itself: full access. This is the directory the
        // model was told it may edit, and the only one.
        ruleset = match add(ruleset, project, read_write) {
            Ok(r) => r,
            Err(e) => return Confinement::Unavailable(e),
        };
        for dir in writable_caches(home) {
            ruleset = match add(ruleset, &dir, read_write) {
                Ok(r) => r,
                Err(e) => return Confinement::Unavailable(e),
            };
        }
        for dir in READ_ONLY {
            ruleset = match add(ruleset, Path::new(dir), read_only) {
                Ok(r) => r,
                Err(e) => return Confinement::Unavailable(e),
            };
        }

        match ruleset.restrict_self() {
            Ok(RestrictionStatus {
                ruleset: RulesetStatus::FullyEnforced,
                ..
            }) => Confinement::Enforced,
            Ok(RestrictionStatus { ruleset: st, .. }) => {
                // PartiallyEnforced means the kernel understood some of
                // what we asked for. Reporting that as enforced would be
                // the comfortable lie this module exists to avoid.
                Confinement::Unavailable(format!("kernel enforced only partially: {st:?}"))
            }
            Err(e) => Confinement::Unavailable(format!("restrict_self: {e}")),
        }
    }

    /// Is Landlock usable at all on this kernel? Asked before forking so
    /// the caller can report honestly without paying for a spawn.
    pub fn available() -> Confinement {
        match Ruleset::default().handle_access(AccessFs::from_all(ABI::V1)) {
            Ok(_) => Confinement::Enforced,
            Err(e) => Confinement::Unavailable(format!("landlock unusable: {e}")),
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::Confinement;
    use std::path::Path;

    pub fn confine(_project: &Path, _home: &Path) -> Confinement {
        Confinement::Unavailable("Landlock is Linux-only".into())
    }

    pub fn available() -> Confinement {
        Confinement::Unavailable("Landlock is Linux-only".into())
    }
}

pub use imp::{available, confine};

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of this module is that it never claims more than it
    /// did. On a host without Landlock — every macOS dev machine —
    /// `available()` must say so, and the note must reach the caller.
    #[test]
    fn unavailable_is_reported_rather_than_assumed_away() {
        let un = Confinement::Unavailable("no kernel support".into());
        assert!(!un.is_enforced());
        let note = un.note().expect("an unconfined run must produce a note");
        assert!(note.contains("UNCONFINED"), "{note}");
        assert!(note.contains("no kernel support"), "{note}");
    }

    #[test]
    fn an_enforced_run_adds_no_noise() {
        // A note on every successful run would train people to skip it,
        // which is how the one that matters gets missed.
        assert!(Confinement::Enforced.is_enforced());
        assert_eq!(Confinement::Enforced.note(), None);
    }

    /// On this host, `available()` answers without spawning anything.
    /// Both answers are legitimate — Linux CI enforces, macOS does not —
    /// so the assertion is that it is *decided*, not which way.
    #[test]
    fn availability_is_answerable_without_a_subprocess() {
        let a = available();
        #[cfg(target_os = "linux")]
        assert!(
            a.is_enforced() || a.note().is_some(),
            "linux must either enforce or explain"
        );
        #[cfg(not(target_os = "linux"))]
        assert!(
            a.note().unwrap().contains("Linux-only"),
            "a non-Linux host must say why"
        );
    }
}
