//! The multi-turn agent loop. The harness owns the message history; each
//! turn the backend either issues one tool call or signals done. Tool
//! results (and verifier findings after each mutation) are appended to
//! the history, and the loop continues until the model signals done with
//! a clean verifier, the verifier passes right after an edit, or the turn
//! budget runs out.

use crate::jail::Jail;
use crate::tools::{ToolCall, ToolOutcome, ToolSpec, execute_tool, tool_specs};
use crate::{Backend, ForgeError, analyze};
use lisa_ledger::{Event, Ledger, preview_of};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

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

/// Any `.dart` regular file under the project (skipping the `.dart_tool`
/// cache) counts as source; the pubspec scaffold alone does not. Symlinks
/// are not followed (#33): a linked directory could walk outside the
/// project or cycle forever, and a dangling `x.dart` link is not source.
fn has_dart_sources(project: &Path) -> bool {
    fn walk(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let Ok(ftype) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if ftype.is_dir() {
                if path.file_name().is_some_and(|n| n == ".dart_tool") {
                    continue;
                }
                if walk(&path) {
                    return true;
                }
            } else if ftype.is_file() && path.extension().is_some_and(|e| e == "dart") {
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
    /// Transcript size ceiling: beyond it, stale tool results are elided
    /// oldest-first (small local models drown in old file dumps).
    pub history_char_budget: usize,
    /// Where this run is recorded (issue #54). `VISION.md` promises that
    /// "every action it took is in the Ledger", and this loop — the one
    /// thing that edits your files with nobody watching — recorded
    /// nothing at all until now.
    ///
    /// The agentd contract, and NOT optional (issue #129). Every tool
    /// call is appended before it runs, and a failed append aborts the
    /// run.
    ///
    /// This was an `Option` defaulting to `None`, which made "no ledger
    /// entry, no action" something a caller opted into — so `forge()`
    /// and `AgentConfig::default()` edited files with nobody watching
    /// and recorded nothing. An invariant a caller can forget is not an
    /// invariant. There is no `Default` for this struct for the same
    /// reason: constructing one requires deciding where the record goes.
    pub ledger: Arc<Ledger>,
}

impl AgentConfig {
    /// Defaults, with the one decision that cannot be defaulted.
    pub fn new(ledger: Arc<Ledger>) -> Self {
        Self {
            max_turns: 32,
            verifier: Verifier::Dart,
            history_char_budget: 48_000,
            ledger,
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

/// What the loop is doing right now — surfaced to observers so a CLI can
/// narrate the run live (the difference between an agent and a spinner).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Turn { n: usize, max: usize },
    Call { name: String, detail: String },
    CallResult { ok: bool, chars: usize },
    VerifierFindings { chars: usize },
    VerifierClean,
    DoneClaimed,
}

/// Replace stale bulky tool results with a stub once the transcript
/// outgrows `budget_chars`, oldest-first, always keeping the most recent
/// `keep_recent` tool results verbatim. Small local models drown in old
/// file dumps long before they run out of turns.
fn elide_stale_tool_results(history: &mut [Message], budget_chars: usize, keep_recent: usize) {
    let total: usize = history.iter().map(|m| m.content.len()).sum();
    if total <= budget_chars {
        return;
    }
    let tool_idx: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::Tool && !m.content.starts_with("[elided"))
        .map(|(i, _)| i)
        .collect();
    let mut excess = total.saturating_sub(budget_chars);
    for &i in tool_idx.iter().rev().skip(keep_recent).rev() {
        if excess == 0 {
            break;
        }
        let dropped = history[i].content.len();
        history[i].content =
            format!("[elided {dropped}-char tool result — re-run the tool if needed]");
        excess = excess.saturating_sub(dropped);
    }
}

/// A family of tools the loop can offer the model (ADR-0025). The
/// workspace family (jailed file + command access) is the one this crate
/// ships; the Agent Bus family and the harness family (memory, skills)
/// live with their owners and plug in here, so every surface runs ONE
/// loop instead of re-deriving routing per verb.
pub trait ToolProvider {
    fn specs(&self) -> Vec<ToolSpec>;
    fn execute(&self, call: &ToolCall) -> ToolOutcome;
}

/// The jailed workspace tools: read/write/edit/grep/list/run inside one
/// project directory, path traversal impossible by construction.
pub struct WorkspaceTools {
    jail: Jail,
}

impl WorkspaceTools {
    pub fn new(project: &Path) -> Result<Self, ForgeError> {
        Ok(Self {
            jail: Jail::new(project)?,
        })
    }
}

impl ToolProvider for WorkspaceTools {
    fn specs(&self) -> Vec<ToolSpec> {
        tool_specs()
    }

    fn execute(&self, call: &ToolCall) -> ToolOutcome {
        execute_tool(&self.jail, call)
    }
}

/// Merge the families into one catalog. First provider to claim a name
/// wins, so a caller's ordering is its precedence — and a later family
/// can never silently shadow the jail.
fn dispatch<'a>(providers: &'a [&dyn ToolProvider], name: &str) -> Option<&'a dyn ToolProvider> {
    providers
        .iter()
        .copied()
        .find(|p| p.specs().iter().any(|s| s.name == name))
}

/// The agent loop: converse with the backend one tool call at a time,
/// executing each call against the jail, until done or out of turns.
pub fn forge_agent(
    task: &str,
    project: &Path,
    backend: &mut dyn Backend,
    config: &AgentConfig,
) -> Result<AgentReport, ForgeError> {
    forge_agent_observed(task, project, backend, config, &mut |_| {})
}

/// `forge_agent` with a live observer for every loop event.
pub fn forge_agent_observed(
    task: &str,
    project: &Path,
    backend: &mut dyn Backend,
    config: &AgentConfig,
    observe: &mut dyn FnMut(AgentEvent),
) -> Result<AgentReport, ForgeError> {
    let workspace = WorkspaceTools::new(project)?;
    forge_agent_with_tools(task, project, backend, config, &[&workspace], observe)
}

/// Record the intent of a tool call before it runs, returning the entry
/// id so the outcome can reference it. A failed append is an error: no
/// ledger entry, no action.
fn ledger_start(ledger: &Ledger, task: &str, call: &ToolCall) -> Result<Option<i64>, ForgeError> {
    let args = call.args.to_string();
    Ok(Some(ledger.append(&Event {
        kind: "forge.tool".into(),
        app_id: "dev.lisaos.forge".into(),
        input_hash: blake3::hash(args.as_bytes()).to_hex().to_string(),
        preview: preview_of(&format!("{} {args}", call.name)),
        status: "started".into(),
        detail: serde_json::json!({ "tool": call.name, "task": task }).to_string(),
        ..Default::default()
    })?))
}

/// Record what the call actually did. A guard refusal is called out
/// explicitly — a refused action is the most interesting line in the log,
/// and burying it under a generic "error" would waste it (ADR-0029).
fn ledger_finish(
    ledger: &Ledger,
    call_ref: Option<i64>,
    call: &ToolCall,
    outcome: &ToolOutcome,
) -> Result<(), ForgeError> {
    let refused = outcome.text.starts_with("error: refused")
        || outcome.text.starts_with("error: needs confirmation");
    let status = if refused {
        "refused"
    } else if outcome.text.starts_with("error") {
        "failed"
    } else {
        "ok"
    };
    ledger.append(&Event {
        kind: "forge.tool".into(),
        app_id: "dev.lisaos.forge".into(),
        preview: preview_of(&outcome.text),
        status: status.into(),
        detail: serde_json::json!({
            "tool": call.name,
            "mutated": outcome.mutated,
        })
        .to_string(),
        ref_id: call_ref,
        ..Default::default()
    })?;
    Ok(())
}

/// The loop itself, over any set of tool families (ADR-0025 phase 1).
/// `project` remains the verifier's working directory; tools come from
/// `providers`, so a caller with no workspace at all is legitimate.
pub fn forge_agent_with_tools(
    task: &str,
    project: &Path,
    backend: &mut dyn Backend,
    config: &AgentConfig,
    providers: &[&dyn ToolProvider],
    observe: &mut dyn FnMut(AgentEvent),
) -> Result<AgentReport, ForgeError> {
    let specs: Vec<ToolSpec> = providers.iter().flat_map(|p| p.specs()).collect();
    let mut history = vec![
        Message::system(SYSTEM_PROMPT),
        Message::user(format!("Task: {task}")),
    ];
    let mut verifier_output = String::new();
    for turn in 1..=config.max_turns {
        elide_stale_tool_results(&mut history, config.history_char_budget, 4);
        observe(AgentEvent::Turn {
            n: turn,
            max: config.max_turns,
        });
        match backend.next_action(&history, &specs)? {
            AgentAction::Done(summary) => {
                observe(AgentEvent::DoneClaimed);
                // "Done" only counts if the verifier agrees. A `None`
                // verifier always agrees — the model's word is the check.
                match config.verifier.check(project)? {
                    None => {
                        observe(AgentEvent::VerifierClean);
                        return Ok(AgentReport {
                            turns: turn,
                            summary,
                            verifier_output,
                            verified: !config.verifier.is_none(),
                        });
                    }
                    Some(findings) => {
                        observe(AgentEvent::VerifierFindings {
                            chars: findings.len(),
                        });
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
                observe(AgentEvent::Call {
                    name: call.name.clone(),
                    detail: call
                        .args
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or_default()
                        .to_string(),
                });
                // No ledger entry, no action (issue #54): the intent is
                // recorded BEFORE the tool runs, so a crash mid-write
                // still leaves evidence of what was attempted. A failed
                // append aborts the run rather than acting unobserved.
                let call_ref = ledger_start(&config.ledger, task, &call)?;

                let outcome = match dispatch(providers, &call.name) {
                    Some(p) => p.execute(&call),
                    None => ToolOutcome::err(format!(
                        "unknown tool `{}`; available: {}",
                        call.name,
                        specs.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
                    )),
                };
                ledger_finish(&config.ledger, call_ref, &call, &outcome)?;
                observe(AgentEvent::CallResult {
                    ok: !outcome.text.starts_with("error"),
                    chars: outcome.text.len(),
                });
                history.push(Message::assistant_call(call.clone()));
                history.push(Message::tool_result(call.id.clone(), outcome.text));
                if outcome.mutated {
                    // The project changed: a passing verifier ends the loop
                    // immediately, findings go back into the conversation.
                    match config.verifier.check(project)? {
                        None if !config.verifier.is_none() => {
                            observe(AgentEvent::VerifierClean);
                            return Ok(AgentReport {
                                turns: turn,
                                summary: String::new(),
                                verifier_output: String::new(),
                                verified: true,
                            });
                        }
                        Some(findings) => {
                            observe(AgentEvent::VerifierFindings {
                                chars: findings.len(),
                            });
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

    /// ADR-0025: a second tool family plugs into the same loop, and the
    /// workspace family keeps precedence over a later one claiming the
    /// same name — a bus tool can never shadow the jail.
    #[test]
    fn a_second_tool_family_joins_the_same_loop() {
        struct BusLike;
        impl ToolProvider for BusLike {
            fn specs(&self) -> Vec<ToolSpec> {
                vec![
                    ToolSpec {
                        name: "create_note",
                        description: "bus tool",
                        parameters: json!({"type": "object", "properties": {}}),
                    },
                    // Deliberately collides with the workspace family.
                    ToolSpec {
                        name: "write_file",
                        description: "impostor",
                        parameters: json!({"type": "object", "properties": {}}),
                    },
                ]
            }
            fn execute(&self, call: &ToolCall) -> ToolOutcome {
                ToolOutcome::ok(format!("bus ran {}", call.name), false)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pubspec.yaml"), "name: t\n").unwrap();
        let workspace = WorkspaceTools::new(dir.path()).unwrap();
        let bus = BusLike;
        let providers: [&dyn ToolProvider; 2] = [&workspace, &bus];

        // The catalog carries both families.
        let names: Vec<&str> = providers
            .iter()
            .flat_map(|p| p.specs())
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"read_file") && names.contains(&"create_note"));

        // The bus tool dispatches to the bus...
        let mut backend = ScriptedBackend::new(vec![
            AgentAction::Call(ToolCall {
                id: "c1".into(),
                name: "create_note".into(),
                args: json!({}),
            }),
            AgentAction::Done("noted".into()),
        ]);
        let config = AgentConfig {
            max_turns: 4,
            verifier: Verifier::None,
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };
        let report = forge_agent_with_tools(
            "note it",
            dir.path(),
            &mut backend,
            &config,
            &providers,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.summary, "noted");
        let ran = backend
            .last_history
            .iter()
            .any(|m| m.role == Role::Tool && m.content.contains("bus ran create_note"));
        assert!(ran, "the bus family should have executed its own tool");

        // ...while the colliding name still resolves to the workspace.
        let claimed = dispatch(&providers, "write_file").unwrap();
        let out = claimed.execute(&ToolCall {
            id: "c2".into(),
            name: "write_file".into(),
            args: json!({"path": "a.txt", "content": "x"}),
        });
        assert!(
            !out.text.contains("bus ran"),
            "workspace must win a name collision, got: {}",
            out.text
        );
        assert!(dir.path().join("a.txt").exists(), "the jailed write ran");
    }

    /// A throwaway Ledger for tests.
    ///
    /// Every test now needs one, which is the point of #129: the loop
    /// cannot run unledgered even by accident, so the tests exercise the
    /// same path production does.
    fn scratch_ledger(dir: &std::path::Path) -> std::sync::Arc<lisa_ledger::Ledger> {
        std::sync::Arc::new(lisa_ledger::Ledger::open(dir.join("test-ledger.db")).unwrap())
    }

    /// Issue #54: the loop that edits your files unattended used to
    /// record nothing, while VISION.md promised "every action it took
    /// is in the Ledger". Every tool call now lands twice — intent
    /// before, outcome after — and a guard refusal is called out as
    /// its own status rather than buried under "failed".
    #[test]
    fn every_tool_call_lands_in_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let ledger =
            std::sync::Arc::new(lisa_ledger::Ledger::open(dir.path().join("l.db")).unwrap());

        // write_file succeeds; run_command is refused by lisa-guard.
        let mut backend = ScriptedBackend::new(vec![
            AgentAction::Call(ToolCall {
                id: "1".into(),
                name: "write_file".into(),
                args: serde_json::json!({"path": "a.txt", "content": "hi"}),
            }),
            AgentAction::Call(ToolCall {
                id: "2".into(),
                name: "run_command".into(),
                args: serde_json::json!({"program": "sh", "args": ["-c", "id"]}),
            }),
            AgentAction::Done("done".into()),
        ]);
        let config = AgentConfig {
            verifier: Verifier::None,
            ledger: ledger.clone(),
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };
        forge_agent(
            "write a file then try a shell",
            &project,
            &mut backend,
            &config,
        )
        .unwrap();

        let entries = ledger.tail(50).unwrap();
        let forge: Vec<_> = entries.iter().filter(|e| e.kind == "forge.tool").collect();
        assert_eq!(
            forge.len(),
            4,
            "two calls => intent + outcome each: {forge:?}"
        );

        let statuses: Vec<&str> = forge.iter().map(|e| e.status.as_str()).collect();
        assert_eq!(statuses.iter().filter(|s| **s == "started").count(), 2);
        assert!(
            statuses.contains(&"ok"),
            "the write should be ok: {statuses:?}"
        );
        assert!(
            statuses.contains(&"refused"),
            "the guard refusal must be distinguishable from a plain failure: {statuses:?}"
        );
        // The outcome references the intent, so a reader can pair them.
        assert!(
            forge.iter().any(|e| e.ref_id.is_some()),
            "outcomes must reference their intent entry"
        );
    }

    /// No ledger entry, no action: if the Ledger cannot be written the
    /// loop stops rather than editing files unobserved.
    ///
    /// Injecting that failure is fiddlier than it looks. Deleting the
    /// database does nothing — SQLite keeps writing happily to the
    /// unlinked inode, so the first version of this test passed for
    /// entirely the wrong reason. Making the *containing directory*
    /// read-only is what actually fails the write, because SQLite must
    /// create its journal alongside the database.
    #[cfg(unix)]
    #[test]
    fn a_dead_ledger_stops_the_loop_before_it_acts() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        let ldir = dir.path().join("ledger");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&ldir).unwrap();
        let ledger = std::sync::Arc::new(lisa_ledger::Ledger::open(ldir.join("l.db")).unwrap());

        std::fs::set_permissions(&ldir, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Confirm the injection actually took, so a permissive
        // filesystem (or running as root) fails the test loudly rather
        // than making it vacuous.
        let writable = ledger
            .append(&lisa_ledger::Event {
                kind: "probe".into(),
                ..Default::default()
            })
            .is_ok();
        if writable {
            std::fs::set_permissions(&ldir, std::fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipping: this filesystem still allows writes to a 0555 directory");
            return;
        }

        let mut backend = ScriptedBackend::new(vec![AgentAction::Call(ToolCall {
            id: "1".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": "a.txt", "content": "hi"}),
        })]);
        let config = AgentConfig {
            verifier: Verifier::None,
            ledger,
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };
        let result = forge_agent("write", &project, &mut backend, &config);

        std::fs::set_permissions(&ldir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(result, Err(ForgeError::Ledger(_))),
            "an unwritable ledger must abort the run, got {result:?}"
        );
        assert!(
            !project.join("a.txt").exists(),
            "a file was written with no ledger entry"
        );
    }

    #[test]
    fn elision_drops_oldest_tool_results_and_keeps_recent() {
        let mut history = vec![Message::system("s"), Message::user("t")];
        for i in 0..10 {
            history.push(Message::assistant_call(ToolCall {
                id: format!("c{i}"),
                name: "read_file".into(),
                args: json!({"path": "x"}),
            }));
            history.push(Message::tool_result(format!("c{i}"), "y".repeat(1_000)));
        }
        elide_stale_tool_results(&mut history, 6_000, 4);
        let elided: Vec<bool> = history
            .iter()
            .filter(|m| m.role == Role::Tool)
            .map(|m| m.content.starts_with("[elided"))
            .collect();
        assert!(elided[0], "oldest tool result must be elided");
        assert!(
            elided.iter().rev().take(4).all(|e| !e),
            "the 4 most recent tool results must survive verbatim"
        );
        let total: usize = history.iter().map(|m| m.content.len()).sum();
        assert!(
            total < 10_000,
            "transcript must actually shrink, got {total}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn dart_source_walk_ignores_symlinks() {
        // #33: a symlinked dir must not let the walk escape the project,
        // and a dangling `x.dart` link is not source.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("real.dart"), "void main() {}\n").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pubspec.yaml"), "name: t\n").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
        std::os::unix::fs::symlink("/nonexistent/x.dart", dir.path().join("ghost.dart")).unwrap();
        assert!(
            !has_dart_sources(dir.path()),
            "symlinked/dangling entries must not count as source"
        );
        std::fs::write(dir.path().join("real.dart"), "void main() {}\n").unwrap();
        assert!(has_dart_sources(dir.path()));
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
            ..AgentConfig::new(scratch_ledger(dir.path()))
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
            ..AgentConfig::new(scratch_ledger(dir.path()))
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
            ..AgentConfig::new(scratch_ledger(dir.path()))
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
            ..AgentConfig::new(scratch_ledger(dir.path()))
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
            ..AgentConfig::new(scratch_ledger(dir.path()))
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
            ..AgentConfig::new(scratch_ledger(dir.path()))
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
