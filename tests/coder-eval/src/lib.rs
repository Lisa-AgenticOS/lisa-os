//! The committed tier of the coder eval harness (ADR-0067).
//!
//! A fixture is a tiny self-contained Rust crate with a real planted
//! bug: `fail_to_pass` names cargo tests that are RED on the shipped
//! tree and must go GREEN after a fix; `pass_to_pass` names tests that
//! are green and must stay green. The score is `resolved / total` —
//! a fixture whose environment breaks is a counted FAIL, never a
//! hidden skip (ADR-0067 §3).
//!
//! Two lanes share this module:
//!
//! - the **oracle lane** (`tests/oracle.rs`, CI, model-free): applies
//!   each fixture's known-good patch through the REAL tool path
//!   (`WorkspaceTools`), then grades. It proves the machinery — staging,
//!   tool dispatch, restore, grading — not model quality, and it is the
//!   positive control the model lane's failures are read against.
//! - the **model lane** (`src/main.rs`, opt-in): the same fixtures run
//!   through `forge_agent_with_tools` against a live model; its
//!   `resolved/total` is the per-change smoke number ADR-0068/0069
//!   report before/after. It can only see LARGE moves (ADR-0067 §1
//!   fixes the noise floor at ~±9pp for tens of tasks) — improvement
//!   claims belong to the public tier, which is NOT built yet.
//!
//! Grading happens after **restore**: the fixture's pristine `tests/`
//! directory is copied back over the workspace, so a trajectory that
//! edited the grader loses the edit, not the grade (ADR-0065's
//! restore; it does not stop reading-and-special-casing — that residual
//! is the public tier's to close, and is named, not hidden).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A fixture's manifest (`task.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    /// What the agent is asked to do — the bug report, not the fix.
    pub prompt: String,
    /// Tests that are red before and must be green after.
    pub fail_to_pass: Vec<String>,
    /// Tests that are green before and must stay green.
    pub pass_to_pass: Vec<String>,
}

/// One full-file write of the known-good fix (`patch.json` is a list of
/// these). Full files, not diffs: the oracle drives `write_file`, the
/// same complete-content tool the model gets.
#[derive(Debug, Clone, Deserialize)]
pub struct PatchFile {
    pub path: String,
    pub content: String,
}

/// One fixture on disk: `task.json`, `patch.json`, and `tree/` (the
/// crate the agent works on).
#[derive(Debug, Clone)]
pub struct Fixture {
    pub name: String,
    pub dir: PathBuf,
    pub task: Task,
}

/// Why a fixture counts as failed. Everything is a FAIL with a reason —
/// the denominator never shrinks (ADR-0067 §3).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Failure {
    /// The environment or harness broke before grading.
    Environment(String),
    /// A `fail_to_pass` test was not red before the fix — the fixture
    /// does not reproduce its own bug, which is a harness defect.
    NotRedBefore(String),
    /// A `pass_to_pass` test was not green before the fix.
    NotGreenBefore(String),
    /// After the run, a `fail_to_pass` test is still red.
    StillRed(String),
    /// After the run, a `pass_to_pass` test went red.
    Regressed(String),
    /// The agent loop itself returned an error.
    Agent(String),
}

/// One fixture's outcome, machine-readable (ADR-0067 §5).
#[derive(Debug, Clone, Serialize)]
pub struct TaskReport {
    pub name: String,
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    pub wall_ms: u128,
}

/// The run's summary: the score is `resolved / total`, total being
/// every fixture found — never fewer.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub lane: String,
    pub resolved: usize,
    pub total: usize,
    pub tasks: Vec<TaskReport>,
}

impl Report {
    pub fn summary(&self) -> String {
        format!(
            "coder-eval [{}]: {}/{} resolved",
            self.lane, self.resolved, self.total
        )
    }
}

