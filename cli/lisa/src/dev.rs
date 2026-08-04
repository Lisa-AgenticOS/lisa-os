//! `lisa dev` — the developer namespace (ADR-0050).
//!
//! One verb so far: `lisa dev check [path]`, which runs everything
//! mechanical over an app tree and exits non-zero. It is the **single
//! authority on what a valid Lisa app is** (ADR-0050 §4), which is why
//! it is a verb rather than a check inside the Forge: the same judgement
//! then covers the loop, CI, and a person editing the file afterwards.
//! `lisa forge` calls it as its verifier — `Verifier::Command { program:
//! "lisa", args: ["dev", "check"] }`, the arm that already existed
//! (`libs/forge-harness/src/agent.rs`).
//!
//! ## What it checks, and why each one
//!
//! Every rule cites the incident that produced it. A check with no
//! incident behind it is somebody's taste with an exit code
//! (ADR-0050 §Consequences), so there are three:
//!
//! 1. **The tree contains GJS source at all.** Issue #29: `dart analyze`
//!    exits clean on a project with no sources, which let a model's bare
//!    "done" converge on an empty scaffold. A verifier that passes on
//!    nothing is worse than none.
//! 2. **No top-level `await` in the entry module.** The footgun that
//!    produces no log line at all: the module binds its socket,
//!    advertises it, and answers nothing. It cost this repo twice — Mail
//!    and Preview (`docs/ANATOMY-OF-AN-APP.md` §7).
//! 3. **The manifest parses and validates**, through **agentd's own
//!    parser** rather than a second implementation (ADR-0050 §4.1).
//!    Issue #241 is what a manifest nobody validates costs: a shipped
//!    app whose declared tools never reached the model, with nothing
//!    erroring, warning or logging.
//!
//! ## What it deliberately does not do
//!
//! **It does not run the app's tests, and it does not execute the app.**
//! ADR-0050 §3 lists the app's own suite as part of the eventual check,
//! and it belongs there — but the Forge calls this verb on code the
//! model wrote seconds ago, and `Verifier::check` runs plain argv with
//! none of the confinement `ShellTool` applies (`libs/forge-harness/src/confine.rs`).
//! A checker that executes model-authored code as part of verifying it
//! would hand the loop an escape the jail exists to prevent. Running the
//! suite lands when it can run confined; until then this file says so
//! rather than implying coverage it has not got.
//!
//! It also makes **no JavaScript syntax or type claim**. `gjs` has no
//! check-only mode, there is no analyzer in the tree, and ADR-0047
//! accepted losing Dart's static types as part of choosing GJS. Two of
//! ADR-0050's six traps are covered here, one partly, three not at all —
//! that ledger is the honest form, and overstating it would make this
//! the same kind of defect the traps are made of.
//!
//! `lisa dev new` (ADR-0050 §3) is not built. The checker carries the
//! argument; the generator carries only the convenience, and ADR-0050's
//! own reversal note says to keep the first and drop the second if one
//! has to go.

use anyhow::bail;
use std::path::{Path, PathBuf};

/// One thing wrong with an app tree, in the words the model and the
/// person both read.
#[derive(Debug, PartialEq)]
pub struct Finding {
    pub rule: &'static str,
    pub message: String,
}

impl Finding {
    fn new(rule: &'static str, message: impl Into<String>) -> Finding {
        Finding {
            rule,
            message: message.into(),
        }
    }
}

/// `lisa dev check [path]` — exits non-zero with findings on stdout.
///
/// Findings go to **stdout** because the Forge feeds a failing
/// verifier's output back to the model as the next turn's instructions
/// (`Verifier::Command` concatenates stdout and stderr), and a person
/// reading a terminal wants them in the same place.
pub fn check_cmd(path: Option<PathBuf>) -> anyhow::Result<()> {
    let dir = path.unwrap_or_else(|| PathBuf::from("."));
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let findings = check(&dir);
    if findings.is_empty() {
        println!("{}: ok", dir.display());
        return Ok(());
    }
    for f in &findings {
        println!("[{}] {}", f.rule, f.message);
    }
    bail!(
        "{} finding(s) — see `lisa dev check` in cli/lisa/README.md",
        findings.len()
    );
}

