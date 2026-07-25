//! The multi-turn agent loop. The harness owns the message history; each
//! turn the backend either issues one tool call or signals done. Tool
//! results (and verifier findings after each mutation) are appended to
//! the history, and the loop continues until the model signals done with
//! a clean verifier, the verifier passes right after an edit, or the turn
//! budget runs out.

use crate::jail::Jail;
use crate::tools::{ToolCall, ToolSpec, execute_tool, tool_specs};
use crate::{Backend, ForgeError, analyze};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One message in the agent conversation. `tool_call` is set on assistant
/// messages that invoke a tool; `tool_call_id` links a tool result back
/// to the call that produced it (OpenAI wire protocol).
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_call: Option<ToolCall>,
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::bare(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::bare(Role::User, content)
    }

    pub fn assistant_text(content: impl Into<String>) -> Self {
        Self::bare(Role::Assistant, content)
    }

    pub fn assistant_call(call: ToolCall) -> Self {
        let mut msg = Self::bare(Role::Assistant, "");
        msg.tool_call = Some(call);
        msg
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut msg = Self::bare(Role::Tool, content);
        msg.tool_call_id = Some(call_id.into());
        msg
    }

    fn bare(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call: None,
            tool_call_id: None,
        }
    }
}

/// What the backend decided on a turn: call a tool, or finish with a
/// summary. A backend signals done by replying without a tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentAction {
    Call(ToolCall),
    Done(String),
}

/// How the loop decides the project is in a good state. `Dart` keeps the
/// original `dart analyze` behavior; `Command` runs any check (non-zero
/// exit = findings); `None` trusts the model's done signal — the loop
/// then ends only when the backend says so.
#[derive(Debug, Clone)]
pub enum Verifier {
    Dart,
    Command { program: String, args: Vec<String> },
    None,
}

impl Verifier {
    pub fn is_none(&self) -> bool {
        matches!(self, Verifier::None)
    }

    /// Ok(None) when clean, Ok(Some(findings)) when not.
    pub fn check(&self, project: &Path) -> Result<Option<String>, ForgeError> {
        match self {
            // `dart analyze` exits clean on a project with no sources at
            // all, which let a model's bare "done" converge on an empty
            // scaffold (issue #29). No sources = findings, not a pass.
            Verifier::Dart => {
                if !has_dart_sources(project) {
                    return Ok(Some(
                        "the project contains no Dart source files yet — \
                         nothing has been written, so the task cannot be done. \
                         Write the code first."
                            .into(),
                    ));
                }
                analyze(project)
            }
            Verifier::Command { program, args } => {
                let out = Command::new(program)
                    .args(args)
                    .current_dir(project)
                    .output()
                    .map_err(|e| {
                        ForgeError::Analyzer(format!("running verifier `{program}`: {e}"))
                    })?;
                if out.status.success() {
                    return Ok(None);
                }
                Ok(Some(format!(
                    "`{program}` exited with {}\n{}{}",
                    out.status,
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                )))
            }
            Verifier::None => Ok(None),
        }
    }
}

/// Any `.dart` file under the project (skipping the `.dart_tool` cache)
/// counts as source; the pubspec scaffold alone does not.
fn has_dart_sources(project: &Path) -> bool {
    fn walk(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == ".dart_tool") {
                    continue;
                }
                if walk(&path) {
                    return true;
                }
            } else if path.extension().is_some_and(|e| e == "dart") {
                return true;
            }
        }
        false
    }
    walk(project)
}

pub struct AgentConfig {
    /// Hard cap on backend turns (one tool call or done-signal each), so
    /// read/inspect turns don't consume the edit budget but the loop can
    /// never spin forever.
    pub max_turns: usize,
    pub verifier: Verifier,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 32,
            verifier: Verifier::Dart,
        }
    }
}

#[derive(Debug)]
pub struct AgentReport {
    pub turns: usize,
    pub summary: String,
    /// Findings from the last verifier run that failed (empty on clean).
    pub verifier_output: String,
    /// True when a real verifier passed; false when `Verifier::None`
    /// trusted the model's word.
    pub verified: bool,
}

const SYSTEM_PROMPT: &str = "\
You are the Lisa Forge, an autonomous coding agent working inside a jailed project \
directory. You inspect and modify the project by calling the provided tools.

