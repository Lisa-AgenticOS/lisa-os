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

// ---------------------------------------------------------------------
// The disk guard (#130 phase 0, ADR-0034)
//
// repart weights are `var 3 : home 1` because models dominate /var. A
// dev container plus its packages is easily several GB, and it lands in
// $HOME — where it competes with the person's documents rather than
// with model weights. So `lisa dev` refuses loudly when the filesystem
// is tight instead of filling it.
//
// "Loudly" is the whole requirement. A tool that fills a disk and then
// fails at some later, unrelated write has told the person nothing
// about what happened, and the write that finally fails is usually
// somebody else's.

/// How much has to remain free AFTER an install, whatever the request.
///
/// Not a percentage: a percentage of a 2 TB disk reserves 40 GB nobody
/// needs, and a percentage of a 64 GB one reserves too little to boot a
/// desktop that logs. This is an absolute floor sized for "the session
/// keeps working" — journald, temp files, the context store growing,
/// and one A/B update's staging.
pub const HEADROOM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// What the guard decided, and why.
#[derive(Debug, PartialEq, Eq)]
pub enum Room {
    /// Enough space, with this much left over afterwards.
    Enough { remaining: u64 },
    /// Not enough. Carries the numbers the message needs — a refusal
    /// that does not say how short it is leaves the person guessing at
    /// how much to free.
    Tight {
        free: u64,
        needed: u64,
        short_by: u64,
    },
}

/// Is there room for `needed` bytes, keeping [`HEADROOM_BYTES`] free?
///
/// Pure, so the arithmetic is testable without a filesystem — the part
/// that has to be right is the boundary, and a test that needs a full
/// disk to exercise it never gets written.
pub fn room_for(free: u64, needed: u64) -> Room {
    // `checked_add`, not `saturating_add`. Saturation clamps the
    // REQUIREMENT down to `u64::MAX`, so a request near the ceiling
    // compares as satisfiable and the guard says yes to infinity — the
    // one direction a size check must never fail in. Overflow means the
    // request is absurd, which is the tightest possible answer.
    let Some(required) = needed.checked_add(HEADROOM_BYTES) else {
        return Room::Tight {
            free,
            needed,
            short_by: u64::MAX,
        };
    };
    if free >= required {
        Room::Enough {
            remaining: free - needed,
        }
    } else {
        Room::Tight {
            free,
            needed,
            short_by: required - free,
        }
    }
}

