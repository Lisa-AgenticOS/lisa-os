//! Which PROCESS is on the other end — and why a connection is not one
//! (ADR-0033, issue #289).
//!
//! [`crate::PeerId`] is the identity of a **connection**. That is the
//! right answer to "is this the same caller as before?", and it is the
//! wrong answer to "is this the same *program* as before", because a
//! process may open as many connections as it likes. On the reference
//! device `/usr/share/dbus-1/session.conf` ships
//!
//! ```text
//! <allow own="*"/>
//! ```
//!
//! so a second connection may also own any well-known name it fancies.
//! Put those two facts together and the peer that parked a privileged
//! call answers it from `:1.6` while having asked from `:1.5`, and every
//! check written as "the answerer is not the requester" — which is what
//! `Owner::allows` computes — says yes.
//!
//! # Why a pidfd and not a pid
//!
//! A bare pid names a process only until that process exits, after which
//! the kernel is free to hand the number to somebody else. A parked
//! confirmation lives for minutes, so "store the requester's pid and
//! compare it later" is exactly the window ADR-0033 warns about.
//!
//! The broker's `GetConnectionCredentials` reply carries a **pidfd**
//! (`ProcessFD`, verified present on the reference device's dbus), and a
//! pidfd pins its pid: the kernel will not recycle the number while the
//! descriptor is open. So a [`Process`] built from one is safe to *hold*
//! across a park — which is what `agentd` does — and the comparison it
//! answers stays true for as long as it is held.
//!
//! # What it does not answer
//!
//! Same pid is same process; **different pid is not necessarily a
//! different program**. A process that `fork()`s gets a child with a new
//! pid running the same executable. That is why this is one of two
//! checks and never the only one: the other is program identity
//! (`exe_of_peer` against an allowlist), and the two fail in different
//! directions on purpose.

use crate::IdentityError;

/// The process behind a connection, pinned for as long as this value
/// lives.
///
/// Cheap to clone (the pidfd is shared), and deliberately opaque: the
/// only questions it answers are "which pid" (for the audit trail) and
/// "is this the same running process as that one".
#[derive(Debug, Clone)]
pub struct Process {
    pid: u32,
    /// Holding this keeps `pid` un-recyclable. `None` means the pid was
    /// supplied without one — see [`Process::unpinned`].
    #[cfg(unix)]
    pinned: Option<std::sync::Arc<std::os::fd::OwnedFd>>,
}

impl Process {
    /// The process behind `peer`, pinned by the broker's pidfd.
    ///
    /// Refuses rather than falling back to the bare pid, exactly as
    /// [`crate::exe_of_peer`] does: a fallback would silently reopen the
    /// reuse window this type exists to close (#136).
    #[cfg(unix)]
    pub fn of_peer(peer: &crate::Peer) -> Result<Process, IdentityError> {
        let pid = crate::pid_of_peer(peer)?;
        Ok(Process {
            pid,
            // Cloned, not borrowed: the caller stores this across a
            // parked confirmation, and the pin has to outlive the reply
            // that produced it.
            pinned: peer.process_fd.clone(),
        })
    }

    /// A pid with nothing pinning it.
    ///
    /// For tests, and for the honest expression of "the transport told
    /// us a number and nothing more". **Never the sole basis of a
    /// decision**: an unpinned pid may already belong to somebody else.
    /// Named the way it is so that a reader of a call site can see which
    /// kind they are holding, the same way [`crate::Peer::without_process_fd`]
    /// is spelled out.
    pub fn unpinned(pid: u32) -> Process {
        Process {
            pid,
            #[cfg(unix)]
            pinned: None,
        }
    }

    /// The pid, for the audit trail. Not an identity on its own.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Is the kernel's answer to "which process" the same for both?
    pub fn is_same_as(&self, other: &Process) -> bool {
        self.pid == other.pid
    }

    /// Whether this value pins its pid. A `Process` that does not is
    /// diagnostic material, not evidence.
    pub fn is_pinned(&self) -> bool {
        #[cfg(unix)]
        {
            self.pinned.is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

/// "These two connections are the same process", for callers that may
/// hold nothing at all.
///
/// Answers `false` when either side is unknown. That is the **permissive**
/// direction for this one question, and it is deliberate: this check is
/// a companion to program identity, not a replacement for it, and a
/// caller we cannot place is already refused by the allowlist half. Made
/// a named function rather than an inline `matches!` so the choice is
/// visible and has a test.
pub fn same_process(a: Option<&Process>, b: Option<&Process>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.is_same_as(b),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The #289 shape, at the level of the primitive: two connections,
    /// one process. `PeerId` says "different"; `Process` says "same",
    /// and only the second answer is about who is running.
    #[test]
    fn two_connections_of_one_process_are_one_process() {
        let first = Process::unpinned(4242);
        let second = Process::unpinned(4242);
        assert!(first.is_same_as(&second));
        assert!(same_process(Some(&first), Some(&second)));

        // …and the connection identities differ, which is the trap.
        assert_ne!(
            crate::PeerId::Bus(":1.5".into()),
            crate::PeerId::Bus(":1.6".into())
        );
    }

    #[test]
    fn different_processes_are_different() {
        let a = Process::unpinned(4242);
        let b = Process::unpinned(4243);
        assert!(!a.is_same_as(&b));
        assert!(!same_process(Some(&a), Some(&b)));
    }

    /// An unknown side never answers "same process". The companion
    /// allowlist check is what refuses a caller we cannot place; this
    /// one must not invent a relationship it does not have.
    #[test]
    fn an_unknown_side_is_never_the_same_process() {
        let p = Process::unpinned(1);
        assert!(!same_process(Some(&p), None));
        assert!(!same_process(None, Some(&p)));
        assert!(!same_process(None, None));
    }

    /// A `Process` built from a bare number carries no pin, and says so.
    /// Anything that treats the two as interchangeable is reopening the
    /// reuse window (#136).
    #[test]
    fn an_unpinned_process_admits_that_it_is_unpinned() {
        assert!(!Process::unpinned(std::process::id()).is_pinned());
    }

    /// The broker must have supplied a pidfd, or there is no process to
    /// name — the same refusal `exe_of_peer` makes, for the same reason.
    #[cfg(unix)]
    #[test]
    fn a_peer_without_a_pidfd_yields_no_process() {
        let peer = crate::Peer::without_process_fd(
            crate::PeerId::Bus(":1.7".into()),
            Some(0),
            // A live, correct pid — and still not enough.
            Some(std::process::id()),
        );
        assert_eq!(
            Process::of_peer(&peer).map(|p| p.pid()),
            Err(IdentityError::NoProcessFd)
        );
    }
}
