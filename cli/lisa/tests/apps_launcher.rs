//! The apps channel, end to end: what `lisa apps` installs is what
//! `/usr/bin/lisa-app` launches (issue #239).
//!
//! This is an integration test rather than a unit test because BOTH sides
//! have to be the shipped ones — the real CLI binary and the real launcher
//! script from `os/packages/lisa/lisa-app`. The defect it exists for could
//! not be caught any other way: every unit on each side was correct, and
//! the two sides simply disagreed about a directory. A test that spelled
//! the path itself would have agreed with whichever side it copied.
//!
//! Nothing here hardcodes a payload path. The state root is a tempdir; the
//! installer decides where the tree goes, the launcher asks where it went.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A payload tarball shaped the way release.yml ships one: the contents of
/// the tree at the tarball root, no wrapping directory.
fn payload(dir: &Path, marker: &str) -> PathBuf {
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("assistant")).unwrap();
    std::fs::create_dir_all(src.join("mail")).unwrap();
    std::fs::write(
        src.join("assistant/lisa-assistant.js"),
        format!("// {marker}\n"),
    )
    .unwrap();
    std::fs::write(src.join("mail/lisa-mail.js"), format!("// {marker}\n")).unwrap();
    let tarball = dir.join("lisa-apps_20260804.99.tar.zst");
    let ok = Command::new("tar")
        .args(["--zstd", "-cf"])
        .arg(&tarball)
        .arg("-C")
        .arg(&src)
        .arg(".")
        .status()
        .expect("running tar");
    assert!(ok.success(), "tar --zstd failed (zstd support required)");
    tarball
}

/// A PATH directory holding the two programs the launcher calls: the real
/// `lisa` under test, and a `gjs` that prints the script it was handed
/// instead of running it.
fn fake_bin(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    write_exec(
        &bin.join("lisa"),
        &format!("#!/bin/sh\nexec {} \"$@\"\n", env!("CARGO_BIN_EXE_lisa")),
    );
    // lisa-app execs `gjs -m <script> [args...]`.
    write_exec(&bin.join("gjs"), "#!/bin/sh\ncat \"$2\"\n");
    bin
}

fn write_exec(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct World {
    _dir: tempfile::TempDir,
    state: PathBuf,
    sysroot: PathBuf,
    bin: PathBuf,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let sysroot = dir.path().join("sysroot");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&sysroot).unwrap();
        let bin = fake_bin(dir.path());
        Self {
            _dir: dir,
            state,
            sysroot,
            bin,
        }
    }

    fn lisa(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_lisa"))
            .args(args)
            .env("LISA_APPS_STATE", &self.state)
            .env("LISA_APPS_ROOT", &self.sysroot)
            .output()
            .expect("running lisa")
    }

    /// Run the shipped launcher exactly as a `.desktop` entry does.
    fn lisa_app(&self, rel: &str) -> std::process::Output {
        Command::new("sh")
            .arg(repo().join("os/packages/lisa/lisa-app"))
            .arg(rel)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("LISA_APPS_STATE", &self.state)
            .env("LISA_APPS_ROOT", &self.sysroot)
            .env_remove("LISA_APPS_DIR")
            .output()
            .expect("running lisa-app")
    }
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// THE test for #239: install a payload, launch an app, and prove the code
/// that ran is the code that was installed.
///
/// Neither the installer's directory nor the launcher's is written down
/// here — only the marker inside the payload. Any future disagreement
/// between the two sides fails this regardless of which one moved.
#[test]
fn the_launcher_runs_the_payload_that_was_just_installed() {
    let w = World::new();
    let marker = "MARKER-239-installed-tree";
    let tarball = payload(w._dir.path(), marker);

    let out = w.lisa(&[
        "apps",
        "install",
        "shell",
        tarball.to_str().unwrap(),
        "--version",
        "20260804.99",
    ]);
    assert!(
        out.status.success(),
        "install failed: {}{}",
        stdout(&out),
        stderr(&out)
    );

    let run = w.lisa_app("assistant/lisa-assistant.js");
    assert!(
        run.status.success(),
        "lisa-app could not launch the app it had just been given: {}{}",
        stdout(&run),
        stderr(&run)
    );
    assert!(
        stdout(&run).contains(marker),
        "the launcher ran something else than the installed tree: {:?}",
        stdout(&run)
    );

    // Same for an app out of `apps/` — Mail, Surfer and Preview ride this
    // channel too (#239 defect 2), and they are resolved by the same
    // launcher with a relative path under the same tree.
    let mail = w.lisa_app("mail/lisa-mail.js");
    assert!(
        stdout(&mail).contains(marker),
        "an apps/ surface did not resolve to the installed tree: {:?}",
        stdout(&mail)
    );
}

