//! Who is calling (ADR-0033).
//!
//! Five independent adversarial reviews of `agentd`, the portal,
//! `remoted` and `contextd` converged on one sentence: **nothing in Lisa
//! verifies who is calling.** Provenance and `actor` are asserted on the
//! wire, `Confirm`/`Undo` carry no identity at all, portal grants are
//! mintable by any host process, session objects are not bound to their
//! opener, and memory namespaces are whatever the caller says they are.
//!
//! That is not twenty bugs. It is one missing primitive, absent twenty
//! times, and this crate is it.
//!
//! ADR-0030 says a guardrail must not be reachable from inside the
//! probabilistic system. Caller identity is the input every other check
//! depends on — tiers, grants, quotas, ownership — so when the caller
//! supplies it, *nothing downstream is a boundary*. A pure function over
//! attacker-controlled inputs is not a boundary.
//!
//! # Two mechanisms, deliberately separate
//!
//! **Ownership** ([`PeerId`]) answers *"is this the same caller as
//! before?"* — the cheap, exact question. It needs no `/proc`, no
//! credentials round-trip, and it is what binds a parked confirmation, a
//! portal session, or a memory namespace to whoever created it.
//!
//! **Credentials** ([`Peer`]) answer *"which user and process is this?"*
//! — needed only where a decision depends on the program itself, such as
//! "may this caller mint grants for other apps". They come from the
//! D-Bus broker ([`resolve`]) or, for a plain unix socket, from the
//! kernel ([`unix::peer_of_socket`]).
//!
//! Most of the filed issues are fixed by the first, which is why it is
//! usable without a message broker.
//!
//! # A third question, learned the hard way
//!
//! [`Process`] answers *"are these two connections the same running
//! process?"*. It exists because the first mechanism was being used to
//! answer it and cannot: a unique name identifies a **connection**, and
//! one process may hold many. `agentd` refused a model host's
//! self-approval by comparing unique names, and the same process
//! answered from a second socket while owning the consent surface's
//! name — `<allow own="*"/>` ships in `session.conf` (#289). Same pid,
//! two `PeerId`s, and every "is this somebody else?" check said yes.

pub mod app;
mod credentials;
mod identity;
pub mod manager;
mod process;
pub mod prompt_surface;
#[cfg(unix)]
pub mod unix;

pub use credentials::{Peer, resolve, resolve_unique_name};
pub use identity::{IdentityError, exe_of_pid};
#[cfg(unix)]
pub use identity::{exe_of_peer, pid_of_peer};
pub use process::{Process, same_process};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PeerError {
    #[error("the caller could not be identified — refusing")]
    Unidentified,
    #[error("this object belongs to a different caller")]
    NotTheOwner,
    #[error("bus: {0}")]
    Bus(#[from] zbus::Error),
}

/// The identity of a connection, as assigned by the transport rather
/// than claimed by the peer.
///
/// **`Direct` is one peer, not "any peer".** A daemon that serves
/// several untrusted clients over separate p2p sockets would give them
/// all the same `PeerId`, and ownership would not distinguish them. Lisa
/// serves the session bus in production and uses p2p only in tests, so
/// this is sound here — but it is a real constraint, not an oversight,
/// and anything that starts accepting multiple p2p clients needs a
/// per-connection identity instead.
///
/// This is the whole point: a unique bus name is handed out by the
/// message broker, is never chosen by the sender, is unique for the
/// life of the connection, and is **never reused** after the connection
/// drops. A caller cannot forge one, and cannot inherit a dead peer's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PeerId {
    /// A unique bus name (`:1.42`) from a message broker.
    Bus(String),
    /// A point-to-point connection. There is exactly one peer, so the
    /// connection *is* the identity — this is not a fallback that
    /// weakens anything, it is the correct answer for that transport
    /// (and it is the transport the daemons' own tests use).
    Direct,
}

impl PeerId {
    /// Who sent this message, decided by the TRANSPORT.
    ///
    /// The first version of this read `header.sender()` alone, and that
    /// was the very bug this crate exists to fix (issue #132). A
    /// message's SENDER field is filled in *by the message bus*; on a
    /// point-to-point link nothing rewrites it and zbus's
    /// `Builder::sender()` is public, so a client can put whatever it
    /// likes there. A reviewer had one p2p client present as `:1.10`,
    /// `:1.11` and `:1.10` again on a single socket and self-approve its
    /// own destructive call.
    ///
    /// So the connection decides, not the message.
    /// `Connection::unique_name()` is `Some` only when a broker assigned
    /// us one — which is exactly the condition "there is a bus here that
    /// fills SENDER in". On p2p the header is ignored entirely.
    ///
    /// Fails closed: a brokered connection whose message has no sender
    /// is anomalous (the spec says the bus always sets it), and
    /// answering `Direct` there would let it match a p2p-owned object.
    pub fn of(
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> Result<PeerId, PeerError> {
        // `unique_name()` is Some only when a broker assigned us one.
        Self::decide(
            conn.unique_name().is_some(),
            header.sender().map(|s| s.as_str()),
        )
    }

    /// The decision itself, separated so it can be tested.
    ///
    /// `PeerId::of` needs a live `Connection`, which a unit test cannot
    /// cheaply build — so before this split the only part an attacker
    /// touches had no test at all. That is how #132 shipped.
    pub(crate) fn decide(brokered: bool, sender: Option<&str>) -> Result<PeerId, PeerError> {
        if !brokered {
            // p2p: one peer, and nothing it writes about itself counts.
            return Ok(PeerId::Direct);
        }
        match sender {
            Some(name) => Ok(PeerId::Bus(name.to_string())),
            None => Err(PeerError::Unidentified),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            PeerId::Bus(name) => name,
            PeerId::Direct => "(direct)",
        }
    }
}

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The caller an object belongs to.
///
/// Store one alongside anything a *later* call may act on — a parked
/// confirmation, a session object, a memory namespace — and check it
/// before acting. That single discipline closes #93 (any peer approves
/// any confirmation), #108 (any app cancels any session) and #101 (any
/// peer wipes any namespace).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Owner(PeerId);

impl Owner {
    pub fn of(id: PeerId) -> Owner {
        Owner(id)
    }

