//! The fork-and-exec-gjs attack, and the executable that stops it
//! (#289, ADR-0030 §2, ADR-0033, CLAUDE.md 6b).
//!
//! # The attack
//!
//! `dev.lisaos.Consent1` is D-Bus-activatable, so ordinarily nobody owns
//! it, and `session.conf` ships `<allow own="*"/>`, so the name goes to
//! whoever calls `RequestName` first. #289 closed the two obvious walks
//! around that — the requester's own connection (`consent.self_approval`)
//! and a *second* connection of the requester's process
//! (`consent.same_process`) — by making agentd ask the kernel what
//! **program** is behind the answering connection.
//!
//! Which left the program list. The shipped dialog was
//! `Exec=/usr/bin/lisa-app consent/lisa-consentd.js`, and `lisa-app`
//! ends in `exec gjs -m "$found"`, so agentd had to trust
//! `/usr/bin/gjs` — an interpreter. **An interpreter on an allowlist
//! authorises every program that interpreter can run.** So:
//!
//! 1. the model's host parks a destructive call;
//! 2. it `fork()`s and `exec`s `gjs` on a script of its own choosing —
//!    the child gets a **fresh pid**, so `consent.same_process` does not
//!    fire, and a **fresh connection**, so `consent.self_approval` does
//!    not either;
//! 3. the child takes `dev.lisaos.Consent1`;
//! 4. `/proc/<child>/exe` is `/usr/bin/gjs-console`, which is on the
//!    allowlist, so agentd calls it the human's dialog;
//! 5. it approves the call, and the Ledger records that the consent
//!    surface said yes.
//!
//! Step 4 was measured on the reference device, not assumed: a process
//! that forks and execs `gjs` reports `/usr/bin/gjs-console`, exactly as
//! the shipped dialog did (pid 18669).
//!
//! # What is real here and what is not
//!
//! Everything except the kernel. `AgentBus::confirm` and
//! `lisa_guard::judge_approval` are the shipped code; the exe→bool step
//! is the shipped `lisa_peer::manager` pair that `agentd`'s
//! `exe_is_consent_program` calls, run against a tempdir standing in for
//! `/usr/bin`. What a real `/proc/<pid>/exe` reports for a
//! fork+exec-gjs child is the device measurement above.
//!
//! # The fix these tests pin
//!
//! `/usr/bin/lisa-consentd` — a binary that owns the name **itself** and
//! spawns the GJS dialog as a child (`shell/consent/daemon`). A native
//! launcher that `exec`s the GJS surface would change nothing: after
//! `execve` the exe is `gjs` again. The process that owns the name has
//! to be the binary.
//!
//! Shipping it is necessary and **not sufficient**: while
//! `/usr/bin/gjs` is still on `CONSENT_SURFACE_PROGRAMS`, the attack
//! above still works, because the allowlist is a disjunction.
//! `the_shipped_allowlist_still_admits_an_interpreter` below records
//! exactly which entry is left to remove.

use lisa_agentd::bus::{AgentBus, Answerer, CallRequest, Dispatcher, Outcome, RecordingDispatcher};
use lisa_agentd::journal::UndoJournal;
use lisa_agentd::manifest::Manifest;
use lisa_agentd::registry::Registry;
use lisa_agentd::tier::Provenance;
use lisa_peer::PeerId;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The model's host: the connection that parks the call.
fn loop_peer() -> PeerId {
    PeerId::Bus(":1.42".into())
}

fn loop_process() -> lisa_peer::Process {
    lisa_peer::Process::unpinned(4242)
}

/// The forked child. A different pid and a different connection — which
/// is the whole trick, because those are the two facts #289 added.
fn forked_peer() -> PeerId {
    PeerId::Bus(":1.77".into())
}

fn forked_process() -> lisa_peer::Process {
    lisa_peer::Process::unpinned(4277)
}

