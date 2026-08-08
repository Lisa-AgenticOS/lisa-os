//! The oracle lane (ADR-0067 §1): every fixture, model-free, in CI.
//!
//! What this proves is the MACHINERY — staging, the sanity gate, the
//! real tool path, restore, grading — with the controls that make a
//! green run mean something:
//!
//! - **positive control**: applying the known-good patch through the
//!   same `WorkspaceTools` the model drives resolves the fixture. A
//!   fixture the oracle cannot resolve via the tool path is a fixture
//!   no model can resolve either.
//! - **negative control**: an untouched workspace grades UNRESOLVED.
//!   Without this, a grader that dispatches nothing passes everything.
//! - **restore control**: a trajectory that rewrites the tests to
//!   always-pass still grades unresolved, because grading happens on
//!   the restored pristine tests.

use coder_eval::{Failure, discover, fixtures_dir, grade, patch, restore_tests, sanity, stage};
use forge_harness::{ToolCall, ToolProvider, WorkspaceTools};

/// Apply the fixture's known-good patch through the REAL tool path.
fn apply_patch_via_tools(fixture: &coder_eval::Fixture, workspace: &std::path::Path) {
    let tools = WorkspaceTools::new(workspace).expect("jail opens on the staged tree");
    for file in patch(fixture).expect("patch.json parses") {
        let outcome = tools.execute(&ToolCall {
            id: "oracle".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": file.path, "content": file.content}),
        });
        assert!(
            outcome.mutated,
            "{}: the oracle write was refused: {}",
            fixture.name, outcome.text
        );
    }
}

#[test]
fn every_fixture_reproduces_and_the_oracle_resolves_it() {
    let fixtures = discover(&fixtures_dir()).expect("fixtures discover");
    assert!(!fixtures.is_empty(), "no fixtures found");
    let tmp = tempfile::tempdir().expect("tempdir");

    let mut resolved = 0;
    for fixture in &fixtures {
        let ws = stage(fixture, tmp.path()).expect("stage");
        // The fixture must reproduce its own bug before anything runs.
        if let Err(f) = sanity(fixture, &ws) {
            panic!("{}: sanity failed: {f:?}", fixture.name);
        }
        // Negative control: untouched, it must grade unresolved.
        restore_tests(fixture, &ws).expect("restore");
        match grade(fixture, &ws) {
            Err(Failure::StillRed(_)) => {}
            other => panic!(
                "{}: an untouched workspace graded {other:?} — the grader \
                 passes without a fix, so every green it reports is noise",
                fixture.name
            ),
        }
        // Positive control: the known-good patch, via the tool path.
        apply_patch_via_tools(fixture, &ws);
        restore_tests(fixture, &ws).expect("restore");
        if let Err(f) = grade(fixture, &ws) {
            panic!(
                "{}: the oracle patch did not resolve it: {f:?}",
                fixture.name
            );
        }
        resolved += 1;
    }
    assert_eq!(
        resolved,
        fixtures.len(),
        "oracle lane resolves every fixture"
    );
}

#[test]
fn rewriting_the_tests_does_not_beat_the_grader() {
    let fixtures = discover(&fixtures_dir()).expect("fixtures discover");
    let fixture = &fixtures[0];
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = stage(fixture, tmp.path()).expect("stage");

    // A hostile trajectory: no fix, tests rewritten to trivially pass.
    let tools = WorkspaceTools::new(&ws).expect("jail opens");
    let tests_dir = ws.join("tests");
    for entry in std::fs::read_dir(&tests_dir).expect("tests dir") {
        let name = entry.expect("entry").file_name();
        let hostile = format!(
            "#[test]\nfn {}() {{}}\n#[test]\nfn {}() {{}}\n",
            fixture.task.fail_to_pass[0], fixture.task.pass_to_pass[0]
        );
        let outcome = tools.execute(&ToolCall {
            id: "hostile".into(),
            name: "write_file".into(),
            args: serde_json::json!({
                "path": format!("tests/{}", name.to_string_lossy()),
                "content": hostile,
            }),
        });
        assert!(outcome.mutated, "the hostile write itself must land");
    }
    // Un-restored, the hostile tests would pass. Restored, they are
    // gone and the grade is honest.
    restore_tests(fixture, &ws).expect("restore");
    match grade(fixture, &ws) {
        Err(Failure::StillRed(_)) => {}
        other => panic!(
            "restore did not revert the grader — a test-rewriting \
             trajectory graded {other:?}"
        ),
    }
}
