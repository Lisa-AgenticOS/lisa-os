//! The agent's tool set (`docs/PLAN.md` §5.12.1). Every tool is declared
//! as a Hermes/OpenAI-style function spec with a JSON input schema — the
//! backend is constrained to those schemas, so a tool call arrives
//! grammar-valid and never as free-form text the harness has to parse.
//!
//! Every file operation is mediated by the [`Jail`], and every command by
//! `lisa-guard` (ADR-0029): the model only ever supplies project-relative
//! paths, and neither traversal nor a pivot to a shell is reachable from
//! any tool it can call. Tool *failures* (bad path, missing file, rejected
//! command) are returned as result text so the model can see the mistake
//! and retry — the boundary itself never softens.
//!
//! The limit worth stating out loud: `run_tests` spawns the project's own
//! toolchain over source the model just wrote, and that subprocess is not
//! confined by anything here. Containing it needs Landlock (ADR-0029
//! phase 3).

use crate::Edit;
use crate::jail::Jail;
use lisa_guard::{Verdict, check_command};
use serde_json::{Value, json};
use std::process::Command;

/// Programs `run_command` may execute — the allowlist lives in
/// `lisa-guard` so the policy has one home (ADR-0029); re-exported here
/// because the `run_command` input schema advertises it as an enum.
pub use lisa_guard::ALLOWED_COMMANDS;

/// One tool invocation, decoded from a backend tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Backend-assigned id, echoed back with the result (OpenAI wire
    /// protocol). Synthesized ids are fine for scripted backends.
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// A Hermes-style tool declaration: name, description, and the JSON
/// schema the backend is constrained to when calling it.
///
/// `name` and `description` are owned rather than `&'static str`. The
/// built-in tools below are literals and would not care, but the
/// Assistant's tools are *discovered at runtime* over the Agent Bus
/// (ADR-0025): they arrive as JSON from a manifest an app registered, so
/// there is no `'static` lifetime to borrow from. A borrowed spec cannot
/// describe them at all — this type is the shared vocabulary between
/// compiled-in tools and discovered ones, so it has to admit both.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSpec {
    /// Take anything string-shaped, so a `&'static str` literal and a
    /// `String` parsed out of a manifest read the same at the call site.
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }

    /// The OpenAI-compat wire shape: `{"type": "function", "function": ...}`.
    pub fn wire(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

/// What a tool call produced. `text` is appended to the message history
/// verbatim; `mutated` tells the agent loop the project changed on disk
/// (so the verifier is worth another run).
#[derive(Debug)]
pub struct ToolOutcome {
    pub text: String,
    pub mutated: bool,
}

impl ToolOutcome {
    pub fn ok(text: impl Into<String>, mutated: bool) -> Self {
        Self {
            text: text.into(),
            mutated,
        }
    }

    /// Public so other tool families (ADR-0025) can report failures in
    /// the same shape the loop's narration and history already expect.
    pub fn err(text: impl Into<String>) -> Self {
        Self::ok(format!("error: {}", text.into()), false)
    }

    /// Did this call fail?
    ///
    /// The failure marker is a text prefix, because the outcome is what
    /// the MODEL reads and it has to be legible there. That made "is
    /// this an error" a string comparison spelled at each call site —
    /// `starts_with("error")` in the loop's narration,
    /// `starts_with("error: refused")` in the ledger — and a fourth
    /// caller (#245's skill activation) is where those spellings start
    /// to disagree. One method, one convention.
    pub fn is_err(&self) -> bool {
        self.text.starts_with("error:")
    }
}

#[cfg(test)]
mod refusal_reflection_tests {
    use super::*;
    use serde_json::json;

    fn call_shellish(jail: &Jail, mem: &RefusalMemory) -> ToolOutcome {
        let call = ToolCall {
            id: "t".into(),
            name: "run_command".into(),
            args: json!({"program": "bash", "args": ["-c", "id"]}),
        };
        execute_tool(jail, mem, &call)
    }

    /// The reflection ladder (ADR-0061 steal 2): a first refusal
    /// redirects, an identical retry is named as a loop, the third
    /// mutes — and none of it changes any verdict, because there is
    /// nothing here a model could say to change one (rule 6a).
    #[test]
    fn identical_refusals_cost_more_each_time_and_mute_at_three() {
        let dir = tempfile::tempdir().unwrap();
        let jail = crate::jail::Jail::new(dir.path()).unwrap();
        let mem = RefusalMemory::default();

        let first = call_shellish(&jail, &mem);
        assert!(first.is_err());
        assert!(first.text.contains("OUTCOME"), "{}", first.text);
        assert!(first.text.contains("not negotiable"), "{}", first.text);

        let second = call_shellish(&jail, &mem);
        assert!(second.text.contains("attempt 2"), "{}", second.text);
        assert!(
            second.text.contains("loop, not progress"),
            "{}",
            second.text
        );

        let third = call_shellish(&jail, &mem);
        assert!(third.text.contains("muted"), "{}", third.text);

        // The fourth never reaches the guard: the mute answers first.
        let fourth = call_shellish(&jail, &mem);
        assert!(
            fourth.text.contains("muted for the rest of this run"),
            "{}",
            fourth.text
        );
    }

    /// Refusal memory is per-COMMAND: a different refused command gets
    /// the full first-refusal redirect, not a stale count.
    #[test]
    fn a_different_command_starts_its_own_ladder() {
        let dir = tempfile::tempdir().unwrap();
        let jail = crate::jail::Jail::new(dir.path()).unwrap();
        let mem = RefusalMemory::default();
        call_shellish(&jail, &mem);
        call_shellish(&jail, &mem);

        let other = execute_tool(
            &jail,
            &mem,
            &ToolCall {
                id: "t".into(),
                name: "run_command".into(),
                args: json!({"program": "sh", "args": ["-c", "id"]}),
            },
        );
        assert!(other.text.contains("not negotiable"), "{}", other.text);
        assert!(!other.text.contains("attempt 2"), "{}", other.text);
    }

    /// An ALLOWED command is untouched by any amount of refusal
    /// history — the memory shapes wasted attempts, never verdicts.
    #[test]
    fn refusal_history_never_leaks_into_allowed_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let jail = crate::jail::Jail::new(dir.path()).unwrap();
        let mem = RefusalMemory::default();
        for _ in 0..4 {
            call_shellish(&jail, &mem);
        }
        let ok = execute_tool(
            &jail,
            &mem,
            &ToolCall {
                id: "t".into(),
                name: "run_command".into(),
                args: json!({"program": "cat", "args": ["a.txt"]}),
            },
        );
        assert!(!ok.is_err(), "{}", ok.text);
        assert!(ok.text.contains("hello"), "{}", ok.text);
    }
}