struct Fixture {
    bus: Arc<AgentBus>,
    dispatcher: Arc<RecordingDispatcher>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.keep().join("ledger.db")).unwrap());
    let dispatcher = Arc::new(RecordingDispatcher::returning(json!({"ok": true})));
    let mut registry = Registry::new();
    registry
        .insert(
            Manifest::from_json(
                &json!({
                    "lisa_manifest": 1,
                    "app_id": "app.lisaos.notes",
                    "mcp": { "transport": "unix", "activatable": true },
                    "tools": [
                        { "name": "delete_note", "tier": "destructive",
                          "description": "Delete a note",
                          "input_schema": { "type": "object", "required": ["note_id"],
                              "properties": { "note_id": {"type":"string"} } } }
                    ]
                })
                .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
    Fixture {
        bus: Arc::new(AgentBus::new(
            registry,
            ledger,
            UndoJournal::open_in_memory().unwrap(),
            Arc::clone(&dispatcher) as Arc<dyn Dispatcher>,
        )),
        dispatcher,
    }
}

/// A destructive call parked by the process hosting the model.
fn park_a_destructive_call(bus: &AgentBus) -> u64 {
    let outcome = bus
        .request(CallRequest {
            actor: "assistant".into(),
            app_id: "app.lisaos.notes".into(),
            tool: "delete_note".into(),
            args: json!({"note_id": "everything"}),
            chain: vec![Provenance::parse("user")],
            caller: loop_peer(),
            requester_hosts_a_model: true,
            requester_process: Some(loop_process()),
        })
        .expect("the destructive call should park, not fail");
    match outcome {
        Outcome::AwaitingConfirmation { call_id, .. } => call_id,
        other => panic!("a destructive call did not park: {other:?}"),
    }
}

/// Put a fake program on disk so `resolve_managers` can canonicalise it.
fn program(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, b"#!/bin/sh\n").unwrap();
    p.canonicalize().unwrap()
}

/// `daemons/agentd/src/dbus.rs::exe_is_consent_program`, over an
/// allowlist a test can choose.
///
/// Reimplemented rather than imported because the agentd function is
/// private and takes the shipped constant. It is two lines and they are
/// the shipped helpers: if `may_manage`'s comparison ever loosens, this
/// loosens with it, which is the property worth having.
fn is_consent_program(answerer_exe: &Path, allowlist: &[PathBuf]) -> bool {
    lisa_peer::manager::may_manage(
        true,
        Some(answerer_exe),
        &lisa_peer::manager::resolve_managers(allowlist),
    )
    .is_ok()
}

/// The whole chain, from "what program is the answerer running" to "did
/// the destructive call run".
///
/// Returns `(refusal rule, dispatched)`. `None` for the rule means the
/// approval went through, or that it was refused as `NotYours` — which
/// carries no rule on purpose, because telling a stranger *why* it may
/// not answer call 41 confirms that call 41 exists (#93, #131).
fn attempt(answerer_exe: &Path, allowlist: &[PathBuf]) -> (Option<&'static str>, usize) {
    let f = fixture();
    let call_id = park_a_destructive_call(&f.bus);
    assert_eq!(f.dispatcher.dispatched(), 0, "a destructive call executed");

    // Exactly what `dbus.rs::confirm` builds: the name from the broker,
    // the program from the kernel, the process from the pidfd.
    let answerer = if is_consent_program(answerer_exe, allowlist) {
        Answerer::surface(forked_peer())
    } else {
        // It holds the name — `RequestName` succeeded — but it is not
        // running a consent-surface program.
        Answerer::name_squatter(forked_peer())
    }
    .from_process(Some(forked_process()));

    let rule = match f.bus.confirm(call_id, true, &answerer) {
        Ok(_) => None,
        Err(e) => e.rule(),
    };
    (rule, f.dispatcher.dispatched())
}

/// **The attack, against the tree as it shipped.** The child forked from
/// the model's host, running `gjs`, releases the model's own destructive
/// call — and the bus dispatches it.
///
/// This is the test that has to go from green to red-by-inversion: it
/// asserts the *defect*, so that the day the allowlist stops naming an
/// interpreter, `the_forked_gjs_child_is_refused_once_the_dialog_has_its_own_binary`
/// below is what describes the machine.
#[test]
fn a_forked_gjs_child_approves_the_models_own_destructive_call() {
    let dir = tempfile::tempdir().unwrap();
    // The allowlist as `CONSENT_SURFACE_PROGRAMS` ships it: the
    // dedicated binary (not installed, so `resolve_managers` drops it)
    // and the interpreter (installed).
    let gjs = program(dir.path(), "gjs-console");
    let allowlist = vec![dir.path().join("lisa-consentd"), gjs.clone()];

    let (rule, dispatched) = attempt(&gjs, &allowlist);
    assert_eq!(
        rule, None,
        "the forked child was refused — the attack no longer works this way"
    );
    assert_eq!(
        dispatched, 1,
        "the destructive call did NOT run; the attack is supposed to succeed here"
    );
}

