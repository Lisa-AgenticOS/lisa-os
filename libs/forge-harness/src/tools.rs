//! The agent's tool set (`docs/PLAN.md` §5.12.1). Every tool is declared
//! as a Hermes/OpenAI-style function spec with a JSON input schema — the
//! backend is constrained to those schemas, so a tool call arrives
//! grammar-valid and never as free-form text the harness has to parse.
//!
//! Every file operation is mediated by the [`Jail`]: the model only ever
//! supplies project-relative paths, and traversal stays impossible no
//! matter which tool it calls. Tool *failures* (bad path, missing file,
//! rejected command) are returned as result text so the model can see the
//! mistake and retry — the jail boundary itself never softens.

use crate::Edit;
use crate::jail::Jail;
use serde_json::{Value, json};
use std::process::Command;

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
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

impl ToolSpec {
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
}

/// Programs `run_command` may execute. Deliberately small: file reads,
/// searches, and the Dart/Rust toolchains. No shell — arguments are passed
/// to `exec` directly, so there is no shell expansion to abuse.
pub const ALLOWED_COMMANDS: &[&str] = &[
    "dart", "flutter", "cargo", "rustc", "ls", "cat", "grep", "find", "echo", "pwd", "mkdir",
    "touch",
];

const MAX_FILE_CHARS: usize = 30_000;
const MAX_CMD_CHARS: usize = 12_000;
const MAX_GREP_HITS: usize = 200;
const MAX_LIST_ENTRIES: usize = 500;

/// The full tool set offered to the backend, with input schemas.
pub fn tool_specs() -> Vec<ToolSpec> {
    let rel = |what: &str| {
        json!({"type": "string", "description":
            format!("Project-relative {what} — e.g. `bin/main.dart`. Never absolute, never containing `..`.")})
    };
    vec![
        ToolSpec {
            name: "read_file",
            description: "Read the complete contents of a project file.",
            parameters: json!({
                "type": "object",
                "properties": {"path": rel("file path")},
                "required": ["path"],
            }),
        },
        ToolSpec {
            name: "list_dir",
            description: "List a project directory (`.` for the root); directories end with `/`.",
            parameters: json!({
                "type": "object",
                "properties": {"path": rel("directory path")},
                "required": ["path"],
            }),
        },
        ToolSpec {
            name: "grep",
            description: "Search file contents for a literal substring; returns `path:line: text` \
                          matches. Hidden and build-output directories are skipped.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Literal substring to search for."},
                    "path": rel("file or directory to search; omit to search the whole project"),
                },
                "required": ["pattern"],
            }),
        },
        ToolSpec {
            name: "write_file",
            description: "Write a COMPLETE file (new or full replacement). Prefer `edit_file` for \
                          targeted changes to existing files.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": rel("file path"),
                    "content": {"type": "string", "description": "The complete new file content."},
                },
                "required": ["path", "content"],
            }),
        },
        ToolSpec {
            name: "edit_file",
            description: "Targeted find/replace in an existing file: `old_string` must match the \
                          current content exactly (including indentation) and be unique unless \
                          `replace_all` is set.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": rel("file path"),
                    "old_string": {"type": "string", "description": "Exact text to find."},
                    "new_string": {"type": "string", "description": "Replacement text."},
                    "replace_all": {"type": "boolean", "description": "Replace every occurrence (default false)."},
                },
                "required": ["path", "old_string", "new_string"],
            }),
        },
        ToolSpec {
            name: "run_command",
            description: "Run an allowlisted command in the project root (no shell). Use it for \
                          toolchain commands like `dart analyze`; file operations should go through \
                          the dedicated tools.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "program": {"type": "string", "enum": ALLOWED_COMMANDS},
                    "args": {"type": "array", "items": {"type": "string"},
                             "description": "Arguments; paths must stay inside the project."},
                },
                "required": ["program"],
            }),
        },
        ToolSpec {
            name: "run_tests",
            description: "Run the project's test suite (`dart test` for a pubspec project, \
                          `cargo test` for a Cargo project).",
            parameters: json!({
                "type": "object",
                "properties": {},
            }),
        },
    ]
}

