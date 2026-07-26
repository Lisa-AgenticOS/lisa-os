//! The Agent Bus call state machine (`docs/PLAN.md` §5.4, §5.10).
//!
//! Request → tier resolution (with rule-6 provenance escalation) →
//! silent execute *or* park for confirmation → confirm/deny → execute.
//! Invariants enforced here, not by app goodwill:
//!
//! - **No ledger entry, no action.** The `tool.call` entry is appended
//!   before dispatch; if the Ledger is unavailable the call never runs.
//! - **No unconfirmed privileged calls.** Only a `read`-tier tool with
//!   a fully trusted trigger chain executes silently; everything else
//!   waits for `confirm()`. Pending confirmations expire.
//! - **Every executed privileged call is journaled** with its resolved
//!   compensation (or an explicit "not undoable"), so `undo()` reverts
//!   the last agent action.
//!
//! The MCP wire transport is behind [`Dispatcher`]; until the per-app
//! unix-socket client lands (next M5 slice, ADR-0009), production wires
//! [`NullDispatcher`] and tests use [`RecordingDispatcher`].

use crate::journal::{self, JournalError, UndoJournal};
use crate::manifest::{ToolDecl, validate_args};
use crate::registry::Registry;
use crate::tier::{Confirmation, Provenance, Resolution, resolve};
use lisa_ledger::{Event, Ledger, LedgerError, preview_of};
use lisa_peer::{Owner, PeerId};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

/// How long a parked confirmation stays answerable.
pub const CONFIRMATION_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Error)]
pub enum BusError {
    #[error("ledger unavailable — refusing to act: {0}")]
    Ledger(#[from] LedgerError),
    #[error("{0}")]
    Journal(#[from] JournalError),
    #[error("no pending call {0} (already answered, or expired and collected)")]
    UnknownCall(u64),
    /// Someone other than the peer that parked the call tried to answer
    /// it (#93). Deliberately indistinguishable from `UnknownCall` to a
    /// caller, so a sweep cannot use the error to map which ids exist.
    #[error("no pending call {0} (already answered, or expired and collected)")]
    NotYours(u64),
}

/// A tool invocation as requested by a client of the bus.
#[derive(Debug, Clone)]
pub struct CallRequest {
    /// Identity of the requesting client ("host" until the portal
    /// attaches real per-app identity).
    pub actor: String,
    /// Target MCP server (manifest `app_id`).
    pub app_id: String,
    pub tool: String,
    pub args: Value,
    /// Provenance of everything in the trigger chain (the user turn +
    /// every context chunk that steered this call). Empty = unknown =
    /// untrusted (fail closed).
    pub chain: Vec<Provenance>,
    /// Who is calling, as the *transport* reports it — not as the
    /// message claims (ADR-0033, issue #93). `actor` above is asserted
    /// and therefore only a label; this is identity.
    pub caller: PeerId,
}

/// What happened to a request (or a confirmation).
#[derive(Debug)]
pub enum Outcome {
    /// Dispatched and completed.
    Executed {
        call_id: u64,
        ledger_ref: i64,
        result: Value,
    },
    /// Dispatched and failed (already ledgered).
    Failed {
        call_id: u64,
        ledger_ref: i64,
        error: String,
    },
    /// Parked; the user must answer via `confirm()`. `spec` is the
    /// typed-diff material for the chip/modal.
    AwaitingConfirmation {
        call_id: u64,
        confirmation: Confirmation,
        escalated: bool,
        spec: Value,
    },
    /// Refused without dispatch (unknown tool, invalid args, user
    /// denial, expiry).
    Denied { call_id: u64, reason: String },
}

/// Result of an `undo()` request.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum UndoReport {
    /// Journal has no active entries.
    Nothing,
    /// The last action declared no compensation; it is skipped so the
    /// next `undo()` reaches the action below it.
    NotUndoable { app_id: String, tool: String },
    /// Compensation dispatched successfully.
    Undone {
        app_id: String,
        tool: String,
        undo_tool: String,
        result: Value,
    },
    /// Compensation dispatch failed; the entry stays active for retry.
    Failed {
        app_id: String,
        undo_tool: String,
        error: String,
    },
}

/// The MCP transport boundary: deliver one tool call to an app's MCP
/// server and return its result. The real per-app unix-socket client
/// (with D-Bus-activation-style spawn-on-demand) is the next M5 slice.
pub trait Dispatcher: Send + Sync {
    fn dispatch(&self, app_id: &str, tool: &str, args: &Value) -> Result<Value, String>;
}

/// Production placeholder until the MCP wire client lands: every
/// dispatch fails cleanly (and is ledgered as failed).
pub struct NullDispatcher;

impl Dispatcher for NullDispatcher {
    fn dispatch(&self, app_id: &str, tool: &str, _args: &Value) -> Result<Value, String> {
        Err(format!(
            "no MCP transport wired for {app_id}/{tool} yet (PLAN §5.4 next slice)"
        ))
    }
}

// Wire the real per-app MCP transport (`libs/mcp-bus`) into the bus.
// `McpDispatcher` speaks its own crate-local `mcp_bus::Dispatcher`; this
// bridge lets it stand in for `NullDispatcher` wherever the bus wants an
// `Arc<dyn Dispatcher>`. Legal here (not in the binary) because `Dispatcher`
// is defined in this crate — the orphan rule allows a local trait on a
// foreign type. See ADR-0013.
impl Dispatcher for mcp_bus::McpDispatcher {
    fn dispatch(&self, app_id: &str, tool: &str, args: &Value) -> Result<Value, String> {
        mcp_bus::Dispatcher::dispatch(self, app_id, tool, args)
    }
}

/// Test-support dispatcher: records every dispatched call and returns a
/// canned result. Used by this crate's tests and tests/injection-suite.
#[derive(Default)]
pub struct RecordingDispatcher {
    pub calls: Mutex<Vec<(String, String, Value)>>,
    pub result: Value,
}

impl RecordingDispatcher {
    pub fn returning(result: Value) -> RecordingDispatcher {
        RecordingDispatcher {
            calls: Mutex::new(Vec::new()),
            result,
        }
    }