    /// Whether `caller` is the **connection** this object belongs to.
    ///
    /// Not "the same process" and not "the same program" — a process may
    /// hold any number of connections, so a `false` here means only
    /// *"some other socket"*. Anything that reads a `false` as "somebody
    /// independent is involved" needs [`Process`] as well (#289).
    pub fn allows(&self, caller: &PeerId) -> bool {
        &self.0 == caller
    }

    /// `Ok(())` if `caller` owns this, `NotTheOwner` otherwise — the
    /// form that drops straight into a `?` chain at the top of a method.
    pub fn require(&self, caller: &PeerId) -> Result<(), PeerError> {
        if self.allows(caller) {
            Ok(())
        } else {
            Err(PeerError::NotTheOwner)
        }
    }

    pub fn id(&self) -> &PeerId {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #132. A p2p client can set the SENDER field to anything —
    /// `Builder::sender()` is public and nothing rewrites it without a
    /// broker. A reviewer used exactly this to present as `:1.10`,
    /// `:1.11` and `:1.10` again on ONE socket and self-approve a
    /// destructive call. The forged value must be ignored entirely.
    #[test]
    fn a_forged_sender_is_ignored_without_a_broker() {
        for forged in [
            ":1.10",
            ":1.11",
            "dev.lisaos.Shell",
            "org.freedesktop.DBus",
            "",
        ] {
            assert_eq!(
                PeerId::decide(false, Some(forged)).unwrap(),
                PeerId::Direct,
                "p2p honoured a forged sender `{forged}`"
            );
        }
        assert_eq!(PeerId::decide(false, None).unwrap(), PeerId::Direct);
    }

    /// On a broker the sender is filled in by the bus, so it is the
    /// identity — and its absence is anomalous enough to refuse rather
    /// than answer `Direct`, which would match a p2p-owned object.
    #[test]
    fn a_brokered_message_without_a_sender_fails_closed() {
        assert_eq!(
            PeerId::decide(true, Some(":1.42")).unwrap(),
            PeerId::Bus(":1.42".into())
        );
        assert!(matches!(
            PeerId::decide(true, None),
            Err(PeerError::Unidentified)
        ));
    }

    /// The two transports must never produce an identity that satisfies
    /// the other's owner — otherwise a forged bus name on p2p (or the
    /// reverse) would cross the boundary.
    #[test]
    fn a_p2p_caller_can_never_satisfy_a_bus_owner() {
        let bus_owner = Owner::of(PeerId::Bus(":1.10".into()));
        let p2p_caller = PeerId::decide(false, Some(":1.10")).unwrap();
        assert!(
            !bus_owner.allows(&p2p_caller),
            "a forged p2p sender satisfied a bus owner"
        );
    }

    #[test]
    fn a_bus_peer_owns_only_its_own_objects() {
        let alice = PeerId::Bus(":1.10".into());
        let mallory = PeerId::Bus(":1.11".into());
        let owner = Owner::of(alice.clone());

        assert!(owner.allows(&alice));
        assert!(owner.require(&alice).is_ok());

        assert!(!owner.allows(&mallory));
        assert!(matches!(
            owner.require(&mallory),
            Err(PeerError::NotTheOwner)
        ));
    }

    /// A bus peer must never be able to reach a p2p-owned object or vice
    /// versa — otherwise a daemon that accepts both transports would let
    /// the brokered side claim the direct side's objects.
    #[test]
    fn the_transports_do_not_bleed_into_each_other() {
        let direct = Owner::of(PeerId::Direct);
        assert!(direct.allows(&PeerId::Direct));
        assert!(!direct.allows(&PeerId::Bus(":1.10".into())));

        let bus = Owner::of(PeerId::Bus(":1.10".into()));
        assert!(!bus.allows(&PeerId::Direct));
    }

    /// Unique names are never reused by the broker, so a reconnecting
    /// process gets a new one and cannot inherit the dead peer's
    /// objects. This test pins the *semantics we rely on*: identity is
    /// the exact string, never a prefix or a numeric neighbour.
    #[test]
    fn a_reconnecting_peer_does_not_inherit() {
        let owner = Owner::of(PeerId::Bus(":1.10".into()));
        for impostor in [":1.1", ":1.100", ":1.10 ", " :1.10", ":1.011", "1.10", ""] {
            assert!(
                !owner.allows(&PeerId::Bus(impostor.into())),
                "`{impostor}` was accepted as `:1.10`"
            );
        }
    }
}