#[cfg(test)]
mod run_tests_tests {
    use super::*;

    /// A Lisa app's suite is a real suite, and it now RUNS.
    ///
    /// This test used to assert the honest refusal — "no JS runtime is
    /// on the command allowlist" — which was the right answer while it
    /// was true (#246). #269 added `gjs`/`node` under a policy that lets
    /// a runtime run a file and never an argument, so the refusal became
    /// a lie and this assertion had to move with it. A test pinning a
    /// message is a test that fails when the message stops being true,
    /// which is what it is for.
    #[test]
    fn a_lisa_app_suite_is_run_rather_than_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tests")).unwrap();
        // A suite that exits 0 on any runtime, so this asserts REACHING
        // the runtime rather than the runtime's own opinion.
        std::fs::write(dir.path().join("tests/notes.test.js"), "// suite\n").unwrap();
        assert!(has_js_suite(dir.path()));

        let jail = crate::jail::Jail::new(dir.path()).unwrap();
        let out = run_tests(&jail, &RefusalMemory::default());
        // Whatever the runtime says, it must not be the guard refusing
        // the spawn or the tool calling a real suite unrecognised.
        assert!(
            !out.text.contains("allowlist"),
            "the guard refused the spawn: {}",
            out.text
        );
        assert!(
            !out.text.contains("no recognized test setup"),
            "a real suite was called unrecognized: {}",
            out.text
        );
    }

    /// The half of #269 that matters: the runtime may run a file and
    /// never an argument. Asserted here as well as in the guard's own
    /// corpus, because this is the caller that would notice a policy
    /// change — the guard could relax and this tool would silently gain
    /// a shell.
    #[test]
    fn the_runtime_cannot_be_asked_to_evaluate_a_string() {
        for (program, flag) in [("gjs", "-c"), ("node", "-e"), ("node", "--eval")] {
            let v = lisa_guard::check_command(program, &[flag, "print(1)"]);
            assert!(
                matches!(v, lisa_guard::Verdict::Deny { .. }),
                "{program} {flag} was not refused: {v:?}"
            );
        }
    }

    /// …and a tree with no suite at all still says so, naming every
    /// layout it looked for.
    #[test]
    fn an_empty_tree_still_reports_nothing_to_run() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_js_suite(dir.path()));
        let jail = crate::jail::Jail::new(dir.path()).unwrap();
        let out = run_tests(&jail, &RefusalMemory::default());
        assert!(
            out.text.contains("no recognized test setup"),
            "{}",
            out.text
        );
        assert!(out.text.contains("tests/*.test.js"), "{}", out.text);
    }
}

