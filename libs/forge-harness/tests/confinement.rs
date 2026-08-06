//! Does the kernel actually refuse? (ADR-0029 phase 3, #307, #309)
//!
//! The unit tests in `confine.rs` check that the module *reports*
//! honestly — that an unavailable jail says so. They cannot check that a
//! jail which claims to be closed is closed, because that is a property
//! of the kernel, not of our types. So these tests do the only thing
//! that settles it: they run a child that tries the escape and look at
//! whether the file is there afterwards.
//!
//! Linux-only, and that is not a gap this file can close: Landlock is a
//! Linux LSM and on macOS `available()` reports `Unavailable`, the hook
//! is never installed, and the child genuinely runs unconfined. A macOS
//! run of this file executes nothing, which is the honest outcome — the
//! confinement is a Linux property and only a Linux host can witness it.
#![cfg(target_os = "linux")]

use forge_harness::confine::{Confinement, confine_command};
use forge_harness::{ShellTool, ToolCall, ToolProvider};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A base directory OUTSIDE `/tmp`.
///
/// `std::env::temp_dir()` is in the writable set on purpose (a build
/// needs scratch space), so a probe that writes into a `tempfile`
/// tempdir would succeed whether or not the jail closed and prove
/// nothing. `CARGO_TARGET_TMPDIR` is under `target/`, which is granted
/// to nothing.
fn base(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("confinement-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("target tmpdir");
    dir
}

/// Landlock needs a kernel that has it. A machine without one is not a
/// failure of this code and must not be reported as a pass either — the
/// skip is printed, and CI runs on a kernel that has it.
fn skip_unless_enforced(c: &Confinement) -> bool {
    if let Some(note) = c.note() {
        eprintln!("SKIPPED: no confinement to observe here — {note}");
        return true;
    }
    false
}

/// #309: the writable set is what a build needs, not the whole of
/// `~/.cargo`.
///
/// `~/.cargo/bin` is on the user's `$PATH` and `~/.cargo/config.toml` is
/// read by every later `cargo` invocation — the same escape #63 closed
/// at the argv layer (`cargo --config 'target.…runner=["/bin/sh"]'`),
/// through a different door. A confined child that can write either one
/// has persistence outside the jail and runs UNCONFINED the next time
/// the user opens a terminal.
#[test]
fn a_confined_child_cannot_write_cargo_bin_or_cargo_config() {
    let base = base("cargo-home");
    let project = base.join("project");
    let home = base.join("home");
    let outside = base.join("outside");
    for dir in [
        &project,
        &outside,
        &home.join(".cargo/bin"),
        &home.join(".cargo/registry"),
    ] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(home.join(".cargo/config.toml"), "# the user's own\n").unwrap();

    // Each line prints only if its redirection opened. A refused open
    // makes the redirection fail, so `&&` never reaches the echo.
    let script = r#"
        echo probe > /dev/null                 && echo WROTE-devnull
        cat "$H/.cargo/config.toml" > /dev/null && echo READ-cargo-config
        printf x > "$H/.cargo/bin/probe"       && echo WROTE-cargo-bin
        printf x > "$H/.cargo/config.toml"     && echo WROTE-cargo-config
        printf x > "$H/.cargo/registry/probe"  && echo WROTE-registry
        printf x > "$P/probe"                  && echo WROTE-project
        printf x > "$O/probe"                  && echo WROTE-outside
        exit 0
    "#;
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(script)
        .env("H", &home)
        .env("P", &project)
        .env("O", &outside)
        .stderr(Stdio::null());
    let confinement = confine_command(&mut cmd, &project, &home);
    if skip_unless_enforced(&confinement) {
        return;
    }
    let out = cmd.output().expect("spawn the probe");
    let seen = String::from_utf8_lossy(&out.stdout).to_string();

    // What a build legitimately needs must still work, or the fix is a
    // broken toolchain rather than a jail.
    //
    // `2>/dev/null` appears in roughly every build script ever written,
    // and a ruleset that refuses it is one people will switch off.
    assert!(
        seen.contains("WROTE-devnull"),
        "a confined child cannot redirect to /dev/null: {seen}"
    );
    // A rustup `cargo` LIVES in `~/.cargo/bin`, so read has to survive
    // the narrowing: the child could not exec the toolchain otherwise,
    // `exec` being after `restrict_self`.
    assert!(
        seen.contains("READ-cargo-config"),
        "~/.cargo is not even readable, so a rustup toolchain cannot run: {seen}"
    );
    assert!(
        seen.contains("WROTE-registry"),
        "the registry cache is not writable, so no build will work: {seen}"
    );
    assert!(
        seen.contains("WROTE-project"),
        "the project is not writable: {seen}"
    );

    // What it must not reach.
    assert!(
        !seen.contains("WROTE-cargo-bin"),
        "a confined child wrote ~/.cargo/bin, which is on the user's PATH: {seen}"
    );
    assert!(
        !seen.contains("WROTE-cargo-config"),
        "a confined child wrote ~/.cargo/config.toml, which every later cargo reads: {seen}"
    );
    assert!(
        !seen.contains("WROTE-outside"),
        "a confined child wrote outside the project and the caches: {seen}"
    );

    // Not just what the child said — what is on disk.
    assert!(
        !home.join(".cargo/bin/probe").exists(),
        "~/.cargo/bin/probe exists"
    );
    assert!(!outside.join("probe").exists(), "the escape file exists");
    assert_eq!(
        fs::read_to_string(home.join(".cargo/config.toml")).unwrap(),
        "# the user's own\n",
        "the user's cargo config was overwritten by a confined child"
    );
}

/// #307: `run_shell` runs an arbitrary line, and it gets the same jail
/// the narrower `run_command` gets.
///
/// The consent gate is real and is not what this tests. It tests the
/// layer under it: a person approving a one-liner they did not read to
/// the end is approving something confined to the project, not
/// something with their whole filesystem in reach.
#[test]
fn the_shell_tool_child_is_confined_to_the_project() {
    if skip_unless_enforced(&forge_harness::confine::available()) {
        return;
    }
    let base = base("shell-escape");
    let project = base.join("project");
    let outside = base.join("outside");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let tool = ShellTool::new(&project, |_| true).unwrap();
    let line = format!(
        "printf x > inside && echo WROTE-inside; printf x > {}/probe && echo WROTE-outside",
        outside.display()
    );
    let out = tool.execute(&ToolCall {
        id: "1".into(),
        name: "run_shell".into(),
        args: serde_json::json!({ "command": line }),
    });

    assert!(
        out.text.contains("WROTE-inside") && project.join("inside").exists(),
        "the shell cannot write its own project directory: {}",
        out.text
    );
    assert!(
        !out.text.contains("WROTE-outside"),
        "run_shell wrote outside the project: {}",
        out.text
    );
    assert!(
        !outside.join("probe").exists(),
        "run_shell left a file outside the project: {}",
        out.text
    );
}
