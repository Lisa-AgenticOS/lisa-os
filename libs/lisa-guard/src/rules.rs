//! The destructive-action rules (ADR-0029 §2, §4).
//!
//! Each rule reads one [`Invocation`] — a single program with its
//! arguments, already unwrapped from `sudo`/`env`/pipes by the caller —
//! and returns a [`Verdict`]. Rules are pure and order-independent; the
//! caller takes the worst answer.
//!
//! `Deny` is reserved for actions whose damage a confirmation dialog
//! cannot make acceptable: destroying the system or a whole home
//! directory, writing raw devices, escalating privilege, and erasing the
//! audit trail. Everything else that merely deserves a second look —
//! egress, package changes, history rewrites, power state — is `Confirm`,
//! because a `Deny` the user routinely needs to work around is a `Deny`
//! they will learn to disable.

use crate::Verdict;
use crate::shell::Invocation;

/// Paths where recursive deletion or permission changes are never an
/// agent's business, and the roots beneath which one more component is
/// still too broad (`/home/someone`, `/usr/lib`).
const SYSTEM_ROOTS: &[&str] = &[
    "/", "/bin", "/boot", "/dev", "/efi", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc",
    "/root", "/run", "/sbin", "/srv", "/sys", "/usr", "/var",
];

/// Programs that write block devices directly. `dd` is handled separately
/// because it only matters with `of=`.
const DEVICE_WRITERS: &[&str] = &[
    "mkfs",
    "wipefs",
    "blkdiscard",
    "sgdisk",
    "fdisk",
    "parted",
    "badblocks",
    "cfdisk",
];

pub(crate) fn scan(inv: &Invocation) -> Verdict {
    let mut verdict = Verdict::Allow;
    for rule in [
        privilege_escalation,
        recursive_delete,
        find_actions,
        device_write,
        recursive_permissions,
        system_path_write,
        audit_erasure,
        power_state,
        package_mutation,
        network_egress,
        version_control,
    ] {
        verdict = verdict.worst(rule(inv));
        // Deny is terminal; nothing later can make the answer worse and
        // the first reason is the most specific one.
        if verdict.is_denied() {
            break;
        }
    }
    verdict
}

/// An agent never escalates. If a task genuinely needs root, that is a
/// decision for the human at their own prompt, not a step in a plan.
fn privilege_escalation(inv: &Invocation) -> Verdict {
    if inv.escalated {
        return Verdict::deny(
            "escalate.privilege",
            "runs as root — an agent never escalates privilege on its own",
        );
    }
    Verdict::Allow
}

fn recursive_delete(inv: &Invocation) -> Verdict {
    if inv.program != "rm" && inv.program != "rmdir" && inv.program != "shred" {
        // `find -delete` is refused at the argument-policy layer, which
        // catches it before the target is even known.
        return Verdict::Allow;
    }

    // Explicitly turning off the one safety coreutils ships is never an
    // accident, whatever the target turns out to be.
    if inv.has_flag("--no-preserve-root") {
        return Verdict::deny(
            "rm.no_preserve_root",
            "disables the root-deletion guard built into `rm`",
        );
    }

    let recursive = inv.has_any_short_flag(&['r', 'R']) || inv.has_flag("--recursive");
    let forced = inv.has_any_short_flag(&['f']) || inv.has_flag("--force");
    if !recursive && !forced && inv.program == "rm" {
        return Verdict::Allow;
    }

    for target in inv.operands() {
        if is_system_target(target) {
            return Verdict::deny(
                "rm.system_path",
                format!("recursively deletes `{target}`, which is a system or home root"),
            );
        }
        // Depth-independent (review round 2, #73): `/etc/systemd/system`
        // is three levels down and still the OS.
        if is_under_system_root(target) {
            return Verdict::deny(
                "rm.system_path",
                format!("recursively deletes `{target}`, which belongs to the OS image"),
            );
        }
    }

    // Targets arriving over a pipe (`find … | xargs rm -rf`) are unknown
    // at this point, so the guard cannot clear them.
    if recursive && forced && inv.operands().next().is_none() {
        return Verdict::confirm(
            "rm.piped_targets",
            "recursive force-delete whose targets come from a pipe — the guard cannot see what it removes",
        );
    }
    Verdict::Allow
}

