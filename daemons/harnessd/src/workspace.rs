//! The folder the assistant may touch.
//!
//! The Assistant has read-tier bus tools always — they cannot change
//! anything. File tools are different: to write code it needs a place to
//! write, and handing it one is a grant, not a setting.
//!
//! # The rule
//!
//! **A workspace comes from a person choosing a folder, never from the
//! model choosing one.** The model cannot widen it, cannot name a
//! different one mid-run, and gets no file tools at all until a folder
//! exists. That is the same shape Claude Desktop uses, and the same shape
//! ADR-0030 asks for: the capability is granted from outside the loop.
//!
//! `validate` is what a client's path has to survive. A folder may be a
//! workspace when it is, in this order: absolute; real; a directory;
//! inside this user's home; not the home itself; and **not hidden, nor
//! inside anything hidden**.
//!
//! # Why the last one, and where it stops
//!
//! The line above used to read "the client is a desktop surface the user
//! is driving, so this is not defence against a hostile caller". That was
//! never checked, and issue #229 disproved it: any peer on the session
//! bus could start a run. So this IS a boundary, and it needs a policy
//! for what may be a jail root rather than an assumption about who is
//! asking.
//!
//! ADR-0029's second test is the one to keep in mind here: a guardrail
//! sits between the model and the machine, never between a person and
//! their own machine. So the rule is drawn where the two are actually
//! different. A person choosing `~/code/app` in a file chooser is doing
//! their job and is not interfered with. Nothing chooses `~/.ssh`
//! because it is working on `~/.ssh`; it is chosen because that is where
//! the keys are.
//!
//! The honest cost: someone whose project genuinely *is* a dotfile
//! folder — `~/.config/nvim` is the real example — has to copy it
//! somewhere else, or work on it another way. That is a real limit and
//! it is priced deliberately: the failure it prevents is a credential
//! leaving the machine, and the failure it causes is a copy.

use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub enum Refusal {
    NotAbsolute,
    Missing,
    NotADirectory,
    /// Somewhere no assistant should be turned loose, whoever asked.
    Forbidden(&'static str),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotAbsolute => write!(f, "the working folder must be an absolute path"),
            Refusal::Missing => write!(f, "that folder does not exist"),
            Refusal::NotADirectory => write!(f, "that path is not a folder"),
            Refusal::Forbidden(why) => write!(f, "{why}"),
        }
    }
}

/// Is any part of `rel` a hidden folder?
///
/// `rel` is the workspace path with the user's home stripped off, so the
/// home directory's own spelling never counts — a test fixture under
/// macOS's `/private/var/folders/…/.tmpXXXX` is a legitimate home.
///
/// This is issue #231's rule, and it is deliberately STRUCTURAL rather
/// than a list of the credential stores we happened to think of. The
/// file already says why: a denylist is unwinnable, and the one written
/// here would have had to grow an entry for every program that has ever
/// kept a token in a dotfile. What is true of all of them is the shape —
/// a leading dot is the convention by which a program says "this is
/// mine, not the user's work" — so that is what is checked.
fn hides_something(rel: &Path) -> bool {
    rel.components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
}

/// Roots that are never a workspace, however the request arrives.
///
/// Not an exhaustive denylist — that game is unwinnable, and the real
/// containment is the jail plus the home requirement below. This catches
/// the specific mistakes that would be catastrophic and are one typo
/// away: the root itself, and the user's whole home.
fn forbidden(path: &Path, home: Option<&Path>) -> Option<&'static str> {
    if path == Path::new("/") {
        return Some("the root of the filesystem is not a working folder");
    }
    if let Some(home) = home
        && path == home
    {
        return Some("your whole home folder is too broad — pick the project you are working on");
    }
    for sys in [
        "/etc", "/usr", "/boot", "/var", "/sys", "/proc", "/dev", "/efi",
    ] {
        if path == Path::new(sys) || path.starts_with(sys) {
            return Some("system folders are not a working folder");
        }
    }
    None
}

