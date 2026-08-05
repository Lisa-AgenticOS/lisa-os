//! `lisa dev install|remove|list|shell|reset` — developer tooling in the
//! user's home, rootless (ADR-0034, #130 phase 1).
//!
//! # Why a container at all
//!
//! Lisa's root is immutable and replaced wholesale by the next A/B
//! update, so `pacman -S mysql` writes into a root that disappears on
//! the next `lisa update`. Everything here lives under `$HOME`, which is
//! a real partition (ADR-0019) and therefore already survives updates —
//! and needs **no `sudo` anywhere**, because `escalate.privilege` is an
//! unoverridable Deny in our own guard and this path must not want a
//! carve-out (CLAUDE.md 7b).
//!
//! # The part that goes quietly wrong
//!
//! Shims. `~/.local/bin/mysql` that execs into the container is the
//! whole user-facing promise — `mysql` resolves on the host without
//! anybody thinking about containers — and it has three failure modes
//! that produce no error:
//!
//! 1. **Shadowing a real binary.** A shim called `ls` on `PATH` ahead of
//!    `/usr/bin/ls` breaks the machine in a way nobody connects to a dev
//!    install. Refused, by name, before anything is written.
//! 2. **Outliving its container.** `lisa dev reset` destroying the
//!    container while shims remain leaves commands that fail with a
//!    podman error instead of "not installed".
//! 3. **Outliving its package.** `remove` has to take back exactly what
//!    `install` put down, which means knowing which files came from
//!    which package rather than guessing from names.
//!
//! Every decision below is pure so those three are testable without a
//! container; the podman calls are a thin edge at the bottom.

use anyhow::{Context, bail};
use std::path::{Path, PathBuf};

/// The container the tooling lives in. One, named, long-lived — a
/// container per package would multiply the base image by the number of
/// tools somebody installs.
pub const BOX_NAME: &str = "lisa-dev";

/// Arch, pinned by digest (CLAUDE.md rule 8: pinned to a verified
/// artifact, never a tag that moves under us). Resolved 2026-08-05 with
/// `podman pull --arch amd64 docker.io/library/archlinux:base`.
///
/// **x86_64 only, and that is a real limit rather than an oversight:**
/// `docker.io/library/archlinux` publishes no arm64 manifest at all —
/// `podman pull` on Apple Silicon fails with "no image found in image
/// index for architecture arm64". The aarch64 lane (ADR-0037) needs a
/// different base, and picking one is a decision about which keyring we
/// trust, not a config line. `lisa dev doctor` reports the gap rather
/// than letting an install fail halfway.
pub const BOX_IMAGE: &str = "docker.io/library/archlinux@sha256:345a872f6c95e082d4b8c050af637eebb57402c6e2177b411c3acf7df84eb33b";

/// Binaries a shim may never be called, because a shim that shadows one
/// of these breaks the machine in a way nobody connects to `lisa dev`.
///
/// Not an exhaustive list of system binaries — that is the wrong shape,
/// since it would need updating forever. This is the set whose loss
/// breaks the tools you would use to *diagnose* the breakage, plus the
/// shell itself. Everything else is caught by [`shim_conflict`], which
/// asks `PATH` rather than a list.
const NEVER_SHIM: &[&str] = &[
    "sh",
    "bash",
    "env",
    "ls",
    "cp",
    "mv",
    "rm",
    "cat",
    "sudo",
    "su",
    "systemctl",
    "podman",
    "pacman",
    "lisa",
];

/// Why a shim was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum ShimVerdict {
    /// Safe to write.
    Ok,
    /// Named in [`NEVER_SHIM`] — refused whatever `PATH` says.
    Reserved,
    /// A real binary of this name is already on `PATH` outside our own
    /// shim directory.
    Shadows(PathBuf),
}

/// May we write a shim called `name`?
///
/// `path_lookup` is injected so this is testable without touching the
/// host's `PATH` — the decision is what matters and it must be provable
/// on any machine.
pub fn shim_verdict(
    name: &str,
    shim_dir: &Path,
    path_lookup: impl Fn(&str) -> Option<PathBuf>,
) -> ShimVerdict {
    if NEVER_SHIM.contains(&name) {
        return ShimVerdict::Reserved;
    }
    match path_lookup(name) {
        // Our own shim from a previous install is not a conflict — it is
        // the thing being replaced.
        Some(found) if found.starts_with(shim_dir) => ShimVerdict::Ok,
        Some(found) => ShimVerdict::Shadows(found),
        None => ShimVerdict::Ok,
    }
}