    pub fn dispatched(&self) -> usize {
        self.calls.lock().expect("calls lock").len()
    }
}

impl Dispatcher for RecordingDispatcher {
    fn dispatch(&self, app_id: &str, tool: &str, args: &Value) -> Result<Value, String> {
        self.calls.lock().expect("calls lock").push((
            app_id.to_string(),
            tool.to_string(),
            args.clone(),
        ));
        Ok(self.result.clone())
    }
}

struct Pending {
    /// Only the peer that parked this may answer it (#93). Before this,
    /// `Confirm(id, true)` took no identity at all and ids were
    /// sequential from 1, so any peer could sweep the range and release
    /// somebody else's privileged call — including ahead of the human.
    owner: Owner,
    req: CallRequest,
    decl: ToolDecl,
    resolution: Resolution,
    start_ref: i64,
    created: Instant,
}

pub struct AgentBus {
    registry: Mutex<Registry>,
    ledger: Arc<Ledger>,
    journal: UndoJournal,
    dispatcher: Arc<dyn Dispatcher>,
    pending: Mutex<HashMap<u64, Pending>>,
    next_id: AtomicU64,
}

impl AgentBus {
    pub fn new(
        registry: Registry,
        ledger: Arc<Ledger>,
        journal: UndoJournal,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> AgentBus {
        AgentBus {
            registry: Mutex::new(registry),
            ledger,
            journal,
            dispatcher,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Tool listing as a JSON value (`lisa tools list`, D-Bus ListTools).
    pub fn list_tools(&self) -> Value {
        serde_json::to_value(self.registry.lock().expect("registry lock").list())
            .unwrap_or_else(|_| json!([]))
    }

    /// Discovery ("what can handle 'add a task'?") as a JSON value.
    pub fn discover(&self, query: &str) -> Value {
        serde_json::to_value(self.registry.lock().expect("registry lock").discover(query))
            .unwrap_or_else(|_| json!([]))
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().expect("pending lock").len()
    }

    /// Entry point: resolve policy for `req` and either execute (silent
    /// tier) or park it for confirmation. Everything is ledgered,
    /// including refusals.
    pub fn request(&self, req: CallRequest) -> Result<Outcome, BusError> {
        let call_id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let decl = self
            .registry
            .lock()
            .expect("registry lock")
            .tool(&req.app_id, &req.tool)
            .cloned();
        let Some(decl) = decl else {
            let reason = format!("unknown tool {}/{}", req.app_id, req.tool);
            self.ledger_deny(&req, None, "unknown-tool", &reason)?;
            return Ok(Outcome::Denied { call_id, reason });
        };

        if let Err(e) = validate_args(&decl.input_schema, &req.args) {
            let reason = format!("args rejected by input_schema: {e}");
            self.ledger_deny(&req, None, "invalid-args", &reason)?;
            return Ok(Outcome::Denied { call_id, reason });
        }

        let resolution = resolve(decl.tier, &req.chain);
        let start_ref = self.ledger.append(&Event {
            kind: "tool.call".into(),
            app_id: req.actor.clone(),
            input_hash: blake3::hash(req.args.to_string().as_bytes())
                .to_hex()
                .to_string(),
            preview: preview_of(&format!("{}/{} {}", req.app_id, req.tool, req.args)),
            status: "started".into(),
            detail: detail_json(&req, &resolution).to_string(),
            ..Default::default()
        })?;

        match resolution.confirmation {
            Confirmation::Silent => Ok(self.execute(call_id, &req, &decl, start_ref)),
            confirmation => {
                let spec = json!({
                    "call_id": call_id,
                    "actor": req.actor,
                    "app_id": req.app_id,
                    "tool": decl.name,
                    "description": decl.description,
                    "args": req.args,
                    "tier": resolution.declared.as_str(),
                    "effective_tier": resolution.effective.as_str(),
                    "confirmation": confirmation.as_str(),
                    "escalated": resolution.escalated,
                    "chain": req.chain.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    "undoable": decl.undo.is_some(),
                });
                self.pending.lock().expect("pending lock").insert(
                    call_id,
                    Pending {
                        owner: Owner::of(req.caller.clone()),
                        req,
                        decl,
                        resolution,
                        start_ref,
                        created: Instant::now(),
                    },
                );
                Ok(Outcome::AwaitingConfirmation {
                    call_id,
                    confirmation,
                    escalated: resolution.escalated,
                    spec,
                })
            }
        }
    }

    /// Answer a parked confirmation. Approval is itself ledgered
    /// (`tool.confirm`) before dispatch; denial and expiry append
    /// `tool.deny`.
    pub fn confirm(
        &self,
        call_id: u64,
        approve: bool,
        caller: &PeerId,
    ) -> Result<Outcome, BusError> {
        // Ownership is checked while still holding the lock, and the
        // entry is only removed once it is the right peer — otherwise a
        // foreign caller could evict somebody else's parked call and
        // turn a confirmation into a denial-of-service (#93).
        let pending = {
            let mut pending = self.pending.lock().expect("pending lock");
            match pending.get(&call_id) {
                None => return Err(BusError::UnknownCall(call_id)),
                Some(p) if p.owner.require(caller).is_err() => {
                    return Err(BusError::NotYours(call_id));
                }
                Some(_) => pending.remove(&call_id).expect("just checked"),
            }
        };

        if pending.created.elapsed() > CONFIRMATION_TTL {
            let reason = "confirmation expired".to_string();
            self.ledger_deny(&pending.req, Some(pending.start_ref), "expired", &reason)?;
            return Ok(Outcome::Denied { call_id, reason });
        }
        if !approve {
            let reason = "denied by user".to_string();
            self.ledger_deny(&pending.req, Some(pending.start_ref), "denied", &reason)?;
            return Ok(Outcome::Denied { call_id, reason });
        }
        self.ledger.append(&Event {
            kind: "tool.confirm".into(),
            app_id: pending.req.actor.clone(),
            preview: preview_of(&format!(
                "{} approved {}/{}",
                pending.resolution.confirmation.as_str(),
                pending.req.app_id,
                pending.req.tool
            )),
            status: "ok".into(),
            detail: detail_json(&pending.req, &pending.resolution).to_string(),
            ref_id: Some(pending.start_ref),
            ..Default::default()
        })?;
        Ok(self.execute(call_id, &pending.req, &pending.decl, pending.start_ref))
    }

    /// Revert the last agent action (`lisa undo`, PLAN §5.4). The
    /// compensation call is user-initiated, so it dispatches directly —
    /// ledgered as `tool.undo`.
    pub fn undo(&self, actor: &str) -> Result<UndoReport, BusError> {
        let Some(entry) = self.journal.last_active()? else {
            return Ok(UndoReport::Nothing);
        };
        let (Some(undo_tool), Some(undo_args_json)) = (&entry.undo_tool, &entry.undo_args_json)
        else {
            self.ledger.append(&Event {
                kind: "tool.undo".into(),
                app_id: actor.to_string(),
                preview: preview_of(&format!(
                    "{}/{} is not undoable — skipped",
                    entry.app_id, entry.tool
                )),
                status: "skipped".into(),
                detail: json!({"journal_id": entry.id, "ledger_ref": entry.ledger_ref}).to_string(),
                ref_id: Some(entry.ledger_ref),
                ..Default::default()
            })?;
            self.journal.set_state(entry.id, "skipped")?;
            return Ok(UndoReport::NotUndoable {
                app_id: entry.app_id,
                tool: entry.tool,
            });
        };

        let undo_args: Value = serde_json::from_str(undo_args_json).unwrap_or(Value::Null);
        let start_ref = self.ledger.append(&Event {
            kind: "tool.undo".into(),
            app_id: actor.to_string(),
            input_hash: blake3::hash(undo_args_json.as_bytes()).to_hex().to_string(),
            preview: preview_of(&format!(
                "undo {}/{} via {undo_tool}",
                entry.app_id, entry.tool
            )),
            status: "started".into(),
            detail: json!({"journal_id": entry.id, "reverts": entry.ledger_ref}).to_string(),
            ref_id: Some(entry.ledger_ref),
            ..Default::default()
        })?;
        match self
            .dispatcher
            .dispatch(&entry.app_id, undo_tool, &undo_args)
        {
            Ok(result) => {
                self.ledger.append(&Event {
                    kind: "tool.complete".into(),
                    app_id: actor.to_string(),
                    preview: preview_of(&result.to_string()),
                    status: "ok".into(),
                    ref_id: Some(start_ref),
                    ..Default::default()
                })?;
                self.journal.set_state(entry.id, "undone")?;
                Ok(UndoReport::Undone {
                    app_id: entry.app_id,
                    tool: entry.tool,
                    undo_tool: undo_tool.clone(),
                    result,
                })
            }
            Err(error) => {
                self.ledger.append(&Event {
                    kind: "tool.complete".into(),
                    app_id: actor.to_string(),
                    preview: preview_of(&error),
                    status: "error".into(),
                    detail: error.clone(),
                    ref_id: Some(start_ref),
                    ..Default::default()
                })?;
                Ok(UndoReport::Failed {
                    app_id: entry.app_id,
                    undo_tool: undo_tool.clone(),
                    error,
                })
            }
        }
    }

    /// Dispatch an approved (or silent-tier) call, journal privileged
    /// results, and close the Ledger trace. Ledger append failures here
    /// cannot un-happen the action, so they are not fatal to the caller.
    fn execute(&self, call_id: u64, req: &CallRequest, decl: &ToolDecl, start_ref: i64) -> Outcome {
        match self.dispatcher.dispatch(&req.app_id, &req.tool, &req.args) {
            Ok(result) => {
                let mut notes: Vec<String> = Vec::new();
                if decl.tier.is_privileged() {
                    let undo = decl.undo.as_ref().and_then(|u| {
                        match journal::resolve_undo(u, &req.args, &result) {
                            Ok(args) => Some((u.tool.clone(), args)),
                            Err(e) => {
                                notes.push(format!("undo not resolvable: {e}"));
                                None
                            }
                        }
                    });
                    if decl.undo.is_none() {
                        notes.push("tool declares no undo".to_string());
                    }
                    if let Err(e) = self.journal.record(journal::NewRecord {
                        ledger_ref: start_ref,
                        actor: &req.actor,
                        app_id: &req.app_id,
                        tool: &req.tool,
                        args: &req.args,
                        result: &result,
                        undo,
                    }) {
                        notes.push(format!("journal write failed: {e}"));
                    }
                }
                let _ = self.ledger.append(&Event {
                    kind: "tool.complete".into(),
                    app_id: req.actor.clone(),
                    preview: preview_of(&result.to_string()),
                    status: "ok".into(),
                    detail: json!({"notes": notes}).to_string(),
                    ref_id: Some(start_ref),
                    ..Default::default()
                });
                Outcome::Executed {
                    call_id,
                    ledger_ref: start_ref,
                    result,
                }
            }
            Err(error) => {
                let _ = self.ledger.append(&Event {
                    kind: "tool.complete".into(),
                    app_id: req.actor.clone(),
                    preview: preview_of(&error),
                    status: "error".into(),
                    detail: error.clone(),
                    ref_id: Some(start_ref),
                    ..Default::default()
                });
                Outcome::Failed {
                    call_id,
                    ledger_ref: start_ref,
                    error,
                }
            }
        }
    }

    fn ledger_deny(
        &self,
        req: &CallRequest,
        ref_id: Option<i64>,
        status: &str,
        reason: &str,
    ) -> Result<i64, BusError> {
        Ok(self.ledger.append(&Event {
            kind: "tool.deny".into(),
            app_id: req.actor.clone(),
            input_hash: blake3::hash(req.args.to_string().as_bytes())
                .to_hex()
                .to_string(),
            preview: preview_of(&format!("{}/{}: {reason}", req.app_id, req.tool)),
            status: status.into(),
            detail: reason.into(),
            ref_id,
            ..Default::default()
        })?)
    }
}

fn detail_json(req: &CallRequest, resolution: &Resolution) -> Value {
    json!({
        "target": req.app_id,
        "tool": req.tool,
        "tier": resolution.declared.as_str(),
        "effective_tier": resolution.effective.as_str(),
        "confirmation": resolution.confirmation.as_str(),
        "escalated": resolution.escalated,
        "chain": req.chain.iter().map(ToString::to_string).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, fixture_calendar_json};
    use serde_json::json;

    struct Fixture {
        _dir: tempfile::TempDir,
        bus: AgentBus,
        ledger: Arc<Ledger>,
        dispatcher: Arc<RecordingDispatcher>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Arc::new(Ledger::open(dir.path().join("ledger.db")).unwrap());
        let dispatcher = Arc::new(RecordingDispatcher::returning(json!({"event_id": "evt-1"})));
        let mut registry = Registry::new();
        registry
            .insert(Manifest::from_json(&fixture_calendar_json()).unwrap())
            .unwrap();
        let bus = AgentBus::new(
            registry,
            Arc::clone(&ledger),
            UndoJournal::open_in_memory().unwrap(),
            Arc::clone(&dispatcher) as Arc<dyn Dispatcher>,
        );
        Fixture {
            _dir: dir,
            bus,
            ledger,
            dispatcher,
        }
    }

    fn call(app: &str, tool: &str, args: Value, chain: Vec<Provenance>) -> CallRequest {
        CallRequest {
            actor: "host".into(),
            app_id: app.into(),
            tool: tool.into(),
            args,
            chain,
            caller: lisa_peer::PeerId::Direct,
        }
    }

    fn user() -> Vec<Provenance> {
        vec![Provenance::User]
    }

    fn ledger_kinds(ledger: &Ledger) -> Vec<(String, String)> {
        let mut kinds: Vec<(String, String)> = ledger
            .tail(100)
            .unwrap()
            .into_iter()
            .map(|e| (e.kind, e.status))
            .collect();
        kinds.reverse(); // oldest first
        kinds
    }

    #[test]
    fn read_tier_with_trusted_chain_executes_silently_and_is_ledgered() {
        let f = fixture();
        let outcome = f
            .bus
            .request(call("org.gnome.Calendar", "list_events", json!({}), user()))
            .unwrap();
        assert!(matches!(outcome, Outcome::Executed { .. }));
        assert_eq!(f.dispatcher.dispatched(), 1);
        assert_eq!(
            ledger_kinds(&f.ledger),
            vec![
                ("tool.call".to_string(), "started".to_string()),
                ("tool.complete".to_string(), "ok".to_string()),
            ]
        );
    }

    #[test]
    fn write_tier_parks_for_chip_and_never_dispatches_unconfirmed() {
        let f = fixture();
        let outcome = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "add_event",
                json!({"title": "dentist", "start": "2026-07-24T10:00:00Z"}),
                user(),
            ))
            .unwrap();
        let Outcome::AwaitingConfirmation {
            confirmation,
            escalated,
            spec,
            ..
        } = outcome
        else {
            panic!("write tier must wait: {outcome:?}");
        };
        assert_eq!(confirmation, Confirmation::Chip);
        assert!(!escalated);
        assert_eq!(spec["tool"], "add_event");
        assert_eq!(spec["undoable"], true);
        assert_eq!(f.dispatcher.dispatched(), 0, "no dispatch before consent");
        assert_eq!(f.bus.pending_count(), 1);
    }

    /// Issue #93 (critical): `Confirm(id, approve)` carried no caller
    /// identity and ids were sequential from 1, so any session-bus peer
    /// could sweep the range and release somebody else's parked
    /// privileged call — including racing ahead of the human on a
    /// modal-tier call whose trigger chain was untrusted. That is the M5
    /// acceptance criterion ("0 unconfirmed privileged calls") stated
    /// verbatim, so this test is the acceptance block in miniature.
    #[test]
    fn a_foreign_peer_cannot_answer_someone_elses_confirmation() {
        let f = fixture();
        let alice = lisa_peer::PeerId::Bus(":1.10".into());
        let mallory = lisa_peer::PeerId::Bus(":1.11".into());

        let mut req = call(
            "org.gnome.Calendar",
            "add_event",
            json!({"title": "dentist", "start": "2026-07-24T10:00:00Z"}),
            user(),
        );
        req.caller = alice.clone();
        let Outcome::AwaitingConfirmation { call_id, .. } = f.bus.request(req).unwrap() else {
            panic!("a destructive tool with an empty chain must park");
        };

        // Mallory sweeps. Every id, not just the right one.
        for id in 1..=call_id + 3 {
            assert!(
                f.bus.confirm(id, true, &mallory).is_err(),
                "peer {mallory} answered call {id}"
            );
        }
        assert_eq!(
            f.dispatcher.dispatched(),
            0,
            "a foreign peer's approval dispatched a privileged call"
        );

        // The rightful owner is unaffected by the failed sweep — the
        // parked call must still be there, not evicted.
        let outcome = f.bus.confirm(call_id, true, &alice).unwrap();
        assert!(
            matches!(outcome, Outcome::Executed { .. }),
            "the owner could no longer answer their own call: {outcome:?}"
        );
        assert_eq!(f.dispatcher.dispatched(), 1);
    }

    /// The refusal must not double as an oracle. A wrong-owner answer
    /// renders identically to a nonexistent id, so sweeping cannot map
    /// which call ids are live — otherwise the fix for #93 would hand an
    /// attacker the reconnaissance it needs for the next attempt.
    #[test]
    fn a_rejected_confirmation_does_not_reveal_which_ids_exist() {
        for id in [1u64, 7, 4242] {
            assert_eq!(
                BusError::NotYours(id).to_string(),
                BusError::UnknownCall(id).to_string(),
                "the two refusals are distinguishable for id {id}"
            );
        }
    }

    #[test]
    fn confirm_executes_journals_compensation_and_closes_the_trace() {
        let f = fixture();
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "add_event",
                json!({"title": "dentist", "start": "2026-07-24T10:00:00Z"}),
                user(),
            ))
            .unwrap()
        else {
            panic!("expected pending");
        };
        let outcome = f
            .bus
            .confirm(call_id, true, &lisa_peer::PeerId::Direct)
            .unwrap();
        assert!(matches!(outcome, Outcome::Executed { .. }));
        assert_eq!(f.dispatcher.dispatched(), 1);
        assert_eq!(
            ledger_kinds(&f.ledger),
            vec![
                ("tool.call".to_string(), "started".to_string()),
                ("tool.confirm".to_string(), "ok".to_string()),
                ("tool.complete".to_string(), "ok".to_string()),
            ]
        );
        // Undo now reverts it via the manifest-declared compensation.
        let report = f.bus.undo("host").unwrap();
        let UndoReport::Undone {
            undo_tool, result, ..
        } = report
        else {
            panic!("expected undone: {report:?}");
        };
        assert_eq!(undo_tool, "delete_event");
        assert_eq!(result, json!({"event_id": "evt-1"}));
        let calls = f.dispatcher.calls.lock().unwrap();
        assert_eq!(calls[1].1, "delete_event");
        assert_eq!(
            calls[1].2,
            json!({"event_id": "evt-1"}),
            "mapped from $result"
        );
    }

