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
//! That callback runs in a forked child that has not exec'd yet, so the
//! usual `pre_exec` rule applies: it must not allocate or take locks.
//!
//! **It does allocate.** `confine()` builds its path list with
//! `Vec`/`PathBuf` in the child, which is a `malloc` after `fork` — safe
//! only as long as no other thread held the allocator's lock at the
//! moment of the fork. This paragraph used to claim the opposite
//! ("every path is resolved and every rule built before the fork"), and
//! the claim is the defect, not the allocation: the fix is to open the
//! `PathFd`s in the parent and move them into the closure, and until
//! somebody does that the comment says what the code does.
//!
//! # Honest degradation
//!
//! On macOS, on kernels without Landlock, and on ABI versions too old
//! for the access rights we need, the subprocess runs **unconfined** and
//! the caller is told so. Reporting confinement that did not happen
//! would be worse than not having it: the whole value of a guardrail is
//! that somebody can rely on it.

use std::path::Path;

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

    /// Read-only system trees a toolchain genuinely needs: the compiler,
    /// the SDK, and the system's own libraries. Anything not named here
    /// or in one of the `$HOME` lists below is invisible to the child.
    ///
    /// `/proc` and `/dev` are NOT here — see [`READ_WRITE_FILES`].
    const READ_ONLY: &[&str] = &["/usr", "/etc", "/lib", "/lib64", "/bin", "/sbin"];

    /// `/dev` and `/proc`: readable, and existing files WRITABLE.
    ///
    /// These were in [`READ_ONLY`] with the note that "a compiler that
    /// cannot open `/dev/null` fails in ways nobody will connect to a
    /// security change" — and then `AccessFs::from_read` was used, which
    /// has no `WriteFile` in it, so `2>/dev/null` returned EACCES and
    /// the warning described the code's own behaviour. A probe run
    /// against the shipped ruleset could not redirect to `/dev/null` at
    /// all (#309).
    ///
    /// `WriteFile` only: nothing here may be created, removed or made
    /// into a device node, so this opens `/dev/null` and `/dev/tty`, not
    /// the ability to add to `/dev`.
    const READ_WRITE_FILES: &[&str] = &["/dev", "/proc"];

    /// The writable trees, from the module-level definition.
    ///
    /// Deliberately a delegation and not a copy: `writable_roots`
    /// is what tests reason about, and a jail whose list and a
    /// test's list can differ is a jail nobody is checking (#310).
    use super::writable_caches;

    /// Toolchains that live in `$HOME` rather than in `/usr`: readable
    /// and executable, never writable.
    ///
    /// A rustup `cargo` is `~/.cargo/bin/cargo`, and the shim it execs
    /// is under `~/.rustup`. Neither is under any path in [`READ_ONLY`],
    /// so a ruleset that does not name them cannot run a build at all on
    /// a machine that installed Rust the ordinary way — `exec` happens
    /// *after* `restrict_self`. Read is what a toolchain needs; the
    /// write grants are the narrow ones below.
    ///
    /// **Limit, stated rather than implied:** read covers
    /// `~/.cargo/credentials.toml`, a crates.io token, and Landlock has
    /// no way to subtract a path from a granted tree. The child cannot
    /// *write* anything the user will later run, which is what #309 is
    /// about; it can still read a publishing token it has no business
    /// with. Closing that means enumerating the children of `~/.cargo`
    /// at rule-build time, which is a readdir in a forked child — worth
    /// doing, not worth doing here.
    fn readable_toolchains(home: &Path) -> Vec<std::path::PathBuf> {
        vec![
            home.join(".cargo"),
            home.join(".rustup"),
            home.join(".pub-cache"),
        ]
    }

    /// Individual FILES a build writes, granted one by one.
    ///
    /// Cargo's locks and its gc bookkeeping live at the top of
    /// `$CARGO_HOME`, beside `bin/` and `config.toml` — granting the
    /// directory to reach them is exactly what #309 is. A Landlock rule
    /// can name a regular file, so these are named, and `prepare_caches`
    /// creates them in the parent beforehand: a rule cannot name a path
    /// that does not exist, and a lock cargo cannot open stops the build
    /// with an error nobody will trace back to here.
    fn writable_files(home: &Path) -> Vec<std::path::PathBuf> {
        vec![
            home.join(".cargo/.package-cache"),
            home.join(".cargo/.package-cache-mutate"),
            home.join(".cargo/.global-cache"),
        ]
    }

    /// V2, not V1, and the difference is the whole toolchain: ABI v1 has
    /// no `Refer` right, and a ruleset without it denies EVERY rename
    /// between two directories — `EXDEV`, even when both are fully
    /// granted. `cargo build` renames its outputs, so under a v1 ruleset
    /// it dies at the first crate with "Invalid cross-device link". That
    /// is a confinement no build survives, and therefore one nobody
    /// keeps switched on. `Refer` permits moves only WITHIN the granted
    /// set, so it widens nothing the child could not already write.
    ///
    /// `available()` asks about the same level, so the two agree. A
    /// kernel with Landlock but only v1 therefore reports available and
    /// then *refuses to spawn* rather than running unconfined — the safe
    /// direction, and the one `run_program` already took.
    const ABI_LEVEL: ABI = ABI::V2;

    /// Build the ruleset and enforce it. Called in the child, between
    /// fork and exec.
    pub fn confine(project: &Path, home: &Path) -> Confinement {
        let abi = ABI_LEVEL;
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
        // skipped: `.pub-cache` on a machine with no Dart toolchain
        // is simply absent.
        // Not `mut`: the closure consumes and returns the ruleset rather
        // than mutating a capture. clippy's -D warnings catches this only
        // on Linux, since this whole module is cfg'd to it — a macOS dev
        // host cannot lint the code that matters most here.
        let add = |set: RulesetCreated, dir: &Path, access| -> Result<RulesetCreated, String> {
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
        // Read-only first, write over the top of it: Landlock resolves a
        // path by its DEEPEST matching rule, so `~/.cargo` read-only and
        // `~/.cargo/registry` read-write coexist and mean what they say.
        for dir in readable_toolchains(home) {
            ruleset = match add(ruleset, &dir, read_only) {
                Ok(r) => r,
                Err(e) => return Confinement::Unavailable(e),
            };
        }
        for dir in writable_caches(home) {
            ruleset = match add(ruleset, &dir, read_write) {
                Ok(r) => r,
                Err(e) => return Confinement::Unavailable(e),
            };
        }
        // File rules take FILE rights only. `from_all` carries
        // directory-only rights (MakeReg, RemoveDir, …) and the kernel
        // rejects the whole rule with EINVAL when they name a regular
        // file — which arrives at the caller as a spawn that failed for
        // no visible reason.
        let file_rw = AccessFs::from_file(abi);
        for file in writable_files(home) {
            ruleset = match add(ruleset, &file, file_rw) {
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
        let read_and_write_files = read_only | AccessFs::WriteFile;
        for dir in READ_WRITE_FILES {
            ruleset = match add(ruleset, Path::new(dir), read_and_write_files) {
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
        match Ruleset::default().handle_access(AccessFs::from_all(ABI_LEVEL)) {
            Ok(_) => Confinement::Enforced,
            Err(e) => Confinement::Unavailable(format!("landlock unusable: {e}")),
        }
    }

    /// Create the writable caches, in the PARENT, before the fork.
    ///
    /// Landlock rules can only name a path that exists, and `confine`
    /// skips one that does not — so a cache directory the toolchain has
    /// not created yet would be silently unwritable and the build would
    /// fail a long way from here. Creating it costs nothing (the
    /// toolchain would create it on its first run anyway) and it is the
    /// price of naming `~/.cargo/registry` instead of `~/.cargo`.
    pub fn prepare_caches(home: &Path) {
        for dir in writable_caches(home) {
            // Best effort: a home that cannot be written to is the
            // caller's problem to notice, not something to panic over
            // between a user's keypress and their command.
            let _ = std::fs::create_dir_all(dir);
        }
        for file in writable_files(home) {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file);
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

    pub fn prepare_caches(_home: &Path) {}
}

pub use imp::{available, confine};

/// The directory trees a confined child may WRITE, for a given project
/// and home.
///
/// Exported so a test can ask "is my probe path inside the jail?"
/// against the real list instead of a second copy of it. #310 is why:
/// `confinement.rs` chose its probe directories under
/// `CARGO_TARGET_TMPDIR` and asserted in a comment that this was
/// "granted to nothing". True when the checkout is in `$HOME`. In the
/// packaging lane makepkg builds under `/tmp`, and `temp_dir()` is in
/// the writable set on purpose — so every probe was written INSIDE the
/// jail, all seven succeeded, and the suite reported that Landlock had
/// failed. It had not. The test was measuring the inside of the box.
///
/// A test that hardcoded the list instead would drift the moment this
/// one changed, which is the same defect one step later.
pub fn writable_roots(project: &Path, home: &Path) -> Vec<std::path::PathBuf> {
    let mut roots = vec![project.to_path_buf()];
    roots.extend(writable_caches(home));
    roots
}

/// Trees a build legitimately writes outside the project: the package
/// caches and a temp dir. Named explicitly rather than inherited, so the
/// list is auditable — and defined ONCE, here, because the ruleset and
/// any test that reasons about the ruleset must not consult two lists.
///
/// **The cache, not the whole of `~/.cargo` (#309).** `~/.cargo` also
/// holds `bin/`, which rustup puts on the user's `$PATH`, and
/// `config.toml`, which every later `cargo` invocation reads —
/// `[target.'cfg(all())'] runner = […]` there is the same escape #63
/// closed at the argv layer, through a different door. A confined child
/// that can write either one has arbitrary execution as the user,
/// unconfined, the next time they open a terminal. The registry is
/// `registry/` and `git/`; those are what a build needs.
pub fn writable_caches(home: &Path) -> Vec<std::path::PathBuf> {
    vec![
        home.join(".cargo/registry"),
        home.join(".cargo/git"),
        home.join(".pub-cache/hosted"),
        home.join(".pub-cache/git"),
        std::env::temp_dir(),
    ]
}

/// The home directory whose caches the confinement will make writable.
///
/// `/` when the environment does not say: a home nobody can name gets
/// no grant, which is the safe direction to fail in.
pub fn user_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"))
}

/// Install the confinement on a command that has not been spawned yet,
/// and report what the child will actually get.
///
/// This is the ONLY place the `pre_exec` hook is written. `run_command`
/// and `run_shell` spawn children over the same ruleset, and two copies
/// of an unsafe hook are two places for the policy to drift — which is
/// exactly what #307 found, with the broader of the two tools carrying
/// no copy at all.
///
/// A caller that gets `Unavailable` back has a child that will run
/// **unconfined**, and is expected to say so where somebody reads it.
pub fn confine_command(
    cmd: &mut std::process::Command,
    project: &Path,
    home: &Path,
) -> Confinement {
    let confinement = available();
    #[cfg(unix)]
    if confinement.is_enforced() {
        imp::prepare_caches(home);
        let project = project.to_path_buf();
        let home = home.to_path_buf();
        // SAFETY: this runs in the forked child before exec. It takes
        // no locks; the two paths it needs were resolved above, before
        // the fork, and `confine` only issues syscalls over them.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(cmd, move || {
                if confine(&project, &home).is_enforced() {
                    Ok(())
                } else {
                    // Refuse to run rather than run unconfined after
                    // deciding confinement was available: silently
                    // dropping the jail is the failure this exists to
                    // prevent.
                    Err(std::io::Error::other("landlock confinement failed"))
                }
            });
        }
    }
    confinement
}

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
