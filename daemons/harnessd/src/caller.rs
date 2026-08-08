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
//! # Why the answer is a bus name AND a program, both read here
//!
//! Program identity everywhere in Lisa is `/proc/<pid>/exe` via the
//! broker's pidfd (`lisa_peer::exe_of_peer`) — never `comm`, never
//! anything the message asserts. This daemon now reads it directly, and
//! that is the whole of ADR-0064.
//!
//! It used to be unable to. `lisa-harnessd.service` carried `ProtectHome`,
//! `ProtectSystem=strict` and `PrivateDevices`, which a user manager can
//! only deliver through an implicit private user namespace — and from
//! inside one, `readlink /proc/<peer>/exe` is EACCES for every peer
//! outside it (#161). So the program half was fetched from **agentd**,
//! which runs outside the namespace, over `dev.lisaos.Agent1.IsPromptSurface`.
//!
//! The #306 close-replay showed that was the bug one hop along:
//! `dev.lisaos.Agent1` is itself a claimable well-known name, unowned
//! whenever agentd is not up (it zombied once — #347), so under
//! `<allow own="*"/>` a squatter could take Agent1, take
//! `app.lisaos.Assistant`, and answer the identity question about
//! *itself*. Delegating identity to a name a peer can claim is the same
//! defect the delegation was meant to fix.
//!
//! ADR-0064's fix: drop the mount-class sandbox from the unit so this
//! daemon can read `/proc` itself, and delete the oracle. The sandbox
//! loss is real — harnessd is the process the model runs inside — and it
//! is accepted because the guardrails that bound the *model* live in the
//! bus (tiers and provenance escalation, ADR-0036 §3) and on the tools
//! (Landlock, #307/#309), not in harnessd's own filesystem view;
//! `NoNewPrivileges`, `IPAddressDeny=any` and `RestrictAddressFamilies`
//! stay. `os/repo-tools/check-user-units.py` no longer lists this unit
//! in ALLOWED — the gate now enforces the decision.
//!
//! The two broker answers this leans on are unforgeable by the sender:
//! `GetConnectionCredentials` → the peer's uid, and `GetNameOwner` →
//! which connection owns a well-known name. The exe read is the third,
//! and [`lisa_peer::prompt_surface`] holds the one shared definition of
//! "is a prompt surface" both daemons must agree on.
//!
//! # The honest limit that remains
//!
//! The Assistant is `Exec=/usr/bin/lisa-app assistant/lisa-assistant.js`,
//! and `lisa-app` ends in `exec gjs`. So the program check refuses every
//! compiled squatter and the demonstrated `python3` one, and does not
//! refuse a hostile GJS script. An Assistant with an executable of its
//! own is what closes that, and `PROMPT_SURFACE_PROGRAMS` already lists
//! the path it would have.

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
    ///
    /// Unforgeable and **not sufficient**: it says this peer called
    /// `RequestName` first, which under `<allow own="*"/>` anybody may
    /// do (#306).
    pub owns_prompt_surface: bool,
    /// The program behind this connection is a prompt surface — its
    /// `/proc/<pid>/exe` (through the broker's pidfd) is on
    /// [`lisa_peer::prompt_surface::PROMPT_SURFACE_PROGRAMS`]. Read here
    /// directly now that this daemon is out of the user namespace that
    /// made `/proc/<peer>/exe` EACCES (#161, #306, ADR-0064).
    pub runs_a_prompt_program: bool,
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
        runs_a_prompt_program: false,
    };
}

