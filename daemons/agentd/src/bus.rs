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

/// How many confirmations may be parked at once, in total and per
/// requesting peer (#137).
///
/// `RequestCall` is reachable by any session peer, and every parked
/// call retains a full `CallRequest` — including caller-supplied `args`
/// bounded only by the tool's own schema. Without a cap the map grows
/// until the daemon is OOM-killed, taking every legitimately parked
/// confirmation with it. Denying the Agent Bus denies the confirmation
/// surface, so this is a soft bypass of "no unconfirmed privileged
/// calls": it makes confirmation unavailable rather than defeating it.
pub const MAX_PENDING: usize = 128;
pub const MAX_PENDING_PER_OWNER: usize = 16;

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
    /// The requester tried to approve its OWN destructive call while a
    /// desktop consent surface was running (#135). Unlike `NotYours`
    /// this is safe to distinguish: the caller already knows the call
    /// exists — it parked it — so the message is no oracle, and a
    /// silent refusal here would look like a bug rather than a policy.
    #[error(
        "call {0} is destructive-tier: only the desktop consent surface \
         may approve it — the peer that asked for it may only withdraw it"
    )]
    NeedsConsentSurface(u64),
}

/// What the *broker* says about the caller's relationship to the
/// desktop consent surface (`dev.lisaos.Overlay1`).
///
/// Never claimed by the caller. agentd asks the message bus who owns
/// the consent surface's well-known name and compares it to the
/// transport-assigned sender (ADR-0033) — the same authority `PeerId`
/// already rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentRole {
    /// This caller owns the consent surface's name: it is the human's
    /// dialog, and it may approve.
    Surface,
    /// A consent surface is running, and this is not it.
    Other,
    /// No consent surface exists — a headless host, the p2p transport
    /// the daemon's own tests use, or a session where the overlay is not
    /// running. There is nobody else to ask, so the requester answers
    /// its own call and the Ledger records that it did.
    Absent,
}

/// Who is answering a parked call, and in what capacity.
#[derive(Debug, Clone)]
pub struct Answerer {
    pub peer: PeerId,
    pub consent: ConsentRole,
}

impl Answerer {
    /// A peer answering with no consent surface in the session.
    pub fn alone(peer: PeerId) -> Answerer {
        Answerer {
            peer,
            consent: ConsentRole::Absent,
        }
    }

    /// The desktop consent surface — the human's dialog.
    pub fn surface(peer: PeerId) -> Answerer {
        Answerer {
            peer,
            consent: ConsentRole::Surface,
        }
    }