/// `find`'s action half. The forge harness refuses these predicates
/// outright at the argv layer, but a *suggestion* is a different context:
/// `find . -name '*.tmp' -exec rm {} \;` is a thing people legitimately
/// want typed for them, so it asks rather than refuses — unless the
/// search root makes the blast radius the whole system.
fn find_actions(inv: &Invocation) -> Verdict {
    if inv.program != "find" {
        return Verdict::Allow;
    }
    const ACTIONS: &[&str] = &["-delete", "-exec", "-execdir", "-ok", "-okdir"];
    let Some(action) = inv.args.iter().find(|a| ACTIONS.contains(&a.as_str())) else {
        return Verdict::Allow;
    };
    for target in inv.operands() {
        if is_system_target(target) {
            return Verdict::deny(
                "find.system_scope",
                format!("runs `{action}` across `{target}`, which is a system or home root"),
            );
        }
    }
    Verdict::confirm(
        "find.action",
        format!("`{action}` runs on every match, and the match set is not visible here"),
    )
}

fn device_write(inv: &Invocation) -> Verdict {
    if inv.program == "dd" {
        if let Some(target) = dd_output(inv)
            && normalize_target(&target).starts_with("/dev/")
        {
            return Verdict::deny(
                "disk.raw_write",
                format!("writes raw bytes to the block device `{target}`"),
            );
        }
        return Verdict::Allow;
    }

    let writes_devices = DEVICE_WRITERS.contains(&inv.program.as_str())
        || inv.program.starts_with("mkfs.")
        || inv.program.starts_with("mke2fs");
    if writes_devices {
        return Verdict::deny(
            "disk.raw_write",
            format!("`{}` reformats or repartitions a block device", inv.program),
        );
    }

    if inv.program == "shred" && inv.operands().any(|t| t.starts_with("/dev/")) {
        return Verdict::deny("disk.raw_write", "shreds a block device");
    }
    Verdict::Allow
}

fn recursive_permissions(inv: &Invocation) -> Verdict {
    if inv.program != "chmod" && inv.program != "chown" && inv.program != "chgrp" {
        return Verdict::Allow;
    }
    if !inv.has_any_short_flag(&['R']) && !inv.has_flag("--recursive") {
        return Verdict::Allow;
    }
    for target in inv.operands() {
        if is_system_target(target) || is_under_system_root(target) {
            return Verdict::deny(
                "perm.system_path",
                format!(
                    "`{}` rewrites ownership or permissions recursively across `{target}`",
                    inv.program
                ),
            );
        }
    }
    Verdict::Allow
}

/// Writing into the OS itself. On Lisa this is doubly wrong: the root
/// filesystem is one half of an A/B pair and is replaced wholesale by
/// `lisa update`, so an edit here is both dangerous and futile.
fn system_path_write(inv: &Invocation) -> Verdict {
    /// Programs whose *last* operand is the destination — the earlier
    /// ones are sources, and reading `/etc/os-release` is ordinary work
    /// (review round 2, #75).
    const DESTINATION_IS_LAST: &[&str] = &["cp", "mv", "install", "ln"];
    /// Programs where every operand is written.
    const ALL_OPERANDS_WRITTEN: &[&str] = &["tee", "truncate", "sed"];

    let targets: Vec<String> = if DESTINATION_IS_LAST.contains(&inv.program.as_str()) {
        inv.operands()
            .last()
            .map(str::to_string)
            .into_iter()
            .collect()
    } else if ALL_OPERANDS_WRITTEN.contains(&inv.program.as_str()) {
        // `sed` only writes with -i; without it this is a read.
        if inv.program == "sed" && !inv.has_any_short_flag(&['i']) && !inv.has_flag("--in-place") {
            return Verdict::Allow;
        }
        inv.operands().map(str::to_string).collect()
    } else if inv.program == "dd" {
        // `dd of=/etc/passwd` never reaches the device rule (#69).
        dd_output(inv).into_iter().collect()
    } else {
        return Verdict::Allow;
    };

    for target in targets {
        if is_under_system_root(&target) {
            return Verdict::deny(
                "fs.system_write",
                format!("writes into `{target}`, which belongs to the OS image"),
            );
        }
    }
    Verdict::Allow
}

