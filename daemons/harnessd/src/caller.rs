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
//! # Why the answer is a bus name AND a program, and why the program
//! comes from another daemon
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
//! is the ADR-0033 property that matters.
//!
//! **They are not enough** (#306). A well-known name is owned by whoever
//! asks for it first, and `app.lisaos.Assistant` is D-Bus-activatable and
//! therefore not running most of the time. Demonstrated on the reference
//! device from an ordinary `python3` process: `RequestName` returned
//! `1` — PRIMARY_OWNER — for the name that buys `Trigger::Prompt`, and
//! `Prompt` buys the file and `run_command` family, write-tier bus
//! tools, `user` provenance that **agentd accepts** because
//! `/usr/bin/lisa-harnessd` is on its `MODEL_HOSTS`, and the owner-only
//! memory methods. No prompt injection and no model required: just
//! `RequestName` first.
//!
//! This file argued that the squat is "a far louder place to be
//! standing". It is not: a squatter can hold the name for the seconds a
//! `Run` takes and release it, and the next launch works normally.
//!
//! So the missing half is program identity, and it is fetched from
//! **agentd**, which is not in a user namespace and can read
//! `/proc/<peer>/exe` (`dev.lisaos.Agent1.IsPromptSurface`, and
//! `agentd/src/dbus.rs::PROMPT_SURFACE_PROGRAMS` for the allowlist and
//! its limits). Fetched, not trusted-by-assertion: agentd answers about
//! a **unique** bus name this daemon learned from the broker, and the
//! answer is a boolean about the kernel's view of that connection.
//!
//! # The honest limit that remains
//!
//! The Assistant is `Exec=/usr/bin/lisa-app assistant/lisa-assistant.js`,
//! and `lisa-app` ends in `exec gjs`. So the program check refuses every
//! compiled squatter and the demonstrated `python3` one, and does not
//! refuse a hostile GJS script. An Assistant with an executable of its
//! own is what closes that, and `PROMPT_SURFACE_PROGRAMS` already lists
//! the path it would have.
//!
//! And if agentd cannot be reached, the answer is `false` and the
//! ceiling is `Trigger::Event`. That is fail-closed and it is a real
//! availability coupling: a harness whose agentd is down loses the
//! prompt class rather than silently keeping it.

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
    /// agentd says the program behind this connection is one of its
    /// `PROMPT_SURFACE_PROGRAMS` — `/proc/<pid>/exe` through the
    /// broker's pidfd, read by a daemon that is not inside a user
    /// namespace and can therefore read it at all (#161, #306).
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
    // Only asked when the name says it might matter. agentd's answer is
    // an identity disclosure guarded by an exe check on us, so there is
    // no reason to make it about peers whose ceiling is `Event` either
    // way — and it keeps the cross-daemon round trip off the path of
    // every ordinary `Run`.
    let runs_a_prompt_program =
        owns_prompt_surface && agentd_says_prompt_surface(conn, caller_name).await;
    CallerFacts {
        same_user,
        owns_prompt_surface,
        runs_a_prompt_program,
    }
}