/// Check a client-supplied folder. `home` is `$HOME`; passed in so the
/// rules are testable without one.
///
/// **Containment is the home requirement, not the denylist.** A prefix
/// denylist looks like the defence and is not: canonicalisation can move
/// a path out from under it — `/etc` resolves to `/private/etc` on
/// macOS, so `starts_with("/etc")` stops matching the very thing it was
/// written for. Found by the test, not by reading it. The denylist stays
/// as a second line for the mistakes that would be catastrophic; the
/// requirement that does the work is "inside this user's home".
///
/// No `$HOME` therefore means no workspace at all. Refusing everything
/// when we cannot tell where home is fails closed; the alternative is a
/// grant whose only real check has silently evaporated.
pub fn validate(raw: &str, home: Option<&Path>) -> Result<PathBuf, Refusal> {
    let Some(home) = home else {
        return Err(Refusal::Forbidden(
            "cannot tell where your home directory is, so no folder can be granted",
        ));
    };
    let home = &home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(Refusal::NotAbsolute);
    }
    // Resolve before judging: `/home/me/proj/../../..` is `/`, and a
    // denylist that reads the unresolved string is decoration.
    let real = path.canonicalize().map_err(|_| Refusal::Missing)?;
    if !real.is_dir() {
        return Err(Refusal::NotADirectory);
    }
    // The check that actually contains: inside the user's own space.
    // Not because elsewhere is exotic, but because a working folder
    // outside it is nearly always a mistake, and the one time it is not,
    // saying no costs a copy.
    let Ok(rel) = real.strip_prefix(home) else {
        return Err(Refusal::Forbidden(
            "pick a folder inside your home directory",
        ));
    };
    // Inside home is not enough (#231). `~/.ssh` passed every check
    // above — absolute, real, a directory, under home, not home itself —
    // and was therefore a legal jail root, so `read_file
    // authorized_keys` handed the model the user's key material.
    // Demonstrated on the reference machine.
    //
    // Nothing stopped the WRITE side either except `ProtectHome=read-only`
    // in the unit, and a systemd option is not a policy: it is true until
    // the day the unit changes, and then this is a silent regression with
    // no test to notice.
    if hides_something(rel) {
        return Err(Refusal::Forbidden(
            "that is a hidden folder — hidden folders are where programs keep \
             configuration and credentials, not where you keep your work. Pick \
             the project folder itself",
        ));
    }
    if let Some(why) = forbidden(&real, Some(home)) {
        return Err(Refusal::Forbidden(why));
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_relative_path_is_refused() {
        let h = home();
        let h = h.path();
        assert_eq!(
            validate("projects/thing", Some(h)),
            Err(Refusal::NotAbsolute)
        );
        assert_eq!(validate("../escape", Some(h)), Err(Refusal::NotAbsolute));
    }

    #[test]
    fn a_missing_folder_is_refused_before_anything_else() {
        let h = home();
        assert_eq!(
            validate("/definitely/not/here/at/all", Some(h.path())),
            Err(Refusal::Missing)
        );
    }

    /// Fail closed: with no home, nothing is grantable. The home
    /// requirement is the containment, so losing it must not silently
    /// leave the denylist doing the job alone.
    #[test]
    fn without_a_home_nothing_is_granted() {
        assert!(matches!(validate("/tmp", None), Err(Refusal::Forbidden(_))));
    }

    #[test]
    fn traversal_is_judged_after_resolution_not_before() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let proj = home.join("proj");
        std::fs::create_dir(&proj).unwrap();

        // Spelled to look contained; resolves to the home root.
        let sneaky = format!("{}/proj/..", home.display());
        assert!(
            matches!(validate(&sneaky, Some(&home)), Err(Refusal::Forbidden(_))),
            "a path that resolves to home must be refused however it is spelled"
        );
        // The honest spelling of the same folder is fine.
        assert_eq!(
            validate(proj.to_str().unwrap(), Some(&home)).unwrap(),
            proj.canonicalize().unwrap()
        );
    }

    #[test]
    fn the_whole_home_is_too_broad() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        assert!(matches!(
            validate(home.to_str().unwrap(), Some(&home)),
            Err(Refusal::Forbidden(_))
        ));
    }

    /// System paths are refused by the home requirement first — the
    /// denylist is the second line, and the reason it cannot be the
    /// first is in `validate`'s doc comment.
    #[test]
    fn system_folders_are_never_a_workspace() {
        let h = home();
        for p in ["/etc", "/usr", "/", "/var", "/boot"] {
            assert!(
                matches!(
                    validate(p, Some(h.path())),
                    Err(Refusal::Forbidden(_)) | Err(Refusal::Missing)
                ),
                "{p} was allowed"
            );
        }
    }

    #[test]
    fn outside_home_is_refused_even_when_it_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().canonicalize().unwrap();
        let home = elsewhere.join("home-of-someone");
        std::fs::create_dir(&home).unwrap();
        // `elsewhere` exists and is a directory, but is not under home.
        assert!(matches!(
            validate(elsewhere.to_str().unwrap(), Some(&home)),
            Err(Refusal::Forbidden(_))
        ));
    }

    /// Issue #231. Every check here passed for `~/.ssh`: absolute,
    /// exists, a directory, inside home, not home itself, not a system
    /// path. So it was a legal jail root, and `read_file
    /// authorized_keys` returned the user's key material to the model —
    /// demonstrated on the device before this landed.
    ///
    /// Writes were stopped only by `ProtectHome=read-only` in the unit,
    /// which is a systemd side effect and not a policy: it stops being
    /// true the day the unit changes.
    #[test]
    fn a_credential_store_is_not_a_working_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        for store in [".ssh", ".gnupg", ".aws", ".config", ".local/share/keyrings"] {
            let dir = home.join(store);
            std::fs::create_dir_all(&dir).unwrap();
            assert!(
                matches!(
                    validate(dir.to_str().unwrap(), Some(&home)),
                    Err(Refusal::Forbidden(_))
                ),
                "~/{store} was accepted as a working folder"
            );
        }
    }

    /// The rule is structural, not a denylist of the stores we happened
    /// to think of — that game is unwinnable and the file already says
    /// so. Anything *under* a hidden folder is out too, and so is a
    /// hidden folder nobody has ever heard of.
    #[test]
    fn nothing_inside_a_hidden_folder_is_a_working_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        for path in [
            ".ssh/keys",
            ".some-app-invented-tomorrow",
            ".config/lisa/secrets",
            "projects/.git",
            "projects/.git/refs",
        ] {
            let dir = home.join(path);
            std::fs::create_dir_all(&dir).unwrap();
            assert!(
                matches!(
                    validate(dir.to_str().unwrap(), Some(&home)),
                    Err(Refusal::Forbidden(_))
                ),
                "~/{path} was accepted as a working folder"
            );
        }
    }

    /// ADR-0029's second test, kept honest: a guardrail sits between the
    /// model and the machine, never between a person and their own
    /// machine. An ordinary project folder — including one whose name
    /// merely contains a dot — is still granted.
    #[test]
    fn an_ordinary_project_folder_is_still_the_persons_to_grant() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        for path in [
            "code/app",
            "Documents/notes-site",
            "code/my.app",
            "code/v1.2/src",
        ] {
            let dir = home.join(path);
            std::fs::create_dir_all(&dir).unwrap();
            assert_eq!(
                validate(dir.to_str().unwrap(), Some(&home)).unwrap(),
                dir.canonicalize().unwrap(),
                "~/{path} was refused, and it is a person's own folder"
            );
        }
    }

    /// The refusal has to say what to do about it. "That folder is
    /// forbidden" sends someone hunting for a permission to change;
    /// naming the reason is what turns it into one decision.
    #[test]
    fn the_hidden_folder_refusal_says_why_and_what_to_do() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let dir = home.join(".ssh");
        std::fs::create_dir_all(&dir).unwrap();
        let why = validate(dir.to_str().unwrap(), Some(&home))
            .unwrap_err()
            .to_string();
        assert!(
            why.contains("hidden"),
            "the refusal does not say why: {why}"
        );
    }

    #[test]
    fn a_real_project_folder_is_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let proj = home.join("code/app");
        std::fs::create_dir_all(&proj).unwrap();
        assert_eq!(
            validate(proj.to_str().unwrap(), Some(&home)).unwrap(),
            proj.canonicalize().unwrap()
        );
    }
}
