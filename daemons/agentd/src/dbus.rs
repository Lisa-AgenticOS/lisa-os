//! D-Bus surface: `dev.lisaos.Agent1` (`docs/PLAN.md` §5.4).
//!
//! The Agent Bus as seen by shell surfaces and scripts. The overlay
//! backend (`dev.lisaos.Overlay1`, PLAN §5.7.1) becomes a client of this
//! interface at M5; `lisa tools/call/undo` CLI verbs ride it too.
//!
//! Shape (JSON payloads are strings — rich structures stay one
//! serialization, and `busctl`/scripts read them directly):
//!
//! ```text
//! ListTools() → (s tools_json)
//! Discover(s query) → (s tools_json)
//! RequestCall(s app_id, s tool, s args_json, a{sv} options)
//!     → (t call_id, s disposition, s detail_json)
//!     options: "actor" (s), "provenance" (as — the trigger chain;
//!              omitted/empty = unknown = escalates, rule 6)
//!     disposition: "executed" | "failed" | "confirm-chip" |
//!                  "confirm-modal" | "denied" | "refused"
//! Confirm(t call_id, b approve) → (s status, s detail_json)
//! Undo() → (s report_json)
//! signal ConfirmationRequested(t call_id, s spec_json)
//! signal RefusalReported(t call_id, s report_json)
//! ```
//!
//! Tested over zbus p2p (no bus daemon needed → runs on macOS dev
//! hosts); session-bus registration is used on real systems.

use crate::bus::{AgentBus, Answerer, BusError, CallRequest, ConsentRole, Outcome};
use crate::tier::{Claimant, Confirmation, Provenance};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

pub struct Agent1 {
    bus: Arc<AgentBus>,
}

impl Agent1 {
    pub fn new(bus: Arc<AgentBus>) -> Agent1 {
        Agent1 { bus }
    }
}

/// The well-known name of the desktop consent surface — the human's
/// confirmation dialog (`shell/consent`, issue #145).
///
/// Identity comes from the BROKER's answer to "who owns this name",
/// never from anything a caller asserts (ADR-0033).
///
/// This was `dev.lisaos.Overlay1` — the overlay BACKEND, which also
/// hosts the model. A call the overlay originated therefore came back to
/// `Confirm` from a peer that was both requester and surface, and the
/// model approved itself (#145). `bus.rs` now refuses that pairing
/// whatever the name says; pointing at a name a separate process owns is
/// the other half, so the dialog is a different program and not merely a
/// different role.
///
/// **And a name is still not an identity** (#289). This comment used to
/// go on to argue that `/proc/<pid>/exe` "would not help here", because
/// the surface runs under `/usr/bin/gjs`. That is a reason the exe check
/// is *weak*, not a reason to have no check at all — and the gap it left
/// was total: `session.conf` ships `<allow own="*"/>`, so the peer that
/// called `RequestName` first became the universal approver, and the
/// model's own host qualified by opening a second connection. See
/// [`CONSENT_SURFACE_PROGRAMS`].
const CONSENT_SURFACE: &str = "dev.lisaos.Consent1";

/// Programs that may be the human's consent dialog, as absolute
/// executable paths.
///
/// Read together with [`CONSENT_SURFACE`]: owning the name says a peer
/// *asked* to be the dialog, this says what it *is*. `bus.rs` requires
/// both, because either alone was demonstrated to be forgeable — the
/// name by `RequestName`, and a program that has not been given the name
/// by the broker is just another peer.
///
/// # The limit, stated rather than wished away
///
/// The consent surface is `Exec=/usr/bin/lisa-app consent/lisa-consentd.js`,
/// and `lisa-app` ends in `exec gjs -m "$found"`. So the kernel's answer
/// for the shipped dialog is `/usr/bin/gjs-console` — verified on the
/// reference device, where `dev.lisaos.Consent1` is owned by pid 18669
/// with that exe. An allowlist containing an *interpreter* authorises
/// every program that interpreter can run:
///
/// - it refuses `/usr/bin/lisa-harnessd`, which is the executed exploit
///   in #289 and the entire threat model of ADR-0029/0030 — the process
///   the model is running inside;
/// - it refuses every compiled squatter, and the `python3` one the #306
///   audit actually ran;
/// - it does **not** refuse a hostile GJS script, and a process that
///   `fork()`s and `exec`s `gjs` gets a new pid as well, so it also
///   steps around the same-process check.
///
/// What closes it is the dialog having an executable of its own —
/// `/usr/bin/lisa-consentd` — at which point passing this check means
/// running our code, and our code draws a window the owner has to
/// answer. That is packaging and `shell/consent` work, not agentd's, and
/// the entry below is already here waiting for it: `resolve_managers`
/// drops paths that do not exist, so shipping the binary is the whole of
/// the change on this side.
///
/// `comm` is not an option and never was: a process names itself
/// (ADR-0033 §3).
pub const CONSENT_SURFACE_PROGRAMS: [&str; 2] = [
    // The dedicated binary. Not installed today — listed so that the
    // day it is, it is trusted without a second edit here.
    "/usr/bin/lisa-consentd",
    // What the shipped dialog actually runs as. `/usr/bin/gjs` is a
    // symlink to `gjs-console`; `resolve_managers` canonicalises, so
    // either spelling matches the kernel's answer.
    "/usr/bin/gjs",
];

fn fdo_err(e: BusError) -> zbus::fdo::Error {
    match e {
        // #131: `NotYours` renders identically to `UnknownCall` on
        // purpose (bus.rs), but mapping them to DIFFERENT fdo error
        // NAMES handed the distinction straight back — a sweep read the
        // error name instead of the message and mapped which call ids
        // were live. The pair must be indistinguishable in both.
        BusError::UnknownCall(_) | BusError::NotYours(_) => {
            zbus::fdo::Error::InvalidArgs(e.to_string())
        }
        // Not an oracle: the caller parked this call, so it already
        // knows the id exists (#135).
        BusError::NeedsConsentSurface(..) => zbus::fdo::Error::AccessDenied(e.to_string()),
        other => zbus::fdo::Error::Failed(other.to_string()),
    }
}

