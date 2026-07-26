//! Which program is on the other end (ADR-0033) — and why not `comm`.
//!
//! The portal identifies host callers by reading `/proc/<pid>/comm`
//! (issue #106). A process sets its own `comm`: `prctl(PR_SET_NAME)`, or
//! simply by being exec'd with a chosen `argv[0]`. So a hostile binary
//! renames itself to a victim's `Exec` basename and inherits the
//! victim's grants and quota. That was demonstrated end to end.
//!
//! `/proc/<pid>/exe` is a kernel-maintained symlink to the inode that was
//! actually executed. `PR_SET_NAME` does not touch it, `argv[0]` does not
//! touch it, and a process cannot point it elsewhere without `execve`ing
//! something else — at which point it *is* something else. That is the
//! difference between asking the caller and asking the kernel.
//!
//! Two honest limits, both of which the caller must handle rather than
//! wish away:
//!
//! * A pid is only meaningful while the peer is connected. Pids are
//!   reused, so resolve at the moment of the call and never store one.
//! * The executable may have been replaced or deleted since exec (Linux
//!   appends `" (deleted)"`). That is reported, not silently accepted.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("/proc is not available on this platform — host identity cannot be established")]
    Unsupported,
    #[error("no such process, or it exited before we looked")]
    Gone,
    #[error("the executable was replaced or deleted after exec: {0}")]
    Replaced(String),
}

/// The executable actually running as `pid`.
///
/// Linux only, by construction: there is no portable equivalent, and a
/// wrong answer here is worse than no answer, so other platforms get
/// [`IdentityError::Unsupported`] rather than a fallback that could be
/// spoofed.
pub fn exe_of_pid(pid: u32) -> Result<PathBuf, IdentityError> {
    if !cfg!(target_os = "linux") {
        return Err(IdentityError::Unsupported);
    }
    let link = PathBuf::from(format!("/proc/{pid}/exe"));
    let target = std::fs::read_link(&link).map_err(|_| IdentityError::Gone)?;
    let shown = target.to_string_lossy();
    if shown.ends_with(" (deleted)") {
        return Err(IdentityError::Replaced(shown.into_owned()));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_linux_refuses_rather_than_guessing() {
        if cfg!(target_os = "linux") {
            return;
        }
        assert_eq!(
            exe_of_pid(std::process::id()),
            Err(IdentityError::Unsupported)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn our_own_pid_resolves_to_the_test_binary() {
        let exe = exe_of_pid(std::process::id()).expect("own exe");
        assert_eq!(exe, std::env::current_exe().unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_pid_that_is_not_running_is_gone_not_guessed() {
        // u32::MAX is above any pid_max, so it can never be live.
        assert_eq!(exe_of_pid(u32::MAX), Err(IdentityError::Gone));
    }
}
