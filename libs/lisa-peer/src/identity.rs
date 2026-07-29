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
//! Ask it about a **pidfd**, not a bare pid ([`exe_of_peer`]).
//! "Resolve at the moment of the call" does not close the reuse window:
//! the peer can exit between the broker's `GetConnectionCredentials`
//! reply and our `readlink`, both of which happen inside one call, and a
//! recycled pid then resolves to a different program's binary. Where the
//! answer feeds grant or quota attribution, that is an impersonation
//! (#136). A pidfd pins its pid — the kernel will not recycle it while
//! the descriptor is open, and once the process exits the descriptor
//! reports `Pid: -1` instead of following the pid to its next owner.
//!
//! One honest limit remains: the executable may have been replaced or
//! deleted since exec (Linux appends `" (deleted)"`). That is reported,
//! not silently accepted.

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
    #[error(
        "the broker supplied no process fd — a bare pid cannot attribute \
         identity, and guessing is what this crate exists to remove"
    )]
    NoProcessFd,
}

/// The executable actually running as `peer`, resolved through the
/// broker's pidfd.
///
/// This is the form to use for anything that *decides who someone is*.
/// [`exe_of_pid`] cannot: between learning the pid and reading the
/// symlink, the peer may have exited and its pid been recycled.
///
/// Refuses rather than falling back when no pidfd is available — a
/// fallback to the bare pid would silently reintroduce exactly the
/// window this function exists to close.
#[cfg(unix)]
pub fn exe_of_peer(peer: &crate::Peer) -> Result<PathBuf, IdentityError> {
    exe_of_pid(pid_of_peer(peer)?)
}

/// The pid of `peer`, pinned by the broker's pidfd.
///
/// Use this — never [`crate::Peer::pid`] — whenever a `/proc/<pid>/…`
/// read feeds a decision about *who* the caller is. The portal reads two
/// such paths (`root/.flatpak-info` and `exe`); both must be about the
/// same, un-recycled process, which is what the pidfd guarantees and a
/// bare pid does not (#136).
#[cfg(unix)]
pub fn pid_of_peer(peer: &crate::Peer) -> Result<u32, IdentityError> {
    // Checked before the platform gate: "we were given no pidfd" is true
    // and actionable everywhere, and it is the property under test on a
    // macOS dev host.
    let Some(fd) = peer.process_fd.as_ref() else {
        return Err(IdentityError::NoProcessFd);
    };
    if !cfg!(target_os = "linux") {
        return Err(IdentityError::Unsupported);
    }
    use std::os::fd::AsRawFd;
    pid_of_pidfd(fd.as_raw_fd())
}

/// The pid a pidfd refers to, from the kernel's own view of our fd
/// table — `Pid: -1` once the process is gone.
#[cfg(unix)]
fn pid_of_pidfd(raw: std::os::fd::RawFd) -> Result<u32, IdentityError> {
    let text = std::fs::read_to_string(format!("/proc/self/fdinfo/{raw}"))
        .map_err(|_| IdentityError::Gone)?;
    text.lines()
        .find_map(|line| line.strip_prefix("Pid:"))
        // "-1" (dead) fails to parse as u32, which is the right answer:
        // there is no process to name, and no recycled pid to mistake
        // for one.
        .and_then(|v| v.trim().parse::<u32>().ok())
        .ok_or(IdentityError::Gone)
}

/// The executable actually running as `pid`.
///
/// **Diagnostics and logging only.** For an authentication decision use
/// [`exe_of_peer`]: a bare pid can be recycled between the moment it is
/// learned and the moment it is read (#136).
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

    /// Issue #136. The point of the pidfd is that there is no fallback:
    /// a `Peer` carrying only a pid must refuse to name a program, on
    /// every platform, rather than resolve a pid that may already
    /// belong to somebody else.
    /// Same rule one level down: the portal reads two `/proc` paths for
    /// one caller, and both must be about the pinned process. A `Peer`
    /// with only a pid must not yield one.
    #[cfg(unix)]
    #[test]
    fn a_peer_without_a_pidfd_yields_no_pid_either() {
        let peer = crate::Peer::without_process_fd(
            crate::PeerId::Bus(":1.7".into()),
            Some(0),
            Some(std::process::id()),
        );
        assert_eq!(pid_of_peer(&peer), Err(IdentityError::NoProcessFd));
    }

    #[cfg(unix)]
    #[test]
    fn a_peer_without_a_pidfd_refuses_to_name_a_program() {
        let peer = crate::Peer::without_process_fd(
            crate::PeerId::Bus(":1.7".into()),
            Some(0),
            // A live, correct pid — and still not enough.
            Some(std::process::id()),
        );
        assert_eq!(exe_of_peer(&peer), Err(IdentityError::NoProcessFd));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_pid_that_is_not_running_is_gone_not_guessed() {
        // u32::MAX is above any pid_max, so it can never be live.
        assert_eq!(exe_of_pid(u32::MAX), Err(IdentityError::Gone));
    }
}
