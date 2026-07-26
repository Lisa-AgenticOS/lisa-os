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