/// Every fixture under `dir`, sorted by name so runs are comparable.
pub fn discover(dir: &Path) -> Result<Vec<Fixture>, String> {
    let mut out = Vec::new();
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("reading fixtures dir {dir:?}: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("task.json");
        let raw =
            std::fs::read_to_string(&manifest).map_err(|e| format!("reading {manifest:?}: {e}"))?;
        let task: Task =
            serde_json::from_str(&raw).map_err(|e| format!("parsing {manifest:?}: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        out.push(Fixture {
            name,
            dir: path,
            task,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The repo's own fixtures directory.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Copy `tree/` into a fresh directory under `stage` and return the
/// workspace root the agent (or oracle) will work in.
pub fn stage(fixture: &Fixture, stage: &Path) -> Result<PathBuf, String> {
    let dest = stage.join(&fixture.name);
    copy_tree(&fixture.dir.join("tree"), &dest)
        .map_err(|e| format!("staging {}: {e}", fixture.name))?;
    Ok(dest)
}

/// Restore the grader: the pristine `tests/` directory replaces
/// whatever the trajectory left there. Returns an error only when the
/// copy itself fails — a missing `tests/` in the fixture is a fixture
/// authoring bug caught by the sanity phase.
pub fn restore_tests(fixture: &Fixture, workspace: &Path) -> Result<(), String> {
    let pristine = fixture.dir.join("tree").join("tests");
    let target = workspace.join("tests");
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("clearing tests/: {e}"))?;
    }
    copy_tree(&pristine, &target).map_err(|e| format!("restoring tests/: {e}"))
}

/// Run one named test exactly, offline. `Ok(true)` = the test passed.
/// Compile failure or a missing test name is `Ok(false)` — red is red,
/// whatever the shade — with the output carried for the report.
pub fn test_passes(workspace: &Path, name: &str) -> Result<bool, String> {
    let out = Command::new("cargo")
        .args(["test", "--offline", "-q", name, "--", "--exact"])
        .current_dir(workspace)
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .map_err(|e| format!("running cargo test in {workspace:?}: {e}"))?;
    Ok(out.status.success())
}

/// The pre-fix sanity gate: every `fail_to_pass` red, every
/// `pass_to_pass` green. A fixture failing this is broken machinery and
/// counts as a FAIL for the run (never silently dropped).
pub fn sanity(fixture: &Fixture, workspace: &Path) -> Result<(), Failure> {
    for t in &fixture.task.fail_to_pass {
        match test_passes(workspace, t) {
            Ok(false) => {}
            Ok(true) => return Err(Failure::NotRedBefore(t.clone())),
            Err(e) => return Err(Failure::Environment(e)),
        }
    }
    for t in &fixture.task.pass_to_pass {
        match test_passes(workspace, t) {
            Ok(true) => {}
            Ok(false) => return Err(Failure::NotGreenBefore(t.clone())),
            Err(e) => return Err(Failure::Environment(e)),
        }
    }
    Ok(())
}

/// Grade a workspace after the run (call `restore_tests` first).
pub fn grade(fixture: &Fixture, workspace: &Path) -> Result<(), Failure> {
    for t in &fixture.task.fail_to_pass {
        match test_passes(workspace, t) {
            Ok(true) => {}
            Ok(false) => return Err(Failure::StillRed(t.clone())),
            Err(e) => return Err(Failure::Environment(e)),
        }
    }
    for t in &fixture.task.pass_to_pass {
        match test_passes(workspace, t) {
            Ok(true) => {}
            Ok(false) => return Err(Failure::Regressed(t.clone())),
            Err(e) => return Err(Failure::Environment(e)),
        }
    }
    Ok(())
}

/// The known-good patch, parsed from `patch.json`.
pub fn patch(fixture: &Fixture) -> Result<Vec<PatchFile>, String> {
    let path = fixture.dir.join("patch.json");
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("reading {path:?}: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("parsing {path:?}: {e}"))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            // Build output would only slow the copy down; fixtures are
            // staged from a pristine tree, so `target/` never exists in
            // the repo — skipped defensively for local re-runs.
            if entry.file_name() == "target" {
                continue;
            }
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}
