//! The Forge's default verifier, run for real (#243).
//!
//! `lisa forge` with no flags used to write a `pubspec.yaml` and select
//! `forge_harness::Verifier::Dart`, which on a directory of `.js` files
//! reports *"the project contains no Dart source files yet"* forever —
//! so the headline "talk an app into existence" feature could not
//! produce the kind of app Lisa ships (ADR-0047 §4).
//!
//! It now selects `Verifier::Command { program: <this binary>, args:
//! ["dev", "check"] }` (ADR-0050 §4). This test executes that arm — the
//! real subprocess, the real exit code, the real findings text the loop
//! feeds back to the model — because the unit tests in `dev.rs` check
//! the rules and prove nothing about the wiring. `apps_payload.rs` and
//! `apps_launcher.rs` exist for the same reason (#239).

use std::path::Path;

/// The verifier `forge_cmd` builds for the default lane. Spelled here
/// the way `cli/lisa/src/main.rs` spells it; if the two drift, the
/// assertions below stop describing what the Forge runs — which is why
/// `forge_verifier_is_the_dev_check_verb` also asserts the argv.
fn verifier() -> forge_harness::Verifier {
    forge_harness::Verifier::Command {
        program: env!("CARGO_BIN_EXE_lisa").to_string(),
        args: vec!["dev".into(), "check".into()],
    }
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A well-formed GJS app: the entry module, testable logic, a manifest.
fn scaffold(dir: &Path) {
    write(
        &dir.join("lisa-notes.js"),
        "#!/usr/bin/env -S gjs -m\n\
         import Gtk from 'gi://Gtk?version=4.0';\n\
         import Adw from 'gi://Adw?version=1';\n\
         import { render } from './lib/notes.js';\n\
         function main() {\n  \
         const app = new Adw.Application({ application_id: 'app.lisaos.Notes' });\n  \
         app.connect('activate', () => render(Gtk));\n  \
         return app.run([]);\n\
         }\n\
         main();\n",
    );
    write(
        &dir.join("lib/notes.js"),
        "export function render() {\n  return 'notes';\n}\n",
    );
    write(
        &dir.join("app.lisaos.Notes.json"),
        r#"{"lisa_manifest": 1,
            "app_id": "app.lisaos.Notes",
            "mcp": {"transport": "unix", "activatable": true},
            "tools": [{"name": "list_notes", "tier": "read",
                       "description": "List the notes",
                       "input_schema": {"type": "object", "properties": {}}}]}"#,
    );
}

/// **The gap #243 named.** An empty project must not verify clean, and
/// the findings must talk about JavaScript rather than Dart — otherwise
/// the model is told to write the wrong language by its own verifier.
#[test]
fn an_empty_project_reports_findings_that_name_the_right_toolkit() {
    let dir = tempfile::tempdir().unwrap();
    let findings = verifier()
        .check(dir.path())
        .expect("the verifier runs")
        .expect("an empty project is not done");
    assert!(
        findings.contains("JavaScript"),
        "the verifier still asks for another language: {findings}"
    );
    assert!(
        !findings.contains("Dart") && !findings.contains("dart"),
        "the verifier still asks for Dart: {findings}"
    );
    // And it says what to write, since this text IS the next turn's
    // instructions.
    assert!(findings.contains("lisa-<name>.js"), "{findings}");
    assert!(findings.contains("manifest"), "{findings}");
}

/// The other half: a real GJS app converges. Without this the test
/// above is satisfied by a verifier that fails everything, and the loop
/// would spin to its turn budget on correct code.
#[test]
fn a_well_formed_gjs_app_verifies_clean() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    assert_eq!(
        verifier().check(dir.path()).expect("the verifier runs"),
        None,
        "a valid Lisa app did not verify clean"
    );
}