const MAX_FILE_CHARS: usize = 30_000;
const MAX_CMD_CHARS: usize = 12_000;
const MAX_GREP_HITS: usize = 200;
const MAX_LIST_ENTRIES: usize = 500;

/// The full tool set offered to the backend, with input schemas.
pub fn tool_specs() -> Vec<ToolSpec> {
    let rel = |what: &str| {
        json!({"type": "string", "description":
            format!("Project-relative {what} — e.g. `lib/notes.js`. Never absolute, never containing `..`.")})
    };
    vec![
        ToolSpec::new(
            "read_file",
            "Read the complete contents of a project file.",
            json!({
                "type": "object",
                "properties": {"path": rel("file path")},
                "required": ["path"],
            }),
        ),
        ToolSpec::new(
            "list_dir",
            "List a project directory (`.` for the root); directories end with `/`.",
            json!({
                "type": "object",
                "properties": {"path": rel("directory path")},
                "required": ["path"],
            }),
        ),
        ToolSpec::new(
            "grep",
            "Search file contents for a literal substring; returns `path:line: text` \
             matches. Hidden and build-output directories are skipped.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Literal substring to search for."},
                    "path": rel("file or directory to search; omit to search the whole project"),
                },
                "required": ["pattern"],
            }),
        ),
        ToolSpec::new(
            "write_file",
            "Write a COMPLETE file (new or full replacement). Prefer `edit_file` for \
             targeted changes to existing files.",
            json!({
                "type": "object",
                "properties": {
                    "path": rel("file path"),
                    "content": {"type": "string", "description": "The complete new file content."},
                },
                "required": ["path", "content"],
            }),
        ),
        ToolSpec::new(
            "edit_file",
            "Targeted find/replace in an existing file: `old_string` must match the \
             current content exactly (including indentation) and be unique unless \
             `replace_all` is set.",
            json!({
                "type": "object",
                "properties": {
                    "path": rel("file path"),
                    "old_string": {"type": "string", "description": "Exact text to find."},
                    "new_string": {"type": "string", "description": "Replacement text."},
                    "replace_all": {"type": "boolean", "description": "Replace every occurrence (default false)."},
                },
                "required": ["path", "old_string", "new_string"],
            }),
        ),
        ToolSpec::new(
            "run_command",
            "Run an allowlisted command in the project root (no shell). File \
             operations should go through the dedicated tools.",
            json!({
                "type": "object",
                "properties": {
                    "program": {"type": "string", "enum": ALLOWED_COMMANDS},
                    "args": {"type": "array", "items": {"type": "string"},
                             "description": "Arguments; paths must stay inside the project."},
                },
                "required": ["program"],
            }),
        ),
        ToolSpec::new(
            "run_tests",
            "Run the project's test suite. Recognises a Cargo project \
             (`cargo test`) and a pubspec project (`dart test`). A Lisa app's \
             own `tests/*.test.js` suite is NOT runnable from here yet — run \
             it outside the loop.",
            json!({
                "type": "object",
                "properties": {},
            }),
        ),
    ]
}

/// What a run remembers about its own refusals (ADR-0061 steal 2).
///
/// Keyed by the exact program + argv. Interior mutability because the
/// provider trait hands out `&self`, and a `Mutex` rather than a
/// `RefCell` because harnessd may drive a run from more than one
/// thread. This is loop hygiene, not policy: the guard's verdicts are
/// identical with or without it — only the cost of ignoring them
/// changes.
#[derive(Default)]
pub struct RefusalMemory(std::sync::Mutex<std::collections::HashMap<String, u32>>);