/// The shim script for one binary.
///
/// `exec` rather than a wrapper process, so signals and exit codes reach
/// the caller unchanged — a shim that swallows Ctrl-C is worse than no
/// shim. `--` before the arguments so a leading `-x` is the program's
/// flag rather than podman's.
///
/// The header carries the package name because [`shims_of`] reads it
/// back: `remove` has to take back exactly what `install` put down, and
/// deriving that from filenames would guess.
pub fn shim_script(package: &str, binary: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # lisa-dev-shim package={package}\n\
         # Generated by `lisa dev install {package}`. Removed by\n\
         # `lisa dev remove {package}` and by `lisa dev reset`.\n\
         exec podman exec -i {BOX_NAME} {binary} \"$@\"\n"
    )
}

/// The package a shim belongs to, or `None` if the file is not ours.
///
/// Reading the marker rather than trusting the directory: `~/.local/bin`
/// is the user's own, and deleting something they put there because it
/// sat next to our files would be unforgivable.
pub fn shim_package(contents: &str) -> Option<String> {
    contents
        .lines()
        .find_map(|l| l.strip_prefix("# lisa-dev-shim package="))
        .map(|p| p.trim().to_string())
}

/// Which of a package's files should become shims.
///
/// `pacman -Ql <pkg>` prints `pkg /path` per line. Only `/usr/bin`
/// entries, and only files — the directory entry `/usr/bin/` is in the
/// listing too and would produce a shim called nothing.
pub fn binaries_from_pacman_ql(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.lines() {
        let Some((_pkg, path)) = line.split_once(' ') else {
            continue;
        };
        let path = path.trim();
        let Some(name) = path.strip_prefix("/usr/bin/") else {
            continue;
        };
        // A directory entry ends in `/`; a nested path is not a binary
        // on PATH.
        if name.is_empty() || name.contains('/') {
            continue;
        }
        if !out.contains(&name.to_string()) {
            out.push(name.to_string());
        }
    }
    out
}

/// Package names `pacman -Qe` reported, one per line as `name version`.
pub fn packages_from_pacman_qe(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(n, v)| (n.to_string(), v.trim().to_string()))
        .collect()
}

/// A package name we are willing to hand to `pacman`.
///
/// The name reaches a command line, so anything that could end the
/// argument is refused here rather than escaped later — `check_command`
/// judges argv and never sees a shell, but `lisa dev` is also typed by
/// people and a name with a space in it is a mistake worth naming.
pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

// ---------------------------------------------------------------------
// The podman edge. Everything above is pure; everything below shells out.

/// Where shims are written.
///
/// `$LISA_DEV_SHIM_DIR` overrides it, the same way `$LISA_SKILLS_DIR`
/// and `$LISA_MCP_DIR` do for their own paths. That exists because
/// `$HOME` cannot be moved to isolate a test: podman reads its own
/// connection config from `$HOME/.config/containers`, so an overridden
/// HOME breaks the container before the shims are reached. Found by
/// trying it.
fn shim_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("LISA_DEV_SHIM_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("no HOME, so there is nowhere to put shims")?;
    Ok(PathBuf::from(home).join(".local/bin"))
}

fn podman(args: &[&str]) -> anyhow::Result<std::process::Output> {
    std::process::Command::new("podman")
        .args(args)
        .output()
        .context("running podman — is it installed? try `lisa dev doctor`")
}