/// Each rule, through the subprocess rather than the function: a tree
/// that trips one gets findings naming it.
#[test]
fn the_traps_are_reported_through_the_verifier_the_loop_runs() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path());
    // The footgun with no log line (ANATOMY §7).
    write(
        &dir.path().join("lisa-notes.js"),
        "import Gio from 'gi://Gio';\nawait start();\n",
    );
    let findings = verifier()
        .check(dir.path())
        .unwrap()
        .expect("top-level await is a finding");
    assert!(findings.contains("top-level-await"), "{findings}");

    // A manifest the bus would reject (#241's class): tier is not a tier.
    scaffold(dir.path());
    write(
        &dir.path().join("app.lisaos.Notes.json"),
        r#"{"lisa_manifest": 1, "app_id": "app.lisaos.Notes",
            "tools": [{"name": "wipe", "tier": "destroy",
                       "input_schema": {"type": "object"}}]}"#,
    );
    let findings = verifier()
        .check(dir.path())
        .unwrap()
        .expect("an invalid manifest is a finding");
    assert!(findings.contains("[manifest]"), "{findings}");
}

/// **The whole loop, end to end.** A model writes a GJS app into an
/// empty directory and the run converges — the thing #243 says cannot
/// happen today, because `Verifier::Dart` on a directory of `.js` files
/// reports "no Dart source files yet" forever.
///
/// The backend is scripted rather than a real model, so what this proves
/// is the loop and the verifier, not the model's taste. Everything else
/// is production: the jail mediates every write, the Ledger records
/// every call, and the verifier is the `lisa dev check` subprocess.
///
/// It stops at "converges", not at "launches": running the forged app
/// needs `gjs`, GTK4 and a display, none of which exist on a macOS dev
/// host or in the unit-test lane. `just shell-test` is where JS actually
/// executes.
#[test]
fn the_loop_converges_on_a_gjs_app_written_from_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let ledger = std::sync::Arc::new(
        lisa_ledger::Ledger::open(dir.path().join("ledger.db")).expect("ledger opens"),
    );

    let entry = "#!/usr/bin/env -S gjs -m\n\
                 import Adw from 'gi://Adw?version=1';\n\
                 function main() {\n  \
                 return new Adw.Application({ application_id: 'app.lisaos.Notes' }).run([]);\n\
                 }\nmain();\n";
    let manifest = r#"{"lisa_manifest": 1, "app_id": "app.lisaos.Notes",
        "tools": [{"name": "list_notes", "tier": "read",
                   "input_schema": {"type": "object"}}]}"#;
    let mut backend = forge_harness::ScriptedBackend::new(vec![
        // A bare "done" on an empty tree, which must NOT be believed
        // (#29's gate, in GJS form).
        forge_harness::AgentAction::Done("all set".into()),
        forge_harness::AgentAction::Call(forge_harness::ToolCall {
            id: "1".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": "lisa-notes.js", "content": entry}),
        }),
        forge_harness::AgentAction::Call(forge_harness::ToolCall {
            id: "2".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": "app.lisaos.Notes.json", "content": manifest}),
        }),
    ]);
    let config = forge_harness::AgentConfig {
        max_turns: 8,
        verifier: verifier(),
        ..forge_harness::AgentConfig::new(ledger)
    };
    let report = forge_harness::forge_agent("a notes app", &project, &mut backend, &config)
        .expect("the loop converges on a GJS app");

    assert!(report.verified, "the run ended on the model's word");
    // It took more than one turn, which is the proof the empty-tree
    // "done" was refused rather than accepted.
    assert!(report.turns > 1, "a bare done on nothing was believed");
    assert!(project.join("lisa-notes.js").is_file());
    assert!(project.join("app.lisaos.Notes.json").is_file());
    // No Dart scaffold anywhere: the default lane writes none.
    assert!(!project.join("pubspec.yaml").exists());
    assert!(!project.join("bin").exists());
}

/// The verb exists under the namespace ADR-0050 §2 reserved for it, and
/// takes a path. A rename would leave the Forge calling a verb that is
/// not there, which surfaces as `exited with 2` and nothing useful.
#[test]
fn forge_verifier_is_the_dev_check_verb() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_lisa"))
        .args(["dev", "check", "--help"])
        .output()
        .expect("lisa runs");
    assert!(
        out.status.success(),
        "`lisa dev check --help` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let forge_harness::Verifier::Command { program, args } = verifier() else {
        panic!("the default lane must run a command, not a language-specific arm");
    };
    assert!(program.ends_with("lisa"), "{program}");
    assert_eq!(args, ["dev", "check"]);
}
