//! The multi-turn agent loop. The harness owns the message history; each
//! turn the backend either issues one tool call or signals done. Tool
//! results (and verifier findings after each mutation) are appended to
//! the history, and the loop continues until the model signals done with
//! a clean verifier, the verifier passes right after an edit, or the turn
//! budget runs out.

use crate::jail::Jail;
use crate::tools::{RefusalMemory, ToolCall, ToolOutcome, ToolSpec, execute_tool, tool_specs};
use crate::{Backend, ForgeError, analyze};
use lisa_ledger::{Event, Ledger, preview_of};
use serde_json::Value;
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
    /// Non-text content parts riding along with a USER turn — an image
    /// a person attached, in the OpenAI part shape (issue #209).
    ///
    /// Opaque `Value`s on purpose, the same decision inferenced made for
    /// `Content::Parts`: the harness does not re-model every provider's
    /// part schema, it passes them through. Re-modelling means a release
    /// per new modality and a silent drop for anything unmodelled — and
    /// a dropped image still gets a confident answer about a picture
    /// nobody saw.
    ///
    /// Empty is the normal case, and an empty vec must stay
    /// wire-identical to a message that never had the field: a plain
    /// string `content`, which every text-only engine understands.
    pub attachments: Vec<Value>,
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

    /// A user turn carrying attachments (issue #209). The words stay in
    /// `content`; the parts ride alongside and are ordered after the
    /// text on the wire.
    pub fn user_with_attachments(content: impl Into<String>, attachments: Vec<Value>) -> Self {
        let mut msg = Self::bare(Role::User, content);
        msg.attachments = attachments;
        msg
    }

    fn bare(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call: None,
            tool_call_id: None,
            attachments: Vec::new(),
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

/// Ambient context that arrives BETWEEN turns (ADR-0061 steal 1).
///
/// The producer — harnessd's Context1 search task, or anything else —
/// runs on its own clock and queues fenced text; the loop drains the
/// queue at each turn boundary and never waits. Retrieval kicked off
/// during turn N surfaces at turn N+1, and a turn with nothing arrived
/// costs nothing. The LOOP stays ignorant of sources: provenance
/// fencing and taint are the producer's obligation (rule 6), because
/// the producer is the one that knows where the text came from.
pub trait AmbientSource: Send + Sync {
    /// Everything queued since the last call, already fenced.
    /// MUST return immediately — blocking here stalls the whole run.
    fn take(&self) -> Option<String>;
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
    /// Between-turn ambient context, if any surface feeds one. `None`
    /// means turns see only the prompt, tools and their own history —
    /// the pre-ADR-0061 behaviour, and still the default everywhere a
    /// person has not asked for retrieval.
    pub ambient: Option<Arc<dyn AmbientSource>>,
    /// The skills this run may load — the set `read_skill` serves and
    /// the system prompt's catalog advertises.
    ///
    /// A skill's `tools:` frontmatter is an allowlist, and it was parsed
    /// and never consulted: `allows_tool()` had no callers outside its
    /// own unit tests (#57). The enforcement point below closed that,
    /// and then nothing populated this field except a unit test (#245) —
    /// so `allowed_by` intersected an empty slice on every shipping
    /// surface and a `tools:` line constrained nothing.
    ///
    /// **A skill starts constraining when its body is served, not when
    /// its file exists.** The loop watches for a successful
    /// [`crate::READ_SKILL`] call and marks that skill active for the
    /// rest of the run. Two reasons, and the second is the one that
    /// matters:
    ///
    /// * A skill installed on the machine is an *offer*. Applying every
    ///   offer's allowlist would mean one scoped workflow in
    ///   `~/.local/share/lisa/skills` silently narrowing every unrelated
    ///   conversation.
    /// * The allowlist attaches at the moment untrusted text enters the
    ///   context. A third-party skill body (ADR-0049) cannot widen its
    ///   own allowlist, because the loop applies the frontmatter of the
    ///   body it *served* — not anything the body says. What the model
    ///   controls is whether to load the skill at all, and declining
    ///   forfeits the workflow without gaining a tool.
    ///
    /// This is scoping, not a guardrail in the ADR-0030 sense: it is not
    /// what stands between the model and the machine (that is the guard,
    /// the jail and the tiers). Empty means no restriction — the loop is
    /// running without a skill, not under an empty one.
    pub skills: Vec<harness_core::Skill>,
    /// What the model is told it IS.
    ///
    /// This used to be a constant, and the constant said "you are the
    /// Lisa Forge, an autonomous coding agent working inside a jailed
    /// project directory". Correct for `lisa forge`, and quietly wrong
    /// for every other surface once the loop became a service (ADR-0025):
    /// an Assistant asked about the open web page was being told it was
    /// a coding agent in a project directory, which is a good way to get
    /// an answer about files.
    ///
    /// `AgentConfig::new` keeps the forge prompt, so `forge` is unchanged
    /// and the caller that needs something else has to say so.
    pub system_prompt: String,
    /// Prior turns of THIS conversation, replayed before the new one.
    ///
    /// The loop is deliberately stateless: it holds no session of its
    /// own, so a daemon running it for several clients cannot leak one
    /// conversation into another, and there is no shared store to get
    /// the ownership of wrong. The client owns its history and hands it
    /// over per run — which is also why sessions stay wherever the
    /// client already keeps them.
    pub prior_turns: Vec<Message>,
    /// Non-text parts attached to THIS task by the person who typed it
    /// (issue #209) — an image from the Assistant's composer.
    ///
    /// They ride on the task turn and on nothing else. Per-run rather
    /// than per-message on purpose: the loop appends tool results by the
    /// thousand, and there must be no path by which something the model
    /// produced grows an image a human never showed it.
    ///
    /// Empty is the normal case and changes nothing on the wire.
    pub attachments: Vec<Value>,
    /// The stop button (issue #227). A default one is never set, so a
    /// caller with nothing to stop the loop with runs exactly as before.
    pub cancel: crate::Cancel,
}

impl AgentConfig {
    /// Defaults, with the one decision that cannot be defaulted.
    pub fn new(ledger: Arc<Ledger>) -> Self {
        Self {
            max_turns: 32,
            verifier: Verifier::Dart,
            history_char_budget: 48_000,
            ledger,
            system_prompt: FORGE_SYSTEM_PROMPT.to_string(),
            prior_turns: Vec::new(),
            skills: Vec::new(),
            attachments: Vec::new(),
            cancel: crate::Cancel::default(),
            ambient: None,
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

/// What every forge run is told it is — and, from ADR-0047 on, what a
/// Lisa app is made of.
///
/// This is the instruction the model receives, not documentation about
/// it. It described the Flutter lane for four months after ADR-0047
/// parked that lane (#246), which is why the assertions in
/// `prompt_tests` below exist: a prompt is the one piece of "docs" that
/// changes behaviour, and the only thing that can notice it going stale
/// is a test.
///
/// It names what `lisa dev check` (ADR-0050) actually checks, because
/// the loop feeds that verifier's findings back as the next turn's
/// instructions — a model that has to discover the manifest rule by
/// failing spends turns learning what the prompt could have told it.
pub const FORGE_SYSTEM_PROMPT: &str = "\
You are the Lisa Forge, an autonomous coding agent working inside a jailed project \
directory. You inspect and modify the project by calling the provided tools.

Rules:
- ALL paths are project-relative (e.g. `lisa-notes.js`, `lib/notes.js`). Never \
absolute, never containing `..` — the jail rejects them and the write does not happen.
- Inspect before you edit: use `list_dir`, `read_file`, and `grep` to understand the \
project. Use `edit_file` for targeted changes, `write_file` for new files or complete \
rewrites.
- `run_command` is allowlisted and runs in the project root; use it for toolchain \
commands. Use `run_tests` to run the test suite.
- Analyzer/verifier findings are fed back to you after each edit; fix them.

A Lisa app is GJS + GTK4/Adwaita. It is interpreted source — there is no build step, \
no package manifest to resolve, and nothing to compile. Write these files:

- `lisa-<name>.js` — the entry module. Starts `#!/usr/bin/env -S gjs -m`, imports \
`gi://Gtk?version=4.0` and `gi://Adw?version=1`, and runs an `Adw.Application` whose \
`application_id` is the app id.
- `lib/*.js` — the logic, imported by BOTH the window and the tests. No `gi://` \
imports in here: that is what makes it testable.
- `tests/*.test.js` — one suite per `lib/` module, importing from `../lib/`.
- `app.lisaos.<Name>.json` — the MCP manifest, and the thing that makes a window an \
app. `{\"lisa_manifest\": 1, \"app_id\": \"app.lisaos.<Name>\", \"tools\": [...]}`, where \
each tool has a `name`, a `tier` of read/write/destructive, and an `input_schema` \
whose `type` is `object`. Without it the app publishes no capability and the model \
can never reach it.
- `app.lisaos.<Name>.desktop` with `Exec=lisa-app <name>/lisa-<name>.js`.

Two rules that are checked and are easy to get wrong:

- NEVER use top-level `await` in an entry module. It binds the socket, advertises \
it, and then answers nothing — with no error in any log. Put awaited work inside the \
function that needs it.
- The app id `app.lisaos.<Name>` is ONE string used everywhere: the manifest \
filename, the manifest `app_id`, the `.desktop` basename, the icon name, and the \
`Adw.Application` id. Two spellings of it is a bug nobody sees until launch.

When the task is complete, reply with a short summary and NO tool call.";

/// What the loop is doing right now — surfaced to observers so a CLI can
/// narrate the run live (the difference between an agent and a spinner).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Turn {
        n: usize,
        max: usize,
    },
    /// A fragment of assistant text, as it arrives.
    Delta(String),
    /// Ambient context joined the conversation at a turn boundary
    /// (ADR-0061 steal 1) — surfaced so no transcript ever contains
    /// influence nobody saw arrive.
    Ambient {
        chars: usize,
    },
    Call {
        name: String,
        detail: String,
    },
    CallResult {
        ok: bool,
        chars: usize,
    },
    VerifierFindings {
        chars: usize,
    },
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
    /// Per-run refusal memory (ADR-0061 steal 2): identical refused
    /// commands cost more each time and mute at three. Run-scoped by
    /// construction — a fresh WorkspaceTools starts clean.
    refusals: RefusalMemory,
}

impl WorkspaceTools {
    pub fn new(project: &Path) -> Result<Self, ForgeError> {
        Ok(Self {
            jail: Jail::new(project)?,
            refusals: RefusalMemory::default(),
        })
    }
}

impl ToolProvider for WorkspaceTools {
    fn specs(&self) -> Vec<ToolSpec> {
        tool_specs()
    }

    fn execute(&self, call: &ToolCall) -> ToolOutcome {
        execute_tool(&self.jail, &self.refusals, call)
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
    // A containment breach is not a typo (#126). Both arrive as
    // `error: …`, so both were ledgered as "failed" — indistinguishable
    // from a missing file in the one record meant to tell you what your
    // agent did. An attempt to write outside the jail is the single most
    // interesting thing this loop can do, and it deserves a status a
    // reader can search for.
    let escaped = outcome.text.contains("escapes the project jail")
        || outcome.text.contains("command.path_escape");
    let status = if escaped {
        "escaped"
    } else if refused {
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

/// A skill starts driving the moment its body reaches the model.
///
/// Keyed off the tool NAME the loop dispatched and the skill name in the
/// arguments it dispatched with — both of which the loop chose to send —
/// so the served text cannot nominate itself as some other skill. A
/// failed read activates nothing: the model did not get the workflow, so
/// it is not following it.
fn activate_served_skill(
    offered: &[harness_core::Skill],
    active: &mut Vec<harness_core::Skill>,
    call: &ToolCall,
    outcome: &ToolOutcome,
) {
    if call.name != crate::READ_SKILL || outcome.is_err() {
        return;
    }
    let Some(name) = call.args.get("name").and_then(|v| v.as_str()) else {
        return;
    };
    if let Some(skill) = offered.iter().find(|s| s.name == name)
        && !active.iter().any(|a| a.name == skill.name)
    {
        active.push(skill.clone());
    }
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
    let mut history = vec![Message::system(&config.system_prompt)];
    history.extend(config.prior_turns.iter().cloned());
    history.push(Message::user_with_attachments(
        format!("Task: {task}"),
        config.attachments.clone(),
    ));
    let mut verifier_output = String::new();
    // The skills that are actually driving (#245). Grows only when the
    // loop itself serves a body, which is the one event that means "this
    // workflow is now in the conversation".
    let mut active: Vec<harness_core::Skill> = Vec::new();
    for turn in 1..=config.max_turns {
        // Stop, checked where stopping is free (#227): before a turn
        // begins, and again below once the backend has answered but
        // before its tool call is dispatched. Never between a tool
        // starting and finishing.
        if config.cancel.is_cancelled() {
            return Err(ForgeError::Cancelled);
        }
        elide_stale_tool_results(&mut history, config.history_char_budget, 4);
        // Ambient context queued since the last boundary joins the
        // conversation HERE — visibly (the event keeps the transcript
        // honest) and as a system message, so the backend reads it as
        // situation rather than as something the person just said.
        if let Some(src) = &config.ambient
            && let Some(block) = src.take()
        {
            observe(AgentEvent::Ambient {
                chars: block.chars().count(),
            });
            history.push(Message::system(block));
        }
        observe(AgentEvent::Turn {
            n: turn,
            max: config.max_turns,
        });
        // Streaming: a person watching a chat window sees words appear
        // instead of a spinner. Control flow is unchanged — a tool call
        // is only knowable once complete — so this is purely about how
        // the wait feels, which for a chat surface is most of the thing.
        let action = {
            let mut sink = |d: &str| observe(AgentEvent::Delta(d.to_string()));
            backend.next_action_streaming(&history, &specs, &mut sink, &config.cancel)?
        };
        // The answer is in, nothing has been done with it yet: the last
        // point at which stopping costs only the words on screen.
        if config.cancel.is_cancelled() {
            return Err(ForgeError::Cancelled);
        }
        match action {
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

                let outcome = if !harness_core::Skill::allowed_by(&active, &call.name) {
                    // Refused as tool OUTPUT, not as an error that ends
                    // the run: the model gets told why and can choose
                    // another tool, which is the behaviour that makes an
                    // allowlist a constraint rather than a wall.
                    ToolOutcome::err(format!(
                        "tool `{}` is not permitted by the active skill; allowed: {}",
                        call.name,
                        active
                            .iter()
                            .filter_map(|s| s.tools.as_ref())
                            .flatten()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                } else {
                    match dispatch(providers, &call.name) {
                        Some(p) => p.execute(&call),
                        None => ToolOutcome::err(format!(
                            "unknown tool `{}`; available: {}",
                            call.name,
                            specs
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                    }
                };
                activate_served_skill(&config.skills, &mut active, &call, &outcome);
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
mod activation_tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "c".into(),
            name: name.into(),
            args,
        }
    }

    fn offered() -> Vec<harness_core::Skill> {
        vec![harness_core::Skill::declared(
            "reader",
            Some(vec!["read_file".into()]),
        )]
    }

    /// Serving a body is what turns a skill on.
    #[test]
    fn a_served_body_activates_that_skill_and_only_that_one() {
        let mut active = Vec::new();
        activate_served_skill(
            &offered(),
            &mut active,
            &call(crate::READ_SKILL, json!({"name": "reader"})),
            &ToolOutcome::ok("Read things.", false),
        );
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "reader");

        // Twice is still once — a model re-reading its workflow must not
        // grow the active list without bound.
        activate_served_skill(
            &offered(),
            &mut active,
            &call(crate::READ_SKILL, json!({"name": "reader"})),
            &ToolOutcome::ok("Read things.", false),
        );
        assert_eq!(active.len(), 1);

        // A name we never offered activates nothing: the loop applies
        // the frontmatter of skills IT loaded, not of names the model
        // invents.
        activate_served_skill(
            &offered(),
            &mut active,
            &call(crate::READ_SKILL, json!({"name": "not-a-skill"})),
            &ToolOutcome::ok("whatever", false),
        );
        assert_eq!(active.len(), 1);
    }

    /// A read that FAILED did not put a workflow in the context, so
    /// nothing is driving. Without this, asking for a misspelled skill
    /// would silently narrow the rest of the run to a workflow the model
    /// never received — a restriction with no visible cause.
    #[test]
    fn a_failed_read_activates_nothing() {
        let mut active = Vec::new();
        activate_served_skill(
            &offered(),
            &mut active,
            &call(crate::READ_SKILL, json!({"name": "reader"})),
            &ToolOutcome::err("reading skill \"reader\": no such file"),
        );
        assert!(active.is_empty(), "a failed read activated a skill");
    }

    /// Only `read_skill` activates. Any other tool that happens to take
    /// a `name` argument — and several do — must not.
    #[test]
    fn another_tool_with_a_name_argument_activates_nothing() {
        let mut active = Vec::new();
        activate_served_skill(
            &offered(),
            &mut active,
            &call("read_file", json!({"name": "reader"})),
            &ToolOutcome::ok("contents", false),
        );
        assert!(active.is_empty());
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::FORGE_SYSTEM_PROMPT as P;

    /// **The prompt is the instruction, not documentation** (#246).
    ///
    /// ADR-0047 made GJS + GTK4/Adwaita the default and parked Flutter,
    /// and this string kept telling every forge run to
    /// `import 'package:lisa_ui/lisa_ui.dart'` and root the app with
    /// `LisaApp`. Correcting the ADR and leaving the prompt is the exact
    /// defect ADR-0047 was written about: a decision that lives only in
    /// `docs/adr/` is a decision nobody follows.
    #[test]
    fn the_forge_is_told_to_write_the_toolkit_we_ship() {
        for gone in [
            "Flutter",
            "flutter",
            "Dart",
            "dart",
            "pubspec",
            "lisa_ui",
            "LisaApp",
            "LisaScaffold",
            ".dart",
        ] {
            assert!(
                !P.contains(gone),
                "the forge prompt still says {gone:?} — ADR-0047 parked that lane"
            );
        }
        assert!(P.contains("GJS"), "the prompt does not name the toolkit");
        assert!(P.contains("Adw.Application"));
    }

    /// The prompt has to describe what the verifier requires, or the
    /// loop cannot converge: `lisa dev check` (ADR-0050) demands an
    /// entry module, a manifest whose name is its `app_id`, and no
    /// top-level `await`. A model told none of that discovers all three
    /// by failing.
    #[test]
    fn the_prompt_names_what_the_verifier_checks() {
        assert!(P.contains("lisa_manifest"), "no manifest instruction");
        assert!(P.contains("app_id"));
        assert!(
            P.contains("top-level `await`"),
            "the footgun with no log line is not mentioned"
        );
    }
}

#[cfg(test)]
mod tests {
    mod ambient_seam {
        use super::*;
        use std::sync::{Arc, Mutex};

        /// A source that hands out queued blocks one per take().
        struct Queued(Mutex<Vec<String>>);
        impl AmbientSource for Queued {
            fn take(&self) -> Option<String> {
                let mut q = self.0.lock().unwrap();
                if q.is_empty() {
                    None
                } else {
                    Some(q.remove(0))
                }
            }
        }

        /// Ambient text queued before a turn is IN the history the
        /// backend reads at that turn, as a system message, and the
        /// event announces it — influence never joins silently
        /// (ADR-0061 steal 1).
        #[test]
        fn queued_ambient_context_surfaces_at_the_next_turn_boundary() {
            let dir = tempfile::tempdir().unwrap();
            let mut backend = ScriptedBackend::new(vec![AgentAction::Done("done".into())]);
            let src = Arc::new(Queued(Mutex::new(vec!["[ambient] AMBIENT-42".into()])));
            let mut events = Vec::new();
            let config = AgentConfig {
                max_turns: 2,
                verifier: Verifier::None,
                ambient: Some(src),
                ..AgentConfig::new(scratch_ledger(dir.path()))
            };
            forge_agent_with_tools("go", dir.path(), &mut backend, &config, &[], &mut |e| {
                if let AgentEvent::Ambient { chars } = e {
                    events.push(chars);
                }
            })
            .unwrap();
            let hits: Vec<_> = backend
                .last_history
                .iter()
                .filter(|m| m.content.contains("AMBIENT-42"))
                .collect();
            assert_eq!(hits.len(), 1, "the backend saw the block exactly once");
            assert!(
                matches!(hits[0].role, Role::System),
                "joined as SYSTEM, not as the person"
            );
            assert_eq!(events, vec!["[ambient] AMBIENT-42".chars().count()]);
        }

        /// No block queued: nothing is injected and no event fires —
        /// the pre-seam behaviour, bit for bit.
        #[test]
        fn a_dry_source_changes_nothing() {
            let dir = tempfile::tempdir().unwrap();
            let mut backend = ScriptedBackend::new(vec![AgentAction::Done("done".into())]);
            let src = Arc::new(Queued(Mutex::new(Vec::new())));
            let mut fired = false;
            let config = AgentConfig {
                max_turns: 2,
                verifier: Verifier::None,
                ambient: Some(src),
                ..AgentConfig::new(scratch_ledger(dir.path()))
            };
            forge_agent_with_tools("go", dir.path(), &mut backend, &config, &[], &mut |e| {
                if matches!(e, AgentEvent::Ambient { .. }) {
                    fired = true;
                }
            })
            .unwrap();
            assert!(!fired);
            assert!(
                !backend
                    .last_history
                    .iter()
                    .any(|m| m.content.contains("ambient"))
            );
        }
    }

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
                    ToolSpec::new(
                        "create_note",
                        "bus tool",
                        json!({"type": "object", "properties": {}}),
                    ),
                    // Deliberately collides with the workspace family.
                    ToolSpec::new(
                        "write_file",
                        "impostor",
                        json!({"type": "object", "properties": {}}),
                    ),
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
        let names: Vec<String> = providers
            .iter()
            .flat_map(|p| p.specs())
            .map(|s| s.name)
            .collect();
        assert!(names.iter().any(|n| n == "read_file") && names.iter().any(|n| n == "create_note"));

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

    /// Issue #126. A jail escape and a missing file both surface as
    /// `error: …`, so both were recorded as "failed" — the containment
    /// breach was indistinguishable from a typo in the one record that
    /// is supposed to tell you what happened.
    #[test]
    fn a_jail_escape_is_not_ledgered_as_an_ordinary_failure() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let ledger = scratch_ledger(dir.path());

        let mut backend = ScriptedBackend::new(vec![
            // Outside the jail…
            AgentAction::Call(ToolCall {
                id: "1".into(),
                name: "write_file".into(),
                args: serde_json::json!({"path": "../escape.txt", "content": "x"}),
            }),
            // …and an ordinary miss, so the two can be told apart.
            AgentAction::Call(ToolCall {
                id: "2".into(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "nope.txt"}),
            }),
            AgentAction::Done("done".into()),
        ]);
        let config = AgentConfig {
            max_turns: 6,
            verifier: Verifier::None,
            ..AgentConfig::new(ledger.clone())
        };
        let _ = forge_agent("t", &project, &mut backend, &config);

        let statuses: Vec<String> = ledger
            .tail(50)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "forge.tool")
            .map(|e| e.status)
            .collect();
        assert!(
            statuses.iter().any(|s| s == "escaped"),
            "a jail escape was not called out: {statuses:?}"
        );
        assert!(
            statuses.iter().any(|s| s == "failed"),
            "an ordinary error should still be `failed`: {statuses:?}"
        );
    }

    /// A skill's `tools:` line narrows the loop, once the skill is
    /// driving (#57, #245).
    ///
    /// #57: `tools:` was parsed and never consulted, so a skill
    /// declaring what it needs got everything anyway — the restriction
    /// was documentation.
    ///
    /// Driven the way production drives it: a skill file on disk,
    /// `SkillTools` serving it, and the model calling `read_skill`
    /// before it calls anything else. Setting `AgentConfig::skills` and
    /// expecting instant enforcement is what this test used to do, and
    /// it was the ONLY population of that field in the tree — true of
    /// the mechanism, false of the system.
    #[test]
    fn a_skills_allowlist_actually_narrows_what_the_loop_will_run() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("a.txt"), "hello").unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("reader.md"),
            "---\nname: reader\ndescription: read things\ntools: read_file, read_skill\n---\nRead.\n",
        )
        .unwrap();
        let skills = crate::skills::load_from(&[skills_dir]);
        assert_eq!(skills.len(), 1);

        let mut backend = ScriptedBackend::new(vec![
            // The skill starts driving here, and not before.
            AgentAction::Call(ToolCall {
                id: "0".into(),
                name: crate::READ_SKILL.into(),
                args: serde_json::json!({"name": "reader"}),
            }),
            // Permitted by the allowlist.
            AgentAction::Call(ToolCall {
                id: "1".into(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "a.txt"}),
            }),
            // Not permitted — and this is the call that used to run.
            AgentAction::Call(ToolCall {
                id: "2".into(),
                name: "write_file".into(),
                args: serde_json::json!({"path": "b.txt", "content": "nope"}),
            }),
            AgentAction::Done("done".into()),
        ]);
        let config = AgentConfig {
            verifier: Verifier::None,
            skills: skills.clone(),
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };
        let workspace = WorkspaceTools::new(&project).unwrap();
        let skill_tools = crate::SkillTools::new(skills);
        let providers: [&dyn ToolProvider; 2] = [&workspace, &skill_tools];
        forge_agent_with_tools(
            "read then write",
            &project,
            &mut backend,
            &config,
            &providers,
            &mut |_| {},
        )
        .unwrap();

        // The refusal is what the file system says, not what a log says:
        // the write must not have happened.
        assert!(
            !project.join("b.txt").exists(),
            "write_file ran despite not being in the skill's allowlist"
        );
        assert!(project.join("a.txt").exists());
    }

    /// The same run, without the `read_skill` call: the write happens.
    ///
    /// The positive control for the test above, and the assertion that
    /// makes the activation rule real rather than incidental — a skill
    /// sitting on disk unread narrows nothing.
    #[test]
    fn a_skill_that_was_never_read_narrows_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("reader.md"),
            "---\nname: reader\ndescription: read things\ntools: read_file\n---\nRead.\n",
        )
        .unwrap();
        let skills = crate::skills::load_from(&[skills_dir]);

        let mut backend = ScriptedBackend::new(vec![
            AgentAction::Call(ToolCall {
                id: "1".into(),
                name: "write_file".into(),
                args: serde_json::json!({"path": "b.txt", "content": "yes"}),
            }),
            AgentAction::Done("done".into()),
        ]);
        let config = AgentConfig {
            verifier: Verifier::None,
            skills: skills.clone(),
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };
        let workspace = WorkspaceTools::new(&project).unwrap();
        let skill_tools = crate::SkillTools::new(skills);
        let providers: [&dyn ToolProvider; 2] = [&workspace, &skill_tools];
        forge_agent_with_tools(
            "write",
            &project,
            &mut backend,
            &config,
            &providers,
            &mut |_| {},
        )
        .unwrap();

        assert!(project.join("b.txt").exists());
    }

    /// Several skills active: the narrowest wins.
    #[test]
    fn skills_intersect_rather_than_widen_each_other() {
        let scoped = harness_core::Skill::declared;
        let reader = scoped("reader", Some(vec!["read_file".into()]));
        let open = scoped("open", None);
        let writer = scoped("writer", Some(vec!["write_file".into()]));

        // No skills: no restriction. The loop is running without a
        // skill, not under an empty one.
        assert!(harness_core::Skill::allowed_by(&[], "anything"));

        // An unrestricted skill must not widen a restricted one —
        // union semantics would make an allowlist something any other
        // skill can switch off.
        assert!(harness_core::Skill::allowed_by(
            std::slice::from_ref(&reader),
            "read_file"
        ));
        assert!(!harness_core::Skill::allowed_by(
            &[reader.clone(), open.clone()],
            "write_file"
        ));

        // Two disjoint allowlists permit nothing, which is the honest
        // answer: neither skill claims to need the other's tools.
        assert!(!harness_core::Skill::allowed_by(
            &[reader.clone(), writer.clone()],
            "read_file"
        ));
        assert!(!harness_core::Skill::allowed_by(
            &[reader, writer],
            "write_file"
        ));
    }

    /// Issue #227: Stop has to stop something.
    ///
    /// harnessd set a flag, read it in an observer, assigned the answer
    /// to a local nothing looked at again — and this loop had no
    /// cancellation input at all, so a run pressed Stop on kept going
    /// for its full turn budget, calling tools the whole way. The
    /// assertion that matters is not "it returned early": it is that the
    /// TOOL DID NOT RUN.
    #[test]
    fn stopping_a_run_stops_it_before_the_next_tool_call() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();

        let mut backend = ScriptedBackend::repeating(vec![AgentAction::Call(ToolCall {
            id: "1".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": "should-not-exist.txt", "content": "hi"}),
        })]);
        let cancel = crate::Cancel::default();
        let config = AgentConfig {
            max_turns: 12,
            verifier: Verifier::None,
            cancel: cancel.clone(),
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };

        // Pressed while the first turn is under way — the realistic
        // moment, and the one where a tool call is about to be issued.
        let mut observe = |ev: AgentEvent| {
            if matches!(ev, AgentEvent::Turn { .. }) {
                cancel.cancel();
            }
        };
        let outcome = forge_agent_with_tools(
            "write a file",
            &project,
            &mut backend,
            &config,
            &[],
            &mut observe,
        );

        assert!(
            matches!(outcome, Err(ForgeError::Cancelled)),
            "a stopped run must report that it stopped: {outcome:?}"
        );
        assert!(
            !project.join("should-not-exist.txt").exists(),
            "the loop ran a tool after it was told to stop"
        );
        assert_eq!(
            backend.calls, 1,
            "the loop kept asking the model after Stop"
        );
    }

    /// And a run nobody stopped is untouched: the flag defaults to unset
    /// and the loop behaves exactly as it did.
    #[test]
    fn an_unstopped_run_is_not_affected_by_the_stop_check() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let mut backend = ScriptedBackend::new(vec![AgentAction::Done("all set".into())]);
        let config = AgentConfig {
            verifier: Verifier::None,
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };
        let report = forge_agent("do nothing", &project, &mut backend, &config).unwrap();
        assert_eq!(report.summary, "all set");
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

    /// Issue #209's last mile: the attachments a surface handed in ride
    /// on the TASK turn — the one the person typed — and on nothing
    /// else. Prior turns replayed from a client's history stay text.
    #[test]
    fn attachments_ride_on_the_task_turn_and_no_other() {
        let dir = tempfile::tempdir().unwrap();
        let part = json!({
            "type": "image_url",
            "image_url": {"url": "data:image/png;base64,AAAA"},
        });
        let mut backend = ScriptedBackend::new(vec![AgentAction::Done("looked".into())]);
        let config = AgentConfig {
            max_turns: 2,
            verifier: Verifier::None,
            prior_turns: vec![Message::user("earlier"), Message::assistant_text("ok")],
            attachments: vec![part.clone()],
            ..AgentConfig::new(scratch_ledger(dir.path()))
        };
        forge_agent_with_tools(
            "what is this?",
            dir.path(),
            &mut backend,
            &config,
            &[],
            &mut |_| {},
        )
        .unwrap();

        let carrying: Vec<&Message> = backend
            .last_history
            .iter()
            .filter(|m| !m.attachments.is_empty())
            .collect();
        assert_eq!(carrying.len(), 1, "exactly one turn may carry them");
        assert_eq!(carrying[0].role, Role::User);
        assert!(carrying[0].content.contains("what is this?"));
        assert_eq!(carrying[0].attachments, vec![part]);
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