/// `dd`'s output file, from its `of=` operand.
fn dd_output(inv: &Invocation) -> Option<String> {
    inv.args
        .iter()
        .find_map(|a| a.strip_prefix("of=").map(str::to_string))
}

/// Erasing the record of what happened. Lisa's Ledger is append-only by
/// design; a command that clears shell history or vacuums the journal is
/// covering tracks, and there is no benign version of it in an agent
/// plan.
fn audit_erasure(inv: &Invocation) -> Verdict {
    if inv.program == "history" && inv.has_any_short_flag(&['c', 'w']) {
        return Verdict::deny("audit.erase", "clears the shell history");
    }
    if inv.program == "journalctl" && inv.args.iter().any(|a| a.starts_with("--vacuum-")) {
        return Verdict::deny("audit.erase", "vacuums the systemd journal");
    }
    if inv
        .operands()
        .any(|t| t.starts_with("/var/log") || t.ends_with("_history") || t.ends_with(".ledger"))
        && matches!(inv.program.as_str(), "rm" | "shred" | "truncate" | "tee")
    {
        return Verdict::deny("audit.erase", "destroys log or ledger files");
    }
    Verdict::Allow
}

fn power_state(inv: &Invocation) -> Verdict {
    const VERBS: &[&str] = &["reboot", "poweroff", "halt", "shutdown", "kexec"];
    let direct = VERBS.contains(&inv.program.as_str());
    let via_systemctl = inv.program == "systemctl" && inv.operands().any(|a| VERBS.contains(&a));
    if direct || via_systemctl {
        return Verdict::confirm(
            "power.state",
            "changes the machine's power state — in the middle of an update this can brick it",
        );
    }
    Verdict::Allow
}

fn package_mutation(inv: &Invocation) -> Verdict {
    let mutates = match inv.program.as_str() {
        "pacman" => inv
            .args
            .iter()
            .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains(['S', 'R', 'U'])),
        "apt" | "apt-get" | "dnf" | "yum" | "zypper" => inv
            .operands()
            .any(|a| matches!(a, "install" | "remove" | "purge" | "autoremove" | "upgrade")),
        "npm" | "pnpm" | "yarn" => inv
            .operands()
            .any(|a| matches!(a, "install" | "i" | "add" | "remove" | "uninstall")),
        "pip" | "pip3" => inv.operands().any(|a| matches!(a, "install" | "uninstall")),
        "cargo" => inv.operands().any(|a| matches!(a, "install" | "uninstall")),
        _ => false,
    };
    if mutates {
        return Verdict::confirm(
            "pkg.mutate",
            format!("changes installed software via `{}`", inv.program),
        );
    }
    Verdict::Allow
}

/// Egress is architecture (CLAUDE.md rule 5): anything that leaves the
/// machine is surfaced, never silent.
fn network_egress(inv: &Invocation) -> Verdict {
    const FETCHERS: &[&str] = &["curl", "wget", "nc", "ncat", "ssh", "scp", "rsync", "ftp"];
    if FETCHERS.contains(&inv.program.as_str()) {
        return Verdict::confirm(
            "net.egress",
            format!("`{}` sends or fetches data over the network", inv.program),
        );
    }
    Verdict::Allow
}