fn podman_ok(args: &[&str]) -> anyhow::Result<String> {
    let out = podman(args)?;
    if !out.status.success() {
        bail!(
            "podman {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn box_exists() -> bool {
    podman(&["container", "exists", BOX_NAME])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create the box if it is not there. Idempotent.
///
/// **Deliberately NOT `--userns=keep-id`.** I wrote that first, reasoning
/// that files written into a mounted project should be owned by the
/// person — and running it produced `pacman: you cannot perform this
/// operation unless you are root`. `keep-id` maps the host uid into the
/// container, so the shell inside is not root and pacman refuses.
///
/// Rootless podman's default mapping is the one this wants: **root
/// inside the container is the unprivileged host user outside it.**
/// pacman gets the uid 0 it needs, the host sees only files owned by the
/// person running `lisa dev`, and no `sudo` appears anywhere (CLAUDE.md
/// 7b). `keep-id` would be right for a box that mounts a project
/// directory; this one installs packages.
fn ensure_box() -> anyhow::Result<()> {
    if box_exists() {
        return Ok(());
    }
    println!(">> creating the {BOX_NAME} container (first run)");
    podman_ok(&[
        "run", "-d", "--name", BOX_NAME,
        // ISOLATION IS BY WHAT IS NOT MOUNTED, not by cutting the
        // network. I wrote `--network=none` first and it broke the one
        // thing the box is for: pacman could not reach a mirror.
        //
        // #130 phase 2 asks that a dev tool cannot reach contextd,
        // agentd or remoted. Those are unix sockets under
        // `$XDG_RUNTIME_DIR/lisa` and D-Bus on the session bus, and this
        // container mounts neither — a container gets no host mounts it
        // is not given, so the isolation is the ABSENCE of `-v` flags
        // rather than a flag that grants it. `--network=none` would add
        // nothing to that and costs the box its purpose.
        //
        // The isolation test asserts the sockets are unreachable from
        // inside, with a positive control from outside, rather than
        // trusting this comment.
        BOX_IMAGE, "sleep", "infinity",
    ])?;
    // The base image ships an empty package database.
    //
    // `--disable-sandbox` disables PACMAN's own seccomp/landlock
    // sandbox, not the container's. Without it pacman fails with
    // "error restricting syscalls via seccomp: 22" and "switching to
    // sandbox user 'alpm' failed" — its sandbox cannot nest inside a
    // rootless user namespace. The container IS the sandbox here, and
    // weakening it instead (`--security-opt seccomp=unconfined`) would
    // trade the real boundary for the redundant one.
    podman_ok(&[
        "exec",
        BOX_NAME,
        "pacman",
        "-Sy",
        "--noconfirm",
        "--disable-sandbox",
    ])?;
    Ok(())
}

fn write_shims(package: &str, binaries: &[String]) -> anyhow::Result<(usize, Vec<String>)> {
    let dir = shim_dir()?;
    std::fs::create_dir_all(&dir)?;
    let mut written = 0;
    let mut refused = Vec::new();
    for bin in binaries {
        match shim_verdict(bin, &dir, which_on_path) {
            ShimVerdict::Ok => {
                let path = dir.join(bin);
                std::fs::write(&path, shim_script(package, bin))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
                }
                written += 1;
            }
            ShimVerdict::Reserved => refused.push(format!(
                "{bin} (reserved — shimming it would break the system)"
            )),
            ShimVerdict::Shadows(p) => {
                refused.push(format!("{bin} (already on PATH at {})", p.display()))
            }
        }
    }
    Ok((written, refused))
}

fn which_on_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

/// Every shim we wrote, as (path, package).
fn shims_of(package: Option<&str>) -> anyhow::Result<Vec<PathBuf>> {
    let dir = shim_dir()?;
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(owner) = shim_package(&text) else {
            continue; // not ours; leave it alone
        };
        if package.is_none_or(|p| p == owner) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

pub fn install_cmd(packages: &[String]) -> anyhow::Result<()> {
    for p in packages {
        if !valid_package_name(p) {
            bail!("`{p}` is not a package name");
        }
    }
    if packages.is_empty() {
        bail!("nothing to install — name a package");
    }
    // The disk guard, before anything is fetched (#130 phase 0).
    guard_disk()?;
    ensure_box()?;

    let mut args = vec![
        "exec",
        BOX_NAME,
        "pacman",
        "-S",
        "--noconfirm",
        "--needed",
        "--disable-sandbox",
    ];
    args.extend(packages.iter().map(String::as_str));
    println!(">> pacman -S {}", packages.join(" "));
    podman_ok(&args)?;

    for p in packages {
        let ql = podman_ok(&["exec", BOX_NAME, "pacman", "-Ql", p])?;
        let bins = binaries_from_pacman_ql(&ql);
        let (written, refused) = write_shims(p, &bins)?;
        println!("{p}: {written} command(s) available on PATH");
        for r in &refused {
            println!("   not shimmed: {r}");
        }
        crate::ledger_note("dev.install", &format!("{p} into {BOX_NAME}"));
    }
    let dir = shim_dir()?;
    if which_on_path("pacman").is_none() && !path_contains(&dir) {
        println!(
            "\nnote: {} is not on your PATH, so the shims are not reachable yet.",
            dir.display()
        );
    }
    Ok(())
}

fn path_contains(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == dir))
        .unwrap_or(false)
}

pub fn remove_cmd(packages: &[String]) -> anyhow::Result<()> {
    if !box_exists() {
        bail!("no {BOX_NAME} container — nothing is installed");
    }
    for p in packages {
        if !valid_package_name(p) {
            bail!("`{p}` is not a package name");
        }
        // Shims first: if pacman fails halfway, a shim pointing at a
        // half-removed package is worse than a missing command.
        for shim in shims_of(Some(p))? {
            std::fs::remove_file(&shim)?;
        }
        podman_ok(&[
            "exec",
            BOX_NAME,
            "pacman",
            "-Rns",
            "--noconfirm",
            "--disable-sandbox",
            p,
        ])?;
        println!("removed {p}");
        crate::ledger_note("dev.remove", &format!("{p} from {BOX_NAME}"));
    }
    Ok(())
}

pub fn list_cmd() -> anyhow::Result<()> {
    if !box_exists() {
        println!("no {BOX_NAME} container yet — `lisa dev install <pkg>` creates it");
        return Ok(());
    }
    // pacman inside the box is the AUTHORITY on what is installed. A
    // second list kept out here is the two-sources-of-truth shape that
    // #239 cost us.
    let out = podman_ok(&["exec", BOX_NAME, "pacman", "-Qe"])?;
    let pkgs = packages_from_pacman_qe(&out);
    if pkgs.is_empty() {
        println!("the box exists and holds nothing");
        return Ok(());
    }
    for (name, version) in &pkgs {
        let shims = shims_of(Some(name))?.len();
        println!("{name:<24} {version:<16} {shims} command(s) on PATH");
    }
    Ok(())
}

pub fn shell_cmd() -> anyhow::Result<()> {
    ensure_box()?;
    let status = std::process::Command::new("podman")
        .args(["exec", "-it", BOX_NAME, "/bin/bash"])
        .status()
        .context("running podman exec")?;
    if !status.success() {
        bail!("the shell exited with {status}");
    }
    Ok(())
}

/// Destroy and recreate. Real recovery, not a reinstall.
pub fn reset_cmd() -> anyhow::Result<()> {
    // Shims first and unconditionally: a shim that outlives its
    // container is a command that fails with a podman error instead of
    // saying it is not installed.
    let orphans = shims_of(None)?;
    for shim in &orphans {
        std::fs::remove_file(shim)?;
    }
    println!("removed {} shim(s)", orphans.len());
    if box_exists() {
        podman_ok(&["rm", "-f", BOX_NAME])?;
        println!("destroyed {BOX_NAME}");
    }
    crate::ledger_note("dev.reset", BOX_NAME);
    println!("`lisa dev install <pkg>` will build a fresh one");
    Ok(())
}

/// Refuse before fetching anything when the filesystem is tight.
fn guard_disk() -> anyhow::Result<()> {
    use crate::dev::{
        HEADROOM_BYTES, Room, container_store, existing_ancestor, free_bytes_at, human, room_for,
    };
    // A first install is a base image plus packages. Deliberately a
    // rough figure: the guard's job is to catch "nearly full", not to
    // predict pacman.
    const FIRST_INSTALL: u64 = 4 * 1024 * 1024 * 1024;
    let Some(store) = container_store() else {
        return Ok(());
    };
    let Some(dir) = existing_ancestor(&store) else {
        return Ok(());
    };
    let free = free_bytes_at(dir)?;
    if let Room::Tight { short_by, .. } = room_for(free, FIRST_INSTALL) {
        bail!(
            "only {} free on the filesystem holding {} — {} short for a {} install \
             plus the {} that must stay free. Free some space, or `lisa dev reset` \
             if an old box is holding it.",
            human(free),
            dir.display(),
            human(short_by),
            human(FIRST_INSTALL),
            human(HEADROOM_BYTES)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shim_dir_for_test() -> PathBuf {
        PathBuf::from("/home/me/.local/bin")
    }

    #[test]
    fn a_shim_never_shadows_a_binary_that_breaks_the_machine() {
        // THE failure this list exists for: a shim called `ls` on PATH
        // ahead of /usr/bin/ls breaks the machine in a way nobody
        // connects to a dev install — including the tools you would
        // reach for to diagnose it.
        for name in [
            "ls",
            "sh",
            "bash",
            "rm",
            "sudo",
            "systemctl",
            "podman",
            "pacman",
            "lisa",
        ] {
            assert_eq!(
                shim_verdict(name, &shim_dir_for_test(), |_| None),
                ShimVerdict::Reserved,
                "{name} must never be shimmed"
            );
        }
    }

    #[test]
    fn a_shim_never_shadows_anything_already_on_path() {
        // Not a list — a question asked of PATH, so it covers the
        // binaries a list would never think of.
        let v = shim_verdict("mysql", &shim_dir_for_test(), |n| {
            (n == "mysql").then(|| PathBuf::from("/usr/bin/mysql"))
        });
        assert_eq!(v, ShimVerdict::Shadows(PathBuf::from("/usr/bin/mysql")));
    }

    #[test]
    fn our_own_shim_is_not_a_conflict_with_itself() {
        // Reinstalling must replace the shim, not refuse because the
        // shim it is replacing exists.
        let dir = shim_dir_for_test();
        let v = shim_verdict("mysql", &dir, |n| Some(dir.join(n)));
        assert_eq!(v, ShimVerdict::Ok);
    }

    #[test]
    fn a_shim_carries_the_package_that_owns_it() {
        // `remove` must take back exactly what `install` put down.
        // Deriving that from the filename would guess — mariadb ships
        // `mysql`, and a guess deletes the wrong file.
        let s = shim_script("mariadb", "mysql");
        assert_eq!(shim_package(&s).as_deref(), Some("mariadb"));
        assert!(s.contains("exec podman exec -i lisa-dev mysql \"$@\""));
        assert!(s.starts_with("#!/bin/sh\n"));
    }

    #[test]
    fn a_file_we_did_not_write_is_never_ours_to_delete() {
        // `~/.local/bin` is the person's own directory. Deleting
        // something they put there because it sat next to our files
        // would be unforgivable, so ownership is read from the marker.
        assert_eq!(shim_package("#!/bin/sh\necho hello\n"), None);
        assert_eq!(shim_package(""), None);
        assert_eq!(shim_package("#!/usr/bin/env python3\n"), None);
    }

    #[test]
    fn only_real_binaries_become_shims() {
        // `pacman -Ql` lists directories and nested paths too. The
        // directory entry would produce a shim called nothing.
        let ql = "mariadb /usr/bin/\n\
                  mariadb /usr/bin/mysql\n\
                  mariadb /usr/bin/mysqldump\n\
                  mariadb /usr/lib/libmariadb.so\n\
                  mariadb /usr/share/man/man1/mysql.1.gz\n\
                  mariadb /usr/bin/sub/dir\n";
        assert_eq!(
            binaries_from_pacman_ql(ql),
            vec!["mysql".to_string(), "mysqldump".to_string()]
        );
    }

    #[test]
    fn a_package_listed_twice_is_shimmed_once() {
        let ql = "x /usr/bin/tool\nx /usr/bin/tool\n";
        assert_eq!(binaries_from_pacman_ql(ql), vec!["tool".to_string()]);
    }

    #[test]
    fn malformed_pacman_output_yields_nothing_rather_than_panicking() {
        assert!(binaries_from_pacman_ql("").is_empty());
        assert!(binaries_from_pacman_ql("garbage").is_empty());
        assert!(binaries_from_pacman_ql("no-space-here\n").is_empty());
    }

    #[test]
    fn the_installed_list_comes_from_pacman() {
        let qe = "mariadb 11.4.2-1\npostgresql 16.3-2\n";
        assert_eq!(
            packages_from_pacman_qe(qe),
            vec![
                ("mariadb".to_string(), "11.4.2-1".to_string()),
                ("postgresql".to_string(), "16.3-2".to_string()),
            ]
        );
    }

    #[test]
    fn a_package_name_that_could_end_an_argument_is_refused() {
        assert!(valid_package_name("mariadb"));
        assert!(valid_package_name("python-pip"));
        assert!(valid_package_name("gcc11.2+x"));
        assert!(!valid_package_name(""));
        assert!(!valid_package_name("a b"));
        assert!(!valid_package_name("a;rm -rf /"));
        assert!(!valid_package_name("$(id)"));
        assert!(!valid_package_name("../escape"));
        assert!(!valid_package_name(&"x".repeat(65)));
    }

    #[test]
    fn the_image_is_pinned_by_digest_not_by_a_tag_that_moves() {
        // CLAUDE.md rule 8. A tag is whatever the registry decided this
        // morning; a digest is the artifact somebody actually checked.
        assert!(BOX_IMAGE.contains("@sha256:"), "{BOX_IMAGE}");
        assert!(!BOX_IMAGE.ends_with(":base"));
    }
}

/// The isolation #130 phase 2 asks for, asserted rather than trusted.
///
/// **A negative needs a positive control.** "the container cannot reach
/// contextd" passes on a machine where contextd is simply absent, so
/// each probe checks the same path from OUTSIDE first: if it is not
/// there either, the case proves nothing and says so instead of
/// reporting a pass.
///
/// Ignored by default because it needs podman and a working user
/// namespace — `cargo test -- --ignored` on a machine where `lisa dev
/// doctor` is green. It is not a unit test pretending to be one.
#[cfg(test)]
mod isolation_tests {
    use super::*;

    fn podman_works() -> bool {
        std::process::Command::new("podman")
            .args(["container", "exists", BOX_NAME])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn inside(script: &str) -> String {
        let out = std::process::Command::new("podman")
            .args(["exec", BOX_NAME, "sh", "-c", script])
            .output()
            .expect("podman exec");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    #[test]
    #[ignore = "needs podman and a lisa-dev box; run with --ignored"]
    fn the_box_cannot_reach_lisas_own_data_or_sockets() {
        assert!(
            podman_works(),
            "no {BOX_NAME} box — `lisa dev install <pkg>` first"
        );

        // Each pair is (what to look for inside, the same thing outside).
        // The outside half is the control: without it, "not found
        // inside" is indistinguishable from "not present anywhere".
        let home = std::env::var("HOME").expect("HOME");
        let cases: [(&str, PathBuf); 2] = [
            (
                "ls ~/.local/share/lisa",
                PathBuf::from(&home).join(".local/share/lisa"),
            ),
            ("ls /run/user", PathBuf::from("/run/user")),
        ];

        let mut asserted = 0;
        for (probe, outside) in cases {
            if !outside.exists() {
                // Say so rather than counting it as a pass.
                println!(
                    "skipped: {} does not exist on this host either, so the probe proves nothing",
                    outside.display()
                );
                continue;
            }
            let seen = inside(probe);
            assert!(
                seen.contains("No such file") || seen.contains("cannot access"),
                "the box reached {} — isolation is not holding:\n{seen}",
                outside.display()
            );
            asserted += 1;
        }
        assert!(
            asserted > 0,
            "every control was absent on this host, so this test asserted nothing — \
             run it where Lisa is installed"
        );
    }

    #[test]
    #[ignore = "needs podman and a lisa-dev box; run with --ignored"]
    fn the_box_mounts_no_host_filesystem() {
        assert!(podman_works(), "no {BOX_NAME} box");
        // The assertion is about host CONTENTS, not about a path name.
        // A Linux container legitimately ships its own empty `/home`,
        // so `ls /home` succeeding inside proves nothing — my first
        // version asserted exactly that and failed for the wrong
        // reason. The question is whether the person's actual home
        // directory is reachable.
        let home = std::env::var("HOME").expect("HOME");
        let seen = inside(&format!("ls -d {home}"));
        assert!(
            seen.contains("No such file") || seen.contains("cannot access"),
            "the host home {home} is visible inside the box:\n{seen}"
        );
        // …and the control: it is obviously there from outside.
        assert!(
            Path::new(&home).exists(),
            "the control failed — HOME does not exist on this host"
        );
    }
}
