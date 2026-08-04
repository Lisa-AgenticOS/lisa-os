//! What a caller is ALLOWED to be (ADR-0033, ADR-0036 §1).
//!
//! `Run` takes a `trigger` option, and the whole point of ADR-0036 is
//! that it cannot be believed. A client that could name its own class
//! could launder attacker-supplied content into the class a human typed
//! — an email body arriving as "a person asked for this" — which is the
//! attack ADR-0036 exists to stop.
//!
//! So the message says what the caller *wants*; this module says what
//! the caller *may have*. `Trigger::resolve` takes the lower of the two.
//!
//! # Why the answer is a bus name and not an executable
//!
//! Everywhere else in Lisa, program identity is `/proc/<pid>/exe` via
//! the broker's pidfd (`lisa_peer::exe_of_peer`) — never `comm`, never
//! anything the message asserts. **That mechanism does not work in this
//! daemon**, and shipping it here would be a check that silently never
//! matches.
//!
//! `lisa-harnessd.service` is a PER-USER unit carrying `ProtectHome`,
//! `ProtectSystem=strict` and `PrivateDevices`. A user manager can only
//! deliver those through an implicit private user namespace, and from
//! inside one, ptrace-read of any process outside it is denied — so
//! `readlink /proc/<peer>/exe` returns EACCES for every caller. That is
//! issue #161, and it is why `os/repo-tools/check-user-units.py` lists
//! this unit in ALLOWED. Verified again on the reference machine while
//! writing this: harnessd's `uid_map` reads `1000 1000 1`, and
//! `readlink /proc/<agentd>/exe` succeeds outside the namespace and
//! fails inside it.
//!
//! Two things a user manager's namespace does NOT break, because neither
//! reads another process's `/proc`, are the broker's own answers:
//!
//! - `GetConnectionCredentials` → the peer's uid (from `SO_PEERCRED`);
//! - `GetNameOwner` → which connection currently owns a well-known name.
//!
//! Both are assigned by the broker and unforgeable by the sender, which
//! is the ADR-0033 property that matters. So the ceiling is built out of
//! those two, and this file says so out loud rather than leaving the
//! next reader to "fix" it into an exe check that always refuses.
//!
//! # The honest limit
//!
//! A well-known name is owned by whoever asks for it first. If the
//! Assistant is not running, another session peer can take
//! `app.lisaos.Assistant` and inherit its ceiling. That is a real
//! weakness and it is smaller than the one it replaces: before this, no
//! peer had to do anything at all, and a peer that takes the Assistant's
//! name has also taken over the window the person launches, which is a
//! far louder place to be standing.

use crate::dbus::Trigger;

/// Well-known names whose owner is a surface a person types into.
///
/// One entry, because one surface drives `Run` today: the Assistant
/// window (`shell/assistant`). The overlay is still an `Overlay1` client
/// and `lisa assist` runs the loop in-process, so neither is listed —
/// naming a surface that does not call this daemon would be documenting
/// intent as behaviour. When one of them moves onto Harness1 it is
/// added here, deliberately, as a decision about who may claim that a
/// human typed something.
pub const PROMPT_SURFACES: [&str; 1] = ["app.lisaos.Assistant"];

/// What the transport says about a caller. Every field is an answer from
/// the broker; none of them is anything the message claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallerFacts {
    /// `GetConnectionCredentials` says this connection runs as our uid.
    pub same_user: bool,
    /// `GetNameOwner` says this connection currently owns one of
    /// [`PROMPT_SURFACES`].
    pub owns_prompt_surface: bool,
}

impl CallerFacts {
    /// What we know about a caller we could not identify at all.
    ///
    /// Every field false, so the ceiling is the least trusted class.
    /// This is the value used when the broker cannot be reached or the
    /// header carries no sender: an unidentifiable caller is an event
    /// source, not a person.
    pub const UNKNOWN: CallerFacts = CallerFacts {
        same_user: false,
        owns_prompt_surface: false,
    };
}

/// The highest trust class this caller may claim.
///
/// Pure, so every case below is a unit test rather than a claim. The
/// only way up is to be this user AND to be holding a prompt surface's
/// name; everything else — including a caller we simply could not place
/// — is [`Trigger::Event`], the class whose content is never trusted.
///
/// `Schedule` is deliberately unreachable: nothing in Lisa is a
/// scheduler yet, and a ceiling that hands out a class no shipped peer
/// can legitimately hold would be a hole with no user. A scheduler
/// daemon arrives with its own name and its own arm of this function.
pub fn ceiling(facts: CallerFacts) -> Trigger {
    if facts.same_user && facts.owns_prompt_surface {
        Trigger::Prompt
    } else {
        Trigger::Event
    }
}