/// Bytes available to an unprivileged user on the filesystem holding
/// `path`.
///
/// **`f_bavail`, not `f_bfree`.** The difference is the reserved blocks
/// only root may use — typically 5% of the filesystem. Reading `f_bfree`
/// would promise space this path can never have, since `lisa dev` never
/// escalates (CLAUDE.md 7b).
///
/// The path matters more than it looks: this must be the container
/// store's ACTUAL location, not `$HOME` by name. A machine with a
/// separate mount under the home directory gets a confident wrong
/// answer otherwise, and the guard's whole job is to be right about
/// which disk fills up.
pub fn free_bytes_at(path: &Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    // SAFETY: `statvfs` writes into a zeroed struct we own, and the path
    // is a NUL-terminated string that outlives the call.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // `as`, not `u64::from`: these fields are `u64` on 64-bit Linux and
    // macOS and narrower elsewhere, so `From` does not exist on every
    // target this crate builds for.
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// The nearest existing ancestor of `path`.
///
/// `statvfs` fails on a path that does not exist yet, and the container
/// store legitimately does not on a machine where `lisa dev` has never
/// run. Walking up finds the filesystem the directory WILL be created
/// on, which is the one the answer is about.
pub fn existing_ancestor(path: &Path) -> Option<&Path> {
    let mut p = Some(path);
    while let Some(candidate) = p {
        if candidate.exists() {
            return Some(candidate);
        }
        p = candidate.parent();
    }
    None
}

/// Human bytes: `2.0 GiB`, not `2147483648`.
///
/// A refusal is read by a person deciding what to delete, and nine
/// digits is not a quantity anybody can act on.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod disk_guard_tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_boundary_is_exact_in_both_directions() {
        // The one arithmetic that has to be right, and the one a test
        // needing a full disk never gets written for.
        assert_eq!(
            room_for(5 * GIB + HEADROOM_BYTES, 5 * GIB),
            Room::Enough {
                remaining: HEADROOM_BYTES
            },
            "exactly enough is enough"
        );
        assert!(
            matches!(
                room_for(5 * GIB + HEADROOM_BYTES - 1, 5 * GIB),
                Room::Tight { .. }
            ),
            "one byte short is short"
        );
    }

    #[test]
    fn a_refusal_says_how_short_it_is() {
        // A refusal that does not carry the number leaves the person
        // guessing at how much to free, and they guess low.
        let Room::Tight {
            free,
            needed,
            short_by,
        } = room_for(GIB, 4 * GIB)
        else {
            panic!("1 GiB free must not accept a 4 GiB install");
        };
        assert_eq!(free, GIB);
        assert_eq!(needed, 4 * GIB);
        assert_eq!(short_by, 4 * GIB + HEADROOM_BYTES - GIB);
    }

    #[test]
    fn headroom_is_reserved_even_when_the_request_is_nothing() {
        // `lisa dev shell` on a machine with 100 MB free should refuse
        // too: entering a container writes layers, and a disk that full
        // is one write from breaking the session.
        assert!(matches!(room_for(100 * 1024 * 1024, 0), Room::Tight { .. }));
        assert!(matches!(room_for(HEADROOM_BYTES, 0), Room::Enough { .. }));
    }

    #[test]
    fn an_absurd_request_does_not_wrap_around() {
        // `needed + HEADROOM` overflowing u64 would make the largest
        // possible request look like the smallest — a guard that says
        // yes to infinity.
        assert!(matches!(room_for(u64::MAX, u64::MAX), Room::Tight { .. }));
        assert!(matches!(room_for(GIB, u64::MAX), Room::Tight { .. }));
    }

    #[test]
    fn the_filesystem_measured_is_the_one_the_store_lands_on() {
        // THE TRAP this guard exists to avoid: measuring `$HOME` by name
        // when the container store is on a different mount. `/` and a
        // temp dir are the two filesystems every machine has, and on a
        // dev host they are usually the same one — so this asserts the
        // call SUCCEEDS on a real path and returns a plausible number,
        // rather than asserting two mounts differ on a machine where
        // they do not.
        let root = free_bytes_at(Path::new("/")).expect("statvfs on / must work");
        assert!(root > 0, "a mounted filesystem reports some free space");
        let tmp = free_bytes_at(&std::env::temp_dir()).expect("statvfs on temp must work");
        assert!(tmp > 0);
    }

    #[test]
    fn the_number_is_what_an_unprivileged_writer_can_have() {
        // `f_bavail`, not `f_bfree`: the gap between them is the reserve
        // only root may write into — typically 5% on ext4. Reading
        // `f_bfree` would promise `lisa dev` space it can never have,
        // since it never escalates (CLAUDE.md 7b), and the install would
        // fail partway with ENOSPC after the guard said yes.
        //
        // Only assertable where the filesystem HAS a reserve. APFS and
        // tmpfs usually report the two equal, and on such a host this
        // asserts nothing — said out loud rather than left to look like
        // coverage. Linux CI on ext4 is where it bites.
        let path = std::env::temp_dir();
        let c = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()))
            .unwrap();
        let mut raw: libc::statvfs = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::statvfs(c.as_ptr(), &mut raw) }, 0);

        let bavail = raw.f_bavail as u64 * raw.f_frsize as u64;
        let bfree = raw.f_bfree as u64 * raw.f_frsize as u64;
        let ours = free_bytes_at(&path).unwrap();

        assert_eq!(
            ours, bavail,
            "the guard must report the unprivileged figure"
        );
        if bfree != bavail {
            assert!(
                ours < bfree,
                "this filesystem reserves {} for root and the guard counted it",
                human(bfree - bavail)
            );
        }
    }

    #[test]
    fn a_path_that_does_not_exist_yet_measures_where_it_will_live() {
        // The container store legitimately does not exist before the
        // first `lisa dev install`, and `statvfs` fails on a missing
        // path. Answering "cannot tell" there would refuse every first
        // run on every machine.
        let missing = std::env::temp_dir().join("lisa-dev-not-created-yet/containers/storage");
        assert!(!missing.exists());
        let ancestor = existing_ancestor(&missing).expect("temp dir exists");
        assert!(ancestor.exists());
        assert!(free_bytes_at(ancestor).unwrap() > 0);
    }

    #[test]
    fn a_refusal_is_readable_by_a_person() {
        // Nine digits is not a quantity anybody can act on.
        assert_eq!(human(0), "0 B");
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2 * GIB), "2.0 GiB");
        assert_eq!(human(1536 * 1024 * 1024), "1.5 GiB");
        // The table stops at TiB on purpose — a disk larger than that
        // is not a case worth a unit — so the largest value renders as
        // a very large number of TiB rather than as EiB. Asserted
        // because "it does not crash" is not the same as "it reads".
        assert_eq!(human(u64::MAX), "16777216.0 TiB");
    }
}

