//! forge-harness — the agentic app-building loop (`docs/PLAN.md`
//! §5.12.1): a multi-turn, multi-tool coding agent against any
//! OpenAI-compatible backend — lisa-inferenced's local coder model by
//! default, a BYO agent CLI later, both through the same tool jail.
//!
//! The backend is offered a tool set (`read_file`, `list_dir`, `grep`,
//! `write_file`, `edit_file`, `run_command`, `run_tests`), each declared
//! with a JSON input schema so tool calls arrive grammar-valid and never
//! as free-form model output. Every file operation is mediated by the
//! [`jail::Jail`], so path traversal stays impossible no matter what the
//! model asks for. Each turn the backend either calls one tool or signals
//! done; a [`Verifier`] decides whether "done" is believed — `lisa dev
//! check` for the default GJS lane (ADR-0047, ADR-0050), any other
//! command, `dart analyze` for the parked Flutter lane, or none for a
//! surface with no project at all. Hot-reload preview + VLM
//! self-inspection join the loop next (run-controller).

pub mod agent;
pub mod confine;
pub mod jail;
pub mod openai;
pub mod shell_tool;
pub mod skills;
pub mod tools;
pub mod unix_http;

pub use agent::{
    AgentAction, AgentConfig, AgentEvent, AgentReport, Message, Role, ScriptedBackend,
    ToolProvider, Verifier, WorkspaceTools, forge_agent, forge_agent_observed,
    forge_agent_with_tools,
};
pub use openai::{OpenAiBackend, backend_refusal, streaming_request_body};
pub use shell_tool::{ShellRequest, ShellTool};
pub use skills::{READ_SKILL, SkillTools};
pub use tools::{ToolCall, ToolOutcome, ToolSpec, execute_tool, tool_specs};