/// `lisa apps status` must report the tree that launches, not the symlink
/// that was written. It said `current: 20260804.76, installed: 20260804.76`
/// for three releases while every launch used the baked copy (#239 defect
/// 3) — the shape of #219 (a socket presence read as tool availability)
/// and #192 (a comment asserting a bound nothing enforced).
#[test]
fn status_reports_the_directory_that_actually_launches() {
    let w = World::new();
    let marker = "MARKER-239-status";
    let tarball = payload(w._dir.path(), marker);
    w.lisa(&[
        "apps",
        "install",
        "shell",
        tarball.to_str().unwrap(),
        "--version",
        "20260804.99",
    ]);

    let out = w.lisa(&["apps", "status"]);
    let text = stdout(&out);
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("resolves:"))
        .unwrap_or_else(|| panic!("status names no resolved directory:\n{text}"));
    let dir = PathBuf::from(
        line.trim_start()
            .trim_start_matches("resolves:")
            .split(" — ")
            .next()
            .unwrap()
            .trim(),
    );
    // Not "a plausible path" — the directory status named must be the one
    // holding the payload that was installed.
    let ran = std::fs::read_to_string(dir.join("assistant/lisa-assistant.js"))
        .unwrap_or_else(|e| panic!("status pointed at {}: {e}", dir.display()));
    assert!(ran.contains(marker), "status named the wrong tree: {ran:?}");
    assert!(text.contains("20260804.99"), "no version reported:\n{text}");
}

/// A recorded version that cannot launch must say so. Bookkeeping and
/// reality diverge exactly here, and this is the sentence that would have
/// caught #239 on the device in one command.
#[test]
fn status_says_plainly_when_the_recorded_version_does_not_launch() {
    let w = World::new();
    let tarball = payload(w._dir.path(), "MARKER-239-gone");
    w.lisa(&[
        "apps",
        "install",
        "shell",
        tarball.to_str().unwrap(),
        "--version",
        "20260804.99",
    ]);
    // The record survives; the tree does not. (A tempdir of this test's
    // own making — nothing of the user's is touched.)
    let base = String::from_utf8(w.lisa(&["apps", "path", "shell", "--base"]).stdout).unwrap();
    std::fs::remove_dir_all(Path::new(base.trim()).join("versions/20260804.99")).unwrap();

    let text = stdout(&w.lisa(&["apps", "status"]));
    assert!(
        text.contains("does NOT launch"),
        "status still reports a version that cannot launch:\n{text}"
    );
}

/// #333: a base that is not absolute resolves against the launcher's own
/// cwd — observed once on the reference device as gjs importing a surface
/// relative to $HOME off a mid-sync channel. Wherever such a base comes
/// from, it is never a payload tree: the launcher must skip it, even when
/// the file it names happens to exist.
#[test]
fn a_relative_base_is_refused_even_when_its_file_exists() {
    let w = World::new();
    // A tree that WOULD resolve if the relative base were honored.
    let cwd = w._dir.path().join("home");
    std::fs::create_dir_all(cwd.join("reltree/assistant")).unwrap();
    std::fs::write(
        cwd.join("reltree/assistant/lisa-assistant.js"),
        "// MARKER-333-relative\n",
    )
    .unwrap();
    let run = Command::new("sh")
        .arg(repo().join("os/packages/lisa/lisa-app"))
        .arg("assistant/lisa-assistant.js")
        .current_dir(&cwd)
        .env("PATH", format!("{}:/usr/bin:/bin", w.bin.display()))
        .env("LISA_APPS_STATE", &w.state)
        .env("LISA_APPS_ROOT", &w.sysroot)
        .env("LISA_APPS_DIR", "reltree")
        .output()
        .expect("running lisa-app");
    assert!(
        !stdout(&run).contains("MARKER-333-relative"),
        "the launcher resolved a surface through a relative base: {:?}",
        stdout(&run)
    );
    assert_eq!(
        run.status.code(),
        Some(127),
        "with no absolute tree anywhere, the launch must fail loudly, \
         not fall back to guessing: {}{}",
        stdout(&run),
        stderr(&run)
    );
}

/// The launcher must not carry a payload path of its own. One spelling
/// lives in cli/lisa/src/apps.rs; a second one in a shell script is the
/// defect, and a third would be a "fix" that reproduces it.
#[test]
fn the_launcher_spells_no_payload_path_of_its_own() {
    let script = std::fs::read_to_string(repo().join("os/packages/lisa/lisa-app")).unwrap();
    for line in script.lines() {
        let code = line.split('#').next().unwrap_or("");
        assert!(
            !code.contains("/var/lib/"),
            "lisa-app spells a payload path itself — ask `lisa apps path`: {line}"
        );
    }
    assert!(
        script.contains("lisa apps path shell"),
        "lisa-app does not ask the CLI where the tree is"
    );
}