/// Execute one tool call against the jail. Never fails fatally: every
/// error is reported back as result text for the model to act on.
pub fn execute_tool(jail: &Jail, call: &ToolCall) -> ToolOutcome {
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
        "run_command" => run_command(jail, &call.args),
        "run_tests" => run_tests(jail),
        other => ToolOutcome::err(format!(
            "unknown tool `{other}`; available: {}",
            tool_specs()
                .iter()
                .map(|t| t.name)
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

fn run_command(jail: &Jail, args: &Value) -> ToolOutcome {
    let program = match arg_str(args, "program") {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(e),
    };
    let argv: Vec<&str> = args["args"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    run_program(jail, program, &argv)
}

fn run_tests(jail: &Jail) -> ToolOutcome {
    let root = jail.root();
    let (program, argv): (&str, &[&str]) = if root.join("pubspec.yaml").exists() {
        // Flutter projects need `flutter test` — `dart test` cannot load
        // dart:ui. A pubspec declaring the flutter sdk is the marker.
        let is_flutter = std::fs::read_to_string(root.join("pubspec.yaml"))
            .is_ok_and(|p| p.contains("sdk: flutter"));
        if is_flutter {
            ("flutter", &["test"])
        } else {
            ("dart", &["test"])
        }
    } else if root.join("Cargo.toml").exists() {
        ("cargo", &["test"])
    } else {
        return ToolOutcome::err(
            "no recognized test setup in the project (looked for pubspec.yaml and Cargo.toml)",
        );
    };
    run_program(jail, program, argv)
}

fn run_program(jail: &Jail, program: &str, argv: &[&str]) -> ToolOutcome {
    if !ALLOWED_COMMANDS.contains(&program) {
        return ToolOutcome::err(format!(
            "`{program}` is not allowlisted; allowed: {}",
            ALLOWED_COMMANDS.join(", ")
        ));
    }
    // Command arguments never pass through the jail's path validator, so
    // keep them inside the project the cheap way: no absolute paths, no
    // `..`. Heuristic, but with no shell involved it closes the escape
    // routes an allowlisted program could otherwise open.
    for arg in argv {
        let p = std::path::Path::new(arg);
        if p.is_absolute()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return ToolOutcome::err(format!(
                "argument `{arg}` leaves the project; run_command only works inside it"
            ));
        }
    }
    // On Lisa devices the Flutter SDK lives on the durable partition
    // (`lisa forge setup`, issue #37) — extend the child's PATH so the
    // agent's `flutter`/`dart` calls resolve there too.
    let path = match std::env::var("PATH") {
        Ok(p) => format!("{p}:/var/lib/lisa/flutter/bin"),
        Err(_) => "/var/lib/lisa/flutter/bin".to_string(),
    };
    match Command::new(program)
        .args(argv)
        .env("PATH", path)
        .current_dir(jail.root())
        .output()
    {
        Err(e) => ToolOutcome::err(format!("running `{program}`: {e}")),
        Ok(out) => {
            let status = out
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| out.status.to_string());
            let text = format!(
                "exit: {status}\n{}{}",
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
            &call(
                "write_file",
                json!({"path": "lib/a.dart", "content": "void main() { broken(); }\n"}),
            ),
        );
        assert!(out.mutated, "{out:?}");

        let out = execute_tool(&jail, &call("read_file", json!({"path": "lib/a.dart"})));
        assert!(out.text.contains("broken();"));

        let out = execute_tool(
            &jail,
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
            &call(
                "edit_file",
                json!({"path": "nope.txt", "old_string": "x", "new_string": "y"}),
            ),
        );
        assert!(out.text.starts_with("error:"));
        let out = execute_tool(
            &jail,
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
                &call("write_file", json!({"path": bad, "content": "x"})),
            );
            assert!(!out.mutated);
            assert!(
                out.text.contains("escapes the project jail"),
                "{bad}: {out:?}"
            );
        }
        let out = execute_tool(&jail, &call("read_file", json!({"path": ".."})));
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

        let out = execute_tool(&jail, &call("list_dir", json!({"path": "."})));
        assert!(out.text.contains("lib/"), "{out:?}");

        let out = execute_tool(&jail, &call("grep", json!({"pattern": "needle"})));
        assert!(out.text.contains("lib/main.dart:1:"), "{out:?}");
        assert!(out.text.contains("lib/src/util.dart:1:"), "{out:?}");
        assert!(
            !out.text.contains("hidden"),
            "must not search .git: {out:?}"
        );

        let out = execute_tool(
            &jail,
            &call("grep", json!({"pattern": "comment", "path": "lib/src"})),
        );
        assert!(out.text.contains("util.dart"), "{out:?}");
        let out = execute_tool(&jail, &call("grep", json!({"pattern": "absent"})));
        assert!(out.text.contains("no matches"));
    }

    #[test]
    fn run_command_enforces_allowlist_and_arg_jail() {
        let (_dir, jail) = jail();
        let out = execute_tool(
            &jail,
            &call(
                "run_command",
                json!({"program": "sh", "args": ["-c", "id"]}),
            ),
        );
        assert!(out.text.contains("not allowlisted"));
        let out = execute_tool(
            &jail,
            &call(
                "run_command",
                json!({"program": "cat", "args": ["../../etc/passwd"]}),
            ),
        );
        assert!(out.text.contains("leaves the project"));
        let out = execute_tool(
            &jail,
            &call(
                "run_command",
                json!({"program": "cat", "args": ["/etc/passwd"]}),
            ),
        );
        assert!(out.text.contains("leaves the project"));
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
        let out = execute_tool(&jail, &call("run_tests", json!({})));
        assert!(out.text.contains("no recognized test setup"), "{out:?}");
    }

    #[test]
    fn unknown_tool_is_a_tool_error() {
        let (_dir, jail) = jail();
        let out = execute_tool(&jail, &call("delete_everything", json!({})));
        assert!(out.text.contains("unknown tool"));
    }
}