/// Ask the transport about the caller of `header`.
///
/// Fails towards [`CallerFacts::UNKNOWN`] on every error: a broker that
/// will not answer is a caller we cannot place, and the safe reading of
/// "cannot place" is "not a person".
pub async fn facts_of(conn: &zbus::Connection, header: &zbus::message::Header<'_>) -> CallerFacts {
    let Ok(peer) = lisa_peer::resolve(conn, header).await else {
        return CallerFacts::UNKNOWN;
    };
    // p2p has no broker: no credentials, no name ownership, and — see
    // the module docs — no production caller either. harnessd only ever
    // serves the session bus.
    let lisa_peer::PeerId::Bus(ref caller_name) = peer.id else {
        return CallerFacts::UNKNOWN;
    };
    let same_user = peer.is_same_user_as_us();
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return CallerFacts::UNKNOWN;
    };
    let mut owns_prompt_surface = false;
    for surface in PROMPT_SURFACES {
        let Ok(name) = zbus::names::BusName::try_from(surface) else {
            continue;
        };
        // `GetNameOwner` does NOT start an activatable service, which is
        // what makes this safe to call on every Run: asking who owns the
        // Assistant's name must not launch the Assistant.
        if let Ok(owner) = dbus.get_name_owner(name).await
            && owner.as_str() == caller_name
        {
            owns_prompt_surface = true;
            break;
        }
    }
    CallerFacts {
        same_user,
        owns_prompt_surface,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defect this module exists to close (#229). The ceiling used
    /// to be the literal `Trigger::Prompt` for every caller on the
    /// session bus, so any peer — `busctl`, a background app, anything
    /// that could open the session bus — drove a run in the class a
    /// human typing gets. Demonstrated on the device before this
    /// landed: `busctl --user call … Run` reached the model.
    #[test]
    fn an_unidentified_caller_is_an_event_source_not_a_person() {
        assert_eq!(ceiling(CallerFacts::UNKNOWN), Trigger::Event);
    }

    /// Both halves are required, and neither is enough on its own. A
    /// caller from another uid holding the name is not this user's
    /// surface; this user's ordinary process is not a surface at all.
    #[test]
    fn only_this_users_prompt_surface_reaches_the_prompt_class() {
        assert_eq!(
            ceiling(CallerFacts {
                same_user: true,
                owns_prompt_surface: true
            }),
            Trigger::Prompt
        );
        assert_eq!(
            ceiling(CallerFacts {
                same_user: true,
                owns_prompt_surface: false
            }),
            Trigger::Event,
            "an ordinary peer of this user reached the prompt class"
        );
        assert_eq!(
            ceiling(CallerFacts {
                same_user: false,
                owns_prompt_surface: true
            }),
            Trigger::Event,
            "another user's process reached the prompt class by holding the name"
        );
    }

    /// A ceiling that handed out `Schedule` would be handing out a
    /// class no shipped peer can legitimately hold — a hole with no
    /// user. Nothing may reach it until a scheduler exists.
    #[test]
    fn no_caller_can_reach_the_schedule_class_yet() {
        for facts in [
            CallerFacts::UNKNOWN,
            CallerFacts {
                same_user: true,
                owns_prompt_surface: true,
            },
            CallerFacts {
                same_user: true,
                owns_prompt_surface: false,
            },
            CallerFacts {
                same_user: false,
                owns_prompt_surface: true,
            },
        ] {
            assert_ne!(ceiling(facts), Trigger::Schedule, "{facts:?}");
        }
    }

    /// Adding a surface here is a decision about who may claim "a human
    /// typed this", not a path fix — the same invariant
    /// `lisa_peer::manager`'s allowlist carries. A well-known name only
    /// works as identity if it is a name we ship and own.
    #[test]
    fn only_known_surfaces_may_claim_a_human_typed_it() {
        for name in PROMPT_SURFACES {
            assert!(
                name.starts_with("app.lisaos.") || name.starts_with("dev.lisaos."),
                "unexpected prompt surface {name:?} — adding one is a decision \
                 about who may claim a human typed something"
            );
        }
        let mut sorted = PROMPT_SURFACES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), PROMPT_SURFACES.len(), "duplicate surface");
    }
}
