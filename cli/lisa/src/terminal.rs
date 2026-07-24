//! Terminal integration verbs: `lisa explain` and `lisa suggest`
//! (`docs/PLAN.md` §5.8 Terminal). The shell hooks in
//! `apps/terminal-integration/` are wiring only — every decision lives
//! here (CLAUDE.md rule 4: shell script only in installers and hooks).
//!
//! Safety property: `suggest` NEVER executes anything. It prints ONE
//! command; the Ctrl+G hook only replaces the prompt line with it, so
//! pressing Enter — the user's own hand, after reading the line — is the
//! mandatory review-before-run gate (PLAN §5.8).

use anyhow::{Context, bail};
use serde_json::{Value, json};
use std::io::{IsTerminal, Read};

/// How much of the failure output rides along — the tail, because
/// compilers and stack traces put the actual error last.
const OUTPUT_TAIL_BYTES: usize = 2000;

/// Short system-style preamble for `explain`: prose, no markdown.
const EXPLAIN_SYSTEM: &str = "You explain failed shell commands on a Linux system. In two or \
     three short sentences, say what the error means and the most likely \
     fix. Plain prose only: no markdown, no code fences, no lists.";

/// Compose the chat body for `lisa explain` — pure, unit-tested.
/// Streams at interactive priority (no `lisa_priority` field): the user
/// is sitting at the prompt waiting for it.
pub(crate) fn explain_body(
    command: &str,
    exit: Option<i32>,
    output: &str,
    model: Option<&str>,
) -> Value {
    let mut user = String::from("A shell command failed.\n");
    if !command.trim().is_empty() {
        user.push_str(&format!("Command: {}\n", command.trim()));
    }
    if let Some(code) = exit {
        user.push_str(&format!("Exit code: {code}\n"));
    }
    let excerpt = tail_excerpt(output.trim(), OUTPUT_TAIL_BYTES);
    if !excerpt.is_empty() {
        user.push_str(&format!("Output (tail):\n{excerpt}\n"));
    }
    user.push_str("Explain the failure and the likely fix.");
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": EXPLAIN_SYSTEM},
            {"role": "user", "content": user},
        ],
        "stream": true,
        "max_tokens": 300,
    })
}

/// `lisa explain [--exit N] [command…]` — piped stdin is the failed
/// command's output. A bare `lisa explain` falls back to what the shell
/// hooks stashed (`LISA_LAST_COMMAND` / `LISA_LAST_EXIT`).
pub(crate) fn explain_cmd(
    command: Vec<String>,
    exit: Option<i32>,
    url: &str,
    model: Option<String>,
) -> anyhow::Result<()> {
    let mut command = command.join(" ");
    let mut exit = exit;
    let mut output = String::new();
    if !std::io::stdin().is_terminal() {
        std::io::stdin().read_to_string(&mut output)?;
    }
    if command.trim().is_empty() && output.trim().is_empty() {
        // The hint path: the precmd/PROMPT_COMMAND hook exported the
        // last failure so a bare `lisa explain` knows what to explain.
        if let Ok(c) = std::env::var("LISA_LAST_COMMAND") {
            command = c.trim().to_string();
        }
        if exit.is_none() {
            exit = std::env::var("LISA_LAST_EXIT")
                .ok()
                .and_then(|s| s.trim().parse().ok());
        }
    }
    if command.trim().is_empty() && output.trim().is_empty() {
        bail!(
            "nothing to explain — `lisa explain --exit 127 <command>`, \
             `<command> 2>&1 | lisa explain`, or source the terminal hooks \
             (apps/terminal-integration) so a bare `lisa explain` knows \
             the last failure"
        );
    }
    let body = explain_body(&command, exit, &output, model.as_deref());
    crate::print_chat(url, &body, false)
}

