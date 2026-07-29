//! Who is on the other end of a unix socket (ADR-0033, issue #99).
//!
//! [`crate::resolve`] answers this for a D-Bus caller by asking the
//! broker. A plain unix socket has no broker to ask — the kernel is the
//! only witness, and it is a better one.
//!
//! # Socket permissions are not the boundary here
//!
//! `remoted`'s socket is mode 0600, which is a real defence against
//! *another user*. It is no defence at all against the threat this crate
//! exists for: another **process of the same user** — an app you
//! installed, a Flatpak with session access, something the agent built.
//! All of them can `connect()`. 0600 says "one account"; it does not say
//! "one program".
//!
//! # SO_PEERCRED, and why it is not enough on its own
//!
//! `SO_PEERCRED` gives the pid, uid and gid the peer had when it
//! connected. The uid is directly useful. The **pid is not an identity**
//! (#136): between reading it and reading `/proc/<pid>/exe` the peer can
//! exit and its pid be recycled, so the answer may name a different
//! program entirely.
//!
//! `SO_PEERPIDFD` (Linux 6.5) returns a *pidfd* instead, which pins its
//! pid: the kernel will not recycle it while the descriptor is open, and
//! once the process exits the descriptor stops resolving rather than
//! following the pid to its next owner. That is the same guarantee the
//! D-Bus broker's `ProcessFD` gives, obtained without a broker.
//!
//! Where it is unavailable — an older kernel, a non-Linux host — the
//! peer simply has no pidfd, and everything downstream that needs to
//! name a program refuses. There is no fallback to the bare pid, for
//! exactly the reason above.

use crate::{Peer, PeerId};

/// `SO_PEERPIDFD`, Linux 6.5+.
///
/// Spelled out because the libc crate does not export it for every
/// target we build. The value is from `asm-generic/socket.h` and is
/// architecture-independent for every arch Lisa targets (x86_64 and
/// aarch64 both use the asm-generic numbering).
#[cfg(target_os = "linux")]
const SO_PEERPIDFD: libc::c_int = 77;

/// The peer of a **connected** unix socket.
///
/// Returns a [`Peer`] whose [`PeerId`] is [`PeerId::Direct`] — a
/// connected socket has exactly one peer, so the connection *is* the
/// identity, and there is nothing for it to claim.
///
/// Never fails: a peer we cannot learn anything about is one with no uid
/// and no pidfd, which every check downstream reads as "unidentified"
/// and refuses.
///
/// # Only ever call this on an accepted stream
///
/// On Linux, asking a *listening* socket about its peer does not fail —
/// it answers with **this process's own credentials**, pidfd included.
/// (Found by running the test, which had asserted the opposite; macOS
/// refuses the same call, so the mistake is invisible on a dev host.)
/// A caller that passed a listener would therefore be told the peer is
/// itself, which for a daemon holding an allowlist is the wrong answer
/// in the dangerous direction.
///
/// Lisa's one caller is axum's `IncomingStream`, i.e. always the
/// product of `accept()`. There is no type that can enforce this — a
/// listener and a stream are both `AsRawFd` — so it is written down
/// here instead.
#[cfg(unix)]
pub fn peer_of_socket<S: std::os::fd::AsRawFd>(sock: &S) -> Peer {
    let fd = sock.as_raw_fd();
    let (uid, pid) = peer_cred(fd);
    Peer {
        id: PeerId::Direct,
        uid,
        pid,
        process_fd: peer_pidfd(fd),
    }
}

