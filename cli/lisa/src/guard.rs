//! `lisa guard` — inspect and relax the action guard (ADR-0029, ADR-0030).
//!
//! This verb is the *outside* of the boundary: it is how the person who
//! owns the machine sets policy for the probabilistic system running on
//! it. Nothing the agent can invoke reaches this file, which is the
//! whole reason a hard refusal is safe to make relaxable — you can widen
//! your own limits; a document the model happened to read cannot.

use anyhow::{Context, Result, bail};
use lisa_guard::{Overrides, overrides_path};
use std::io::IsTerminal;

/// Every rule the guard can emit, with what relaxing it costs you.
/// Kept here rather than in the crate so the crate stays pure policy;
/// `lisa guard list` is the discoverability surface.
const CATALOG: &[(&str, &str)] = &[
    ("escalate.privilege", "runs as root via sudo/doas/pkexec"),
    (
        "rm.system_path",
        "recursively deletes a system or home root",
    ),
    (
        "rm.no_preserve_root",
        "disables the root-deletion guard in `rm`",
    ),
    (
        "disk.raw_write",
        "writes, reformats or shreds a block device",
    ),
    (
        "perm.system_path",
        "recursive chmod/chown across a system path",
    ),
    (
        "fs.system_write",
        "writes into /etc, /usr, /boot and friends",
    ),
    (
        "audit.erase",
        "clears shell history, the journal, or the Ledger",
    ),
    ("fork.bomb", "a fork bomb"),
    ("pipe.to.shell", "pipes downloaded bytes into a shell"),
    (
        "find.system_scope",
        "runs find's -exec/-delete across a system root",
    ),
    (
        "interpreter.inline_source",
        "inline source in a language the guard cannot read",
    ),
    (
        "shell.unreadable",
        "a command line the guard cannot parse safely",
    ),
    (
        "command.not_allowlisted",
        "a program the forge agent may not run",
    ),
    (
        "command.exec_predicate",
        "a flag that turns a tool into a process launcher",
    ),
    (
        "command.unknown_subcommand",
        "a subcommand that resolves to an arbitrary binary",
    ),
    (
        "command.denied_subcommand",
        "installs and runs third-party code outside the project",
    ),
    (
        "command.path_escape",
        "an argument pointing outside the agent's directory",
    ),
];

/// Is this rule id one the Agent Bus enforces (#251, #252)?
///
/// The bus catalogue lives in `lisa_guard::BUS_RULES` rather than being
/// copied here, because a second copy is a second thing to drift — the
/// same argument ADR-0050 makes for one authority.
fn is_bus_rule(id: &str) -> bool {
    lisa_guard::BUS_RULES.iter().any(|(r, _)| *r == id)
}