/// The guided-generation task behind `lisa suggest` — a liblisa `Task`
/// (the same machinery as the ambient classifier): system prompt + JSON
/// Schema, sent as `response_format: json_schema` so the reply is
/// grammar-constrained to `{command, explanation}`.
pub(crate) fn suggest_task() -> liblisa::tasks::Task {
    liblisa::tasks::Task {
        name: "shell_suggest".into(),
        system: "You translate a natural-language request into exactly ONE shell \
                 command for a Linux system. Return ONLY the JSON object: command \
                 (a single command line — no shell prompt, no backticks, no \
                 comments) and explanation (one short sentence on what it does, \
                 flagging anything destructive). Prefer common, portable tools; \
                 when the request is ambiguous pick the most conservative reading."
            .into(),
        schema: json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "maxLength": 300},
                "explanation": {"type": "string", "maxLength": 200}
            },
            "required": ["command", "explanation"]
        }),
    }
}

/// `lisa suggest "<what you want>"` — prints the suggestion and STOPS.
/// stdout carries exactly the command (the hooks substitute it into the
/// prompt line unparsed); the explanation goes to stderr, dimmed on a
/// terminal. `--json` emits the raw `{command, explanation}` object
/// instead. Never executes anything — the review gate is the user's own
/// Enter key.
pub(crate) fn suggest_cmd(
    request: &str,
    url: &str,
    model: Option<String>,
    json_out: bool,
) -> anyhow::Result<()> {
    if request.trim().is_empty() {
        bail!("empty request — usage: lisa suggest \"find the 5 biggest files here\"");
    }
    let mut body = suggest_task().request(request);
    if let Some(m) = model {
        body["model"] = m.into();
    }
    let content = crate::chat_completion(url, &body)?;
    let v: Value = serde_json::from_str(content.trim())
        .with_context(|| format!("model reply was not the JSON object: {content}"))?;
    // Model output reaches the terminal AND (via the Ctrl+G hook) the shell
    // line buffer — strip every control character (issue #15). The command
    // additionally collapses to one line: a smuggled newline would split it
    // into "the part you review" + "the part that runs on your Enter".
    let command: String = crate::sanitize_terminal(v["command"].as_str().unwrap_or(""))
        .replace(['\n', '\t'], " ")
        .trim()
        .to_string();
    if command.is_empty() {
        bail!("no command came back — try rewording the request");
    }
    let explanation = crate::sanitize_terminal(v["explanation"].as_str().unwrap_or(""))
        .trim()
        .to_string();
    if json_out {
        println!(
            "{}",
            json!({"command": command, "explanation": explanation})
        );
        return Ok(());
    }
    if !explanation.is_empty() {
        if std::io::stderr().is_terminal() {
            eprintln!("\x1b[2m{explanation}\x1b[0m");
        } else {
            eprintln!("{explanation}");
        }
    }
    println!("{command}");
    Ok(())
}