// ---------------------------------------------------------------------
// `lisa dev doctor` — phase 0's prerequisites, checked rather than assumed
//
// #130 calls each phase-0 item "a silent-failure risk", and that is the
// whole reason this verb exists: rootless containers do not error when
// `/etc/subuid` is missing, they quietly fall back to a single-uid
// namespace and fail later at something unrelated. The nightly asserts
// these on the built image; a person on a real machine had no way to ask.

/// One prerequisite, and what to do when it is not met.
pub struct Prereq {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Is a subuid/subgid range mapped for this user?
///
/// Read from the files rather than by trying a container: a missing
/// range degrades QUIETLY, so "podman started" is not evidence that the
/// mapping exists.
fn subid_range(file: &str, user: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    text.lines()
        .find(|l| l.starts_with(&format!("{user}:")))
        .map(str::to_string)
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// Where rootless podman keeps its store — the path the disk guard must
/// measure, rather than `$HOME` by name.
pub fn container_store() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/containers/storage"))
}

pub fn doctor_cmd(needed_gib: u64) -> anyhow::Result<()> {
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    let mut checks = Vec::new();

    for file in ["/etc/subuid", "/etc/subgid"] {
        let found = subid_range(file, &user);
        checks.push(Prereq {
            name: if file.ends_with("subuid") {
                "subuid range"
            } else {
                "subgid range"
            },
            ok: found.is_some(),
            detail: found.unwrap_or_else(|| {
                format!("no line for `{user}` in {file} — rootless containers degrade silently")
            }),
        });
    }

    // One row per program, named for the program. Two rows both called
    // "uidmap helpers" is a report you cannot act on: it says something
    // is missing twice without saying which.
    for (name, prog) in [
        ("newuidmap", "newuidmap"),
        ("newgidmap", "newgidmap"),
        ("podman", "podman"),
    ] {
        let present = on_path(prog);
        checks.push(Prereq {
            name,
            ok: present,
            detail: if present {
                "on PATH".to_string()
            } else {
                "not on PATH — a subid range with no helper cannot be applied".to_string()
            },
        });
    }

    // Disk, measured where the store actually lands.
    let needed = needed_gib.saturating_mul(1024 * 1024 * 1024);
    match container_store().as_deref().and_then(existing_ancestor) {
        Some(dir) => match free_bytes_at(dir) {
            Ok(free) => match room_for(free, needed) {
                Room::Enough { remaining } => checks.push(Prereq {
                    name: "disk",
                    ok: true,
                    detail: format!(
                        "{} free on the filesystem holding {}; {} would remain after {}",
                        human(free),
                        dir.display(),
                        human(remaining),
                        human(needed)
                    ),
                }),
                Room::Tight { free, short_by, .. } => checks.push(Prereq {
                    name: "disk",
                    ok: false,
                    detail: format!(
                        "{} free on the filesystem holding {} — {} short for {} plus \
                         the {} that must stay free",
                        human(free),
                        dir.display(),
                        human(short_by),
                        human(needed),
                        human(HEADROOM_BYTES)
                    ),
                }),
            },
            Err(e) => checks.push(Prereq {
                name: "disk",
                ok: false,
                detail: format!("could not measure {}: {e}", dir.display()),
            }),
        },
        None => checks.push(Prereq {
            name: "disk",
            ok: false,
            detail: "no HOME, so there is nowhere to put a container store".into(),
        }),
    }

    for c in &checks {
        println!(
            "{} {:<18} {}",
            if c.ok { "ok  " } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    if checks.iter().all(|c| c.ok) {
        println!("\nrootless containers look usable on this machine.");
        return Ok(());
    }
    // Non-zero, so a script can tell "not ready" from "ready". The lines
    // above already said which part.
    bail!(
        "{} prerequisite(s) unmet",
        checks.iter().filter(|c| !c.ok).count()
    )
}
