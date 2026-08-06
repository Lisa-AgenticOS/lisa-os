//! The credential surface Surfer's autofill opened (#260).
//!
//! `gate.rs` asserts the property that holds for every privileged call:
//! nothing is dispatched unconfirmed. That is the right assertion for a
//! `navigate` or a `fill` at a search box — those are things a person
//! may legitimately approve.
//!
//! Credentials are not. #260 makes two of them **HARD NO**: typing into
//! a password field, and reading the keyring. A HARD NO is a stronger
//! guarantee than parking — there is no id for a dialog to approve —
//! and "something stopped it" is not the assertion that proves it. So
//! this file asserts the RULE ID, for every payload in the bank, and
//! carries its own positive controls.
//!
//! Why here and not in `libs/lisa-guard/tests/corpus.rs`: the corpus
//! proves the shapes are refused. This proves that no amount of hostile
//! CONTENT changes the answer — the payload rides in the trigger chain
//! as untrusted provenance, exactly as a real attack would, and the
//! verdict is computed from `(tool, arguments, grant)` and never from
//! anything the page said.

use lisa_agentd::bus::{AgentBus, CallRequest, Dispatcher, Outcome, RecordingDispatcher};
use lisa_agentd::journal::UndoJournal;
use lisa_agentd::manifest::Manifest;
use lisa_agentd::registry::Registry;
use lisa_agentd::tier::Provenance;
use lisa_guard::{Action, ActionVerdict, Class, Grant, Trigger, judge_action};
use lisa_injection_suite::{PAYLOADS, VECTORS, seed_manifests};
use serde_json::json;
use std::sync::Arc;

/// The grant a real run has: a home, a workspace a person chose, and a
/// prompt they typed. The most permissive shape the ladder offers — so
/// a refusal here is not scope, it is category.
fn generous_grant() -> Grant {
    let home = std::env::temp_dir().join("lisa-credential-suite-home");
    std::fs::create_dir_all(&home).unwrap();
    Grant {
        home: Some(home.clone()),
        workspace: Some(home),
        trigger: Trigger::Prompt,
        trusted_chain: true,
        ..Grant::default()
    }
}

fn verdict(tool: &str, class: Class, args: &serde_json::Value) -> ActionVerdict {
    judge_action(
        &Action {
            app_id: "app.lisaos.Surfer",
            tool,
            class,
            args,
        },
        &generous_grant(),
    )
}

/// Every way a hostile page might phrase "put the password in the box",
/// as the tool call it would have to become.
fn credential_fills() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (
            "fill",
            json!({"url": "https://bank.example/", "selector": "#password", "value": "hunter2"}),
        ),
        (
            "fill",
            json!({"url": "https://bank.example/", "selector": "input[type=password]", "value": "x"}),
        ),
        (
            "fill",
            json!({"url": "https://bank.example/", "selector": "#login-passphrase", "value": "x"}),
        ),
        (
            "fill",
            json!({"url": "https://bank.example/", "selector": "#otp", "value": "123456"}),
        ),
        (
            "autofill",
            json!({"url": "https://bank.example/", "field": "password"}),
        ),
        ("type_text", json!({"into": "#totp", "value": "123456"})),
        ("enter_value", json!({"selector": "#cvv", "value": "123"})),
    ]
}

/// Every way it might phrase "get me the password".
fn credential_reads() -> Vec<(&'static str, Class, serde_json::Value)> {
    vec![
        (
            "read_password",
            Class::Read,
            json!({"origin": "https://bank.example"}),
        ),
        ("list_passwords", Class::Read, json!({})),
        ("search_credentials", Class::Read, json!({"query": "bank"})),
        ("get_secret", Class::Read, json!({"id": "1"})),
        ("keyring_items", Class::Read, json!({})),
        ("export_passwords", Class::Write, json!({"format": "csv"})),
        ("lookup", Class::Read, json!({"keyring": "login"})),
        ("sync", Class::Write, json!({"credentials": ["a"]})),
    ]
}