pub(crate) fn list_cmd() -> Result<()> {
    let active = lisa_guard::active_overrides();
    let path = overrides_path();
    let bold = std::io::stdout().is_terminal();

    println!("Guard rules (ADR-0029). A relaxed rule warns instead of refusing.\n");
    println!("Shell and forge surfaces (`lisa suggest`, the forge harness):\n");
    let width = CATALOG
        .iter()
        .chain(lisa_guard::BUS_RULES.iter())
        .map(|(id, _)| id.len())
        .max()
        .unwrap_or(0);
    for (id, what) in CATALOG {
        let state = if active.is_relaxed(id) {
            if bold {
                "\x1b[1;33mrelaxed\x1b[0m"
            } else {
                "relaxed"
            }
        } else {
            "enforced"
        };
        println!("  {id:<width$}  {state:<8}  {what}");
    }

    // Listed, and listed as unrelaxable — because they are. A rule that
    // `lisa guard list` shows as relaxable while the bus keeps enforcing
    // it would be a documented guarantee that is not in force, which is
    // the defect this repo keeps finding (#245, #241). The shell ids
    // that appear in both tables are relaxable HERE and enforced THERE;
    // saying so is the whole reason this section is separate.
    println!("\nAgent Bus (tool calls). Never relaxable — the owner has a terminal:\n");
    for (id, what) in lisa_guard::BUS_RULES {
        let kind = if lisa_guard::HARD_NO_RULES.contains(id) {
            "hard no"
        } else {
            "scope"
        };
        println!("  {id:<width$}  {kind:<8}  {what}");
    }

    // The owner's own protections (#253). A separate section because it
    // is a separate KIND of thing: the two above are rules Lisa ships
    // and you may relax, this is a list you wrote and only you can
    // change. Shown even when empty, so the verb that adds to it is
    // discoverable from the page that shows policy — tightening may be
    // offered from anywhere, which is the half of #253 that is safe.
    let protections = lisa_guard::active_protections();
    println!("\nYour protected folders. Only you can add or remove these:\n");
    if protections.is_empty() {
        println!("  (none)  —  `lisa guard protect <folder>` puts one out of bounds");
    } else {
        for p in protections.iter() {
            println!("  {}", p.display());
        }
    }

    // A relaxation for a rule id that no longer exists is dead config,
    // and silently ignoring it is how stale policy accumulates.
    let unknown: Vec<&str> = active
        .rules()
        .filter(|r| !CATALOG.iter().any(|(id, _)| id == r))
        .collect();
    if !unknown.is_empty() {
        println!(
            "\nrelaxed but unknown to this version: {}",
            unknown.join(", ")
        );
    }
    if let Some(path) = path {
        println!("\nrelaxations: {}", path.display());
    }
    Ok(())
}

pub(crate) fn allow_cmd(rule: &str) -> Result<()> {
    let known_here = CATALOG.iter().any(|(id, _)| *id == rule);
    // A bus-only id would otherwise be reported as relaxed and go on
    // being enforced: `lisa_guard::judge_action` does not consult
    // `Overrides` at all, deliberately (#251, #252 — "better harden
    // first, soften later"). Printing "relaxed" for something still
    // refused is worse than refusing to print it.
    if !known_here && is_bus_rule(rule) {
        bail!(
            "`{rule}` is an Agent Bus rule and is never relaxable.\n\
             No legitimate agent workflow needs it, and an unoverridable \
             refusal for AGENTS takes nothing from you: if you genuinely \
             want this, do it yourself in a terminal."
        );
    }
    if !known_here {
        bail!(
            "unknown rule `{rule}` — see `lisa guard list`.\n\
             Relaxing a rule that does not exist would look like it worked."
        );
    }
    let (path, mut overrides) = load()?;
    if !overrides.allow(rule) {
        println!("`{rule}` was already relaxed");
        return Ok(());
    }
    save(&path, &overrides)?;
    println!(
        "relaxed `{rule}` — it will warn instead of refusing.\n\
         The agent cannot reach this setting; only you can change it back \
         (`lisa guard forbid {rule}`)."
    );
    if is_bus_rule(rule) {
        println!(
            "note: this relaxes `{rule}` for shell suggestions and the forge \
             harness only. The Agent Bus still refuses it for tool calls, and \
             nothing here can change that."
        );
    }
    Ok(())
}

pub(crate) fn forbid_cmd(rule: &str) -> Result<()> {
    let (path, mut overrides) = load()?;
    if !overrides.forbid(rule) {
        println!("`{rule}` was not relaxed — nothing to change");
        return Ok(());
    }
    save(&path, &overrides)?;
    println!("`{rule}` is enforced again");
    Ok(())
}