    /// A peer that is not the consent surface, in a session that has one.
    pub fn ordinary(peer: PeerId) -> Answerer {
        Answerer {
            peer,
            consent: ConsentRole::Other,
        }
    }
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
    /// How long a parked confirmation stays answerable. Always
    /// [`CONFIRMATION_TTL`] in production; tests shorten it.
    ttl: Duration,
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
            ttl: CONFIRMATION_TTL,
        }
    }

    /// Shorten the confirmation TTL. Tests only — expiry is otherwise
    /// two minutes, and a test that sleeps for two minutes is a test
    /// nobody runs.
    #[cfg(test)]
    fn with_ttl(mut self, ttl: Duration) -> AgentBus {
        self.ttl = ttl;
        self
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

    /// Record that a caller claimed `user` provenance it was not
    /// entitled to, and was downgraded (#55).
    ///
    /// The downgrade itself happens in `tier::verify_chain` and is not
    /// a refusal — an app that simply tags its input wrongly must keep
    /// working. What matters is the *pattern*: a peer repeatedly
    /// claiming a human typed this is the signature of something trying
    /// to reach the trusted path, and the only place that signature can
    /// be seen after the fact is the Ledger. It was an `eprintln`,
    /// which is to say it was in the journal of whoever happened to be
    /// looking, for as long as the journal kept it.
    ///
    /// Its own event kind rather than a field on `tool.call`: the call
    /// may never reach the bus (it can be denied earlier), and a
    /// security signal that only appears when the call succeeds is one
    /// you cannot search for.
    pub fn ledger_provenance_downgrade(&self, actor: &str, app_id: &str, tool: &str) {
        // Best-effort by design: failing the call because the note
        // could not be written would turn a Ledger problem into an
        // outage, and the call is not the thing at fault here.
        if let Err(e) = self.ledger.append(&Event {
            kind: "agent.provenance_downgrade".into(),
            app_id: actor.into(),
            preview: preview_of(&format!(
                "{app_id}/{tool} claimed user provenance without being a Lisa program"
            )),
            status: "downgraded".into(),
            detail: format!("asserted=user verified=app:{app_id} tool={tool}"),
            ..Default::default()
        }) {
            eprintln!("agentd: could not ledger a provenance downgrade: {e}");
        }
    }

    /// Drop every parked call past its TTL, handing them back so each
    /// can be ledgered (#137).
    ///
    /// Before this, `confirm()` was the only path that removed from the
    /// map and it checked the TTL *after* removing — so an expired
    /// entry survived until its own owner happened to answer it, and
    /// one that nobody answered survived for the life of the process.
    /// The refusal text has always read "already answered, or expired
    /// and collected"; nothing collected. An abandoned privileged call
    /// now leaves a record instead of just leaking.
    fn collect_expired(&self) -> Vec<Pending> {
        let mut pending = self.pending.lock().expect("pending lock");
        let expired: Vec<u64> = pending
            .iter()
            .filter(|(_, p)| p.created.elapsed() > self.ttl)
            .map(|(id, _)| *id)
            .collect();
        expired
            .into_iter()
            .filter_map(|id| pending.remove(&id))
            .collect()
    }

    /// Evict the oldest parked call of whichever peer is holding the
    /// most, so a flooder pays for its own flood rather than the next
    /// person to ask.
    ///
    /// Refusing outright at the cap would let one peer make the human's
    /// next confirmation impossible — the same denial-of-service in a
    /// different shape.
    fn evict_greediest(&self) -> Option<Pending> {
        let mut pending = self.pending.lock().expect("pending lock");
        let mut per_owner: HashMap<&Owner, usize> = HashMap::new();
        for p in pending.values() {
            *per_owner.entry(&p.owner).or_default() += 1;
        }
        let greediest = per_owner
            .into_iter()
            .max_by_key(|&(_, n)| n)
            .map(|(o, _)| o.clone())?;
        let victim = pending
            .iter()
            .filter(|(_, p)| p.owner == greediest)
            .min_by_key(|(_, p)| p.created)
            .map(|(id, _)| *id)?;
        pending.remove(&victim)
    }

    /// Entry point: resolve policy for `req` and either execute (silent
    /// tier) or park it for confirmation. Everything is ledgered,
    /// including refusals.
    pub fn request(&self, req: CallRequest) -> Result<Outcome, BusError> {
        let call_id = self.next_id.fetch_add(1, Ordering::Relaxed);

        // Collect anything that timed out while nobody was looking, so
        // the map does not grow without bound and the refusal text
        // ("expired and collected") is finally true (#137).
        for stale in self.collect_expired() {
            let reason = "confirmation expired".to_string();
            self.ledger_deny(&stale.req, Some(stale.start_ref), "expired", &reason)?;
        }

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
            Confirmation::Silent => Ok(self.execute(call_id, &req, &decl, &resolution, start_ref)),
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
                // Caps, checked before the request is moved into the
                // map (#137). A peer that parks calls it never answers
                // must not be able to exhaust the daemon.
                let owner = Owner::of(req.caller.clone());
                let (total, mine) = {
                    let pending = self.pending.lock().expect("pending lock");
                    (
                        pending.len(),
                        pending.values().filter(|p| p.owner == owner).count(),
                    )
                };
                if mine >= MAX_PENDING_PER_OWNER {
                    let reason = format!(
                        "{MAX_PENDING_PER_OWNER} confirmations from this caller are \
                         already waiting — answer or withdraw one first"
                    );
                    self.ledger_deny(&req, Some(start_ref), "over-capacity", &reason)?;
                    return Ok(Outcome::Denied { call_id, reason });
                }
                if total >= MAX_PENDING
                    && let Some(evicted) = self.evict_greediest()
                {
                    let reason = "evicted: too many confirmations waiting".to_string();
                    self.ledger_deny(&evicted.req, Some(evicted.start_ref), "evicted", &reason)?;
                }
                self.pending.lock().expect("pending lock").insert(
                    call_id,
                    Pending {
                        owner,
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
    /// Who may answer a parked call, and for what.
    ///
    /// #93 asked for "somebody else cannot release *my* call". The first
    /// implementation answered "only the requester may release its own
    /// call" — a different and weaker property, and one that inverted
    /// ADR-0030 §1: guardrails sit between the model and the machine,
    /// never between a person and their own machine. It fenced the
    /// human's dialog out (its bus name is not the requester's) and left
    /// the requesting peer — for an agent-driven call, the model's own
    /// connection — as the only peer that could approve.
    ///
    /// So authority is split by what the answer *does*:
    ///
    /// - **Withdrawal** (`approve == false`) never causes an action, so
    ///   the requester may always take its own call back, and the
    ///   consent surface may refuse anything.
    /// - **Approval** of a `Modal` (destructive) call belongs to the
    ///   human's dialog whenever one is running. The requester cannot
    ///   approve its own.
    /// - **Approval** of a `Chip` (write) call may also come from the
    ///   requester: the chip is the app's own inline affordance, and
    ///   routing every write through a modal would train people to click
    ///   through it, which is how a confirmation stops being one.
    ///
    /// Returns the error *constructor* so the id is applied at the call
    /// site, where the lock is held.
    fn may_answer(
        p: &Pending,
        approve: bool,
        answerer: &Answerer,
    ) -> Result<(), fn(u64) -> BusError> {
        let is_requester = p.owner.allows(&answerer.peer);
        // Owning the consent name counts as oversight only when you are
        // NOT the peer that asked (#145). The overlay backend both hosts
        // the model and owns `dev.lisaos.Overlay1`, so for a call it
        // originates it arrived here as `Surface` — and `Surface` was
        // treated as proof that somebody else had looked. It was the same
        // process wearing two hats, and the effect was that the model
        // approved its own destructive calls.
        //
        // Independence is a property of the PAIR (requester, answerer),
        // not of either alone, so it is computed as one.
        let is_independent_surface = answerer.consent == ConsentRole::Surface && !is_requester;
        if !is_requester && !is_independent_surface {
            return Err(BusError::NotYours);
        }
        if approve
            && p.resolution.confirmation == Confirmation::Modal
            && !is_independent_surface
            // `Absent` means no consent surface exists anywhere, so
            // refusing would make every destructive call unanswerable on
            // a headless host — the availability failure #135 reported.
            // `Surface`-but-self and `Other` both mean a dialog exists
            // and this is not an independent one.
            && answerer.consent != ConsentRole::Absent
        {
            return Err(BusError::NeedsConsentSurface);
        }
        // The Ledger records when no consent surface was involved.
        Ok(())
    }

    pub fn confirm(
        &self,
        call_id: u64,
        approve: bool,
        answerer: &Answerer,
    ) -> Result<Outcome, BusError> {
        // Authority is checked while still holding the lock, and the
        // entry is only removed once the caller is allowed to answer —
        // otherwise a foreign caller could evict somebody else's parked
        // call and turn a confirmation into a denial-of-service (#93).
        let pending = {
            let mut pending = self.pending.lock().expect("pending lock");
            let Some(p) = pending.get(&call_id) else {
                return Err(BusError::UnknownCall(call_id));
            };
            match Self::may_answer(p, approve, answerer) {
                // Refuse WITHOUT removing: a call the caller may not
                // approve is still the requester's to withdraw.
                Err(e) => return Err(e(call_id)),
                Ok(()) => pending.remove(&call_id).expect("just checked"),
            }
        };

        if pending.created.elapsed() > self.ttl {
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
                "{} approved {}/{} by {}",
                pending.resolution.confirmation.as_str(),
                pending.req.app_id,
                pending.req.tool,
                // WHO approved is the point of the entry: a reader must
                // be able to tell the human's dialog from a requester
                // that answered its own call (#135).
                match answerer.consent {
                    ConsentRole::Surface => "the consent surface".to_string(),
                    _ => format!("{} (no consent surface)", answerer.peer),
                }
            )),
            status: "ok".into(),
            detail: detail_json(&pending.req, &pending.resolution).to_string(),
            ref_id: Some(pending.start_ref),
            ..Default::default()
        })?;
        Ok(self.execute(
            call_id,
            &pending.req,
            &pending.decl,
            &pending.resolution,
            pending.start_ref,
        ))
    }

    /// Revert the last agent action (`lisa undo`, PLAN §5.4). The
    /// compensation call is user-initiated, so it dispatches directly —
    /// ledgered as `tool.undo`.
    /// Revert the newest action **this caller made**.
    ///
    /// `actor` is a Ledger label and authorises nothing; `caller` is the
    /// transport-assigned identity and does (ADR-0033). Before #94 this
    /// took only the label, dispatched the compensation straight to the
    /// MCP transport, and `last_active()` was unscoped — so any peer on
    /// the session bus could revert an action taken by any other, with
    /// no tier resolution and no confirmation, using a compensation that
    /// is frequently destructive-tier.
    ///
    /// Ownership rather than a fresh confirmation is deliberate: undo
    /// reverts an action this peer made, and having made it is the
    /// authority. Re-asking would make undo unusable while adding
    /// nothing a person would act on differently.
    pub fn undo(&self, actor: &str, caller: &PeerId) -> Result<UndoReport, BusError> {
        let Some(entry) = self.journal.last_active(caller.as_str())? else {
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
    /// Dispatch, and record what was done.
    ///
    /// `resolution` rather than just `decl` (issue #98). The journal
    /// used to key off `decl.tier` — the **declared** tier, from the
    /// manifest — while consent keyed off `resolution.effective`, the
    /// tier the bus actually enforced after provenance escalation. So a
    /// call the bus judged write-tier, *asked the user about*, got
    /// consent for and dispatched, executed with no journal entry and
    /// could not be undone.
    ///
    /// That is exactly backwards. Escalation exists because an untrusted
    /// trigger chain means the declared tier cannot be trusted to
    /// describe the consequences (§5.10, Appendix C, CLAUDE.md rule 6);
    /// applying that distrust to consent and then discarding it for
    /// reversibility leaves the calls most likely to have been steered
    /// by hostile content as the ones with no compensation recorded.
    /// Worse, `lisa undo` then reached *past* the escalated call to an
    /// older action — the user approves a chip and undoes something
    /// else.
    ///
    /// The trigger is now the effective tier, plus anything the user was
    /// asked about at all: "we asked a person" is a better reason to
    /// record something than the app's own label for it.
    fn execute(
        &self,
        call_id: u64,
        req: &CallRequest,
        decl: &ToolDecl,
        resolution: &Resolution,
        start_ref: i64,
    ) -> Outcome {
        match self.dispatcher.dispatch(&req.app_id, &req.tool, &req.args) {
            Ok(result) => {
                let mut notes: Vec<String> = Vec::new();
                let asked_a_person = resolution.confirmation != Confirmation::Silent;
                if resolution.effective.is_privileged() || asked_a_person {
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
                        // A read-tier tool has no manifest-declared
                        // inverse (`manifest.rs` forbids `undo` on read
                        // tools), so an escalated read journals as
                        // not-undoable. That is the honest answer, and
                        // it is what stops `undo` skipping past it to
                        // something older.
                        notes.push("tool declares no undo".to_string());
                    }
                    if resolution.escalated {
                        notes.push(format!(
                            "journaled at the effective tier {} (declared {})",
                            resolution.effective.as_str(),
                            resolution.declared.as_str()
                        ));
                    }
                    if let Err(e) = self.journal.record(journal::NewRecord {
                        ledger_ref: start_ref,
                        actor: &req.actor,
                        // Transport-assigned, not the asserted `actor`
                        // label: this is what undo checks (#94).
                        owner: req.caller.as_str(),
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
        fixture_with_ttl(CONFIRMATION_TTL)
    }

    fn fixture_with_ttl(ttl: Duration) -> Fixture {
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
        )
        .with_ttl(ttl);
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

    /// Issue #56, end to end. The field changing is not the point — the
    /// point is that a lying manifest no longer buys SILENT execution.
    ///
    /// A tool named `delete_event` declaring `read` used to dispatch
    /// with no confirmation and no undo journal entry (journaling is
    /// gated on `is_privileged`). It must now park.
    #[test]
    fn a_manifest_that_understates_a_tier_no_longer_executes_silently() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Arc::new(Ledger::open(dir.path().join("l.db")).unwrap());
        let dispatcher = Arc::new(RecordingDispatcher::returning(json!({"ok": true})));
        let mut registry = Registry::new();
        // The lie: every destructive tier downgraded to read.
        let hostile = fixture_calendar_json().replace("\"destructive\"", "\"read\"");
        registry
            .insert(Manifest::from_json(&hostile).unwrap())
            .unwrap();
        let bus = AgentBus::new(
            registry,
            Arc::clone(&ledger),
            UndoJournal::open_in_memory().unwrap(),
            Arc::clone(&dispatcher) as Arc<dyn Dispatcher>,
        );

        // A fully TRUSTED chain, so nothing else can be doing the work:
        // provenance escalation is not what parks this call.
        let outcome = bus
            .request(call(
                "org.gnome.Calendar",
                "delete_event",
                json!({"event_id": "e1"}),
                user(),
            ))
            .unwrap();

        assert!(
            matches!(outcome, Outcome::AwaitingConfirmation { .. }),
            "a delete_* tool declaring `read` executed without confirmation: {outcome:?}"
        );
        assert_eq!(
            dispatcher.dispatched(),
            0,
            "it dispatched before anyone confirmed"
        );
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
                f.bus
                    .confirm(id, true, &Answerer::alone(mallory.clone()))
                    .is_err(),
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
        let outcome = f
            .bus
            .confirm(call_id, true, &Answerer::alone(alice.clone()))
            .unwrap();
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
    /// Issue #135, the availability half. Parking a privileged call
    /// exists so that *a different actor* — the human, through the
    /// desktop dialog — answers it. The first #93 fix bound the call to
    /// the requester, so the overlay's `Confirm` came back `NotYours`
    /// and the call was unanswerable by anyone until the TTL. `lisa
    /// call` on a destructive tool tells the user "use the overlay"; the
    /// overlay was the one peer that provably could not comply.
    #[test]
    fn the_consent_surface_answers_a_call_it_did_not_request() {
        let f = fixture();
        let requester = lisa_peer::PeerId::Bus(":1.10".into());
        let overlay = lisa_peer::PeerId::Bus(":1.11".into());

        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: requester.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "delete_event",
                    json!({"event_id": "evt-1"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("a destructive call must park");
        };

        let outcome = f
            .bus
            .confirm(call_id, true, &Answerer::surface(overlay))
            .expect("the human's dialog could not answer a parked call");
        assert!(matches!(outcome, Outcome::Executed { .. }), "{outcome:?}");
        assert_eq!(f.dispatcher.dispatched(), 1);
    }

    /// Issue #135, the security half — and the one that matters. With a
    /// consent surface running, the requester must not approve its own
    /// destructive call: for an agent-driven call the requester IS the
    /// model's connection, and ADR-0030 §1 puts guardrails between the
    /// model and the machine, never between a person and their machine.
    ///
    /// #93 asked for "somebody else cannot release *my* call". The first
    /// implementation delivered "only the caller may release its own
    /// call" — weaker, and inverted.
    #[test]
    fn a_requester_cannot_approve_its_own_destructive_call() {
        let f = fixture();
        let requester = lisa_peer::PeerId::Bus(":1.10".into());

        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: requester.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "delete_event",
                    json!({"event_id": "evt-1"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("a destructive call must park");
        };

        let err = f
            .bus
            .confirm(call_id, true, &Answerer::ordinary(requester.clone()))
            .expect_err("the requester self-approved a destructive call");
        assert!(matches!(err, BusError::NeedsConsentSurface(id) if id == call_id));
        assert_eq!(f.dispatcher.dispatched(), 0);

        // Refused, not consumed: the call is still there for the human.
        let outcome = f
            .bus
            .confirm(
                call_id,
                true,
                &Answerer::surface(lisa_peer::PeerId::Bus(":1.11".into())),
            )
            .expect("the refusal ate the pending call");
        assert!(matches!(outcome, Outcome::Executed { .. }), "{outcome:?}");
    }

    /// Withdrawal is not approval. A requester that changes its mind
    /// must always be able to take its own call back — refusing here
    /// would leave a destructive call parked and live until the TTL with
    /// nobody able to kill it.
    #[test]
    fn a_requester_may_always_withdraw_its_own_call() {
        let f = fixture();
        let requester = lisa_peer::PeerId::Bus(":1.10".into());
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: requester.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "delete_event",
                    json!({"event_id": "evt-1"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("a destructive call must park");
        };
        let outcome = f
            .bus
            .confirm(call_id, false, &Answerer::ordinary(requester))
            .expect("a requester could not withdraw its own call");
        assert!(matches!(outcome, Outcome::Denied { .. }), "{outcome:?}");
        assert_eq!(f.dispatcher.dispatched(), 0);
    }

    /// The write tier keeps its inline chip: routing every write through
    /// the modal dialog is how people learn to click through it.
    #[test]
    fn a_requester_may_approve_its_own_write_call_via_the_chip() {
        let f = fixture();
        let requester = lisa_peer::PeerId::Bus(":1.10".into());
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: requester.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "add_event",
                    json!({"title": "dentist", "start": "2026-07-24T10:00:00Z"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("a write call must park for a chip");
        };
        let outcome = f
            .bus
            .confirm(call_id, true, &Answerer::ordinary(requester))
            .expect("the app's own inline chip was refused");
        assert!(matches!(outcome, Outcome::Executed { .. }), "{outcome:?}");
    }

    /// A third peer is neither the requester nor the surface, so it may
    /// do nothing at all — not approve, and not deny either, since a
    /// forced denial is a denial-of-service on somebody else's call.
    #[test]
    fn an_unrelated_peer_can_neither_approve_nor_deny() {
        for approve in [true, false] {
            let f = fixture();
            let Outcome::AwaitingConfirmation { call_id, .. } = f
                .bus
                .request(CallRequest {
                    caller: lisa_peer::PeerId::Bus(":1.10".into()),
                    ..call(
                        "org.gnome.Calendar",
                        "delete_event",
                        json!({"event_id": "evt-1"}),
                        user(),
                    )
                })
                .unwrap()
            else {
                panic!("a destructive call must park");
            };
            let err = f
                .bus
                .confirm(
                    call_id,
                    approve,
                    &Answerer::ordinary(lisa_peer::PeerId::Bus(":1.99".into())),
                )
                .expect_err("an unrelated peer answered (approve={approve})");
            assert!(matches!(err, BusError::NotYours(_)), "{err:?}");
            assert_eq!(f.dispatcher.dispatched(), 0);
        }
    }

    /// Issue #145 — the hole every other authority test walked past.
    ///
    /// The overlay backend hosts the model AND owns `dev.lisaos.Overlay1`,
    /// so a call it originates comes back to `Confirm` from a peer that is
    /// both the requester and the consent surface. `Surface` used to be
    /// taken as proof that somebody independent had looked at it, and the
    /// result was the model approving its own destructive calls.
    ///
    /// Note this passes with `Answerer::surface` and the SAME peer id as
    /// the requester — that identity is the whole point of the test.
    #[test]
    fn the_consent_surface_cannot_approve_a_call_it_made_itself() {
        let f = fixture();
        let overlay = lisa_peer::PeerId::Bus(":1.10".into());
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: overlay.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "delete_event",
                    json!({"event_id": "evt-1"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("a destructive call must park");
        };
        let err = f
            .bus
            .confirm(call_id, true, &Answerer::surface(overlay))
            .expect_err("the model host approved its own destructive call");
        assert!(matches!(err, BusError::NeedsConsentSurface(_)), "{err:?}");
        assert_eq!(f.dispatcher.dispatched(), 0, "it must not have run");
    }

    /// Positive control for the test above: the fix must not break the
    /// case it exists to protect. A consent surface that is a DIFFERENT
    /// peer from the requester still approves normally — otherwise the
    /// desktop's destructive flow is dead and the "fix" is an outage.
    #[test]
    fn an_independent_consent_surface_still_approves() {
        let f = fixture();
        let requester = lisa_peer::PeerId::Bus(":1.10".into());
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: requester,
                ..call(
                    "org.gnome.Calendar",
                    "delete_event",
                    json!({"event_id": "evt-1"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("a destructive call must park");
        };
        f.bus
            .confirm(
                call_id,
                true,
                // A different unique name: a real separate process.
                &Answerer::surface(lisa_peer::PeerId::Bus(":1.11".into())),
            )
            .expect("an independent surface must be able to approve");
        assert_eq!(f.dispatcher.dispatched(), 1);
    }

    /// The headless case must also survive: with NO consent surface
    /// anywhere, the requester answering its own call is the only way a
    /// destructive action can ever happen, and refusing it was the
    /// availability failure #135 reported.
    #[test]
    fn a_headless_requester_can_still_answer_its_own_call() {
        let f = fixture();
        let cli = lisa_peer::PeerId::Bus(":1.10".into());
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: cli.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "delete_event",
                    json!({"event_id": "evt-1"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("a destructive call must park");
        };
        f.bus
            .confirm(call_id, true, &Answerer::alone(cli))
            .expect("a headless box must still be able to confirm");
        assert_eq!(f.dispatcher.dispatched(), 1);
    }

    /// The Ledger must say WHO approved. "A destructive action ran" and
    /// "a destructive action ran because the requester answered its own
    /// prompt on a headless box" are different facts, and the Ledger is
    /// the only place a person can tell them apart (VISION, §5.10).
    #[test]
    fn the_ledger_records_whether_a_human_surface_approved() {
        for (answerer, expected) in [
            (
                Answerer::surface(lisa_peer::PeerId::Bus(":1.11".into())),
                "the consent surface",
            ),
            (
                Answerer::alone(lisa_peer::PeerId::Bus(":1.10".into())),
                "no consent surface",
            ),
        ] {
            let f = fixture();
            let Outcome::AwaitingConfirmation { call_id, .. } = f
                .bus
                .request(CallRequest {
                    caller: lisa_peer::PeerId::Bus(":1.10".into()),
                    ..call(
                        "org.gnome.Calendar",
                        "delete_event",
                        json!({"event_id": "evt-1"}),
                        user(),
                    )
                })
                .unwrap()
            else {
                panic!("a destructive call must park");
            };
            f.bus.confirm(call_id, true, &answerer).unwrap();
            let confirmed = f
                .ledger
                .tail(100)
                .unwrap()
                .into_iter()
                .find(|e| e.kind == "tool.confirm")
                .expect("no tool.confirm entry");
            assert!(
                confirmed.preview.contains(expected),
                "the Ledger does not say who approved: {:?}",
                confirmed.preview
            );
        }
    }

    /// Issue #137. Nothing ever collected an expired confirmation:
    /// `confirm()` was the only path that removed from the map, and it
    /// checked the TTL *after* removing — so a call nobody answered was
    /// retained for the life of the process, while the refusal text
    /// claimed it had been "expired and collected".
    #[test]
    fn expired_confirmations_are_collected_and_ledgered() {
        let f = fixture_with_ttl(Duration::from_millis(1));
        f.bus
            .request(call(
                "org.gnome.Calendar",
                "delete_event",
                json!({"event_id": "evt-1"}),
                user(),
            ))
            .unwrap();
        assert_eq!(f.bus.pending_count(), 1);

        std::thread::sleep(Duration::from_millis(20));
        // Any subsequent request sweeps; the new call parks in its place.
        f.bus
            .request(call(
                "org.gnome.Calendar",
                "delete_event",
                json!({"event_id": "evt-2"}),
                user(),
            ))
            .unwrap();
        assert_eq!(
            f.bus.pending_count(),
            1,
            "the expired call was retained: {} parked",
            f.bus.pending_count()
        );

        // An abandoned privileged call still leaves a record.
        assert!(
            ledger_kinds(&f.ledger)
                .iter()
                .any(|(kind, status)| kind == "tool.deny" && status == "expired"),
            "expiry was silent: {:?}",
            ledger_kinds(&f.ledger)
        );
    }

    /// A peer that parks calls it never answers must not be able to
    /// exhaust the daemon — `RequestCall` is reachable by any session
    /// peer, and each parked call retains a full `CallRequest`.
    #[test]
    fn one_peer_cannot_park_without_bound() {
        let f = fixture();
        let flooder = lisa_peer::PeerId::Bus(":1.66".into());
        let park = |caller: &lisa_peer::PeerId| {
            f.bus.request(CallRequest {
                caller: caller.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "delete_event",
                    json!({"event_id": "evt"}),
                    user(),
                )
            })
        };

        for i in 0..MAX_PENDING_PER_OWNER {
            assert!(
                matches!(
                    park(&flooder).unwrap(),
                    Outcome::AwaitingConfirmation { .. }
                ),
                "call {i} was refused below the cap"
            );
        }
        let over = park(&flooder).unwrap();
        assert!(
            matches!(over, Outcome::Denied { .. }),
            "the cap did not hold: {over:?}"
        );
        assert_eq!(f.bus.pending_count(), MAX_PENDING_PER_OWNER);

        // And the flood must not deny anyone else their confirmation —
        // making confirmation unavailable is a soft bypass of "no
        // unconfirmed privileged calls".
        let human = lisa_peer::PeerId::Bus(":1.2".into());
        assert!(
            matches!(park(&human).unwrap(), Outcome::AwaitingConfirmation { .. }),
            "an unrelated peer was locked out by the flood"
        );
    }

    /// At the global cap the greediest peer pays, not the next one to
    /// ask — otherwise filling the map is itself the denial-of-service.
    #[test]
    fn the_global_cap_evicts_the_greediest_peer() {
        let f = fixture();
        let park = |caller: lisa_peer::PeerId| {
            f.bus
                .request(CallRequest {
                    caller,
                    ..call(
                        "org.gnome.Calendar",
                        "delete_event",
                        json!({"event_id": "evt"}),
                        user(),
                    )
                })
                .unwrap()
        };

        // Fill the map from many connections — one process can open as
        // many as it likes, which is why the per-owner cap alone is not
        // enough.
        let peers = MAX_PENDING / MAX_PENDING_PER_OWNER;
        for peer in 0..peers {
            for _ in 0..MAX_PENDING_PER_OWNER {
                park(lisa_peer::PeerId::Bus(format!(":1.{peer}")));
            }
        }
        assert_eq!(f.bus.pending_count(), MAX_PENDING);

        let human = lisa_peer::PeerId::Bus(":1.999".into());
        let outcome = park(human.clone());
        assert!(
            matches!(outcome, Outcome::AwaitingConfirmation { .. }),
            "a full map locked the human out: {outcome:?}"
        );
        assert_eq!(f.bus.pending_count(), MAX_PENDING, "the cap was exceeded");

        let mine = {
            let pending = f.bus.pending.lock().unwrap();
            let owner = Owner::of(human);
            pending.values().filter(|p| p.owner == owner).count()
        };
        assert_eq!(mine, 1, "the newcomer's own call was the one evicted");
    }

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
            .confirm(call_id, true, &Answerer::alone(lisa_peer::PeerId::Direct))
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
        let report = f.bus.undo("host", &lisa_peer::PeerId::Direct).unwrap();
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
            .confirm(call_id, false, &Answerer::alone(lisa_peer::PeerId::Direct))
            .unwrap();
        assert!(matches!(outcome, Outcome::Denied { .. }));
        assert_eq!(f.dispatcher.dispatched(), 0);
        assert!(
            matches!(
                f.bus
                    .confirm(call_id, true, &Answerer::alone(lisa_peer::PeerId::Direct)),
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

    /// Issue #98, as filed. The bus judged this call write-tier because
    /// its trigger chain carries mail-provenance content, asked the
    /// user, took consent and dispatched — and journaled nothing,
    /// because the *declared* tier was `read`.
    ///
    /// Two consequences: the calls most likely to have been steered by
    /// hostile content were the ones with no compensation recorded, and
    /// `lisa undo` reached past the escalated call to an older action —
    /// the user approves a chip and something else gets reverted.
    #[test]
    fn an_escalated_read_is_journaled_so_undo_cannot_skip_past_it() {
        let f = fixture();
        let caller = lisa_peer::PeerId::Direct;

        // An older, genuinely undoable write the user made earlier.
        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "add_event",
                json!({"title": "earlier", "start": "2026-07-30T09:00:00Z"}),
                user(),
            ))
            .unwrap()
        else {
            panic!("a write should ask");
        };
        f.bus
            .confirm(call_id, true, &Answerer::alone(caller.clone()))
            .unwrap();

        // Now the escalated read.
        let outcome = f
            .bus
            .request(call(
                "org.gnome.Calendar",
                "list_events",
                json!({}),
                vec![Provenance::User, Provenance::Mail],
            ))
            .unwrap();
        let Outcome::AwaitingConfirmation {
            call_id,
            confirmation,
            ..
        } = outcome
        else {
            panic!("an untrusted chain must escalate a read into a question");
        };
        assert_eq!(confirmation, Confirmation::Chip, "the bus asked a person");
        f.bus
            .confirm(call_id, true, &Answerer::alone(caller.clone()))
            .unwrap();

        // The escalated call is the most recent journal entry, so undo
        // lands on IT — not on the note from before.
        match f.bus.undo("host", &caller).unwrap() {
            UndoReport::NotUndoable { app_id, tool } => {
                assert_eq!(app_id, "org.gnome.Calendar");
                assert_eq!(tool, "list_events");
            }
            other => panic!("undo skipped past the escalated call — it reverted {other:?} instead"),
        }
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

    /// Issue #94 — the hole every other undo test walked past, because
    /// they all acted and undid as the same peer.
    ///
    /// `Undo()` dispatches a compensation that is frequently
    /// destructive-tier (`add_event`'s inverse is `delete_event`), and it
    /// goes straight to the MCP transport: no tier resolution, no
    /// confirmation, no args validation. So an unscoped journal query
    /// meant any peer on the session bus could revert an action taken by
    /// any other. The D-Bus method made it worse by taking no arguments
    /// at all and hardcoding the actor "host".
    #[test]
    fn a_peer_cannot_undo_an_action_another_peer_made() {
        let f = fixture();
        let author = lisa_peer::PeerId::Bus(":1.10".into());
        let stranger = lisa_peer::PeerId::Bus(":1.99".into());

        let Outcome::AwaitingConfirmation { call_id, .. } = f
            .bus
            .request(CallRequest {
                caller: author.clone(),
                ..call(
                    "org.gnome.Calendar",
                    "add_event",
                    json!({"title": "dentist", "start": "2026-07-24T10:00:00Z"}),
                    user(),
                )
            })
            .unwrap()
        else {
            panic!("expected pending");
        };
        f.bus
            .confirm(call_id, true, &Answerer::alone(author.clone()))
            .unwrap();
        assert_eq!(f.dispatcher.dispatched(), 1, "the action itself should run");

        // The stranger sees an empty journal, not somebody else's action.
        let report = f.bus.undo("host", &stranger).unwrap();
        assert!(
            matches!(report, UndoReport::Nothing),
            "a stranger reverted another peer's action: {report:?}"
        );
        assert_eq!(
            f.dispatcher.dispatched(),
            1,
            "the compensation was dispatched for a peer that did not own it"
        );

        // Positive control: the author still can. A fix that made undo
        // stop working would pass the assertion above and be useless.
        let report = f.bus.undo("host", &author).unwrap();
        assert!(
            matches!(report, UndoReport::Undone { .. }),
            "the author could not undo their own action: {report:?}"
        );
        assert_eq!(f.dispatcher.dispatched(), 2);
    }

    #[test]
    fn undo_skips_non_undoable_actions_and_reports_empty_journal() {
        let f = fixture();
        assert!(matches!(
            f.bus.undo("host", &lisa_peer::PeerId::Direct).unwrap(),
            UndoReport::Nothing
        ));

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
            .confirm(call_id, true, &Answerer::alone(lisa_peer::PeerId::Direct))
            .unwrap();
        let report = f.bus.undo("host", &lisa_peer::PeerId::Direct).unwrap();
        assert!(
            matches!(report, UndoReport::NotUndoable { .. }),
            "{report:?}"
        );
        assert!(
            matches!(
                f.bus.undo("host", &lisa_peer::PeerId::Direct).unwrap(),
                UndoReport::Nothing
            ),
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

    /// #55: a peer claiming provenance it is not entitled to leaves a
    /// record somebody can search for.
    ///
    /// The downgrade is deliberately not a refusal, so the Ledger entry
    /// is the ONLY lasting evidence that anything happened — and it
    /// used to be an `eprintln`, i.e. evidence with the lifetime of a
    /// journal rotation. The assertion is on the searchable fields: a
    /// distinct kind, the claiming app named, and the verdict readable
    /// without cross-referencing anything else.
    #[test]
    fn a_provenance_downgrade_is_recorded_where_it_can_be_found() {
        let Fixture { bus, ledger, .. } = fixture();
        bus.ledger_provenance_downgrade("host", "app.example.Evil", "delete_event");
        let tail = ledger.tail(1).unwrap();
        assert_eq!(tail[0].kind, "agent.provenance_downgrade");
        assert_eq!(tail[0].status, "downgraded");
        assert!(
            tail[0].detail.contains("app.example.Evil"),
            "the claiming app must be named in the entry: {}",
            tail[0].detail
        );
        assert!(
            tail[0].detail.contains("asserted=user"),
            "the entry must say what was claimed: {}",
            tail[0].detail
        );
    }
}