/// Losing uncommitted work is recoverable in principle and catastrophic
/// in practice, so these ask rather than refuse.
fn version_control(inv: &Invocation) -> Verdict {
    if inv.program != "git" {
        return Verdict::Allow;
    }
    let sub = inv.operands().next().unwrap_or("");
    let risky = match sub {
        "reset" => inv.has_flag("--hard"),
        "clean" => inv.has_any_short_flag(&['f']) || inv.has_flag("--force"),
        "push" => inv.has_flag("--force") || inv.has_any_short_flag(&['f']),
        "branch" => inv.args.iter().any(|a| a.as_str() == "-D"),
        "checkout" | "restore" => inv.has_flag("--force") || inv.has_any_short_flag(&['f']),
        _ => false,
    };
    if risky {
        return Verdict::confirm(
            "git.destructive",
            format!("`git {sub}` here discards work that is not committed anywhere else"),
        );
    }
    Verdict::Allow
}

/// Whether a deletion target is the system, a whole home, or broad enough
/// that one more component would not save it.
fn is_system_target(target: &str) -> bool {
    let t = normalize_target(target);
    if t == "/" {
        return true; // `/`, `//`, `/*`, `/.`, `/usr/..` …
    }
    if t == "~" || t == "$HOME" || t == "${HOME}" {
        return true;
    }
    if !t.starts_with('/') {
        return false; // relative — inside the working directory
    }
    if SYSTEM_ROOTS.contains(&t.as_str()) {
        return true;
    }
    // `/home/someone`, `/usr/lib`: two components is still whole-subtree
    // territory for anything rooted in the system list.
    let depth = t.split('/').filter(|s| !s.is_empty()).count();
    depth <= 2 && SYSTEM_ROOTS.contains(&format!("/{}", t.split('/').nth(1).unwrap_or("")).as_str())
}

/// Whether a write target lands inside the OS image (stricter than
/// `is_system_target`: any depth counts, because writing
/// `/etc/systemd/system/x.service` is exactly the case to stop).
fn is_under_system_root(target: &str) -> bool {
    const IMAGE_ROOTS: &[&str] = &[
        "/etc", "/usr", "/boot", "/efi", "/bin", "/sbin", "/lib", "/lib64", "/sys", "/proc",
        "/dev", "/run",
    ];
    let t = normalize_target(target);
    IMAGE_ROOTS
        .iter()
        .any(|r| t == *r || t.starts_with(&format!("{r}/")))
}