/// The highest trust class this caller may claim.
///
/// Pure, so every case below is a unit test rather than a claim. The
/// only way up is **three** facts at once: this user, holding a prompt
/// surface's name, and running a prompt-surface program. Everything
/// else — including a caller we simply could not place — is
/// [`Trigger::Event`], the class whose content is never trusted.
///
/// The third fact is #306. Holding the name says a peer asked for the
/// role; the program says what it is. Either one alone was shown to be
/// available to any process in the session: the name by `RequestName`
/// on an activatable and therefore unowned name, and a program that has
/// not been given the name by the broker is just another peer.
///
/// `Schedule` is deliberately unreachable: nothing in Lisa is a
/// scheduler yet, and a ceiling that hands out a class no shipped peer
/// can legitimately hold would be a hole with no user. A scheduler
/// daemon arrives with its own name and its own arm of this function.
pub fn ceiling(facts: CallerFacts) -> Trigger {
    if facts.same_user && facts.owns_prompt_surface && facts.runs_a_prompt_program {
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
    // The third fact, read HERE rather than asked of agentd (#306,
    // ADR-0064). Only computed when the name says it might matter — a
    // peer that does not own a prompt surface is `Event` whatever it is
    // running, so there is no reason to read its exe. `exe_of_peer`
    // works now because this daemon left the user namespace that made
    // `/proc/<peer>/exe` EACCES (#161): the mount-class sandbox options
    // are gone from lisa-harnessd.service, and the guardrails that bound
    // the model live in the bus and the tools, not in harnessd's own
    // filesystem view (ADR-0064).
    let runs_a_prompt_program = owns_prompt_surface
        && lisa_peer::prompt_surface::is_prompt_surface(
            same_user,
            lisa_peer::exe_of_peer(&peer).ok().as_deref(),
        );
    CallerFacts {
        same_user,
        owns_prompt_surface,
        runs_a_prompt_program,
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

    /// All three facts are required and none is enough on its own. A
    /// caller from another uid holding the name is not this user's
    /// surface; this user's ordinary process is not a surface at all;
    /// and a peer that took the name while running something else is
    /// #306, which is the case this test gained.
    ///
    /// Exhaustive over the eight combinations rather than three
    /// hand-picked ones, because a ceiling is exactly the kind of
    /// function where the case nobody wrote down is the live one.
    #[test]
    fn only_this_users_prompt_surface_reaches_the_prompt_class() {
        for same_user in [true, false] {
            for owns_prompt_surface in [true, false] {
                for runs_a_prompt_program in [true, false] {
                    let facts = CallerFacts {
                        same_user,
                        owns_prompt_surface,
                        runs_a_prompt_program,
                    };
                    let expected = if same_user && owns_prompt_surface && runs_a_prompt_program {
                        Trigger::Prompt
                    } else {
                        Trigger::Event
                    };
                    assert_eq!(ceiling(facts), expected, "{facts:?}");
                }
            }
        }
    }

    /// #306, named so a failure says which defect came back. The
    /// squatter demonstrated on the reference device: an ordinary
    /// `python3` process of this user that called `RequestName` on
    /// `app.lisaos.Assistant` and got `1` — PRIMARY_OWNER — because the
    /// name is activatable and nobody held it.
    ///
    /// It has both facts the old ceiling asked for and it is still an
    /// event source, because it is not running a prompt-surface program.
    #[test]
    fn a_name_squatter_does_not_reach_the_prompt_class() {
        assert_eq!(
            ceiling(CallerFacts {
                same_user: true,
                owns_prompt_surface: true,
                runs_a_prompt_program: false,
            }),
            Trigger::Event,
            "taking `app.lisaos.Assistant` was enough to claim a human typed it"
        );
    }

    /// …and the positive control, without which the test above is
    /// satisfied by a ceiling that grants nothing to anybody. The real
    /// Assistant holds the name AND runs the program.
    #[test]
    fn the_real_prompt_surface_still_reaches_the_prompt_class() {
        assert_eq!(
            ceiling(CallerFacts {
                same_user: true,
                owns_prompt_surface: true,
                runs_a_prompt_program: true,
            }),
            Trigger::Prompt,
            "the Assistant cannot reach the class a person typing gets"
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
                runs_a_prompt_program: true,
            },
            CallerFacts {
                same_user: true,
                owns_prompt_surface: true,
                runs_a_prompt_program: false,
            },
            CallerFacts {
                same_user: true,
                owns_prompt_surface: false,
                runs_a_prompt_program: true,
            },
            CallerFacts {
                same_user: false,
                owns_prompt_surface: true,
                runs_a_prompt_program: true,
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