/// Every rule, over one app tree. Pure: it reads files and returns
/// findings, so the tests are the checks and not a subprocess.
pub fn check(dir: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    let sources = gjs_sources(dir);

    if sources.is_empty() {
        // Issue #29, restated for GJS: nothing has been written, so the
        // task cannot be done, and a verifier that says "clean" here is
        // how a bare "done" converges on an empty directory.
        out.push(Finding::new(
            "no-sources",
            "the project contains no JavaScript source files yet — nothing has been \
             written, so the task cannot be done. Write the app first: an entry \
             module `lisa-<name>.js`, pure logic under `lib/`, and an \
             `app.lisaos.<Name>.json` manifest.",
        ));
        return out;
    }

    for entry in entry_modules(dir, &sources) {
        if let Some(line) = top_level_await(&std::fs::read_to_string(&entry).unwrap_or_default()) {
            out.push(Finding::new(
                "top-level-await",
                format!(
                    "{}:{line}: top-level `await` in an entry module. The module binds \
                     its socket, advertises it, and answers nothing — with no error in \
                     any log. Put the awaited work inside the function that needs it \
                     (docs/ANATOMY-OF-AN-APP.md §7).",
                    rel(dir, &entry)
                ),
            ));
        }
    }

    out.extend(manifest_findings(dir));
    out
}

/// `.js` files under the tree, ignoring the places a checkout grows.
fn gjs_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(dir, &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "js") {
            out.push(p);
        }
    }
}

/// The modules that are *entered*, as opposed to imported: the ones at
/// the top of the tree, and anything named `lisa-*.js`.
///
/// Deliberately generous. The check below is cheap and its false
/// positives are real defects anywhere, so the cost of over-including is
/// a finding somebody should fix regardless.
fn entry_modules(dir: &Path, sources: &[PathBuf]) -> Vec<PathBuf> {
    sources
        .iter()
        .filter(|p| {
            let top = p.parent() == Some(dir);
            let named = p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("lisa-"));
            top || named
        })
        .cloned()
        .collect()
}

/// The 1-based line of a top-level `await`, if there is one.
///
/// Textual and deliberately conservative: it tracks brace depth outside
/// strings and comments, and only reports an `await` seen at depth zero.
/// A parser would be better and there is no JS parser in this tree; what
/// this cannot do is claim more than it checks, so it reports the first
/// one and nothing about the rest.
pub fn top_level_await(src: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_block_comment = false;
    for (i, raw) in src.lines().enumerate() {
        let line = strip_noise(raw, &mut in_block_comment);
        let trimmed = line.trim();
        if depth == 0
            && (trimmed.starts_with("await ")
                || trimmed.contains("= await ")
                || trimmed.contains("return await "))
        {
            return Some(i + 1);
        }
        depth += line.matches(['{', '(', '[']).count() as i32;
        depth -= line.matches(['}', ')', ']']).count() as i32;
        if depth < 0 {
            depth = 0;
        }
    }
    None
}

/// Blank out string literals and comments so their contents cannot look
/// like code. Crude on purpose — it only has to stop a brace or the word
/// `await` inside a string from moving the depth counter.
fn strip_noise(line: &str, in_block_comment: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if *in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
            }
            continue;
        }
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
                out.push(' ');
            }
            None => match c {
                '/' if chars.peek() == Some(&'/') => break,
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    *in_block_comment = true;
                }
                '"' | '\'' | '`' => {
                    quote = Some(c);
                    out.push(' ');
                }
                _ => out.push(c),
            },
        }
    }
    out
}

/// The manifest, through agentd's parser — never a second copy of the
/// grammar (ADR-0050 §4.1).
fn manifest_findings(dir: &Path) -> Vec<Finding> {
    let candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|t| t.contains("\"lisa_manifest\"")))
        .collect();

    if candidates.is_empty() {
        return vec![Finding::new(
            "no-manifest",
            "no MCP manifest in the app root. The manifest is what makes a window an \
             app: without it the app publishes no tools and the model can never reach \
             it (issue #241). Add `app.lisaos.<Name>.json` with `lisa_manifest`, \
             `app_id`, and a `tools` array.",
        )];
    }

    let mut out = Vec::new();
    for path in candidates {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                out.push(Finding::new(
                    "manifest",
                    format!("{}: unreadable: {e}", rel(dir, &path)),
                ));
                continue;
            }
        };
        match lisa_agentd::manifest::Manifest::from_json(&text) {
            Err(e) => out.push(Finding::new(
                "manifest",
                format!("{}: {e}", rel(dir, &path)),
            )),
            // `from_json` parses AND validates (it calls `validate()`
            // itself), so there is nothing left for this file to check
            // about the grammar — which is the point of calling agentd's
            // parser instead of restating it. A second `validate()` here
            // was dead code, and a mutation check is what showed it: no
            // test could tell whether it ran.
            Ok(m) => {
                // The app id is one string everywhere (ADR-0016): the
                // manifest filename, the `.desktop` basename, the socket
                // name, the icon name. Two spellings is #239's shape.
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                if stem != m.app_id {
                    out.push(Finding::new(
                        "manifest-name",
                        format!(
                            "{}: declares app_id {:?}, so the file must be named \
                             {}.json — the app id is the manifest name, the .desktop \
                             basename, the socket name and the icon name, and two \
                             spellings of it is how a launcher and an installer drift \
                             apart (#239, ADR-0016).",
                            rel(dir, &path),
                            m.app_id,
                            m.app_id
                        ),
                    ));
                }
            }
        }
    }
    out
}