Rules:
- ALL paths are project-relative (e.g. `bin/main.dart`, `lib/src/foo.dart`). Never \
absolute, never containing `..` — the jail rejects them and the write does not happen.
- Inspect before you edit: use `list_dir`, `read_file`, and `grep` to understand the \
project. Use `edit_file` for targeted changes, `write_file` for new files or complete \
rewrites.
- `run_command` is allowlisted and runs in the project root; use it for toolchain \
commands. Use `run_tests` to run the test suite.
- Analyzer/verifier findings are fed back to you after each edit; fix them.
- Flutter UI imports `package:lisa_ui/lisa_ui.dart` — the Lisa design system — never \
`package:flutter/material.dart` directly. Root the app with `LisaApp`, build screens \
with `LisaScaffold`, and use the Material widget vocabulary lisa_ui re-exports \
(ElevatedButton, ListView, TextField, showDialog) plus `LisaCard`, `LisaStreamText`, \
and `ConsentChip` where they fit.
- When the task is complete, reply with a short summary and NO tool call.";

/// The agent loop: converse with the backend one tool call at a time,
/// executing each call against the jail, until done or out of turns.
pub fn forge_agent(
    task: &str,
    project: &Path,
    backend: &mut dyn Backend,
    config: &AgentConfig,
) -> Result<AgentReport, ForgeError> {
    let jail = Jail::new(project)?;
    let specs = tool_specs();
    let mut history = vec![
        Message::system(SYSTEM_PROMPT),
        Message::user(format!("Task: {task}")),
    ];
    let mut verifier_output = String::new();
    for turn in 1..=config.max_turns {
        match backend.next_action(&history, &specs)? {
            AgentAction::Done(summary) => {
                // "Done" only counts if the verifier agrees. A `None`
                // verifier always agrees — the model's word is the check.
                match config.verifier.check(project)? {
                    None => {
                        return Ok(AgentReport {
                            turns: turn,
                            summary,
                            verifier_output,
                            verified: !config.verifier.is_none(),
                        });
                    }
                    Some(findings) => {
                        history.push(Message::assistant_text(&summary));
                        history.push(Message::user(format!(
                            "You said you were done, but the verifier still reports:\n\
                             {findings}\nKeep working."
                        )));
                        verifier_output = findings;
                    }
                }
            }
            AgentAction::Call(call) => {
                let outcome = execute_tool(&jail, &call);
                history.push(Message::assistant_call(call.clone()));
                history.push(Message::tool_result(call.id.clone(), outcome.text));
                if outcome.mutated {
                    // The project changed: a passing verifier ends the loop
                    // immediately, findings go back into the conversation.
                    match config.verifier.check(project)? {
                        None if !config.verifier.is_none() => {
                            return Ok(AgentReport {
                                turns: turn,
                                summary: String::new(),
                                verifier_output: String::new(),
                                verified: true,
                            });
                        }
                        Some(findings) => {
                            history.push(Message::user(format!(
                                "Verifier findings after your edit:\n{findings}\nFix them."
                            )));
                            verifier_output = findings;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Err(ForgeError::NoConvergence(config.max_turns))
}

/// A deterministic backend for tests: replays a fixed script of actions
/// and records what it was shown. `new` fails once the script runs out;
/// `repeating` replays the last action forever (a stuck model).
pub struct ScriptedBackend {
    actions: std::collections::VecDeque<AgentAction>,
    last: Option<AgentAction>,
    repeat_last: bool,
    /// How many turns the loop asked for.
    pub calls: usize,
    /// The history as of the most recent call — the full conversation,
    /// since earlier snapshots are prefixes of it.
    pub last_history: Vec<Message>,
}

impl ScriptedBackend {
    pub fn new(actions: Vec<AgentAction>) -> Self {
        Self {
            actions: actions.into(),
            last: None,
            repeat_last: false,
            calls: 0,
            last_history: Vec::new(),
        }
    }

    pub fn repeating(actions: Vec<AgentAction>) -> Self {
        Self {
            repeat_last: true,
            ..Self::new(actions)
        }
    }
}

impl Backend for ScriptedBackend {
    fn next_action(
        &mut self,
        messages: &[Message],
        _tools: &[ToolSpec],
    ) -> Result<AgentAction, ForgeError> {
        self.calls += 1;
        self.last_history = messages.to_vec();
        if let Some(action) = self.actions.pop_front() {
            self.last = Some(action.clone());
            return Ok(action);
        }
        if self.repeat_last
            && let Some(last) = &self.last
        {
            return Ok(last.clone());
        }
        Err(ForgeError::Backend("script exhausted".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_main(content: &str) -> AgentAction {
        AgentAction::Call(ToolCall {
            id: "c1".into(),
            name: "write_file".into(),
            args: json!({"path": "bin/main.dart", "content": content}),
        })
    }

    fn available(program: &str) -> bool {
        Command::new(program).arg("--version").output().is_ok()
    }

    #[test]
    fn dart_verifier_reports_findings_on_a_sourceless_project() {
        // Issue #29: `dart analyze` passes vacuously with no sources, so
        // check() must report findings before ever invoking dart — which
        // also keeps this test independent of a dart install.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pubspec.yaml"), "name: t\n").unwrap();
        let findings = Verifier::Dart.check(dir.path()).unwrap();
        assert!(
            findings.is_some_and(|f| f.contains("no Dart source files")),
            "empty scaffold must not verify clean"
        );
    }

    #[test]
    fn bare_done_on_an_empty_scaffold_does_not_converge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pubspec.yaml"), "name: t\n").unwrap();
        let mut backend = ScriptedBackend::new(vec![
            AgentAction::Done("all done".into()),
            AgentAction::Done("really done".into()),
        ]);
        let config = AgentConfig {
            max_turns: 2,
            verifier: Verifier::Dart,
        };
        let err = forge_agent("build", dir.path(), &mut backend, &config).unwrap_err();
        assert!(matches!(err, ForgeError::NoConvergence(2)));
    }

    #[test]
    fn done_signal_ends_the_loop_with_no_verifier() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = ScriptedBackend::new(vec![
            write_main("void main() {}\n"),
            AgentAction::Done("built it".into()),
        ]);
        let config = AgentConfig {
            max_turns: 8,
            verifier: Verifier::None,
        };
        let report = forge_agent("build", dir.path(), &mut backend, &config).unwrap();
        assert_eq!(report.turns, 2);
        assert_eq!(report.summary, "built it");
        assert!(!report.verified, "Verifier::None verifies nothing");
        assert!(dir.path().join("bin/main.dart").exists());
    }

    #[test]
    fn passing_verifier_ends_the_loop_right_after_an_edit() {
        if !available("true") {
            eprintln!("skipping: `true` not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut backend = ScriptedBackend::new(vec![write_main("void main() {}\n")]);
        let config = AgentConfig {
            max_turns: 8,
            verifier: Verifier::Command {
                program: "true".into(),
                args: vec![],
            },
        };
        let report = forge_agent("build", dir.path(), &mut backend, &config).unwrap();
        assert_eq!(report.turns, 1, "clean verifier converges immediately");
        assert!(report.verified);
    }

    #[test]
    fn failing_verifier_is_fed_back_and_runs_out_of_turns() {
        if !available("false") {
            eprintln!("skipping: `false` not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut backend = ScriptedBackend::repeating(vec![write_main("void main() {}\n")]);
        let config = AgentConfig {
            max_turns: 3,
            verifier: Verifier::Command {
                program: "false".into(),
                args: vec![],
            },
        };
        let err = forge_agent("build", dir.path(), &mut backend, &config);
        assert!(matches!(err, Err(ForgeError::NoConvergence(3))));
        let feedback = backend
            .last_history
            .iter()
            .any(|m| m.role == Role::User && m.content.contains("Verifier findings"));
        assert!(
            feedback,
            "findings must reach the model: {:?}",
            backend.last_history
        );
    }

    #[test]
    fn done_with_findings_keeps_the_loop_going() {
        if !available("false") {
            eprintln!("skipping: `false` not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // Model writes, then prematurely claims done twice: both claims are
        // rejected by the verifier, and the script runs out → Backend error,
        // not a silent success.
        let mut backend = ScriptedBackend::new(vec![
            write_main("void main() {}\n"),
            AgentAction::Done("done".into()),
            AgentAction::Done("really done".into()),
        ]);
        let config = AgentConfig {
            max_turns: 8,
            verifier: Verifier::Command {
                program: "false".into(),
                args: vec![],
            },
        };
        let err = forge_agent("build", dir.path(), &mut backend, &config);
        assert!(matches!(err, Err(ForgeError::Backend(_))));
        let rejected = backend
            .last_history
            .iter()
            .filter(|m| m.role == Role::User && m.content.contains("You said you were done"))
            .count();
        assert_eq!(rejected, 2);
    }

    #[test]
    fn tool_results_reach_the_backend() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "jailhouse rock").unwrap();
        let mut backend = ScriptedBackend::new(vec![
            AgentAction::Call(ToolCall {
                id: "r1".into(),
                name: "read_file".into(),
                args: json!({"path": "note.txt"}),
            }),
            AgentAction::Done("read it".into()),
        ]);
        let config = AgentConfig {
            max_turns: 4,
            verifier: Verifier::None,
        };
        forge_agent("inspect", dir.path(), &mut backend, &config).unwrap();
        let tool_msg = backend
            .last_history
            .iter()
            .find(|m| m.role == Role::Tool)
            .expect("a tool result message");
        assert_eq!(tool_msg.tool_call_id.as_deref(), Some("r1"));
        assert!(tool_msg.content.contains("jailhouse rock"));
    }
}
