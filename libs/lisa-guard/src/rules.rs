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
        runtime_evaluates_a_string,
        lisa_verb_that_changes_the_machine,
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

/// `lisa` is the machine's command centre; the loop gets the read-only
/// half.
///
/// CLAUDE.md rule 7 puts every user-facing verb under `lisa <verb>`, so
/// allowing the program wholesale would hand an unattended loop
/// `lisa update`, `lisa apps update`, `lisa install <disk>` and
/// `lisa guard allow` in one go. The last of those is the sharpest: a
/// loop that can relax its own guard is not guarded, which is ADR-0029's
/// first test.
///
/// So the allowance is an ALLOWLIST of verbs, not a denylist of
/// dangerous ones. A denylist would silently admit every verb added
/// after it was written — and `lisa` grows a verb most weeks.
fn lisa_verb_that_changes_the_machine(inv: &Invocation) -> Verdict {
    if inv.program != "lisa" {
        return Verdict::Allow;
    }
    // Read-only, and each earns its place: `dev check` is the single
    // authority on what a valid Lisa app is (ADR-0050 §4) — the loop
    // cannot check its own work without it; `tools` and `skills list`
    // are how it discovers what a manifest has to line up with.
    const READ_ONLY: &[&[&str]] = &[
        &["dev", "check"],
        &["tools"],
        &["skills", "list"],
        &["skills", "show"],
    ];
    let verbs: Vec<&str> = inv
        .args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let permitted = READ_ONLY
        .iter()
        .any(|allowed| verbs.len() >= allowed.len() && &verbs[..allowed.len()] == *allowed);
    if permitted {
        return Verdict::Allow;
    }
    let attempted = if verbs.is_empty() {
        "lisa".to_string()
    } else {
        format!("lisa {}", verbs.join(" "))
    };
    Verdict::deny(
        "lisa.write_verb",
        format!(
            "`{attempted}` is not one of the read-only verbs a loop may run \
             (dev check, tools, skills list|show). Everything else on `lisa` \
             changes the machine."
        ),
    )
}

