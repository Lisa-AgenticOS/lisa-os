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
//!                  "confirm-modal" | "denied"
//! Confirm(t call_id, b approve) → (s status, s detail_json)
//! Undo() → (s report_json)
//! signal ConfirmationRequested(t call_id, s spec_json)
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
/// never from anything a caller asserts (ADR-0033). Program identity via
/// `/proc/<pid>/exe` would not help here: the surface runs under
/// `/usr/bin/gjs`, so an executable allowlist would authorise *any* GJS
/// program in the session rather than the consent surface.
///
/// This was `dev.lisaos.Overlay1` — the overlay BACKEND, which also
/// hosts the model. A call the overlay originated therefore came back to
/// `Confirm` from a peer that was both requester and surface, and the
/// model approved itself (#145). `bus.rs` now refuses that pairing
/// whatever the name says; pointing at a name a separate process owns is
/// the other half, so the dialog is a different program and not merely a
/// different role.
const CONSENT_SURFACE: &str = "dev.lisaos.Consent1";

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
        BusError::NeedsConsentSurface(_) => zbus::fdo::Error::AccessDenied(e.to_string()),
        other => zbus::fdo::Error::Failed(other.to_string()),
    }
}

/// Is this caller the human's dialog? Asked of the message bus.
///
/// Fails *closed towards `Absent`*, which is deliberate: `Absent` means
/// "no separate surface exists, so the requester answers its own call".
/// The alternative — treating an unreachable broker as `Other` — would
/// make every destructive call unanswerable by anyone, which is the
/// availability failure #135 reported.
async fn consent_role(conn: &zbus::Connection, caller: &lisa_peer::PeerId) -> ConsentRole {
    // p2p has no broker to ask and no desktop session to ask about.
    let lisa_peer::PeerId::Bus(caller_name) = caller else {
        return ConsentRole::Absent;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return ConsentRole::Absent;
    };
    let Ok(name) = zbus::names::BusName::try_from(CONSENT_SURFACE) else {
        return ConsentRole::Absent;
    };
    match dbus.get_name_owner(name).await {
        Ok(owner) if owner.as_str() == caller_name => ConsentRole::Surface,
        Ok(_) => ConsentRole::Other,
        // NameHasNoOwner: the overlay is not running.
        Err(_) => ConsentRole::Absent,
    }
}

/// Is this caller one of Lisa's own programs?
///
/// Program identity from `/proc/<pid>/exe` via the peer's credentials —
/// never `comm`, which a process can rename, and never anything the
/// message asserts (ADR-0033). Only these may claim `user` provenance:
/// the CLI and the overlay backend are the two surfaces a human types
/// into, and "a human typed this" is the one tag that buys trust rather
/// than costing it.
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
    exe_is_lisa_program(
        peer.is_same_user_as_us(),
        lisa_peer::exe_of_peer(&peer).ok().as_deref(),
        &lisa_peer::manager::default_managers(),
    )
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
        } => (
            *call_id,
            "failed".into(),
            serde_json::json!({"error": error, "ledger_ref": ledger_ref}).to_string(),
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
            })
            .map_err(fdo_err)?;
        let reply = outcome_reply(&outcome);
        if let Outcome::AwaitingConfirmation { call_id, spec, .. } = &outcome {
            let _ = Self::confirmation_requested(&emitter, *call_id, spec.to_string()).await;
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
        let answerer = Answerer {
            consent: consent_role(conn, &caller).await,
            peer: caller,
        };
        let outcome = self
            .bus
            .confirm(call_id, approve, &answerer)
            .map_err(fdo_err)?;
        let (_, status, detail) = outcome_reply(&outcome);
        Ok((status, detail))
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

    /// The consent-surface refusal is deliberately NOT disguised: the
    /// caller parked the call, so telling it "the human answers this
    /// one" reveals nothing it did not already know, and a silent
    /// refusal there reads as a bug (#135).
    #[test]
    fn the_consent_surface_refusal_is_its_own_error() {
        let refusal = fdo_err(BusError::NeedsConsentSurface(7));
        assert!(matches!(refusal, zbus::fdo::Error::AccessDenied(_)));
        assert!(
            refusal.to_string().contains("consent surface"),
            "the refusal must say where to go: {refusal}"
        );
    }
}