/// **The same attack, once the dialog has an executable of its own.**
///
/// Note the rule: `None`, and deliberately so. The forked child is a
/// stranger to this call — not the requester's connection, not the
/// requester's process, and not a verified surface — so it lands in
/// `ApprovalVerdict::NotYours`, which carries no rule and no reason
/// because a refusal that explained itself would confirm the call
/// exists (#93, #131). What proves the refusal is `dispatched == 0`.
#[test]
fn the_forked_gjs_child_is_refused_once_the_dialog_has_its_own_binary() {
    let dir = tempfile::tempdir().unwrap();
    let gjs = program(dir.path(), "gjs-console");
    let consentd = program(dir.path(), "lisa-consentd");
    // The allowlist with the interpreter removed — one entry, and it is
    // a program only we ship.
    let allowlist = vec![consentd];

    let (rule, dispatched) = attempt(&gjs, &allowlist);
    assert_eq!(
        rule, None,
        "a squatter must learn nothing at all, not even a rule id"
    );
    assert_eq!(
        dispatched, 0,
        "the forked gjs child still released the model's destructive call"
    );
}

/// The other half, or the feature does not exist: the real dialog CAN
/// release the call. A guardrail with no permitted path is an outage,
/// and on this surface an outage means a destructive call can be parked
/// and withdrawn but never approved.
#[test]
fn the_dialogs_own_binary_can_still_approve() {
    let dir = tempfile::tempdir().unwrap();
    let consentd = program(dir.path(), "lisa-consentd");
    let allowlist = vec![consentd.clone()];

    let (rule, dispatched) = attempt(&consentd, &allowlist);
    assert_eq!(rule, None);
    assert_eq!(
        dispatched, 1,
        "the consent surface could not approve a destructive call — this is an outage, \
         not a guardrail"
    );
}

/// A `lisa-consentd` that merely `exec`s the GJS dialog would not help,
/// and this says why in an assertion: after `execve` the kernel reports
/// the *interpreter*, not the launcher. The exe is the only thing agentd
/// can read, so the launcher's identity is gone the instant it execs.
///
/// This is the reason `shell/consent/daemon` spawns the dialog as a
/// CHILD over a pipe and keeps the bus name itself.
#[test]
fn a_launcher_that_execs_gjs_is_indistinguishable_from_the_attacker() {
    let dir = tempfile::tempdir().unwrap();
    let gjs = program(dir.path(), "gjs-console");
    let consentd = program(dir.path(), "lisa-consentd");
    let allowlist = vec![consentd];

    // What the kernel reports after `exec gjs -m dialog.js`, whoever
    // did the exec'ing: `gjs-console`. Identical to the attacker's.
    let (_, dispatched) = attempt(&gjs, &allowlist);
    assert_eq!(
        dispatched, 0,
        "an exec'ing launcher was distinguishable from the attacker, which cannot be true"
    );
}

/// What is left to do, as an assertion rather than a paragraph.
///
/// `CONSENT_SURFACE_PROGRAMS` is a disjunction, so shipping
/// `/usr/bin/lisa-consentd` closes nothing on its own — the interpreter
/// entry beside it still admits the forked child. The remaining change
/// is one line in `daemons/agentd/src/dbus.rs`:
///
/// ```text
/// -pub const CONSENT_SURFACE_PROGRAMS: [&str; 2] = ["/usr/bin/lisa-consentd", "/usr/bin/gjs"];
/// +pub const CONSENT_SURFACE_PROGRAMS: [&str; 1] = ["/usr/bin/lisa-consentd"];
/// ```
///
/// Written as a test that *fails when the fix lands*, on purpose. A
/// comment saying "remember to remove gjs" is a comment nobody reads; a
/// red test with this name is the one thing that makes the follow-up
/// unmissable, and the fix is to delete this test and keep the two
/// above.
#[test]
fn the_shipped_allowlist_still_admits_an_interpreter() {
    let interpreters: Vec<&str> = lisa_agentd::dbus::CONSENT_SURFACE_PROGRAMS
        .iter()
        .copied()
        .filter(|p| p.ends_with("/gjs") || p.ends_with("/gjs-console") || p.ends_with("/python3"))
        .collect();
    assert_eq!(
        interpreters,
        vec!["/usr/bin/gjs"],
        "the consent allowlist changed. If it no longer names an interpreter, #289 is \
         CLOSED — delete this test. If it names a different one, the hole moved."
    );
    assert!(
        lisa_agentd::dbus::CONSENT_SURFACE_PROGRAMS.contains(&"/usr/bin/lisa-consentd"),
        "the dedicated binary must stay on the list, or the dialog cannot approve anything"
    );
}