/// Is this caller the human's dialog? Asked of the message bus.
///
/// Fails closed (#244). Every answer other than "this connection owns
/// the consent name" now costs the caller the right to approve its own
/// destructive call, including the answers that are really failures: an
/// unreachable broker and a name we could not parse both mean *no
/// dialog was involved*, and that is a refusal, not a fallback.
///
/// The one permissive answer is `NoBroker`, and no caller can reach it
/// on a session bus: it is decided by whether the *transport* gave us a
/// unique name (ADR-0033), which a message cannot influence.
async fn consent_role(conn: &zbus::Connection, caller: &lisa_peer::PeerId) -> ConsentRole {
    // p2p has no broker to ask and no desktop session to ask about.
    let lisa_peer::PeerId::Bus(caller_name) = caller else {
        return ConsentRole::of(false, None);
    };
    ConsentRole::of(
        true,
        consent_name_owner(conn)
            .await
            .as_deref()
            .map(|o| (caller_name.as_str(), o)),
    )
}

/// Who owns `dev.lisaos.Consent1` right now, as the broker sees it.
/// `None` for "nobody", and for every way of failing to ask.
async fn consent_name_owner(conn: &zbus::Connection) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(CONSENT_SURFACE).ok()?;
    // NameHasNoOwner: the consent surface is not running.
    dbus.get_name_owner(name)
        .await
        .ok()
        .map(|o| o.as_str().to_string())
}

/// What the park path must do about the consent surface before it emits
/// `ConfirmationRequested` (#244).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceStart {
    /// Nothing to start: a chip is the app's own inline affordance, and
    /// a p2p connection has no broker that could activate anything.
    NotNeeded,
    AlreadyRunning,
    Start,
}

/// `needed` is "a human has to see something": a modal confirmation, or
/// a refusal report (#251 — the refusal IS a dialog, it just has no
/// approving control. Silent refusal hides an attack, and if hostile
/// content just caused the model to attempt `rm -rf /`, that is
/// precisely the event the owner needs to see).
fn surface_start(brokered: bool, needed: bool, owned: bool) -> SurfaceStart {
    if !brokered || !needed {
        SurfaceStart::NotNeeded
    } else if owned {
        SurfaceStart::AlreadyRunning
    } else {
        SurfaceStart::Start
    }
}

/// Start the consent surface, if a human's dialog is about to be needed
/// and none is running (#244).
///
/// The surface shipped as a D-Bus *activatable* service and was never
/// once running on the reference device, because activation happens on a
/// method call and nothing on the machine ever made one: `GetNameOwner`
/// explicitly does not activate, and `ConfirmationRequested` is a signal,
/// which activates nothing at all. Parking a modal is the moment the
/// dialog is needed, so it is the moment to ask for it.
///
/// Activated rather than left to a session unit on purpose: a unit that
/// died at login leaves the 14:00 destructive call with no dialog and no
/// way back, whereas activation restores the surface the next time one
/// is needed. `StartServiceByName` also returns only once the service
/// owns the name, so the signal that follows cannot outrun its listener.
///
/// A failure here is not fatal and not a bypass: nobody owns the name,
/// so `confirm` refuses. It is logged because "the dialog would not
/// start" is the operator's problem to see.
async fn start_consent_surface(conn: &zbus::Connection, needed: bool) {
    let brokered = conn.unique_name().is_some();
    // Cheap short-circuit: do not ask the broker anything about a chip.
    if surface_start(brokered, needed, false) == SurfaceStart::NotNeeded {
        return;
    }
    let owner = consent_name_owner(conn).await;
    if surface_start(brokered, needed, owner.is_some()) != SurfaceStart::Start {
        // The name is taken. It may be the dialog; it may be a peer that
        // called `RequestName` first, which under `<allow own="*"/>` is
        // anybody, and which the real surface cannot then displace
        // because it asks with `BusNameOwnerFlags.NONE` (#289). agentd
        // cannot take a name away from a peer — what it can do is refuse
        // to treat that peer as the dialog (`confirm` does) and say out
        // loud that a modal has just parked into a session whose consent
        // name is held by something that is not a consent surface.
        //
        // Costs one credentials round trip, on the rare path where a
        // human's dialog is being asked for.
        if let Some(owner) = owner
            && !name_owner_is_a_consent_program(conn, &owner).await
        {
            eprintln!(
                "agentd: {CONSENT_SURFACE} is owned by {owner}, which is not a consent \
                 surface program — no dialog can answer this call and it can only be \
                 withdrawn (#289)"
            );
        }
        return;
    }
    let (Ok(dbus), Ok(name)) = (
        zbus::fdo::DBusProxy::new(conn).await,
        zbus::names::WellKnownName::try_from(CONSENT_SURFACE),
    ) else {
        return;
    };
    if let Err(e) = dbus.start_service_by_name(name, 0).await {
        eprintln!(
            "agentd: no consent surface and {CONSENT_SURFACE} would not start ({e}); \
             this call can now only be withdrawn"
        );
    }
}

/// Is the connection currently holding the consent name running a
/// consent-surface program?
///
/// Asked of the broker and the kernel about a **third** connection — the
/// name's owner, which at park time is not the peer calling us. Fails
/// towards `false`, so an unreadable owner produces the warning rather
/// than silence: "we could not tell" and "it is a squatter" both mean
/// nobody has been shown to be the dialog.
async fn name_owner_is_a_consent_program(conn: &zbus::Connection, owner: &str) -> bool {
    let Ok(peer) = lisa_peer::resolve_unique_name(conn, owner).await else {
        return false;
    };
    exe_is_consent_program(
        peer.is_same_user_as_us(),
        lisa_peer::exe_of_peer(&peer).ok().as_deref(),
    )
}

/// Programs that run a model in-process, as absolute executable paths.
///
/// Two consequences follow from being on this list, and they pull in
/// opposite directions on purpose:
///
/// 1. The program may assert `user` provenance (below), because it
///    derives that class from *its own* caller's transport identity
///    rather than believing a message — `lisa-harnessd` resolves the
///    trigger class from `GetConnectionCredentials` + `GetNameOwner`
///    (ADR-0036 §1, `harnessd/src/caller.rs`). Without this the
///    Assistant's every read was downgraded to `app:` and escalated to
///    a chip nobody could see: observed on the reference iMac on
///    2026-08-05 as 3,000-odd `agent.provenance_downgrade` entries.
/// 2. The program may **never approve a call it made**, at any tier
///    (`lisa_guard::judge_approval`). It is the process the model is
///    running inside, so its `Confirm` is the model's `Confirm`.
///
/// One list, because the two facts are the same fact: this is where the
/// probabilistic system lives. Granting the trust without taking the
/// approval right would be #145 again with a different process name.
///
/// # The honest limit
///
/// `lisa assist` runs the same loop inside `/usr/bin/lisa`, and the CLI
/// is NOT here — because the same executable is also `lisa do`, which
/// is a person typing at their own terminal and must keep answering its
/// own chip (ADR-0030 §1). Program identity cannot tell those apart, so
/// the CLI loop is not offered write-tier tools at all
/// (`bus_tools::read_tier_tools`, and #216's option 1 for that surface).
/// A `lisa-assistd` with its own executable is what would resolve it.
pub const MODEL_HOSTS: [&str; 1] = ["/usr/bin/lisa-harnessd"];