/// The broker-assigned unique name of this caller — the Owner anything
/// a LATER call can act on is stored under (ADR-0033, CLAUDE.md rule 6b).
///
/// Used by [`crate::conversation`] to key a conversation's accumulated
/// taint, so one peer's conversation can never read or extend another's.
/// `None` for a caller the transport could not place: a p2p connection
/// (no broker, no production caller here) or a header with no sender.
/// The consequence is a run with no conversation identity, whose ceiling
/// is `Trigger::Event` in any case.
pub async fn owner_of(
    conn: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> Option<String> {
    let peer = lisa_peer::resolve(conn, header).await.ok()?;
    match peer.id {
        lisa_peer::PeerId::Bus(name) => Some(name),
        lisa_peer::PeerId::Direct => None,
    }
}

/// How long agentd gets to answer before the caller is classed as an
/// event source.
///
/// Generous, because the answer costs one `GetConnectionCredentials` and
/// one `readlink`, and because timing out costs the person their file
/// tools. Bounded, because the alternative is a `Run` that never
/// returns.
const AGENTD_IDENTITY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// agentd's answer to "is `unique_name` running a prompt-surface
/// program".
///
/// This daemon cannot read `/proc/<peer>/exe` from inside its own user
/// namespace (#161) and agentd can, so the question goes over the bus to
/// a daemon in the initial namespace. What travels is a **unique** bus
/// name this process learned from the broker, not a claim; what comes
/// back is a boolean about the kernel's view of that connection.
///
/// Fails towards `false` on every error — an unreachable agentd, a
/// refusal, a reply of the wrong type. The consequence is
/// [`Trigger::Event`], which is the least trusted class, so the failure
/// direction is the safe one (and a loud one: the Assistant loses the
/// prompt class rather than quietly keeping it).
async fn agentd_says_prompt_surface(conn: &zbus::Connection, unique_name: &str) -> bool {
    agentd_says_prompt_surface_within(conn, unique_name, AGENTD_IDENTITY_TIMEOUT).await
}

/// The same question with the deadline named, so a test can watch the
/// deadline work without waiting out the shipped one.
///
/// Bounded at all because a peer that never answers is otherwise a `Run`
/// that never returns. On a session bus an absent agentd is an immediate
/// `NameHasNoOwner` from the broker, so this deadline is for the other
/// case: a live agentd that has stopped replying. Caught by
/// `an_unreachable_agentd_is_not_a_prompt_surface`, which hung forever
/// before the deadline existed.
async fn agentd_says_prompt_surface_within(
    conn: &zbus::Connection,
    unique_name: &str,
    deadline: std::time::Duration,
) -> bool {
    let args = (unique_name,);
    let call = conn.call_method(
        Some("dev.lisaos.Agent1"),
        "/dev/lisaos/Agent1",
        Some("dev.lisaos.Agent1"),
        "IsPromptSurface",
        &args,
    );
    let reply = match tokio::time::timeout(deadline, call).await {
        Ok(reply) => reply,
        Err(_) => {
            eprintln!(
                "harnessd: agentd did not answer within {deadline:?} about {unique_name}; \
                 this run is classed as an event, not a prompt"
            );
            return false;
        }
    };
    match reply {
        Ok(msg) => msg.body().deserialize::<bool>().unwrap_or(false),
        Err(e) => {
            // Worth a journal line: this is the difference between "the
            // Assistant is not trusted today" and "agentd is down", and
            // from the user's side both look like the assistant having
            // forgotten how to open a file.
            eprintln!(
                "harnessd: agentd could not identify {unique_name} ({e}); \
                 this run is classed as an event, not a prompt"
            );
            false
        }
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

    /// A stand-in for agentd, so the round trip that supplies the third
    /// fact can be exercised without a message broker.
    ///
    /// p2p carries method calls with a destination field the peer simply
    /// ignores, which is exactly what is wanted here: what is under test
    /// is that this daemon **asks** and believes the reply, not how the
    /// broker routes it.
    struct StubAgentd {
        answer: bool,
    }

    #[zbus::interface(name = "dev.lisaos.Agent1")]
    impl StubAgentd {
        fn is_prompt_surface(&self, _unique_name: String) -> bool {
            self.answer
        }
    }

    async fn agentd_stub(serve: Option<bool>) -> (zbus::Connection, zbus::Connection) {
        let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
        let guid = zbus::Guid::generate();
        let mut server = zbus::connection::Builder::unix_stream(server_sock)
            .server(guid)
            .unwrap()
            .p2p();
        if let Some(answer) = serve {
            server = server
                .serve_at("/dev/lisaos/Agent1", StubAgentd { answer })
                .unwrap();
        }
        let client = zbus::connection::Builder::unix_stream(client_sock).p2p();
        tokio::try_join!(server.build(), client.build()).unwrap()
    }

    /// The third fact is agentd's, and it is *fetched* — the whole point
    /// of #306 is that this daemon cannot compute it and must not
    /// substitute the name check for it.
    #[tokio::test]
    async fn the_program_fact_is_whatever_agentd_answers() {
        for answer in [true, false] {
            let (_server, client) = agentd_stub(Some(answer)).await;
            assert_eq!(
                agentd_says_prompt_surface(&client, ":1.412").await,
                answer,
                "harnessd did not take agentd's answer for {answer}"
            );
        }
    }

    /// And an agentd that never answers answers `false`, not `true`, and
    /// answers it *eventually* — this test hung forever before the
    /// deadline existed, which is what a `Run` would have done on a live
    /// agentd that had stopped replying. The consequence of the `false`
    /// is `Trigger::Event`, the least trusted class, so the failure
    /// costs capability rather than granting it.
    #[tokio::test]
    async fn an_unreachable_agentd_is_not_a_prompt_surface() {
        let (_server, client) = agentd_stub(None).await;
        assert!(
            !agentd_says_prompt_surface_within(
                &client,
                ":1.412",
                std::time::Duration::from_millis(200)
            )
            .await,
            "a peer nobody could identify was treated as a prompt surface"
        );
    }

    /// The shipped deadline is the one production uses, and a zero or
    /// absent one would turn the check into "agentd is never a source of
    /// truth" or "a `Run` may hang".
    #[test]
    fn the_shipped_deadline_is_bounded_and_not_zero() {
        assert!(!AGENTD_IDENTITY_TIMEOUT.is_zero());
        assert!(AGENTD_IDENTITY_TIMEOUT <= std::time::Duration::from_secs(30));
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
