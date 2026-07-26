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
//! "may this caller mint grants for other apps".
//!
//! Most of the filed issues are fixed by the first, which is why it is
//! usable without a message broker.

mod credentials;
mod identity;

pub use credentials::{Peer, resolve};
pub use identity::{IdentityError, exe_of_pid};

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
    /// The sender of a message, or [`PeerId::Direct`] on a p2p link.
    ///
    /// Never fails: an absent sender on a brokered connection would be
    /// indistinguishable from p2p, so callers that need certainty use
    /// [`resolve`] and get credentials or an error.
    pub fn of(header: &zbus::message::Header<'_>) -> PeerId {
        match header.sender() {
            Some(name) => PeerId::Bus(name.to_string()),
            None => PeerId::Direct,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner(PeerId);

impl Owner {
    pub fn of(id: PeerId) -> Owner {
        Owner(id)
    }

    /// Whether `caller` is the peer this object belongs to.
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
