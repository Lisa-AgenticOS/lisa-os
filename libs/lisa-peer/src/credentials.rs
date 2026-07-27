//! Credentials for the cases ownership cannot answer (ADR-0033).
//!
//! [`crate::PeerId`] answers "same caller as before?". This answers
//! "which user and which process?", which is only needed where the
//! decision depends on the program itself — "may this caller mint grants
//! for *other* apps" (#107), "may this caller flip every egress scope"
//! (#99).
//!
//! Credentials come from the message broker's own
//! `GetConnectionCredentials`, i.e. from the kernel via `SO_PEERCRED`,
//! not from anything the sender put in the message. On a p2p link there
//! is no broker to ask, and also no ambiguity about who the peer is.

use crate::{PeerError, PeerId};
use std::sync::Arc;

/// A caller, as the transport reports it.
#[derive(Debug, Clone)]
pub struct Peer {
    pub id: PeerId,
    /// The uid the kernel reports for the peer process. `None` on p2p.
    pub uid: Option<u32>,
    /// The pid the kernel reports. `None` on p2p.
    ///
    /// **Not sufficient to attribute identity.** A pid is meaningful
    /// only while the peer is connected, and "resolve it at the moment
    /// of the call" does not close the window: the peer can exit between
    /// the broker's credentials reply and our `readlink`, and a recycled
    /// pid then points at somebody else's binary. Use [`process_fd`]
    /// (via `exe_of_peer`) for anything that decides who someone is;
    /// this field is for logging and diagnostics.
    ///
    /// [`process_fd`]: Peer::process_fd
    pub pid: Option<u32>,
    /// The broker's pidfd for the peer process, when it supplied one.
    ///
    /// A pidfd pins its pid: the kernel will not recycle it while the
    /// descriptor is open, and once the process exits the descriptor
    /// stops resolving instead of following the pid to its next owner.
    /// That is what makes it, and not `pid`, the basis of an identity
    /// decision (#136).
    #[cfg(unix)]
    pub process_fd: Option<Arc<std::os::fd::OwnedFd>>,
}

impl Peer {
    /// A peer with no pidfd — p2p, a broker that did not supply one, and
    /// tests. Deliberately explicit: anything built this way cannot
    /// answer "which program is this" (see [`Peer::process_fd`]).
    pub fn without_process_fd(id: PeerId, uid: Option<u32>, pid: Option<u32>) -> Peer {
        Peer {
            id,
            uid,
            pid,
            #[cfg(unix)]
            process_fd: None,
        }
    }

    /// Whether this caller runs as the same user as this process.
    ///
    /// The session daemons serve exactly one user, so a caller from a
    /// different uid is always wrong. Fails closed when the uid is
    /// unknown on a brokered connection.
    pub fn is_same_user_as_us(&self) -> bool {
        match (&self.id, self.uid) {
            // p2p: the socket was handed to one peer; there is no other
            // user to confuse it with.
            (PeerId::Direct, _) => true,
            (PeerId::Bus(_), Some(uid)) => uid == current_uid(),
            (PeerId::Bus(_), None) => false,
        }
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid() is always successful and has no preconditions.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    u32::MAX
}

/// Resolve the caller of `header` on `conn`.
///
/// On a brokered connection this asks `org.freedesktop.DBus` for the
/// sender's credentials. On p2p it returns [`PeerId::Direct`] with no
/// uid/pid, which is the honest answer rather than a guess.
pub async fn resolve(
    conn: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> Result<Peer, PeerError> {
    // The BROKER answers this, never the peer. Issue #133: the first
    // version keyed off the header's sender, so a p2p client that forged
    // a bus-looking sender skipped the early return below — and
    // `GetConnectionCredentials` then went down the SAME p2p socket to
    // the attacker, who answered `uid=0, pid=1` for a process whose real
    // uid was 501. `is_same_user_as_us()` and `exe_of_pid()` were both
    // satisfiable by self-attestation, which is precisely the class of
    // bug this crate exists to remove.
    //
    // So the check is on the CONNECTION: no broker, no question asked.
    if conn.unique_name().is_none() {
        return Ok(Peer::without_process_fd(PeerId::Direct, None, None));
    }

    let id = PeerId::of(conn, header)?;
    let PeerId::Bus(ref name) = id else {
        // Unreachable given the check above, but a future edit that
        // reorders these must not silently start trusting a peer.
        return Err(PeerError::Unidentified);
    };

    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    let bus_name =
        zbus::names::BusName::try_from(name.clone()).map_err(|_| PeerError::Unidentified)?;
    let creds = dbus
        .get_connection_credentials(bus_name)
        .await
        .map_err(|_| PeerError::Unidentified)?;

    Ok(Peer {
        id,
        uid: creds.unix_user_id(),
        pid: creds.process_id(),
        // Kept, not discarded (#136). `resolve()` used to read the uid
        // and pid and throw the pidfd away, leaving every downstream
        // `/proc/<pid>/exe` lookup open to pid reuse — with the
        // mitigation sitting unused in the same reply.
        #[cfg(unix)]
        process_fd: creds.process_fd().and_then(|fd| {
            use std::os::fd::AsFd;
            fd.as_fd().try_clone_to_owned().ok().map(Arc::new)
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_peer_is_always_our_own_user() {
        let peer = Peer::without_process_fd(PeerId::Direct, None, None);
        assert!(peer.is_same_user_as_us());
    }

    /// A brokered caller whose uid we could not learn is refused rather
    /// than assumed — the whole crate exists because assumption is the
    /// bug.
    #[test]
    fn an_unknown_uid_on_the_bus_fails_closed() {
        let peer = Peer::without_process_fd(PeerId::Bus(":1.7".into()), None, None);
        assert!(!peer.is_same_user_as_us());
    }

    #[cfg(unix)]
    #[test]
    fn another_users_uid_is_rejected() {
        let ours = Peer::without_process_fd(PeerId::Bus(":1.7".into()), Some(current_uid()), None);
        assert!(ours.is_same_user_as_us());

        let theirs = Peer::without_process_fd(
            PeerId::Bus(":1.8".into()),
            Some(current_uid().wrapping_add(1)),
            None,
        );
        assert!(!theirs.is_same_user_as_us());
    }
}