fn rel(dir: &Path, p: &Path) -> String {
    p.strip_prefix(dir)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(app_id: &str) -> String {
        format!(
            r#"{{"lisa_manifest": 1, "app_id": "{app_id}",
                 "tools": [{{"name": "list_notes", "tier": "read",
                             "input_schema": {{"type": "object"}}}}]}}"#
        )
    }

    /// A complete, minimal Lisa app passes. Without this the suite below
    /// would be satisfied by a checker that fails everything.
    #[test]
    fn a_well_formed_gjs_app_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(
            d.join("lisa-notes.js"),
            "#!/usr/bin/env -S gjs -m\nimport Gtk from 'gi://Gtk?version=4.0';\n\
             async function main() {\n  await start();\n}\nmain();\n",
        )
        .unwrap();
        std::fs::create_dir(d.join("lib")).unwrap();
        std::fs::write(d.join("lib/notes.js"), "export function noop() {}\n").unwrap();
        std::fs::write(
            d.join("app.lisaos.Notes.json"),
            manifest("app.lisaos.Notes"),
        )
        .unwrap();
        assert_eq!(check(d), Vec::new());
    }

    /// Issue #29's gate, in GJS form: an empty directory can never be
    /// "done", and a verifier that says otherwise is how the loop
    /// converges on nothing.
    #[test]
    fn an_empty_tree_is_never_clean() {
        let dir = tempfile::tempdir().unwrap();
        let f = check(dir.path());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "no-sources");
    }

    /// The footgun with no log line.
    #[test]
    fn top_level_await_in_the_entry_module_is_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(
            d.join("lisa-notes.js"),
            "import Gio from 'gi://Gio';\nawait start();\n",
        )
        .unwrap();
        std::fs::write(
            d.join("app.lisaos.Notes.json"),
            manifest("app.lisaos.Notes"),
        )
        .unwrap();
        let f = check(d);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, "top-level-await");
        assert!(f[0].message.contains("lisa-notes.js:2"), "{:?}", f[0]);
    }

    /// …and the same word inside a function, a string or a comment is
    /// not. A check that fires on `// await` teaches people to ignore it.
    #[test]
    fn await_inside_a_function_a_string_or_a_comment_is_not_a_finding() {
        assert_eq!(
            top_level_await("async function f() {\n  await g();\n}\nf();\n"),
            None
        );
        assert_eq!(top_level_await("// await g();\n"), None);
        assert_eq!(top_level_await("/*\nawait g();\n*/\n"), None);
        assert_eq!(top_level_await("const s = 'await g();';\n"), None);
        assert_eq!(
            top_level_await("const o = {\n  run() {\n    const x = await g();\n  },\n};\n"),
            None
        );
        // But the real shape is caught, in both spellings.
        assert_eq!(top_level_await("const x = await g();\n"), Some(1));
        assert_eq!(top_level_await("await g();\n"), Some(1));
    }

    /// A window with no manifest is not an app — it is a window (#241).
    #[test]
    fn an_app_with_no_manifest_is_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lisa-notes.js"), "print('hi');\n").unwrap();
        let f = check(dir.path());
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, "no-manifest");
    }

    /// The grammar is agentd's, so a manifest this accepts is one the
    /// bus accepts. `tier: "destroy"` is not a tier.
    #[test]
    fn the_manifest_is_judged_by_the_parser_the_bus_uses() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("lisa-notes.js"), "print('hi');\n").unwrap();
        std::fs::write(
            d.join("app.lisaos.Notes.json"),
            r#"{"lisa_manifest": 1, "app_id": "app.lisaos.Notes",
                "tools": [{"name": "wipe", "tier": "destroy",
                           "input_schema": {"type": "object"}}]}"#,
        )
        .unwrap();
        let f = check(d);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, "manifest");

        // …and a tier the bus does know, on a schema it does not: the
        // input schema must be an object type.
        std::fs::write(
            d.join("app.lisaos.Notes.json"),
            r#"{"lisa_manifest": 1, "app_id": "app.lisaos.Notes",
                "tools": [{"name": "wipe", "tier": "destructive",
                           "input_schema": {"type": "string"}}]}"#,
        )
        .unwrap();
        let f = check(d);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("input_schema"), "{:?}", f[0]);
    }

    /// One app id, one spelling (ADR-0016).
    #[test]
    fn the_manifest_filename_must_be_the_app_id() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("lisa-notes.js"), "print('hi');\n").unwrap();
        std::fs::write(d.join("manifest.json"), manifest("app.lisaos.Notes")).unwrap();
        let f = check(d);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, "manifest-name");
    }
}
