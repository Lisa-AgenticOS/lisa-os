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

/// A caller, as the transport reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub id: PeerId,
    /// The uid the kernel reports for the peer process. `None` on p2p.
    pub uid: Option<u32>,
    /// The pid the kernel reports. `None` on p2p, and note that a pid is
    /// only meaningful *while the peer is connected* — never store one
    /// and re-resolve it later, because pids are reused.
    pub pid: Option<u32>,
}

impl Peer {
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
        return Ok(Peer {
            id: PeerId::Direct,
            uid: None,
            pid: None,
        });
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_peer_is_always_our_own_user() {
        let peer = Peer {
            id: PeerId::Direct,
            uid: None,
            pid: None,
        };
        assert!(peer.is_same_user_as_us());
    }

    /// A brokered caller whose uid we could not learn is refused rather
    /// than assumed — the whole crate exists because assumption is the
    /// bug.
    #[test]
    fn an_unknown_uid_on_the_bus_fails_closed() {
        let peer = Peer {
            id: PeerId::Bus(":1.7".into()),
            uid: None,
            pid: None,
        };
        assert!(!peer.is_same_user_as_us());
    }

    #[cfg(unix)]
    #[test]
    fn another_users_uid_is_rejected() {
        let ours = Peer {
            id: PeerId::Bus(":1.7".into()),
            uid: Some(current_uid()),
            pid: None,
        };
        assert!(ours.is_same_user_as_us());

        let theirs = Peer {
            id: PeerId::Bus(":1.8".into()),
            uid: Some(current_uid().wrapping_add(1)),
            pid: None,
        };
        assert!(!theirs.is_same_user_as_us());
    }
}
