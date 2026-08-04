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

    #[test]
    fn catalog_ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (id, _) in CATALOG {
            assert!(seen.insert(id), "duplicate catalog entry `{id}`");
        }
    }
}