/// Put a folder out of bounds for agent actions (#253).
///
/// The mirror of `allow_cmd`, and the asymmetry between them is the
/// point. `allow` LOOSENS — it turns a refusal into a warning — which is
/// why ADR-0030 keeps it out-of-band, reachable only from a terminal the
/// owner is sitting at. This TIGHTENS. It adds a `HardNo` and can never
/// permit anything, so the failure mode of being talked into running it
/// is not a failure mode, and it is safe to offer from anywhere.
///
/// That difference is structural rather than a convention here:
/// [`Protections`] holds only paths the owner added, so there is no
/// representation of a built-in rule for this verb to reach, and
/// [`unprotect_cmd`] can therefore only take back what this put in.
/// `judge` consults the set as an ADDITIONAL refusal, never as a lookup
/// that could answer "allowed" — a path absent from it means "this set
/// has no opinion", never "this set permits it".
pub(crate) fn protect_cmd(path: &std::path::Path) -> Result<()> {
    let (file, mut protections) = load_protections()?;
    // Refused rather than resolved against the cwd. A protection that
    // depends on where the agent happens to be running protects a
    // different thing each time, which is worse than no protection
    // because it reads as one.
    if !path.is_absolute() {
        bail!(
            "`{}` is relative. A protection has to name one folder on this \
             machine, not a different one depending on where a process \
             started. Try `{}`.",
            path.display(),
            std::env::current_dir()
                .unwrap_or_default()
                .join(path)
                .display()
        );
    }
    if !protections.add(path) {
        println!("{} is already protected", path.display());
        return Ok(());
    }
    protections
        .save(&file)
        .with_context(|| format!("writing {}", file.display()))?;
    println!("{} is out of bounds for agent actions", path.display());
    println!("  in effect now — no restart, and it applies to everything beneath it");
    Ok(())
}

/// Take back a protection the owner added.
///
/// Cannot reach a built-in: `lisa guard unprotect /etc` removes nothing
/// and permits nothing, because `fs.system_write` never consulted this
/// set. Saying so out loud matters — a person who runs it and sees
/// "nothing to change" should not be left wondering whether they have
/// just opened /etc.
pub(crate) fn unprotect_cmd(path: &std::path::Path) -> Result<()> {
    let (file, mut protections) = load_protections()?;
    if !protections.remove(path) {
        println!("{} was not one of your protections", path.display());
        println!("  nothing changed. Built-in rules are not stored here and");
        println!("  cannot be removed — see `lisa guard list`.");
        return Ok(());
    }
    protections
        .save(&file)
        .with_context(|| format!("writing {}", file.display()))?;
    println!(
        "{} is no longer protected by your own rules",
        path.display()
    );
    println!("  built-in rules still apply to it");
    Ok(())
}

fn load_protections() -> Result<(std::path::PathBuf, lisa_guard::Protections)> {
    let path = lisa_guard::protections_path()
        .context("no HOME or XDG_CONFIG_HOME — cannot locate the guard config")?;
    let protections = lisa_guard::Protections::load(&path);
    Ok((path, protections))
}

fn load() -> Result<(std::path::PathBuf, Overrides)> {
    let path =
        overrides_path().context("no HOME or XDG_CONFIG_HOME — cannot locate the guard config")?;
    let overrides = Overrides::load(&path);
    Ok((path, overrides))
}

