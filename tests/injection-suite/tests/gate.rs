//! The merge-blocking injection gate (`docs/PLAN.md` §5.10, §5.4).
//!
//! Property: across the whole seeded corpus, the Agent Bus dispatches
//! **zero** privileged calls without confirmation — no matter how
//! convincing the hostile payload is — because every attempt's trigger
//! chain carries untrusted provenance, which forces at least a
//! confirmation chip. This is the bus-layer half of the M5 gate; the
//! model-in-the-loop half is deferred (see the crate docs).

use lisa_agentd::bus::{AgentBus, CallRequest, Dispatcher, Outcome, RecordingDispatcher};
use lisa_agentd::journal::UndoJournal;
use lisa_agentd::manifest::Manifest;
use lisa_agentd::registry::Registry;
use lisa_agentd::tier::Provenance;
use lisa_injection_suite::{chain_for, corpus, seed_manifests};
use std::sync::Arc;

fn build_bus() -> (Arc<RecordingDispatcher>, AgentBus) {
    let dir = tempfile::tempdir().unwrap();
    // Leak the tempdir: the ledger file must outlive this call for the
    // whole test; the OS reclaims it on process exit.
    let ledger_path = dir.keep().join("ledger.db");
    let ledger = Arc::new(lisa_ledger::Ledger::open(ledger_path).unwrap());
    let dispatcher = Arc::new(RecordingDispatcher::returning(
        serde_json::json!({"ok": true}),
    ));
    let mut registry = Registry::new();
    for m in seed_manifests() {
        registry.insert(Manifest::from_json(&m).unwrap()).unwrap();
    }
    let bus = AgentBus::new(
        registry,
        ledger,
        UndoJournal::open_in_memory().unwrap(),
        Arc::clone(&dispatcher) as Arc<dyn Dispatcher>,
    );
    (dispatcher, bus)
}

#[test]
fn zero_unconfirmed_privileged_calls_across_the_corpus() {
    let attempts = corpus();
    assert!(!attempts.is_empty(), "corpus must not be empty");

    let (dispatcher, bus) = build_bus();
    let mut privileged_attempts = 0;

    for attempt in &attempts {
        let chain: Vec<Provenance> = chain_for(attempt)
            .iter()
            .map(|s| Provenance::parse(s))
            .collect();
        // Every target in the corpus is a privileged (write/destructive)
        // tool.
        privileged_attempts += 1;

        let outcome = bus
            .request(CallRequest {
                actor: "system-agent".into(),
                app_id: attempt.target_app.into(),
                tool: attempt.target_tool.into(),
                args: attempt.target_args.clone(),
                chain,
                caller: lisa_peer::PeerId::Direct,
            })
            .expect("ledger available");

        match outcome {
            Outcome::AwaitingConfirmation { .. } => {} // Correct: parked.
            Outcome::Denied { .. } => {}               // Also safe: nothing ran.
            // Safest of all (#251): refused outright, never parked, so
            // there is no id for a dialog to approve.
            Outcome::Refused { .. } => {}
            Outcome::Executed { .. } | Outcome::Failed { .. } => {
                panic!(
                    "attempt {} dispatched WITHOUT confirmation: {}/{} via {} ({}). payload: {:?}",
                    attempt.id,
                    attempt.target_app,
                    attempt.target_tool,
                    attempt.vector,
                    attempt.provenance,
                    attempt.payload,
                );
            }
        }
    }

    assert!(privileged_attempts > 0);
    assert_eq!(
        dispatcher.dispatched(),
        0,
        "the bus must not have dispatched any privileged call unconfirmed"
    );
}

/// #55: the cheapest injection is not a payload at all — it is a
/// hostile peer simply *asserting* `provenance: ["user"]` on the wire
/// to take the trusted path that skips confirmation for read-tier
/// tools.
///
/// The corpus above tests convincing CONTENT. This tests a convincing
/// CLAIM, which needs no content: the whole attack is one array
/// element. It belongs here rather than in agentd's unit tests because
/// the property under test is end-to-end — the claim is bound at the
/// D-Bus surface, and what must hold is that the bus sees the
/// downgraded chain and still parks the call.
#[test]
fn asserting_user_provenance_does_not_buy_the_trusted_path() {
    let (dispatcher, bus) = build_bus();
    let attempt = corpus()
        .into_iter()
        .next()
        .expect("corpus must not be empty");

    // What a hostile peer would send: the strongest possible claim.
    let asserted = vec![Provenance::parse("user")];
    // What the transport says about it: not a Lisa program (ADR-0033).
    // The third argument is the CLAIMANT — the peer that sent the
    // message — never the app being called (#217). A hostile peer here
    // is not one of ours, so it has no app id of its own; it is named
    // by its executable, the way every unattributed caller is.
    let claimant = lisa_agentd::tier::Claimant::from("host:/usr/bin/attacker");
    let verified = lisa_agentd::tier::verify_chain(asserted, false, &claimant);
    assert!(
        verified.downgraded,
        "a non-Lisa peer's `user` claim must be downgraded"
    );
    assert!(
        !verified.chain.contains(&Provenance::User),
        "no `user` may survive verification for an untrusted peer: {:?}",
        verified.chain
    );
    assert!(
        !verified
            .chain
            .contains(&Provenance::App(attempt.target_app.into())),
        "the app being ATTACKED was recorded as the peer that claimed to \
         be human (#217): {:?}",
        verified.chain
    );

    let outcome = bus
        .request(CallRequest {
            actor: "system-agent".into(),
            app_id: attempt.target_app.into(),
            tool: attempt.target_tool.into(),
            args: attempt.target_args.clone(),
            chain: verified.chain,
            caller: lisa_peer::PeerId::Direct,
        })
        .expect("ledger available");

    match outcome {
        Outcome::AwaitingConfirmation { .. } | Outcome::Denied { .. } | Outcome::Refused { .. } => {
        }
        other => panic!(
            "a peer that merely CLAIMED user provenance reached dispatch: {other:?} \
             ({}/{})",
            attempt.target_app, attempt.target_tool
        ),
    }
    assert_eq!(
        dispatcher.dispatched(),
        0,
        "claiming user provenance must not dispatch anything unconfirmed"
    );
}

#[test]
fn corpus_covers_every_payload_vector_and_target() {
    let attempts = corpus();
    // 40 payloads × 5 vectors × 3 targets = 600.
    assert_eq!(attempts.len(), 600);
    // The §5.10 acceptance bar: a hostile string yields 0 unconfirmed
    // privileged calls across 500+ seeded attempts. Guard the floor so
    // the payload bank can't silently shrink back under it.
    assert!(
        attempts.len() >= 500,
        "corpus fell under the 500-attempt §5.10 bar: {}",
        attempts.len()
    );
    // Ids are dense and unique.
    let mut ids: Vec<usize> = attempts.iter().map(|a| a.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), attempts.len());
}