use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ForgeError {
    #[error("backend: {0}")]
    Backend(String),
    #[error("jail: {0}")]
    Jail(#[from] jail::JailError),
    #[error("analyzer: {0}")]
    Analyzer(String),
    #[error("did not converge after {0} iteration(s)")]
    NoConvergence(usize),
    /// No ledger entry, no action (issue #54). The Ledger is the only
    /// record of what an unattended loop did to your files; if it cannot
    /// be written, the loop stops rather than acting unobserved.
    #[error("ledger unavailable — refusing to act: {0}")]
    Ledger(#[from] lisa_ledger::LedgerError),
    /// Somebody pressed Stop (issue #227). Not a failure of the run —
    /// the run did what it was told.
    #[error("Stopped.")]
    Cancelled,
}

/// A stop button, shared with whoever is holding one.
///
/// Issue #227: harnessd had a flag of its own shape, set it from
/// `Cancel(run_id)`, read it in an observer, assigned the result to a
/// local nothing ever looked at again — and the loop had no way to be
/// told in the first place. Stop was a no-op, and three comments
/// described what it would have done.
///
/// So the flag lives HERE, where the loop is, and the loop consults it.
/// A daemon cannot hold one that is not wired to anything, because there
/// is only one kind and the loop takes it.
///
/// What it promises, and does not: a turn's BACKEND call is interrupted
/// — a half-generated sentence costs nothing to abandon — and a TOOL
/// CALL is not. A tool that has started runs to its end and its result
/// is recorded; killing a write halfway is how half-done actions
/// happen. Between those two the loop stops.
#[derive(Clone, Default)]
pub struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// One whole-file edit — the argument shape of the `write_file` tool.
#[derive(Debug, Deserialize)]
pub struct Edit {
    pub path: String,
    pub content: String,
}

/// A completion backend. The production impl speaks OpenAI-compat tool
/// calling; tests script it. Each call sees the full conversation and the
/// tool declarations, and answers with either one tool call or the done
/// signal.
pub trait Backend {
    fn next_action(
        &mut self,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Result<AgentAction, ForgeError>;

    /// The same turn, reporting assistant text as it arrives.
    ///
    /// The loop needs the WHOLE message before it can act — a tool call
    /// is only knowable once its arguments are complete — so streaming
    /// changes nothing about control flow. What it changes is whether a
    /// person watching sees words appear or a spinner: for `forge`,
    /// nobody is watching a turn, and for a chat window that difference
    /// is the entire feel of the thing.
    ///
    /// `cancel` is checked WHILE the answer is arriving (#227): Stop
    /// has to stop the thing a person is watching, and a partly
    /// generated sentence is the one piece of work in this loop that
    /// costs nothing to abandon. An implementation that cannot check
    /// mid-flight simply returns as usual and the loop stops at the next
    /// boundary.
    ///
    /// Default delegates, so a scripted backend stays three lines.
    fn next_action_streaming(
        &mut self,
        messages: &[Message],
        tools: &[ToolSpec],
        on_delta: &mut dyn FnMut(&str),
        cancel: &Cancel,
    ) -> Result<AgentAction, ForgeError> {
        let _ = &on_delta;
        let _ = cancel;
        self.next_action(messages, tools)
    }
}

#[derive(Debug)]
pub struct ForgeReport {
    pub iterations: usize,
    pub analyzer_output: String,
}

/// Run `dart analyze`; Ok(None) on clean, Ok(Some(output)) on findings.
pub fn analyze(project: &Path) -> Result<Option<String>, ForgeError> {
    let out = Command::new("dart")
        .arg("analyze")
        .arg(project)
        .output()
        .map_err(|e| ForgeError::Analyzer(format!("running dart analyze: {e}")))?;
    if out.status.success() {
        return Ok(None);
    }
    Ok(Some(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )))
}

/// The original single-task entry point, kept signature-compatible for
/// callers (cli/lisa). It now runs the full multi-tool agent loop with
/// the `dart analyze` verifier: the backend may read, grep, edit, and run
/// commands, not just emit whole-file edits. `max_iterations` still
/// budgets the work — as a turn cap of 8 turns per iteration, since
/// read/inspect turns don't write files — and `ForgeReport.iterations`
/// reports the turns actually used.
pub fn forge(
    task: &str,
    project: &Path,
    backend: &mut dyn Backend,
    max_iterations: usize,
    ledger: std::sync::Arc<lisa_ledger::Ledger>,
) -> Result<ForgeReport, ForgeError> {
    // The Ledger is a parameter, not a default (issue #129). This
    // function used to fill it in with `None`, so the one loop that
    // edits your files with nobody watching recorded nothing at all.
    let config = AgentConfig {
        max_turns: max_iterations.saturating_mul(8).max(8),
        verifier: Verifier::Dart,
        ..AgentConfig::new(ledger)
    };
    match forge_agent(task, project, backend, &config) {
        Ok(report) => Ok(ForgeReport {
            iterations: report.turns,
            analyzer_output: report.verifier_output,
        }),
        Err(ForgeError::NoConvergence(_)) => Err(ForgeError::NoConvergence(max_iterations)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    /// A throwaway Ledger. Every caller needs one now (issue #129) —
    /// that is the fix, not an inconvenience: the loop cannot run
    /// unledgered even by accident.
    fn scratch_ledger(dir: &std::path::Path) -> std::sync::Arc<lisa_ledger::Ledger> {
        std::sync::Arc::new(lisa_ledger::Ledger::open(dir.join("t.db")).unwrap())
    }

    use super::*;
    use serde_json::json;

    fn dart_available() -> bool {
        Command::new("dart").arg("--version").output().is_ok()
    }

    fn dart_project(dir: &Path) {
        std::fs::write(
            dir.join("pubspec.yaml"),
            "name: forge_test\nenvironment:\n  sdk: ^3.0.0\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
    }

    fn write_main(content: &str) -> AgentAction {
        AgentAction::Call(ToolCall {
            id: "c1".into(),
            name: "write_file".into(),
            args: json!({"path": "bin/main.dart", "content": content}),
        })
    }

    #[test]
    fn loop_converges_when_the_fix_lands() {
        if !dart_available() {
            eprintln!("skipping: dart not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        dart_project(dir.path());
        let mut backend = ScriptedBackend::new(vec![
            write_main("void main() { undefined_symbol(); }\n"),
            write_main("void main() { print('forged'); }\n"),
        ]);
        let report = forge(
            "print forged",
            dir.path(),
            &mut backend,
            3,
            scratch_ledger(dir.path()),
        )
        .unwrap();
        assert_eq!(report.iterations, 2, "broken first, fixed second");
    }

    #[test]
    fn loop_reports_no_convergence() {
        if !dart_available() {
            eprintln!("skipping: dart not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        dart_project(dir.path());
        let mut backend =
            ScriptedBackend::repeating(vec![write_main("void main() { broken(; }\n")]);
        let err = forge(
            "task",
            dir.path(),
            &mut backend,
            1,
            scratch_ledger(dir.path()),
        );
        assert!(matches!(err, Err(ForgeError::NoConvergence(1))));
    }

    #[test]
    fn forge_reports_the_analyzer_findings_on_failure() {
        if !dart_available() {
            eprintln!("skipping: dart not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        dart_project(dir.path());
        let mut backend = ScriptedBackend::new(vec![
            write_main("void main() { undefined_symbol(); }\n"),
            AgentAction::Done("I am done".into()),
        ]);
        // The done signal is rejected by the analyzer and the script runs
        // out — a Backend error, proving findings were not waved through.
        let err = forge(
            "task",
            dir.path(),
            &mut backend,
            3,
            scratch_ledger(dir.path()),
        );
        assert!(matches!(err, Err(ForgeError::Backend(_))));
        let findings_shown = backend
            .last_history
            .iter()
            .any(|m| m.content.contains("undefined_symbol"));
        assert!(findings_shown, "analyzer findings must reach the model");
    }
}