/// Programs that may be a surface a person types into, as absolute
/// executable paths (#306).
///
/// The companion of `harnessd`'s `PROMPT_SURFACES`, which lists the
/// well-known *names*. Same split as [`CONSENT_SURFACE_PROGRAMS`] and
/// the same reason: the name says a peer asked for the role, this says
/// what it is running. `app.lisaos.Assistant` is D-Bus-activatable and
/// therefore usually unowned, so under `<allow own="*"/>` the name alone
/// went to whoever called `RequestName` first — demonstrated on the
/// reference device with an ordinary `python3` process (#306).
///
/// # Why the list lives here and the names live there
///
/// `lisa-harnessd.service` is a per-user unit carrying `ProtectHome`,
/// `ProtectSystem=strict` and `PrivateDevices`, which a user manager can
/// only deliver through an implicit private user namespace — and from
/// inside one, `readlink /proc/<peer>/exe` is EACCES for every peer
/// outside it (#161). Re-verified while writing this, with a positive
/// control: the same command in an unsandboxed transient user unit
/// resolves `/usr/bin/lisa-agentd` and `/usr/bin/gjs-console`, and
/// inside the sandboxed one both are "Permission denied".
///
/// So harnessd cannot answer this question about anybody, and agentd —
/// which runs in the initial user namespace (`uid_map` `0 0 4294967295`,
/// verified on the device) — can. Hence `IsPromptSurface`.
///
/// # The same limit as the consent list
///
/// The Assistant is `Exec=/usr/bin/lisa-app assistant/lisa-assistant.js
/// --gapplication-service`, so the kernel's answer for it is
/// `/usr/bin/gjs-console`. This refuses every compiled squatter and the
/// `python3` one that was actually demonstrated; it does not refuse a
/// hostile GJS script. `/usr/bin/lisa-assistant` is listed for the day
/// the Assistant has an executable of its own, which is what closes it.
pub const PROMPT_SURFACE_PROGRAMS: [&str; 2] = ["/usr/bin/lisa-assistant", "/usr/bin/gjs"];

/// Does this executable belong to a surface a person types into?
///
/// Fails CLOSED: an unreadable peer is not a prompt surface, and the
/// cost of being wrong is that a run is classed `Trigger::Event`, whose
/// content is never trusted — the safe direction.
fn exe_is_prompt_program(same_user: bool, exe: Option<&std::path::Path>) -> bool {
    let programs: Vec<std::path::PathBuf> = PROMPT_SURFACE_PROGRAMS
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    // A model host is never a prompt surface, whatever the allowlists
    // say. Belt and braces over the disjointness test below: the two
    // lists being edited apart is exactly the drift that would turn the
    // loop into its own "human typed this" (#306's laundering chain).
    if exe_hosts_a_model(same_user, exe) {
        return false;
    }
    lisa_peer::manager::may_manage(
        same_user,
        exe,
        &lisa_peer::manager::resolve_managers(&programs),
    )
    .is_ok()
}

