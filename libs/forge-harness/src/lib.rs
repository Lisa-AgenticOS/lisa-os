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
//! done; a [`Verifier`] (`dart analyze` by default, any command, or none)
//! decides whether "done" is believed. Hot-reload preview + VLM
//! self-inspection join the loop next (run-controller).

pub mod agent;
pub mod jail;
pub mod openai;
pub mod tools;

pub use agent::{
    AgentAction, AgentConfig, AgentEvent, AgentReport, Message, Role, ScriptedBackend, Verifier,
    forge_agent, forge_agent_observed,
};
pub use openai::OpenAiBackend;
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
) -> Result<ForgeReport, ForgeError> {
    let config = AgentConfig {
        max_turns: max_iterations.saturating_mul(8).max(8),
        verifier: Verifier::Dart,
        ..Default::default()
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
        let report = forge("print forged", dir.path(), &mut backend, 3).unwrap();
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
        let err = forge("task", dir.path(), &mut backend, 1);
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
        let err = forge("task", dir.path(), &mut backend, 3);
        assert!(matches!(err, Err(ForgeError::Backend(_))));
        let findings_shown = backend
            .last_history
            .iter()
            .any(|m| m.content.contains("undefined_symbol"));
        assert!(findings_shown, "analyzer findings must reach the model");
    }
}