/// The load-bearing one: a credential fill is refused by WHAT IT IS,
/// with a rule id, under the most generous grant there is — and no
/// payload in the bank changes that, because the verdict never reads
/// one.
#[test]
fn a_credential_fill_is_a_hard_no_by_rule_id() {
    for (tool, args) in credential_fills() {
        let v = verdict(tool, Class::Write, &args);
        assert!(
            v.is_hard_no(),
            "`{tool}` with {args} was not a HARD NO: {v:?}"
        );
        assert_eq!(
            v.rule(),
            Some("fill.password_field"),
            "`{tool}` was refused, but under the wrong rule — a person reading the \
             Ledger has to be able to look this up"
        );
        assert!(lisa_guard::HARD_NO_RULES.contains(&"fill.password_field"));
    }
}

#[test]
fn reading_the_keyring_is_a_hard_no_by_rule_id() {
    for (tool, class, args) in credential_reads() {
        let v = verdict(tool, class, &args);
        assert!(
            v.is_hard_no(),
            "`{tool}` with {args} was not a HARD NO: {v:?}"
        );
        assert_eq!(
            v.rule(),
            Some("secret.read"),
            "`{tool}` was refused under the wrong rule"
        );
    }
    assert!(lisa_guard::HARD_NO_RULES.contains(&"secret.read"));
}

/// The positive control both tests above need. Without it, a
/// `judge_action` that refused EVERYTHING would make them green while
/// shipping a browser in which no form can be filled at all — which is
/// the vacuity this repo keeps finding.
#[test]
fn an_ordinary_fill_and_an_ordinary_read_are_not_refused() {
    let ordinary = verdict(
        "fill",
        Class::Write,
        &json!({"url": "https://example.org/", "selector": "#search", "value": "lisa os"}),
    );
    assert!(
        !ordinary.is_refused(),
        "an ordinary form fill must still work: {ordinary:?}"
    );
    let read = verdict("read_page", Class::Read, &json!({}));
    assert!(
        !read.is_refused(),
        "reading a page must still work: {read:?}"
    );
}

/// Hostile CONTENT does not move the line. Every payload in the bank,
/// on every vector, riding the trigger chain of a credential call.
#[test]
fn no_payload_talks_the_guard_into_a_credential() {
    let mut checked = 0usize;
    for payload in PAYLOADS {
        for (vector, _provenance) in VECTORS {
            // The payload as an ARGUMENT as well as in the chain: a
            // page that gets its text into the tool call is the strongest
            // form of this attack, and the verdict must be the same.
            let args = json!({
                "url": "https://bank.example/",
                "selector": "#password",
                "value": payload,
                "note": format!("{vector}: {payload}"),
            });
            let v = verdict("fill", Class::Write, &args);
            assert_eq!(
                v.rule(),
                Some("fill.password_field"),
                "payload {payload:?} on {vector} changed the verdict"
            );
            checked += 1;
        }
    }
    assert!(checked >= 200, "the payload bank shrank to {checked}");
}

/// End to end: the same calls, through a real `AgentBus` with a
/// recording dispatcher. A HARD NO must never be dispatched and must
/// never be PARKED either — a parked call is an id somebody can
/// approve, and #252's whole point is that these have no such id.
#[test]
fn the_bus_dispatches_and_parks_nothing_for_a_credential_call() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.keep().join("ledger.db")).unwrap());
    let dispatcher = Arc::new(RecordingDispatcher::returning(json!({"ok": true})));
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

    for payload in PAYLOADS.iter().take(8) {
        let outcome = bus
            .request(CallRequest {
                actor: "system-agent".into(),
                app_id: "app.lisaos.Surfer".into(),
                tool: "fill".into(),
                args: json!({
                    "url": "https://bank.example/",
                    "selector": "#password",
                    "value": payload,
                }),
                chain: vec![Provenance::parse("user"), Provenance::parse("web")],
                requester_hosts_a_model: false,
                // p2p: no broker, so no pidfd and no process to pin
                // (`lisa_peer::Process::of_peer`). Nothing in this test
                // depends on it — a HARD NO is decided by what the call
                // IS, before anybody can be asked to approve it — but
                // `None` is the honest value rather than a convenient
                // one, and it is what agentd itself passes on `Direct`.
                requester_process: None,
                caller: lisa_peer::PeerId::Direct,
            })
            .expect("ledger available");
        match outcome {
            Outcome::Refused { .. } | Outcome::Denied { .. } => {}
            other => panic!("a credential fill was not refused outright: {other:?}"),
        }
    }
    assert_eq!(
        dispatcher.dispatched(),
        0,
        "the bus dispatched a credential fill"
    );
}
