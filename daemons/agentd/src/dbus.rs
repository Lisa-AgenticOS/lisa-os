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
use crate::tier::{Confirmation, Provenance};
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
/// confirmation dialog (`shell/overlay-extension`, PLAN §5.7.1).
///
/// Identity comes from the BROKER's answer to "who owns this name",
/// never from anything a caller asserts (ADR-0033). Program identity via
/// `/proc/<pid>/exe` would not help here: the backend runs under
/// `/usr/bin/gjs`, so an executable allowlist would authorise *any* GJS
/// program in the session rather than the consent surface.
const CONSENT_SURFACE: &str = "dev.lisaos.Overlay1";

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
        let chain: Vec<Provenance> = options
            .get("provenance")
            .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
            .unwrap_or_default()
            .iter()
            .map(|s| Provenance::parse(s))
            .collect();

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

    /// Revert the last agent action via its journaled compensation.
    fn undo(&self) -> zbus::fdo::Result<String> {
        let report = self.bus.undo("host").map_err(fdo_err)?;
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