/// Last `max_bytes` of `s`, respecting char boundaries.
fn tail_excerpt(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len() - max_bytes;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_body_carries_command_exit_and_output_tail() {
        let body = explain_body(
            "cargo biuld",
            Some(101),
            "error: no such command: `biuld`",
            None,
        );
        assert_eq!(body["stream"], true);
        assert!(
            body["model"].is_null(),
            "no model hint means server default"
        );
        assert!(
            body.get("lisa_priority").is_none(),
            "explain runs at interactive priority"
        );
        assert_eq!(body["messages"][0]["role"], "system");
        let sys = body["messages"][0]["content"].as_str().unwrap();
        assert!(sys.contains("no markdown"), "plain-prose preamble: {sys}");
        let user = body["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("Command: cargo biuld"));
        assert!(user.contains("Exit code: 101"));
        assert!(user.contains("no such command"));
    }

    #[test]
    fn explain_output_is_tail_truncated() {
        let noise = "x".repeat(10_000) + "THE REAL ERROR";
        let body = explain_body("make", Some(2), &noise, None);
        let user = body["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("THE REAL ERROR"), "the tail survives");
        assert!(
            user.len() < OUTPUT_TAIL_BYTES + 200,
            "the head is dropped: {} bytes",
            user.len()
        );
    }

    #[test]
    fn explain_body_works_from_piped_output_alone() {
        let body = explain_body("", None, "segfault at 0x0", Some("qwen3-0.6b"));
        let user = body["messages"][1]["content"].as_str().unwrap();
        assert!(!user.contains("Command:"));
        assert!(!user.contains("Exit code:"));
        assert!(user.contains("segfault"));
        assert_eq!(body["model"], "qwen3-0.6b");
    }

    #[test]
    fn suggest_request_is_guided_generation() {
        let req = suggest_task().request("show the five biggest files here");
        assert_eq!(req["response_format"]["type"], "json_schema");
        assert_eq!(
            req["response_format"]["json_schema"]["name"],
            "shell_suggest"
        );
        let schema = &req["response_format"]["json_schema"]["schema"];
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["properties"]["explanation"].is_object());
        assert_eq!(schema["required"][0], "command");
        assert_eq!(
            req["messages"][1]["content"],
            "show the five biggest files here"
        );
    }

    #[test]
    fn suggest_schema_compiles_to_a_bounded_grammar() {
        // Same guarantee as the ambient classifier: the schema compiles
        // to a GBNF grammar, with the free-text fields length-bounded so
        // a small model cannot spiral.
        let g = suggest_task().grammar().unwrap();
        assert!(g.contains(r#""\"command\"""#), "grammar: {g}");
        assert!(g.contains("{0,300}"), "bounded command: {g}");
        assert!(g.contains("{0,200}"), "bounded explanation: {g}");
    }

    #[test]
    fn tail_excerpt_respects_char_boundaries() {
        let s = "é".repeat(2000); // 2 bytes per char
        let tail = tail_excerpt(&s, 2001);
        assert!(tail.len() <= 2001);
        assert!(tail.chars().all(|c| c == 'é'));
        assert_eq!(tail_excerpt("short", 2000), "short");
    }

    /// The shipped hooks must parse: `zsh -n`, `bash -n`, `sh -n`
    /// (checks skip when a shell is not on the host — CI has all three).
    #[test]
    fn hook_scripts_parse() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/terminal-integration"
        );
        for (shell, file) in [
            ("zsh", "lisa-terminal.zsh"),
            ("bash", "lisa-terminal.bash"),
            ("sh", "lisa-terminal.sh"),
        ] {
            let path = format!("{dir}/{file}");
            assert!(
                std::path::Path::new(&path).exists(),
                "hook file missing: {path}"
            );
            match std::process::Command::new(shell)
                .arg("-n")
                .arg(&path)
                .output()
            {
                Ok(out) => assert!(
                    out.status.success(),
                    "{shell} -n {file}: {}",
                    String::from_utf8_lossy(&out.stderr)
                ),
                Err(_) => eprintln!("skipping {shell} syntax check (shell not installed)"),
            }
        }
    }
}

#[cfg(test)]
mod sanitize_tests {
    use crate::sanitize_terminal;

    #[test]
    fn control_sequences_are_stripped_from_model_output() {
        // ESC/CSI, CR, BEL, backspace all drop; text, newline, tab stay.
        assert_eq!(
            sanitize_terminal("safe\x1b[31mred\x1b[0m\rline\x07\x08!\nnext\ttab"),
            "saferedline!\nnext\ttab"
        );
        assert_eq!(sanitize_terminal("plain"), "plain");
        assert_eq!(sanitize_terminal(""), "");
        // OSC titles and two-char escapes drop whole — no residue.
        assert_eq!(sanitize_terminal("a\x1b]0;evil\x07b\x1bcc"), "abc");
        assert_eq!(sanitize_terminal("tail\x1b"), "tail");
    }
}
