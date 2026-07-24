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

use crate::bus::{AgentBus, BusError, CallRequest, Outcome};
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

fn fdo_err(e: BusError) -> zbus::fdo::Error {
    match e {
        BusError::UnknownCall(_) => zbus::fdo::Error::InvalidArgs(e.to_string()),
        other => zbus::fdo::Error::Failed(other.to_string()),
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
    async fn request_call(
        &self,
        app_id: String,
        tool: String,
        args_json: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
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
    fn confirm(&self, call_id: u64, approve: bool) -> zbus::fdo::Result<(String, String)> {
        let outcome = self.bus.confirm(call_id, approve).map_err(fdo_err)?;
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