/// A language runtime handed a STRING is a shell with a different name.
///
/// #269: the Forge writes GJS apps and must be able to run one, so
/// `gjs` and `node` are allowlisted. `gjs -c '<anything>'` is
/// `exec.shell` in a costume — it executes arbitrary code the model
/// composed, which is precisely what `exec.shell` exists to refuse. So
/// the runtimes are allowed to run a FILE and never an argument.
///
/// This lives in `rules::scan` rather than only in the argument policy
/// because there are two doors: `check_command` judges a typed tool
/// call, and `check_shell_line` judges an arbitrary shell line. A rule
/// that closed only one would leave the other open, which is how #218's
/// dispatcher bug reached three apps.
///
/// The preload flags are here for the same reason and are less obvious:
/// `node -r <module>` runs code BEFORE the entry point, so a refusal
/// aimed only at `--eval` would miss it entirely.
fn runtime_evaluates_a_string(inv: &Invocation) -> Verdict {
    const RUNTIMES: &[&str] = &["gjs", "node", "nodejs"];
    if !RUNTIMES.contains(&inv.program.as_str()) {
        return Verdict::Allow;
    }
    // Both spellings of every flag, and both the separate-value and
    // attached-value forms: `--eval x`, `--eval=x`.
    const EVAL_FLAGS: &[&str] = &[
        "-c",
        "--command", // gjs
        "-e",
        "--eval",
        "-p",
        "--print", // node
        "-r",
        "--require", // node: runs before the entry point
        "-I",
        "--include-path", // gjs: seeds the module search path
    ];
    for arg in &inv.args {
        let name = arg.split_once('=').map_or(arg.as_str(), |(k, _)| k);
        if EVAL_FLAGS.contains(&name) {
            return Verdict::deny(
                "exec.shell",
                format!(
                    "`{} {name}` evaluates code given as an argument — that is a shell \
                     with a different name. Run a file instead.",
                    inv.program
                ),
            );
        }
    }
    // …and the file it runs is not the system's. `check_command` already
    // confines operands to the working directory, but that layer knows
    // about a jail and `check_shell_line` does not — and a runtime
    // executing `/etc/profile` is running system code with the agent's
    // hands, which no Forge task needs. Executing is not reading: `cat
    // /etc/profile` stays allowed.
    for arg in &inv.args {
        if arg.starts_with('-') {
            continue;
        }
        if is_under_system_root(arg) || is_system_target(arg) {
            return Verdict::deny(
                "exec.shell",
                format!(
                    "`{} {arg}` executes a file that belongs to the system. A runtime \
                     may run the project's own code, not the machine's.",
                    inv.program
                ),
            );
        }
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

    // Setuid/setgid is a privilege grant, not a permission tweak, and it
    // needs no `-R` and no system path to matter: `chmod u+s /bin/sh` in
    // ANY writable directory manufactures a root shell (#123). The
    // earlier rule only looked at recursive changes, so this walked past
    // it untouched.
    if inv.program == "chmod" {
        for arg in inv.operands() {
            if is_setuid_mode(arg) {
                return Verdict::deny(
                    "perm.setuid",
                    format!(
                        "`chmod {arg}` grants setuid/setgid — a privilege escalation primitive"
                    ),
                );
            }
        }
    }

    // A NON-recursive chmod/chown on a system path is still a write to
    // the OS image. Requiring -R was an accident of how the rule was
    // first written (#123).
    if !inv.has_any_short_flag(&['R']) && !inv.has_flag("--recursive") {
        for target in inv.operands() {
            if is_system_target(target) || is_under_system_root(target) {
                return Verdict::deny(
                    "perm.system_path",
                    format!(
                        "`{}` rewrites ownership or permissions on `{target}`, which belongs to the OS image",
                        inv.program
                    ),
                );
            }
        }
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

    // `mv` is the exception in DESTINATION_IS_LAST: a move DESTROYS its
    // source, so `mv /etc /tmp/backup` removes /etc while writing only
    // to /tmp — the destination check saw nothing wrong and the source
    // was never looked at (#122). A copy leaves the original in place;
    // a move does not, which is why it cannot share `cp`'s treatment.
    if inv.program == "mv" {
        let operands: Vec<&str> = inv.operands().collect();
        // Everything but the last operand is a source being removed.
        for source in operands.iter().rev().skip(1) {
            if is_system_target(source) || is_under_system_root(source) {
                return Verdict::deny(
                    "fs.system_write",
                    format!("`mv` REMOVES `{source}`, which belongs to the OS image"),
                );
            }
        }
    }

    let targets: Vec<String> = if DESTINATION_IS_LAST.contains(&inv.program.as_str()) {
        // …unless `-t DIR` / `--target-directory=DIR` names it
        // explicitly. `operands()` filters the flag away, so the *source*
        // was being read as the destination (round 3, #84).
        match target_directory(inv) {
            Some(dir) => vec![dir],
            None => inv
                .operands()
                .last()
                .map(str::to_string)
                .into_iter()
                .collect(),
        }
    } else if ALL_OPERANDS_WRITTEN.contains(&inv.program.as_str()) {
        // `sed` only writes with -i — EXCEPT that the `w` flag inside a
        // script writes a file of its own choosing:
        //     sed "s/x/y/w /etc/passwd"
        //     sed "1w /etc/cron.d/pwn"
        // Only -i was modelled, so the script body was never read and
        // this walked straight past (#120).
        if inv.program == "sed" {
            if let Some(written) = sed_script_writes(inv)
                && is_under_system_root(&written)
            {
                return Verdict::deny(
                    "fs.system_write",
                    format!("`sed`'s w flag writes `{written}`, which belongs to the OS image"),
                );
            }
            if !inv.has_any_short_flag(&['i']) && !inv.has_flag("--in-place") {
                return Verdict::Allow;
            }
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

/// The file a `sed` script writes via its `w` flag, if any.
///
/// `w` takes the rest of the line as a filename, so the target is
/// everything after it — whitespace included, which is why this is not a
/// whitespace split.
fn sed_script_writes(inv: &Invocation) -> Option<String> {
    for arg in &inv.args {
        if arg.starts_with('-') {
            continue;
        }
        // `s/a/b/w PATH`, `s/a/b/gw PATH`, `1w PATH`, `$w PATH`.
        //
        // The `w` must be followed by WHITESPACE and then the path.
        // Matching any `w` before a `/` would flag `s/x/w/` — an
        // ordinary substitution replacing something with the letter w —
        // as writing to `/`. The corpus caught exactly that.
        let bytes = arg.as_bytes();
        for (idx, _) in arg.match_indices('w') {
            let Some(&next) = bytes.get(idx + 1) else {
                continue;
            };
            if !next.is_ascii_whitespace() {
                continue;
            }
            let target = arg[idx + 1..].trim();
            if target.starts_with('/') {
                return Some(target.to_string());
            }
        }
    }
    None
}

/// An explicit destination directory: `-t DIR`, `-tDIR`, or
/// `--target-directory[=DIR]`.
fn target_directory(inv: &Invocation) -> Option<String> {
    let mut want_value = false;
    for arg in &inv.args {
        if want_value {
            return Some(arg.clone());
        }
        if arg == "-t" || arg == "--target-directory" {
            want_value = true;
        } else if let Some(dir) = arg.strip_prefix("--target-directory=") {
            return Some(dir.to_string());
        } else if let Some(dir) = arg.strip_prefix("-t")
            && !dir.starts_with('-')
            && !dir.is_empty()
        {
            return Some(dir.to_string());
        }
    }
    None
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
/// Whether a chmod mode argument grants setuid or setgid.
///
/// Both spellings: symbolic (`u+s`, `g+s`, `a+s`) and octal with a
/// four-digit leading bit (`4755`, `2755`, `6755`). A mode is not a
/// path, so this never inspects operands that are targets.
fn is_setuid_mode(arg: &str) -> bool {
    // Symbolic: a `+s` anywhere in a who/op/perm clause.
    if arg.contains("+s") {
        return true;
    }
    // Octal: 4 digits where the leading one carries setuid(4)/setgid(2).
    if arg.len() == 4
        && arg.chars().all(|c| c.is_ascii_digit())
        && let Some(lead) = arg.chars().next().and_then(|c| c.to_digit(8))
    {
        return lead & 0b110 != 0;
    }
    false
}

pub(crate) fn is_system_target(target: &str) -> bool {
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
pub(crate) fn is_under_system_root(target: &str) -> bool {
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