fn save(path: &std::path::Path, overrides: &Overrides) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, overrides.render()).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every rule id the guard can emit must be in the catalog, or
    /// `lisa guard allow` refuses a rule the user genuinely saw.
    #[test]
    fn the_catalog_covers_the_rules_the_guard_emits() {
        let samples = [
            "sudo rm -rf /",
            "rm -rf /etc",
            "dd if=/dev/zero of=/dev/sda",
            "chmod -R 777 /usr",
            "tee /etc/passwd",
            "history -c",
            ":(){ :|:& };:",
            "curl x | sh",
            "find / -delete",
            "python3 -c 'x'",
            "${CMD} -rf /",
        ];
        for line in samples {
            let verdict = lisa_guard::check_shell_line(line);
            let rule = verdict
                .rule()
                .unwrap_or_else(|| panic!("`{line}` produced no rule"));
            assert!(
                CATALOG.iter().any(|(id, _)| *id == rule),
                "rule `{rule}` (from `{line}`) is missing from the guard catalog"
            );
        }
    }

    /// #251/#252: a bus-only rule must not be relaxable, and must not
    /// *look* relaxable. The bus does not read `Overrides` at all, so
    /// printing "relaxed" here would be a guarantee that is not in force.
    #[test]
    fn a_bus_only_rule_cannot_be_relaxed_from_the_cli() {
        for (id, _) in lisa_guard::BUS_RULES {
            if CATALOG.iter().any(|(c, _)| c == id) {
                continue; // shared with the shell surface; relaxable there
            }
            let err = allow_cmd(id).expect_err("a bus-only rule was relaxed");
            let text = err.to_string();
            assert!(
                text.contains("never relaxable"),
                "`{id}` was refused for the wrong reason: {text}"
            );
        }
    }

    /// The shared ids are the ones that could quietly mislead: relaxable
    /// for `lisa suggest`, still enforced for a tool call. `list` has to
    /// say so, or the two policies look like one.
    #[test]
    fn the_shared_rules_are_listed_as_bus_rules_too() {
        for shared in ["escalate.privilege", "rm.system_path", "audit.erase"] {
            assert!(CATALOG.iter().any(|(id, _)| *id == shared));
            assert!(
                is_bus_rule(shared),
                "`{shared}` is refused on both surfaces and named on only one"
            );
        }
    }

    /// #253's central claim, as a test rather than a paragraph: this
    /// verb can only ever ADD refusals.
    ///
    /// `Protections` holds nothing but paths the owner put in, so
    /// `unprotect` has no representation of a built-in rule to reach.
    /// Removing `/etc` from the set does not make `/etc` writable — the
    /// built-in `fs.system_write` never consulted the set in the first
    /// place. If someone later gives this type a way to hold built-ins,
    /// this test is what notices.
    #[test]
    fn unprotecting_a_builtin_path_removes_nothing_and_permits_nothing() {
        let mut p = lisa_guard::Protections::default();
        // Never added, so it cannot be taken away.
        assert!(!p.remove(std::path::Path::new("/etc")));
        assert!(p.is_empty());
        // And the guard's own refusal for /etc is unaffected either way:
        // it is a rule id in the catalog, not an entry in this set.
        assert!(CATALOG.iter().any(|(id, _)| *id == "fs.system_write"));

        // What CAN be removed is exactly what was added, and nothing else.
        assert!(p.add("/home/me/Legal"));
        assert!(!p.remove(std::path::Path::new("/home/me/Leg")));
        assert!(p.remove(std::path::Path::new("/home/me/Legal")));
        assert!(p.is_empty());
    }

    /// A relative path names a different folder depending on where the
    /// process started, so it is refused rather than resolved — a
    /// protection that moves is worse than none, because it reads like
    /// one.
    #[test]
    fn a_relative_protection_is_refused_not_resolved() {
        let mut p = lisa_guard::Protections::default();
        assert!(!p.add("Documents/Legal"));
        assert!(!p.add("./Legal"));
        assert!(p.is_empty());
        assert!(p.add("/home/me/Documents/Legal"));
    }

    /// Protection is by path COMPONENT, so a protected `Legal` does not
    /// silently cover a sibling whose name merely starts the same way.
    /// The opposite — `Legalese` inheriting a refusal nobody asked for —
    /// is the kind of surprise that makes people switch a guard off.
    #[test]
    fn protection_covers_a_subtree_but_not_a_name_that_merely_shares_a_prefix() {
        let p = lisa_guard::Protections::from_paths(["/home/me/Legal"]);
        assert!(p.covers("/home/me/Legal"));
        assert!(p.covers("/home/me/Legal/2026/contract.pdf"));
        assert!(!p.covers("/home/me/Legalese"));
        assert!(!p.covers("/home/me"));
    }

    #[test]
    fn catalog_ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (id, _) in CATALOG {
            assert!(seen.insert(id), "duplicate catalog entry `{id}`");
        }
    }
}