/// The third identical refusal mutes that exact command for the run.
const MUTE_AFTER: u32 = 3;

impl RefusalMemory {
    /// Record one refusal; returns how many came BEFORE it.
    fn note(&self, key: &str) -> u32 {
        let mut m = self.0.lock().unwrap();
        let n = m.entry(key.to_string()).or_insert(0);
        let prior = *n;
        *n += 1;
        prior
    }

    fn muted(&self, key: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .get(key)
            .is_some_and(|n| *n >= MUTE_AFTER)
    }
}

fn refusal_key(program: &str, argv: &[&str]) -> String {
    format!("{program}\u{1f}{}", argv.join("\u{1f}"))
}

/// Execute one tool call against the jail. Never fails fatally: every
/// error is reported back as result text for the model to act on.
pub fn execute_tool(jail: &Jail, mem: &RefusalMemory, call: &ToolCall) -> ToolOutcome {
    match call.name.as_str() {
        "read_file" => match arg_str(&call.args, "path") {
            Ok(path) => match jail.read(path) {
                Ok(content) => ToolOutcome::ok(truncate(&content, MAX_FILE_CHARS), false),
                Err(e) => ToolOutcome::err(e.to_string()),
            },
            Err(e) => ToolOutcome::err(e),
        },
        "list_dir" => {
            let path = call.args["path"].as_str().unwrap_or(".");
            match jail.list(path) {
                Ok(mut entries) => {
                    let total = entries.len();
                    entries.truncate(MAX_LIST_ENTRIES);
                    let mut text = entries.join("\n");
                    if total > MAX_LIST_ENTRIES {
                        text.push_str(&format!(
                            "\n… and {} more entr(ies)",
                            total - MAX_LIST_ENTRIES
                        ));
                    }
                    if text.is_empty() {
                        text = "(empty directory)".into();
                    }
                    ToolOutcome::ok(text, false)
                }
                Err(e) => ToolOutcome::err(e.to_string()),
            }
        }
        "grep" => grep(jail, &call.args),
        "write_file" => match serde_json::from_value::<Edit>(call.args.clone()) {
            Ok(edit) => match jail.write(&edit.path, &edit.content) {
                Ok(()) => ToolOutcome::ok(
                    format!("wrote {} ({} bytes)", edit.path, edit.content.len()),
                    true,
                ),
                Err(e) => ToolOutcome::err(e.to_string()),
            },
            Err(e) => ToolOutcome::err(format!("bad write_file arguments: {e}")),
        },
        "edit_file" => edit_file(jail, &call.args),
        "run_command" => run_command(jail, mem, &call.args),
        "run_tests" => run_tests(jail, mem),
        other => ToolOutcome::err(format!(
            "unknown tool `{other}`; available: {}",
            tool_specs()
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn edit_file(jail: &Jail, args: &Value) -> ToolOutcome {
    let (path, old, new) = match (
        arg_str(args, "path"),
        arg_str(args, "old_string"),
        arg_str(args, "new_string"),
    ) {
        (Ok(p), Ok(o), Ok(n)) => (p, o, n),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return ToolOutcome::err(e),
    };
    let replace_all = args["replace_all"].as_bool().unwrap_or(false);
    let content = match jail.read(path) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(e.to_string()),
    };
    if old.is_empty() {
        return ToolOutcome::err("`old_string` must not be empty");
    }
    let matches = content.matches(old).count();
    if matches == 0 {
        return ToolOutcome::err(format!(
            "`old_string` not found in {path}; read the file again and match the exact text"
        ));
    }
    if matches > 1 && !replace_all {
        return ToolOutcome::err(format!(
            "`old_string` matches {matches} places in {path}; make it more specific or set `replace_all`"
        ));
    }
    let updated = if replace_all {
        content.replace(old, new)
    } else {
        content.replacen(old, new, 1)
    };
    match jail.write(path, &updated) {
        Ok(()) => ToolOutcome::ok(format!("edited {path} ({matches} replacement(s))"), true),
        Err(e) => ToolOutcome::err(e.to_string()),
    }
}

fn grep(jail: &Jail, args: &Value) -> ToolOutcome {
    let pattern = match arg_str(args, "pattern") {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(e),
    };
    let scope = args["path"].as_str().unwrap_or(".");
    let files = match jail.walk(scope) {
        Ok(files) => files,
        Err(e) => return ToolOutcome::err(e.to_string()),
    };
    // `scope` may itself be a single file rather than a directory.
    let files = if files.is_empty() && jail.read(scope).is_ok() {
        vec![scope.to_string()]
    } else {
        files
    };
    let mut hits = Vec::new();
    for file in &files {
        if hits.len() >= MAX_GREP_HITS {
            break;
        }
        let Ok(content) = jail.read(file) else {
            continue; // unreadable (binary, race) — skip, don't die
        };
        for (n, line) in content.lines().enumerate() {
            if line.contains(pattern) {
                hits.push(format!("{file}:{}: {line}", n + 1));
                if hits.len() >= MAX_GREP_HITS {
                    break;
                }
            }
        }
    }
    if hits.is_empty() {
        ToolOutcome::ok(format!("no matches for `{pattern}`"), false)
    } else {
        ToolOutcome::ok(hits.join("\n"), false)
    }
}

fn run_command(jail: &Jail, mem: &RefusalMemory, args: &Value) -> ToolOutcome {
    let program = match arg_str(args, "program") {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(e),
    };
    let argv: Vec<&str> = args["args"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    run_program(jail, mem, program, &argv)
}

/// Does this tree carry a Lisa app's own suite — `tests/*.test.js`, the
/// layout `just shell-test` globs?
fn has_js_suite(root: &std::path::Path) -> bool {
    std::fs::read_dir(root.join("tests"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().ends_with(".test.js"))
}

fn run_tests(jail: &Jail, mem: &RefusalMemory) -> ToolOutcome {
    let root = jail.root();
    let (program, argv): (&str, &[&str]) = if root.join("pubspec.yaml").exists() {
        ("dart", &["test"])
    } else if root.join("Cargo.toml").exists() {
        ("cargo", &["test"])
    } else if has_js_suite(root) {
        // A Lisa app (ADR-0047): `tests/*.test.js`, run by whichever JS
        // runtime the host has — the shape `just shell-test` already
        // uses.
        //
        // `gjs` and `node` joined `lisa-guard`'s ALLOWED_COMMANDS with
        // #269's policy: a runtime may run a FILE and never an argument.
        // Every eval and preload flag (`-c`, `--eval`, `-p`, `-r`, `-I`)
        // is refused at both doors, and so is executing a file that
        // belongs to the system. So this spawns a test file inside the
        // jail and nothing else.
        //
        // `node` first: it is the runtime CI has, and `just shell-test`
        // prefers gjs only when it is present. `run_program` reports the
        // spawn failure if neither is installed, which is a truthful
        // answer rather than a guard verdict about a suite that exists.
        if which("gjs") {
            ("gjs", &["-m"])
        } else {
            ("node", &["--test"])
        }
    } else {
        return ToolOutcome::err(
            "no recognized test setup in the project (looked for `tests/*.test.js`, \
             pubspec.yaml and Cargo.toml)",
        );
    };
    run_program(jail, mem, program, argv)
}

/// Is this program on `PATH`?
///
/// Used to pick a JS runtime rather than to decide anything about
/// safety — the guard's allowlist is what decides that, and it does not
/// care which of the two is installed.
fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

fn run_program(jail: &Jail, mem: &RefusalMemory, program: &str, argv: &[&str]) -> ToolOutcome {
    // One policy point for every surface (ADR-0029). The previous check
    // here — reject absolute paths and `..` — let `find . -exec sh -c
    // '<anything>' \;` through, because every token in it is a plain
    // relative name.
    //
    // Nobody is watching this loop, so a verdict that would ask a human
    // is refused rather than assumed: `Confirm` needs consent that does
    // not exist here. The reason goes back as tool output so the model
    // can pick another route instead of retrying blind.
    //
    // And blind is the word (ADR-0061 steal 2, jcode's reflection gate
    // made 6a-safe): a refusal comes back with a REDIRECT the retry has
    // to have read, an identical retry is named as the loop it is, and
    // the third one mutes that exact command for the rest of the run.
    // Nothing here is an approval path — no wording the model produces
    // changes any verdict; the memory only shapes what wasted attempts
    // cost.
    let key = refusal_key(program, argv);
    if mem.muted(&key) {
        return ToolOutcome::err(format!(
            "`{program}` with these arguments is muted for the rest of this run: \
             it was refused {MUTE_AFTER} times and the verdict cannot change. \
             State the OUTCOME you need in your summary if no allowed route exists."
        ));
    }
    match check_command(program, argv) {
        Verdict::Allow => {}
        verdict => {
            let prior = mem.note(&key);
            return ToolOutcome::err(match prior {
                0 => format!(
                    "{verdict}\nBefore another attempt: state what OUTCOME you need \
                     — not the command — and pick a route the catalogue allows. \
                     The guard is not negotiable from inside this loop."
                ),
                n if n + 1 >= MUTE_AFTER => format!(
                    "{verdict}\nThis identical command has now been refused {} times \
                     and is muted for the rest of the run. Change the approach, or \
                     record the limitation honestly in your summary.",
                    n + 1
                ),
                n => format!(
                    "{verdict}\nIdentical command, attempt {}: the verdict cannot \
                     change, so repeating it is a loop, not progress. What OUTCOME \
                     does this command serve? Reach it another way.",
                    n + 1
                ),
            });
        }
    }
    // Confine the CHILD, not us (ADR-0029 phase 3, #53). `cargo test`
    // compiles and runs build.rs and test bodies that the model just
    // wrote, as this user, outside every rule in lisa-guard — once
    // execve has happened the guard is not in the process any more.
    //
    // The hook itself lives in `confine::confine_command`, because
    // `run_shell` needs the same one and a second copy is a second
    // policy (#307).
    let mut cmd = Command::new(program);
    cmd.args(argv).current_dir(jail.root());
    let confinement =
        crate::confine::confine_command(&mut cmd, jail.root(), &crate::confine::user_home());
    match cmd.output() {
        Err(e) => ToolOutcome::err(format!("running `{program}`: {e}")),
        Ok(out) => {
            let status = out
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| out.status.to_string());
            // If the child ran unconfined, the model — and the Ledger
            // preview, and whoever reads the transcript afterwards —
            // is told. Reporting a jail that did not close would be
            // worse than not having one, because the point of a
            // guardrail is that somebody can rely on it.
            let note = confinement
                .note()
                .map(|n| format!("note: {n}\n"))
                .unwrap_or_default();
            let text = format!(
                "{note}exit: {status}\n{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            ToolOutcome::ok(truncate(text.trim_end(), MAX_CMD_CHARS), false)
        }
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .ok_or_else(|| format!("missing or non-string argument `{key}`"))
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…\n[truncated, {} bytes total]", &text[..end], text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jail() -> (tempfile::TempDir, Jail) {
        let dir = tempfile::tempdir().unwrap();
        let jail = Jail::new(dir.path()).unwrap();
        (dir, jail)
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: name.into(),
            args,
        }
    }

    #[test]
    fn write_read_edit_roundtrip() {
        let (_dir, jail) = jail();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "write_file",
                json!({"path": "lib/a.dart", "content": "void main() { broken(); }\n"}),
            ),
        );
        assert!(out.mutated, "{out:?}");

        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("read_file", json!({"path": "lib/a.dart"})),
        );
        assert!(out.text.contains("broken();"));

        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "edit_file",
                json!({"path": "lib/a.dart", "old_string": "broken();", "new_string": "print('ok');"}),
            ),
        );
        assert!(out.mutated, "{out:?}");
        assert_eq!(
            jail.read("lib/a.dart").unwrap(),
            "void main() { print('ok'); }\n"
        );
    }

    #[test]
    fn edit_requires_unique_match() {
        let (_dir, jail) = jail();
        jail.write("a.txt", "x x").unwrap();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "edit_file",
                json!({"path": "a.txt", "old_string": "x", "new_string": "y"}),
            ),
        );
        assert!(
            !out.mutated && out.text.contains("matches 2 places"),
            "{out:?}"
        );
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "edit_file",
                json!({"path": "a.txt", "old_string": "x", "new_string": "y", "replace_all": true}),
            ),
        );
        assert!(out.mutated);
        assert_eq!(jail.read("a.txt").unwrap(), "y y");
    }

    #[test]
    fn edit_missing_file_and_missing_text_are_tool_errors() {
        let (_dir, jail) = jail();
        jail.write("a.txt", "hello").unwrap();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "edit_file",
                json!({"path": "nope.txt", "old_string": "x", "new_string": "y"}),
            ),
        );
        assert!(out.text.starts_with("error:"));
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "edit_file",
                json!({"path": "a.txt", "old_string": "zzz", "new_string": "y"}),
            ),
        );
        assert!(out.text.contains("not found"));
    }

    #[test]
    fn jail_rejections_come_back_as_tool_text() {
        let (_dir, jail) = jail();
        for bad in ["../outside.txt", "/etc/passwd", "ok/../../x"] {
            let out = execute_tool(
                &jail,
                &RefusalMemory::default(),
                &call("write_file", json!({"path": bad, "content": "x"})),
            );
            assert!(!out.mutated);
            assert!(
                out.text.contains("escapes the project jail"),
                "{bad}: {out:?}"
            );
        }
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("read_file", json!({"path": ".."})),
        );
        assert!(out.text.contains("escapes the project jail"));
    }

    #[test]
    fn list_and_grep_see_the_tree() {
        let (_dir, jail) = jail();
        jail.write("lib/main.dart", "void main() { print('needle'); }\n")
            .unwrap();
        jail.write("lib/src/util.dart", "// needle in a comment\n")
            .unwrap();
        jail.write(".git/hidden", "needle").unwrap();

        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("list_dir", json!({"path": "."})),
        );
        assert!(out.text.contains("lib/"), "{out:?}");

        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("grep", json!({"pattern": "needle"})),
        );
        assert!(out.text.contains("lib/main.dart:1:"), "{out:?}");
        assert!(out.text.contains("lib/src/util.dart:1:"), "{out:?}");
        assert!(
            !out.text.contains("hidden"),
            "must not search .git: {out:?}"
        );

        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("grep", json!({"pattern": "comment", "path": "lib/src"})),
        );
        assert!(out.text.contains("util.dart"), "{out:?}");
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("grep", json!({"pattern": "absent"})),
        );
        assert!(out.text.contains("no matches"));
    }

    #[test]
    fn run_command_enforces_allowlist_and_arg_jail() {
        let (_dir, jail) = jail();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "run_command",
                json!({"program": "sh", "args": ["-c", "id"]}),
            ),
        );
        assert!(out.text.contains("command.not_allowlisted"), "{}", out.text);
        for escaping in ["../../etc/passwd", "/etc/passwd"] {
            let out = execute_tool(
                &jail,
                &RefusalMemory::default(),
                &call("run_command", json!({"program": "cat", "args": [escaping]})),
            );
            assert!(out.text.contains("command.path_escape"), "{}", out.text);
        }
    }

    /// ADR-0029: `find` is allowlisted and every token below is a plain
    /// relative name, so the old absolute/`..` check waved this straight
    /// through to a full shell.
    #[test]
    fn run_command_cannot_pivot_to_a_shell_through_find() {
        let (_dir, jail) = jail();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "run_command",
                json!({"program": "find", "args": [".", "-exec", "sh", "-c", "id", ";"]}),
            ),
        );
        assert!(out.text.contains("command.exec_predicate"), "{}", out.text);
        assert!(!out.mutated);

        jail.write("lib/a.dart", "void main() {}").unwrap();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "run_command",
                json!({"program": "find", "args": [".", "-delete"]}),
            ),
        );
        assert!(out.text.contains("command.exec_predicate"), "{}", out.text);
        assert!(jail.read("lib/a.dart").is_ok(), "the tree was deleted");
    }

    #[test]
    fn run_command_runs_in_project_root() {
        if Command::new("echo").arg("--version").output().is_err() {
            eprintln!("skipping: echo not on PATH");
            return;
        }
        let (_dir, jail) = jail();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call(
                "run_command",
                json!({"program": "echo", "args": ["forged"]}),
            ),
        );
        assert!(out.text.contains("exit: 0"), "{out:?}");
        assert!(out.text.contains("forged"));
    }

    #[test]
    fn run_tests_reports_unconfigured_projects() {
        let (_dir, jail) = jail();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("run_tests", json!({})),
        );
        assert!(out.text.contains("no recognized test setup"), "{out:?}");
    }

    #[test]
    fn unknown_tool_is_a_tool_error() {
        let (_dir, jail) = jail();
        let out = execute_tool(
            &jail,
            &RefusalMemory::default(),
            &call("delete_everything", json!({})),
        );
        assert!(out.text.contains("unknown tool"));
    }
}