/// uid and pid from `SO_PEERCRED`. The pid is for logs; see the module
/// docs for why it is not an identity.
#[cfg(all(unix, target_os = "linux"))]
fn peer_cred(fd: std::os::fd::RawFd) -> (Option<u32>, Option<u32>) {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` is a live, correctly-sized ucred and `len` is its
    // size; getsockopt writes at most that many bytes and reports what
    // it wrote. The fd is borrowed from a live socket for this call.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return (None, None);
    }
    (
        Some(cred.uid),
        u32::try_from(cred.pid).ok().filter(|p| *p != 0),
    )
}

/// macOS and the BSDs spell it differently and have no pidfd at all.
/// `getpeereid` still gives a trustworthy uid, which is what the
/// same-user check needs.
#[cfg(all(unix, not(target_os = "linux")))]
fn peer_cred(fd: std::os::fd::RawFd) -> (Option<u32>, Option<u32>) {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: both out-params are live locals of the right type, and the
    // fd is borrowed from a live socket for the duration of the call.
    let rc = unsafe { libc::getpeereid(fd, &raw mut uid, &raw mut gid) };
    if rc != 0 {
        return (None, None);
    }
    (Some(uid), None)
}

/// The peer's pidfd, when the kernel will give us one.
#[cfg(all(unix, target_os = "linux"))]
fn peer_pidfd(fd: std::os::fd::RawFd) -> Option<std::sync::Arc<std::os::fd::OwnedFd>> {
    use std::os::fd::FromRawFd;
    let mut pidfd: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `pidfd` is a live c_int and `len` its size; on success the
    // kernel installs a new descriptor and writes it there, which we
    // take ownership of exactly once below.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            SO_PEERPIDFD,
            (&raw mut pidfd).cast(),
            &raw mut len,
        )
    };
    if rc != 0 || pidfd < 0 {
        // ENOPROTOOPT on kernels before 6.5. Not an error worth
        // reporting per connection — it is a property of the machine,
        // and the surfaces that need a pidfd say so themselves.
        return None;
    }
    // SAFETY: getsockopt just handed us an owned descriptor that nothing
    // else refers to.
    Some(std::sync::Arc::new(unsafe {
        std::os::fd::OwnedFd::from_raw_fd(pidfd)
    }))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn peer_pidfd(_fd: std::os::fd::RawFd) -> Option<std::sync::Arc<std::os::fd::OwnedFd>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    /// The kernel knows who we are, and it is us.
    ///
    /// A socketpair's peer is this very process, so this asserts the
    /// plumbing end to end without needing a second program: the uid
    /// must be ours, and must be *learned*, not defaulted.
    #[test]
    fn a_connected_socket_reports_our_own_credentials() {
        let (a, _b) = UnixStream::pair().unwrap();
        let peer = peer_of_socket(&a);
        assert_eq!(peer.id, PeerId::Direct);
        assert_eq!(
            peer.uid,
            Some(unsafe { libc::getuid() }),
            "the peer's uid was not learned from the kernel"
        );
        assert!(
            peer.is_same_user_as_us(),
            "our own socketpair peer was judged another user"
        );
    }

    /// On Linux the pidfd is the whole point: without it nothing
    /// downstream may name a program, so a silent absence would quietly
    /// disable the check rather than fail it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_connected_socket_yields_a_pidfd_that_resolves_to_us() {
        let (a, _b) = UnixStream::pair().unwrap();
        let peer = peer_of_socket(&a);
        let Some(_) = peer.process_fd.as_ref() else {
            // Kernels before 6.5 genuinely cannot. Say which, rather
            // than passing quietly.
            eprintln!("SO_PEERPIDFD unavailable on this kernel — skipping");
            return;
        };
        assert_eq!(
            crate::exe_of_peer(&peer).unwrap(),
            std::env::current_exe().unwrap(),
            "the pidfd did not resolve to this test binary"
        );
    }

    /// A listening socket answers about ITSELF, and this pins the
    /// footgun.
    ///
    /// Written first as "an unconnected socket has no peer", which
    /// Linux disagreed with: it returns this process's own uid *and its
    /// own pidfd*, so the caller is told "the peer is you". macOS's
    /// `getpeereid` refuses the same call, which is exactly why the
    /// assumption survived every dev-host run and only fell over in a
    /// container.
    ///
    /// Nothing in Lisa passes a listener — axum hands us accepted
    /// streams — and no type can prevent it, since a listener and a
    /// stream are both `AsRawFd`. So the behaviour is pinned here: a
    /// future refactor that starts asking listeners meets this test
    /// rather than an authorization bug.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_listening_socket_answers_about_this_process_not_a_peer() {
        let listener =
            std::os::unix::net::UnixListener::bind(tempfile::tempdir().unwrap().path().join("s"))
                .unwrap();
        let peer = peer_of_socket(&listener);
        assert_eq!(
            peer.uid,
            Some(unsafe { libc::getuid() }),
            "a listener reports its own uid, not a peer's"
        );
        if peer.process_fd.is_some() {
            assert_eq!(
                crate::exe_of_peer(&peer).unwrap(),
                std::env::current_exe().unwrap(),
                "a listener's pidfd resolves to this very process"
            );
        }
    }
}