/// Collapse the spellings of one path into a single canonical form, so a
/// rule written against `/etc` also catches `/etc/`, `//etc`, `/./etc`,
/// `/etc/*` and `/usr/../etc`.
///
/// Purely lexical — it never touches the filesystem, because the target
/// may not exist and the rule holds either way. Review round 1 (#62)
/// found the previous suffix-stripping version blind to `.`, `..` and
/// doubled separators.
pub(crate) fn normalize_target(target: &str) -> String {
    let trimmed = target.trim().trim_matches(['"', '\'']);
    if trimmed == "~" || trimmed == "$HOME" || trimmed == "${HOME}" {
        return trimmed.to_string();
    }
    let absolute = trimmed.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in trimmed.split('/') {
        match segment {
            // Empty (`//`, trailing `/`), `.`, and a bare `*` glob all
            // name the directory they sit in.
            "" | "." | "*" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if absolute {
        format!("/{}", parts.join("/"))
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(program: &str, args: &[&str]) -> Invocation {
        Invocation {
            escalated: false,
            program: program.into(),
            args: args.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn root_inv(program: &str, args: &[&str]) -> Invocation {
        Invocation {
            escalated: true,
            ..inv(program, args)
        }
    }

    #[test]
    fn root_deletion_is_denied_however_it_is_spelled() {
        for target in [
            "/",
            "/*",
            "/etc",
            "/usr/lib",
            "/home",
            "/home/lisa",
            "~",
            "$HOME",
        ] {
            let v = scan(&inv("rm", &["-rf", target]));
            assert!(v.is_denied(), "`rm -rf {target}` returned {v}");
        }
        assert!(scan(&inv("rm", &["-rf", "--no-preserve-root", "/"])).is_denied());
        assert!(scan(&inv("rm", &["--recursive", "--force", "/etc"])).is_denied());
    }

    #[test]
    fn project_local_deletion_is_allowed() {
        assert!(scan(&inv("rm", &["-rf", "build"])).is_allowed());
        assert!(scan(&inv("rm", &["-rf", "./.dart_tool"])).is_allowed());
        assert!(scan(&inv("rm", &["stale.txt"])).is_allowed());
    }

    #[test]
    fn piped_delete_targets_ask_because_they_are_invisible() {
        let v = scan(&inv("rm", &["-rf"]));
        assert_eq!(v.rule(), Some("rm.piped_targets"));
        assert!(v.is_overridable());
    }

    #[test]
    fn devices_and_filesystems_are_denied() {
        assert!(scan(&inv("dd", &["if=/dev/zero", "of=/dev/sda"])).is_denied());
        assert!(scan(&inv("mkfs.btrfs", &["-f", "/dev/sda4"])).is_denied());
        assert!(scan(&inv("wipefs", &["-a", "/dev/nvme0n1"])).is_denied());
        // Writing an image file, not a device, is ordinary work.
        assert!(scan(&inv("dd", &["if=/dev/zero", "of=disk.img"])).is_allowed());
    }

    #[test]
    fn escalation_is_denied_outright() {
        assert!(scan(&root_inv("ls", &["-la"])).is_denied());
        assert_eq!(
            scan(&root_inv("pacman", &["-Rns", "gdm"])).rule(),
            Some("escalate.privilege")
        );
    }

    #[test]
    fn os_paths_are_write_protected() {
        assert!(scan(&inv("tee", &["/etc/passwd"])).is_denied());
        assert!(
            scan(&inv(
                "cp",
                &["evil.service", "/usr/lib/systemd/system/x.service"]
            ))
            .is_denied()
        );
        assert!(scan(&inv("sed", &["-i", "s/a/b/", "/etc/fstab"])).is_denied());
        // Reading is fine; only -i writes.
        assert!(scan(&inv("sed", &["s/a/b/", "/etc/fstab"])).is_allowed());
        assert!(scan(&inv("cp", &["a.dart", "lib/b.dart"])).is_allowed());
    }

    #[test]
    fn covering_tracks_is_denied() {
        assert!(scan(&inv("history", &["-c"])).is_denied());
        assert!(scan(&inv("journalctl", &["--vacuum-time=1s"])).is_denied());
        assert!(scan(&inv("rm", &["-f", "/var/log/lisa.ledger"])).is_denied());
    }

    #[test]
    fn reversible_but_expensive_actions_ask() {
        for (program, args) in [
            ("git", &["reset", "--hard"][..]),
            ("git", &["push", "--force"][..]),
            ("curl", &["https://example.com"][..]),
            ("systemctl", &["reboot"][..]),
            ("npm", &["install", "left-pad"][..]),
        ] {
            let v = scan(&inv(program, args));
            assert!(v.is_overridable(), "`{program} {args:?}` returned {v}");
        }
    }

    #[test]
    fn ordinary_work_is_untouched() {
        for (program, args) in [
            ("cargo", &["test"][..]),
            ("flutter", &["analyze", "--no-pub"][..]),
            ("git", &["status"][..]),
            ("git", &["commit", "-m", "fix"][..]),
            ("ls", &["-la", "lib"][..]),
            ("grep", &["-rn", "needle", "src"][..]),
        ] {
            let v = scan(&inv(program, args));
            assert!(v.is_allowed(), "`{program} {args:?}` returned {v}");
        }
    }
}