    #[test]
    fn deny_refuses_without_dispatch_and_double_answer_fails() {
        let f = fixture();
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "delete_event",
                json!({"event_id": "evt-1"}),
                user(),
            ))
            .unwrap()
        else {
            panic!("expected pending");
        };
        let outcome = f
            .bus
            .confirm(call_id, false, &lisa_peer::PeerId::Direct)
            .unwrap();
        assert!(matches!(outcome, Outcome::Denied { .. }));
        assert_eq!(f.dispatcher.dispatched(), 0);
        assert!(
            matches!(
                f.bus.confirm(call_id, true, &lisa_peer::PeerId::Direct),
                Err(BusError::UnknownCall(_))
            ),
            "an answered confirmation is gone"
        );
        let kinds = ledger_kinds(&f.ledger);
        assert_eq!(
            kinds.last().unwrap(),
            &("tool.deny".to_string(), "denied".to_string())
        );
    }

    #[test]
    fn untrusted_chain_escalates_read_to_chip_and_write_to_modal() {
        let f = fixture();
        let chain = vec![Provenance::User, Provenance::Mail];
        let outcome = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "list_events",
                json!({}),
                chain.clone(),
            ))
            .unwrap();
        let Outcome::AwaitingConfirmation {
            confirmation,
            escalated,
            ..
        } = outcome
        else {
            panic!("escalated read must wait: {outcome:?}");
        };
        assert_eq!(confirmation, Confirmation::Chip);
        assert!(escalated);

        let outcome = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "add_event",
                json!({"title": "x", "start": "now"}),
                chain,
            ))
            .unwrap();
        let Outcome::AwaitingConfirmation { confirmation, .. } = outcome else {
            panic!("expected pending");
        };
        assert_eq!(
            confirmation,
            Confirmation::Modal,
            "write + untrusted = modal"
        );
        assert_eq!(f.dispatcher.dispatched(), 0);
    }

    #[test]
    fn empty_chain_fails_closed_even_for_read() {
        let f = fixture();
        let outcome = f
            .bus
            .request(call("org.gnome.Calendar", "list_events", json!({}), vec![]))
            .unwrap();
        assert!(
            matches!(outcome, Outcome::AwaitingConfirmation { .. }),
            "unknown origin must not execute silently"
        );
    }

    #[test]
    fn unknown_tool_and_invalid_args_are_denied_and_ledgered() {
        let f = fixture();
        let outcome = f
            .bus
            .request(call("org.gnome.Calendar", "explode", json!({}), user()))
            .unwrap();
        assert!(matches!(outcome, Outcome::Denied { .. }));

        let outcome = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "add_event",
                json!({"title": "no start field"}),
                user(),
            ))
            .unwrap();
        let Outcome::Denied { reason, .. } = outcome else {
            panic!("invalid args must be denied");
        };
        assert!(reason.contains("input_schema"), "{reason}");
        assert_eq!(f.dispatcher.dispatched(), 0);
        assert_eq!(f.ledger.count().unwrap(), 2, "both refusals ledgered");
    }

    #[test]
    fn undo_skips_non_undoable_actions_and_reports_empty_journal() {
        let f = fixture();
        assert!(matches!(f.bus.undo("host").unwrap(), UndoReport::Nothing));

        // delete_event declares no undo → journaled as not-undoable.
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "delete_event",
                json!({"event_id": "evt-1"}),
                user(),
            ))
            .unwrap()
        else {
            panic!("expected pending");
        };
        f.bus
            .confirm(call_id, true, &lisa_peer::PeerId::Direct)
            .unwrap();
        let report = f.bus.undo("host").unwrap();
        assert!(
            matches!(report, UndoReport::NotUndoable { .. }),
            "{report:?}"
        );
        assert!(
            matches!(f.bus.undo("host").unwrap(), UndoReport::Nothing),
            "skipped entries leave the stack"
        );
    }

    #[test]
    fn dispatch_failure_is_ledgered_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Arc::new(Ledger::open(dir.path().join("ledger.db")).unwrap());
        let mut registry = Registry::new();
        registry
            .insert(Manifest::from_json(&fixture_calendar_json()).unwrap())
            .unwrap();
        let bus = AgentBus::new(
            registry,
            Arc::clone(&ledger),
            UndoJournal::open_in_memory().unwrap(),
            Arc::new(NullDispatcher),
        );
        let outcome = bus
            .request(call("org.gnome.Calendar", "list_events", json!({}), user()))
            .unwrap();
        let Outcome::Failed { error, .. } = outcome else {
            panic!("NullDispatcher must fail: {outcome:?}");
        };
        assert!(error.contains("no MCP transport"), "{error}");
        let tail = ledger.tail(1).unwrap();
        assert_eq!(tail[0].kind, "tool.complete");
        assert_eq!(tail[0].status, "error");
    }
}