/// Is this caller one of Lisa's own programs?
///
/// Program identity from `/proc/<pid>/exe` via the peer's credentials —
/// never `comm`, which a process can rename, and never anything the
/// message asserts (ADR-0033). Only these may claim `user` provenance:
/// the CLI and Settings are the surfaces a human drives directly, and
/// the harness is the one program that computes the class from its own
/// caller's transport identity. "A human typed this" is the one tag
/// that buys trust rather than costing it.
///
/// Fails CLOSED: an unreadable peer is not a Lisa program. The cost of
/// being wrong here is a downgrade to `app:` provenance, which asks for
/// confirmation more often — the safe direction.
async fn caller_is_lisa_program(
    conn: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> bool {
    let Ok(peer) = lisa_peer::resolve(conn, header).await else {
        return false;
    };
    let same_user = peer.is_same_user_as_us();
    let exe = lisa_peer::exe_of_peer(&peer).ok();
    exe_is_lisa_program(
        same_user,
        exe.as_deref(),
        &lisa_peer::manager::default_managers(),
    ) || exe_hosts_a_model(same_user, exe.as_deref())
}

/// Does this caller run a model in-process?
///
/// Same authority as everything else here: the kernel's answer for
/// `/proc/<pid>/exe`, symlink-resolved on both sides (#215 — the
/// allowlist compared as written never matched a payload behind a
/// `current` symlink, and the failure looked like a security decision).
///
/// Fails CLOSED **towards being a model host is false**, which is the
/// permissive direction for [`crate::bus::AgentBus::confirm`] — so it
/// is deliberately paired with `caller_is_lisa_program` above, where
/// the same unreadable peer fails closed towards *less* trust. A caller
/// we cannot identify therefore gets neither the provenance nor a
/// silent path: its `user` claim is downgraded, everything it asks for
/// escalates, and the escalated tier is a modal only the dialog can
/// answer.
fn exe_hosts_a_model(same_user: bool, exe: Option<&std::path::Path>) -> bool {
    if !same_user {
        return false;
    }
    let Some(exe) = exe else {
        return false;
    };
    let hosts: Vec<std::path::PathBuf> = MODEL_HOSTS.iter().map(std::path::PathBuf::from).collect();
    lisa_peer::manager::may_manage(
        same_user,
        Some(exe),
        &lisa_peer::manager::resolve_managers(&hosts),
    )
    .is_ok()
}

/// The transport's answer to "does this caller host a model", for the
/// call about to be parked.
async fn caller_hosts_a_model(conn: &zbus::Connection, header: &zbus::message::Header<'_>) -> bool {
    let Ok(peer) = lisa_peer::resolve(conn, header).await else {
        return false;
    };
    exe_hosts_a_model(
        peer.is_same_user_as_us(),
        lisa_peer::exe_of_peer(&peer).ok().as_deref(),
    )
}

/// Is this caller running one of the consent-surface programs?
///
/// The same authority as everything else here: the kernel's answer for
/// `/proc/<pid>/exe`, through the broker's pidfd, resolved on both sides.
/// Separated from a live connection so the decision has a test (the
/// split #132 and #215 both exist because of).
///
/// Fails CLOSED, and the closed direction is the strict one: a caller we
/// cannot place is not the dialog, so it cannot approve anything it did
/// not park. The cost of being wrong is that a modal cannot be answered
/// until a real dialog appears, which is #244's behaviour and not a
/// bypass.
fn exe_is_consent_program(same_user: bool, exe: Option<&std::path::Path>) -> bool {
    let programs: Vec<std::path::PathBuf> = CONSENT_SURFACE_PROGRAMS
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    lisa_peer::manager::may_manage(
        same_user,
        exe,
        &lisa_peer::manager::resolve_managers(&programs),
    )
    .is_ok()
}

/// Everything the transport can say about the peer answering a
/// confirmation: its program, and the process behind it.
///
/// One `resolve` for both, because they must describe the *same* pinned
/// process — asking twice would leave a window in which the answers came
/// from two different peers (the #136 lesson, one level up).
async fn answerer_identity(
    conn: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> (bool, Option<lisa_peer::Process>) {
    let Ok(peer) = lisa_peer::resolve(conn, header).await else {
        return (false, None);
    };
    let is_program = exe_is_consent_program(
        peer.is_same_user_as_us(),
        lisa_peer::exe_of_peer(&peer).ok().as_deref(),
    );
    (is_program, lisa_peer::Process::of_peer(&peer).ok())
}

/// The process behind the caller of `RequestCall`, pinned so the pid it
/// reports cannot be recycled while the call sits parked (#289).
async fn caller_process(
    conn: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> Option<lisa_peer::Process> {
    let peer = lisa_peer::resolve(conn, header).await.ok()?;
    lisa_peer::Process::of_peer(&peer).ok()
}

/// Who is calling, for the Ledger — from the transport, never from the
/// message (#217, ADR-0033).
///
/// The message carries an `actor` string and an `app_id`, and NEITHER
/// names the sender: `actor` is a self-description ("assistant"), and
/// `app_id` is the app being called. Both were used here, which is how
/// a provenance downgrade came to be attributed to its target.
async fn caller_claimant(conn: &zbus::Connection, header: &zbus::message::Header<'_>) -> Claimant {
    let Ok(peer) = lisa_peer::resolve(conn, header).await else {
        return Claimant::from(Claimant::UNKNOWN);
    };
    #[cfg(unix)]
    let exe = lisa_peer::exe_of_peer(&peer).ok();
    #[cfg(not(unix))]
    let exe: Option<std::path::PathBuf> = None;
    claimant_label(exe.as_deref(), &peer.id)
}

/// The naming rule, separated so it can be tested without a live peer.
///
/// The executable first, because it is the same string across restarts
/// and is therefore the one worth grepping the Ledger for; the unique
/// bus name second, because it at least distinguishes one connection
/// from another within a session; and `host:unknown` last, shared by
/// every peer we could not place at all.
fn claimant_label(exe: Option<&std::path::Path>, peer: &lisa_peer::PeerId) -> Claimant {
    if let Some(exe) = exe {
        // One producer for `host:<exe>`, so the Ledger and the portal
        // spell an unattributed caller the same way.
        return Claimant::from(lisa_peer::app::AppIdentity::unattributed(exe).app_id);
    }
    match peer {
        lisa_peer::PeerId::Bus(name) => Claimant::from(format!("peer:{name}")),
        _ => Claimant::from(Claimant::UNKNOWN),
    }
}

/// The decision itself, separated so it can be tested.
///
/// `caller_is_lisa_program` needs a live `Connection` and a real peer,
/// which a unit test cannot cheaply build — so the only part that
/// decides anything had no test, which is how #215 shipped.
///
/// `configured` is the shipped allowlist as WRITTEN; the resolution is
/// this function's job.
fn exe_is_lisa_program(
    same_user: bool,
    exe: Option<&std::path::Path>,
    configured: &[std::path::PathBuf],
) -> bool {
    // RESOLVED, not compared verbatim (#215). `/proc/<pid>/exe` reports
    // the file the kernel actually executed, with every symlink already
    // followed — and the channel CLI ships behind a `current` symlink
    // that `lisa apps update` moves, so the kernel says
    // `…/runtime/versions/20260803.75/bin/lisa` where the allowlist
    // says `…/runtime/current/bin/lisa`. Compared as written, those
    // never match, and EVERY call from the channel CLI was
    // provenance-downgraded to `app:` — a security decision that was
    // really a stale path. `resolve_managers` exists for exactly this
    // and was not being called.
    let managers = lisa_peer::manager::resolve_managers(configured);
    lisa_peer::manager::may_manage(same_user, exe, &managers).is_ok()
}

fn disposition_of(confirmation: Confirmation) -> &'static str {
    match confirmation {
        Confirmation::Silent => "executed", // Silent calls return as executed.
        Confirmation::Chip => "confirm-chip",
        Confirmation::Modal => "confirm-modal",
    }
}

fn outcome_reply(outcome: &Outcome) -> (u64, String, String) {
    match outcome {
        Outcome::Executed {
            call_id,
            ledger_ref,
            result,
        } => (
            *call_id,
            "executed".into(),
            serde_json::json!({"result": result, "ledger_ref": ledger_ref}).to_string(),
        ),
        Outcome::Failed {
            call_id,
            ledger_ref,
            error,
            provenance,
        } => (
            *call_id,
            "failed".into(),
            // `provenance` rides the failed reply for the same reason it
            // rides the executed one: the error text quotes what the
            // tool saw, and the consumer taints on the tag (#313).
            serde_json::json!({
                "error": error,
                "ledger_ref": ledger_ref,
                "provenance": provenance,
            })
            .to_string(),
        ),
        Outcome::AwaitingConfirmation {
            call_id,
            confirmation,
            spec,
            ..
        } => (
            *call_id,
            disposition_of(*confirmation).into(),
            spec.to_string(),
        ),
        Outcome::Denied { call_id, reason } => (
            *call_id,
            "denied".into(),
            serde_json::json!({"reason": reason}).to_string(),
        ),
        // Its own disposition, not `denied` (#251). A caller that cannot
        // tell "a person said no" from "this will never be available"
        // will retry the second one forever, and the retry loop is what
        // turns a refusal into a flood.
        Outcome::Refused {
            call_id,
            rule,
            reason,
            ..
        } => (
            *call_id,
            "refused".into(),
            serde_json::json!({"rule": rule, "reason": reason}).to_string(),
        ),
    }
}

#[zbus::interface(name = "dev.lisaos.Agent1")]
impl Agent1 {
    /// Liveness probe.
    fn ping(&self) -> String {
        format!("lisa-agentd {}", env!("CARGO_PKG_VERSION"))
    }

    /// All registered tools as a JSON array
    /// (`[{app_id, name, tier, description, undoable}]`).
    fn list_tools(&self) -> String {
        self.bus.list_tools().to_string()
    }

    /// Discovery: rank tools against a natural-language query.
    fn discover(&self, query: String) -> String {
        self.bus.discover(&query).to_string()
    }

    /// Request a tool call. Read-tier calls with a fully trusted chain
    /// execute immediately; everything else parks and emits
    /// ConfirmationRequested (answer via Confirm). Every path is
    /// ledgered before anything happens.
    // Six of these are the RequestCall wire signature (Appendix B) and
    // two are zbus injections; the arity is the interface's, not ours.
    #[allow(clippy::too_many_arguments)]
    async fn request_call(
        &self,
        app_id: String,
        tool: String,
        args_json: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(u64, String, String)> {
        let args: serde_json::Value = serde_json::from_str(&args_json)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(format!("args_json: {e}")))?;
        let actor = options
            .get("actor")
            .and_then(|v| v.downcast_ref::<&str>().ok().map(str::to_string))
            .unwrap_or_else(|| "host".to_string());
        let asserted: Vec<Provenance> = options
            .get("provenance")
            .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
            .unwrap_or_default()
            .iter()
            .map(|s| Provenance::parse(s))
            .collect();
        // The chain arrived in a message; whether this caller may claim
        // `user` is decided by the transport (ADR-0033, #55). Only a
        // known Lisa program may say "a human typed this" — that is the
        // one tag which BUYS trust, and it was previously free to
        // anything on the session bus.
        let trusted = caller_is_lisa_program(conn, &header).await;
        // The CLAIMANT, not the callee (#217). `app_id` is the app whose
        // tool is being called — the target — and passing it here made
        // the downgrade record name the victim as the peer that claimed
        // to be human. `Claimant` is a newtype so the two can no longer
        // be swapped by writing the wrong variable.
        let claimant = caller_claimant(conn, &header).await;
        let verified = crate::tier::verify_chain(asserted, trusted, &claimant);
        if verified.downgraded {
            // Recorded, not refused: refusing would break any app that
            // simply tagged its input wrongly. A peer repeatedly
            // claiming to be the human is the signature worth being
            // able to grep the Ledger for afterwards — which is why
            // this goes to the Ledger and not only to stderr, where it
            // lived until #55's audit: a journal line nobody queries is
            // not an audit trail.
            eprintln!(
                "agentd: {claimant} asserted user provenance without being a Lisa \
                 program, calling {app_id}/{tool}; downgraded to app:{claimant}"
            );
            self.bus
                .ledger_provenance_downgrade(&claimant, &app_id, &tool);
        }
        let chain = verified.chain;
        // Recorded with the call because it decides something LATER: a
        // model host may not answer its own parked confirmation, and by
        // then the answerer may be a different peer entirely (#216).
        let hosts_a_model = caller_hosts_a_model(conn, &header).await;
        // Likewise, and for the harder half of the same question: the
        // peer that answers may be a different CONNECTION of this same
        // process, which every name-based check reads as somebody else
        // (#289). Pinned by the broker's pidfd and held for as long as
        // the call is parked, so the pid cannot be recycled underneath
        // the comparison.
        let requester_process = caller_process(conn, &header).await;

        let outcome = self
            .bus
            .request(CallRequest {
                actor,
                app_id,
                tool,
                args,
                chain,
                // Transport-assigned, not message-claimed (ADR-0033).
                // The CONNECTION decides whether the header's sender is
                // trustworthy — on p2p it is not (#132).
                caller: lisa_peer::PeerId::of(conn, &header)
                    .map_err(|e| zbus::fdo::Error::AccessDenied(e.to_string()))?,
                requester_hosts_a_model: hosts_a_model,
                requester_process,
            })
            .map_err(fdo_err)?;
        let reply = outcome_reply(&outcome);
        match &outcome {
            Outcome::AwaitingConfirmation {
                call_id,
                confirmation,
                spec,
                ..
            } => {
                // Before the signal, never after: the surface subscribes
                // to it as it starts, and a signal emitted into an empty
                // session is a dialog that never appears (#244).
                //
                // A chip parked by a MODEL HOST needs the dialog just as
                // much as a modal does, because nothing else may answer
                // it (#216). Leaving it out was the difference between a
                // guardrail and a hang: the loop's write would park with
                // no surface to release it and no way for the person to
                // know it had.
                start_consent_surface(conn, *confirmation == Confirmation::Modal || hosts_a_model)
                    .await;
                let _ = Self::confirmation_requested(&emitter, *call_id, spec.to_string()).await;
            }
            // A refusal is REPORTED, never confirmed (#251). It goes out
            // on its own signal, so a surface cannot mistake it for
            // something to draw buttons on, and there is no parked call
            // behind it for anyone to approve.
            Outcome::Refused {
                call_id, report, ..
            } => {
                start_consent_surface(conn, true).await;
                let _ = Self::refusal_reported(&emitter, *call_id, report.to_string()).await;
            }
            _ => {}
        }
        Ok(reply)
    }

    /// Answer a pending confirmation. Status: "executed" | "failed" |
    /// "denied".
    async fn confirm(
        &self,
        call_id: u64,
        approve: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<(String, String)> {
        // The caller's identity comes from the connection (#132), and
        // its AUTHORITY over this call from what the broker says about
        // the consent surface (#135) — never from the message.
        let caller = lisa_peer::PeerId::of(conn, &header)
            .map_err(|e| zbus::fdo::Error::AccessDenied(e.to_string()))?;
        // Owning the consent name is a claim staked with `RequestName`;
        // the program and the process behind the connection are what
        // the kernel says. All three, because #289 is what one of them
        // costs (CLAUDE.md 6b).
        let (is_consent_program, process) = answerer_identity(conn, &header).await;
        let answerer = Answerer {
            consent: consent_role(conn, &caller).await,
            peer: caller,
            is_consent_program,
            process,
        };
        let outcome = self
            .bus
            .confirm(call_id, approve, &answerer)
            .map_err(fdo_err)?;
        let (_, status, detail) = outcome_reply(&outcome);
        Ok((status, detail))
    }

    /// Is the connection `unique_name` running a prompt-surface program?
    ///
    /// The one question `lisa-harnessd` cannot answer for itself. Its
    /// unit is sandboxed into a private user namespace, from which
    /// `/proc/<peer>/exe` is EACCES for every peer (#161); agentd is in
    /// the initial namespace and can read it. Without this, harnessd's
    /// "a human typed this" ceiling rested entirely on who owned
    /// `app.lisaos.Assistant`, and that name is activatable, ordinarily
    /// unowned, and grantable to anybody under `<allow own="*"/>`
    /// (#306).
    ///
    /// # This is an identity oracle, and the guard is the whole of its
    /// safety
    ///
    /// It tells its caller something about a third connection, so it is
    /// restricted to callers that are themselves model hosts by
    /// `/proc/<pid>/exe` — the same list, and the same authority, that
    /// decides who may claim `user` provenance. Everyone else gets
    /// `AccessDenied` with a fixed string that says nothing about the
    /// name they asked about, so it cannot be used to probe which
    /// connections exist.
    ///
    /// It answers only about **unique** names (`:1.412`).
    /// `lisa_peer::resolve_unique_name` refuses anything else, because a
    /// well-known name's owner can change between the question and the
    /// answer and the point of the exercise is to stop reasoning about
    /// names.
    async fn is_prompt_surface(
        &self,
        unique_name: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<bool> {
        if !caller_hosts_a_model(conn, &header).await {
            // One fixed string for every refusal: "you may not ask" must
            // not be distinguishable from "there is no such connection".
            return Err(zbus::fdo::Error::AccessDenied(
                "only a program that hosts a model may ask about a peer's identity".into(),
            ));
        }
        let Ok(peer) = lisa_peer::resolve_unique_name(conn, &unique_name).await else {
            // Fails closed, and says so as a plain `false` rather than
            // an error: "we could not place it" and "it is not a prompt
            // surface" have the same consequence, and the caller must
            // not be able to tell a live unique name from a dead one.
            return Ok(false);
        };
        Ok(exe_is_prompt_program(
            peer.is_same_user_as_us(),
            lisa_peer::exe_of_peer(&peer).ok().as_deref(),
        ))
    }

    /// Revert the caller's last agent action via its journaled
    /// compensation.
    ///
    /// The identity comes from the transport, never from the message
    /// (ADR-0033). This method used to take no arguments at all and
    /// hardcode the actor `"host"`, so any peer on the session bus could
    /// revert any other peer's action and the Ledger would attribute it
    /// to "host" (#94).
    fn undo(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        let caller = lisa_peer::PeerId::of(conn, &header)
            .map_err(|e| zbus::fdo::Error::AccessDenied(e.to_string()))?;
        let report = self.bus.undo("host", &caller).map_err(fdo_err)?;
        serde_json::to_string(&report)
            .map_err(|e| zbus::fdo::Error::Failed(format!("serializing report: {e}")))
    }

    /// Emitted when a call parks for confirmation; `spec_json` carries
    /// the typed-diff material (tool, args, tiers, escalation, chain).
    #[zbus(signal)]
    async fn confirmation_requested(
        emitter: &SignalEmitter<'_>,
        call_id: u64,
        spec_json: String,
    ) -> zbus::Result<()>;

    /// Emitted when a call is REFUSED (#251). There is no parked call
    /// behind it and no `Confirm` that can answer it: `report_json` is
    /// for a dialog that reports, with one button and no approving
    /// control. It carries no arguments and no command, so nothing
    /// downstream can rebuild the refused action from it.
    #[zbus(signal)]
    async fn refusal_reported(
        emitter: &SignalEmitter<'_>,
        call_id: u64,
        report_json: String,
    ) -> zbus::Result<()>;
}

/// Register on the session bus (real systems; tests use p2p).
pub async fn serve(bus: Arc<AgentBus>) -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .name("dev.lisaos.Agent1")?
        .serve_at("/dev/lisaos/Agent1", Agent1::new(bus))?
        .build()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #131. bus.rs makes `NotYours` and `UnknownCall` render as
    /// the same *string* so a sweep cannot map which call ids are live —
    /// and then this function handed the distinction straight back by
    /// mapping them to different D-Bus error NAMES. A client reads
    /// `org.freedesktop.DBus.Error.InvalidArgs` vs `.Failed` without
    /// ever looking at the message, so the oracle was still open.
    ///
    /// Both halves have to match: name and message.
    #[test]
    fn the_two_refusals_are_indistinguishable_over_the_wire() {
        for id in [1u64, 7, 4242] {
            let unknown = fdo_err(BusError::UnknownCall(id));
            let not_yours = fdo_err(BusError::NotYours(id));
            assert_eq!(
                std::mem::discriminant(&unknown),
                std::mem::discriminant(&not_yours),
                "different fdo error names for id {id}: {unknown:?} vs {not_yours:?}"
            );
            assert_eq!(unknown.to_string(), not_yours.to_string());
        }
    }

    /// Issue #215. The channel CLI lives behind a `current` symlink
    /// (`…/runtime/current/bin/lisa` → `…/runtime/versions/<ver>/bin
    /// /lisa`, confirmed on the reference machine), and
    /// `/proc/<pid>/exe` reports the resolved file. Comparing the
    /// allowlist as written therefore matched nothing, and every call
    /// from the CLI a person typed into was downgraded to `app:`
    /// provenance — a stale path wearing a security decision's clothes.
    #[test]
    fn a_manager_behind_a_moving_symlink_is_still_a_lisa_program() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("versions/20260803.75/bin");
        std::fs::create_dir_all(&real).unwrap();
        let binary = real.join("lisa");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        let binary = binary.canonicalize().unwrap();

        let channel = dir.path().join("current");
        std::os::unix::fs::symlink(dir.path().join("versions/20260803.75"), &channel).unwrap();
        let configured = vec![channel.join("bin/lisa")];

        // What the kernel reports is the RESOLVED file; what the
        // allowlist names is the symlinked one. They are one program.
        assert!(
            exe_is_lisa_program(true, Some(&binary), &configured),
            "the channel CLI was not recognised as a Lisa program"
        );
    }

    /// Resolving must not turn the allowlist into a wildcard: the
    /// refusals it is there to make have to survive it.
    #[test]
    fn resolving_the_allowlist_still_refuses_everyone_else() {
        let dir = tempfile::tempdir().unwrap();
        let ours = dir.path().join("lisa");
        let theirs = dir.path().join("totally-normal-app");
        for p in [&ours, &theirs] {
            std::fs::write(p, b"#!/bin/sh\n").unwrap();
        }
        let configured = vec![ours.clone()];
        let ours = ours.canonicalize().unwrap();
        let theirs = theirs.canonicalize().unwrap();

        assert!(exe_is_lisa_program(true, Some(&ours), &configured));
        assert!(
            !exe_is_lisa_program(true, Some(&theirs), &configured),
            "an ordinary program was accepted as a Lisa program"
        );
        assert!(
            !exe_is_lisa_program(false, Some(&ours), &configured),
            "another user's process was accepted"
        );
        assert!(
            !exe_is_lisa_program(true, None, &configured),
            "a caller with no readable exe was accepted"
        );
        assert!(
            !exe_is_lisa_program(true, Some(&ours), &[]),
            "an empty allowlist authorised somebody"
        );
    }

    /// Issue #244, the first half: the consent surface was packaged,
    /// activatable and never once running, because nothing on the
    /// machine ever called a method on `dev.lisaos.Consent1`.
    /// `GetNameOwner` deliberately does not activate, and
    /// `ConfirmationRequested` is a signal — signals never activate
    /// anything. So a name that only agentd cares about had nobody to
    /// start it, and every confirmation landed in the no-surface branch.
    ///
    /// Parking a modal is the one moment a human's dialog is needed, so
    /// that is where the surface gets started.
    #[test]
    fn a_parking_modal_starts_the_consent_surface_and_nothing_else_does() {
        use crate::tier::Confirmation;
        let needed = |c: Confirmation| c == Confirmation::Modal;
        // A modal on a message bus with nobody owning the name: start it.
        assert_eq!(
            surface_start(true, needed(Confirmation::Modal), false),
            SurfaceStart::Start
        );
        // Already up: activating again would be a second dialog.
        assert_eq!(
            surface_start(true, needed(Confirmation::Modal), true),
            SurfaceStart::AlreadyRunning
        );
        // A chip is the app's own inline affordance — it is not the
        // human's dialog and must not conjure one.
        assert_eq!(
            surface_start(true, needed(Confirmation::Chip), false),
            SurfaceStart::NotNeeded
        );
        // p2p (agentd's own test transport): no broker, so there is
        // nothing to ask and nothing that could be activated.
        assert_eq!(
            surface_start(false, needed(Confirmation::Modal), false),
            SurfaceStart::NotNeeded
        );
        // A refusal needs the surface too (#251): it is a dialog that
        // reports. Silent refusal hides an attack from the one person
        // who needs to know it happened.
        assert_eq!(surface_start(true, true, false), SurfaceStart::Start);
    }

    /// Issue #244. `consent_role` failed closed *towards the permissive
    /// branch*: every error — an unreachable broker, an unparsable name
    /// — resolved to "no surface exists, so the requester answers its
    /// own call". Only one thing may mean that now, and it is not an
    /// error: a connection with no broker at all.
    #[test]
    fn only_an_unbrokered_transport_may_mean_there_is_no_surface() {
        assert_eq!(
            ConsentRole::of(false, None),
            ConsentRole::NoBroker,
            "p2p has no broker to ask"
        );
        assert_eq!(
            ConsentRole::of(true, None),
            ConsentRole::Missing,
            "a bus with nobody owning the name is a MISSING surface, not an absent one"
        );
        assert_eq!(
            ConsentRole::of(true, Some((":1.10", ":1.10"))),
            ConsentRole::Surface
        );
        assert_eq!(
            ConsentRole::of(true, Some((":1.10", ":1.99"))),
            ConsentRole::Other
        );
    }

    /// The consent-surface refusal is deliberately NOT disguised: the
    /// caller parked the call, so telling it "the human answers this
    /// one" reveals nothing it did not already know, and a silent
    /// refusal there reads as a bug (#135).
    ///
    /// The strings come from `judge_approval` rather than being typed
    /// here, so a reason that stopped saying where to go would fail
    /// this test instead of quietly passing it.
    #[test]
    fn the_consent_surface_refusal_is_its_own_error() {
        let both = [
            lisa_guard::Approval {
                approve: true,
                is_requester: true,
                owns_consent_name: false,
                answerer_is_consent_program: false,
                answerer_is_requesters_process: true,
                requester_hosts_a_model: true,
                class: lisa_guard::ConfirmClass::Chip,
                brokered: true,
            },
            lisa_guard::Approval {
                approve: true,
                is_requester: true,
                owns_consent_name: false,
                answerer_is_consent_program: false,
                answerer_is_requesters_process: true,
                requester_hosts_a_model: false,
                class: lisa_guard::ConfirmClass::Modal,
                brokered: true,
            },
        ];
        for approval in both {
            let lisa_guard::ApprovalVerdict::Refused { rule, reason } =
                lisa_guard::judge_approval(&approval)
            else {
                panic!("{approval:?} should have been refused");
            };
            let refusal = fdo_err(BusError::NeedsConsentSurface(7, rule, reason));
            assert!(matches!(refusal, zbus::fdo::Error::AccessDenied(_)));
            let text = refusal.to_string();
            assert!(
                text.contains("consent surface"),
                "the refusal must say where to go: {text}"
            );
            // …and it must name the rule, or `lisa guard list` cannot be
            // the place a person looks it up (ADR-0030 §5).
            assert!(
                text.contains(rule),
                "the refusal must name `{rule}`: {text}"
            );
        }
    }

    /// The list that decides both halves of #216: who may claim a human
    /// typed something, and who may never approve their own call.
    #[test]
    fn only_absolute_lisa_paths_are_model_hosts() {
        for host in MODEL_HOSTS {
            assert!(
                std::path::Path::new(host).is_absolute(),
                "`{host}` is not an absolute path, so `/proc/<pid>/exe` can never equal it"
            );
            assert!(
                host.contains("lisa"),
                "adding `{host}` grants user provenance — it is a decision, not a path fix"
            );
        }
    }

    /// The park-time squat warning fails closed (#289). "We could not
    /// read the name owner" and "the name owner is a squatter" have the
    /// same consequence — nobody has been shown to be the dialog — so
    /// both must produce the warning rather than silence.
    ///
    /// p2p is the transport with no broker to ask, which is exactly that
    /// state; a version of this that answered `true` there would go
    /// quiet on every session it could not inspect.
    #[tokio::test]
    async fn an_owner_we_cannot_read_is_not_treated_as_the_dialog() {
        let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
        let guid = zbus::Guid::generate();
        let server = zbus::connection::Builder::unix_stream(server_sock)
            .server(guid)
            .unwrap()
            .p2p()
            .build();
        let client = zbus::connection::Builder::unix_stream(client_sock)
            .p2p()
            .build();
        let (_server, client) = tokio::try_join!(server, client).unwrap();

        assert!(
            !name_owner_is_a_consent_program(&client, ":1.279").await,
            "an owner nobody could read was reported as the consent surface"
        );
    }

    /// The consent-surface allowlist is a decision about who may say
    /// yes on a person's behalf, so it gets the same invariants as
    /// `MODEL_HOSTS` — and one more that matters far more than either.
    #[test]
    fn a_consent_surface_program_is_never_also_a_model_host() {
        for program in CONSENT_SURFACE_PROGRAMS {
            assert!(
                std::path::Path::new(program).is_absolute(),
                "`{program}` is not absolute, so `/proc/<pid>/exe` can never equal it"
            );
            // The whole architecture of #145 and #216 in one assertion:
            // the process that runs the model and the process that
            // draws the human's dialog must not be the same program.
            // Adding an entry here that is also a model host would
            // re-open self-approval through the front door, with the
            // Ledger recording a consent surface.
            assert!(
                !MODEL_HOSTS.contains(&program),
                "`{program}` is both a model host and a consent surface"
            );
            // …and it cannot be one of the management binaries either,
            // because those are the surfaces `lisa do` runs in and the
            // person answering their own chip must stay distinguishable
            // from the dialog answering for them.
            assert!(
                !lisa_peer::manager::DEFAULT_MANAGERS.contains(&program),
                "`{program}` is both a management program and a consent surface"
            );
        }
        let mut sorted = CONSENT_SURFACE_PROGRAMS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            CONSENT_SURFACE_PROGRAMS.len(),
            "duplicate consent surface program"
        );
    }

    /// #306's laundering chain in one assertion. A model host that were
    /// also a prompt surface could class its own run as "a human typed
    /// this", and agentd would then accept the `user` provenance it
    /// derived from itself — a loop vouching for its own input.
    #[test]
    fn a_prompt_surface_program_is_never_a_model_host() {
        for program in PROMPT_SURFACE_PROGRAMS {
            assert!(
                std::path::Path::new(program).is_absolute(),
                "`{program}` is not absolute, so `/proc/<pid>/exe` can never equal it"
            );
            assert!(
                !MODEL_HOSTS.contains(&program),
                "`{program}` is both a model host and a prompt surface"
            );
            // …and the predicate enforces it independently of the list,
            // so the two drifting apart cannot open the hole.
            assert!(
                !exe_is_prompt_program(true, Some(std::path::Path::new(MODEL_HOSTS[0]))),
                "the model host was accepted as a prompt surface"
            );
        }
        assert!(
            !exe_is_prompt_program(true, None),
            "a caller with no readable exe was accepted as a prompt surface"
        );
        assert!(
            !exe_is_prompt_program(
                false,
                Some(std::path::Path::new(PROMPT_SURFACE_PROGRAMS[0]))
            ),
            "another user's process was accepted as a prompt surface"
        );
    }

    /// Issue #289. Owning `dev.lisaos.Consent1` said a peer had *asked*
    /// to be the dialog; nothing said what it was. Every branch of the
    /// program half, without a live peer.
    #[test]
    fn a_peer_we_cannot_place_is_not_the_consent_surface() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("gjs-console");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();
        let real = real.canonicalize().unwrap();

        // The real thing is decided by `CONSENT_SURFACE_PROGRAMS`, which
        // names files that may not exist on a dev host — so the branches
        // are exercised through the same resolver with a local
        // allowlist, exactly as `exe_is_lisa_program` is.
        let allow = |same_user, exe: Option<&std::path::Path>| {
            lisa_peer::manager::may_manage(
                same_user,
                exe,
                &lisa_peer::manager::resolve_managers(std::slice::from_ref(&real)),
            )
            .is_ok()
        };
        assert!(allow(true, Some(&real)));
        assert!(!allow(true, None), "an unidentified peer became the dialog");
        assert!(
            !allow(false, Some(&real)),
            "another user's process became the dialog"
        );

        // And the shipped predicate itself refuses the two things it
        // must refuse on any host: a peer with no readable executable,
        // and a peer from another uid.
        assert!(
            !exe_is_consent_program(true, None),
            "a caller with no readable exe was accepted as the consent surface"
        );
        assert!(
            !exe_is_consent_program(
                false,
                Some(std::path::Path::new(CONSENT_SURFACE_PROGRAMS[0]))
            ),
            "another user's process was accepted as the consent surface"
        );
        assert!(
            !exe_is_consent_program(true, Some(std::path::Path::new(MODEL_HOSTS[0]))),
            "the model host was accepted as the consent surface"
        );
    }

    /// Every branch of the model-host test, without a live peer. The
    /// permissive answer needs BOTH facts: our uid and an allowlisted
    /// executable.
    #[test]
    fn a_peer_we_cannot_place_is_not_a_model_host() {
        assert!(!exe_hosts_a_model(true, None), "unidentified peer");
        assert!(
            !exe_hosts_a_model(false, Some(std::path::Path::new(MODEL_HOSTS[0]))),
            "another user's process running our harness binary"
        );
        assert!(
            !exe_hosts_a_model(true, Some(std::path::Path::new("/usr/bin/gjs"))),
            "any GJS program in the session"
        );
        // The CLI is deliberately absent — see MODEL_HOSTS' limit note.
        for manager in lisa_peer::manager::DEFAULT_MANAGERS {
            assert!(
                !exe_hosts_a_model(true, Some(std::path::Path::new(manager))),
                "`{manager}` became a model host, which costs it the right to \
                 answer its own `lisa do` chip"
            );
        }
    }
}
