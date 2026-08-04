//! `lisa assist` — the CLI's entry into the one agent loop (ADR-0025).
//!
//! The tool family itself lives in `libs/bus-tools`, shared with
//! `lisa-harnessd` so every surface offers the same tools under the same
//! rules. What stays here is the part that IS the CLI: a terminal
//! consent prompt for the shell tool, and the verb.

use bus_tools::AgentBusTools;
use forge_harness::ToolProvider;

/// `lisa assist "<what you want>"` — the multi-turn assistant on the
/// harness (ADR-0025), as opposed to `lisa do`, which routes a single
/// utterance to exactly one tool call and stops.
///
/// The difference is the loop: this can search, read the result, decide
/// it needs something else, and search again, which is the whole reason
/// ADR-0025 asks for one agent loop rather than a router per surface.
///
/// The verifier is `None` — there is no project to check. The loop
/// therefore ends when the model says it is done or the turn budget runs
/// out, and `project` only exists because the verifier would have used
/// it.
pub fn assist_cmd(
    utterance: &str,
    url: &str,
    model: Option<String>,
    max_turns: usize,
) -> anyhow::Result<()> {
    let Some(bus) = AgentBusTools::discover()? else {
        anyhow::bail!(
            "no session bus — `lisa assist` needs a desktop session with lisa-agentd running"
        );
    };
    if bus.is_empty() {
        anyhow::bail!(
            "lisa-agentd registered no read-tier tools, so there is nothing to work with. \
             `lisa tools` lists what it does know about."
        );
    }
    eprintln!("assist: {} read-tier bus tool(s)", bus.len());

    let mut backend = forge_harness::OpenAiBackend {
        url: url.to_string(),
        model,
    };
    // Same reasoning as the forge loop (#54, #129): a loop that acts on
    // your behalf is exactly the thing that must be on the record, and a
    // machine with an unwritable Ledger refuses to run rather than
    // acting off it.
    let ledger = std::sync::Arc::new(lisa_ledger::Ledger::open(
        lisa_ledger::Ledger::default_path(),
    )?);
    // Skills, offered the same way every other surface offers them
    // (#245): the catalog rides in the prompt, the bodies stay on disk
    // behind `read_skill`, and a skill's `tools:` line takes effect once
    // its body has been served. Before this, `lisa assist` took
    // `AgentConfig::skills`'s default of `Vec::new()` — so a skill could
    // not narrow anything here no matter what its frontmatter said.
    let skills = forge_harness::skills::load();
    let catalog = forge_harness::skills::catalog_lines(&skills);
    let config = forge_harness::AgentConfig {
        max_turns,
        verifier: forge_harness::Verifier::None,
        system_prompt: with_skill_catalog(forge_harness::agent::FORGE_SYSTEM_PROMPT, &catalog),
        skills: skills.clone(),
        ..forge_harness::AgentConfig::new(ledger)
    };
    let mut observe = |ev: forge_harness::AgentEvent| {
        use forge_harness::AgentEvent as E;
        match ev {
            E::Turn { n, max } => eprintln!("[turn {n}/{max}]"),
            E::Call { name, detail } => eprintln!("  · {name} {detail}"),
            E::CallResult { ok: false, chars } => {
                eprintln!("    ! tool error ({chars} chars)")
            }
            _ => {}
        }
    };
    // The shell tool (ADR-0036 §6): the long tail the typed tools will
    // never cover. Without it the model does what a person would — tell
    // you to paste the command yourself — which is the same action with
    // none of the checks and none of the Ledger.
    //
    // Its consent callback is a TERMINAL PROMPT, which is the honest
    // thing here and also the limit: `lisa assist` is a surface with a
    // human at a tty. A schedule or an event trigger has no tty, and
    // must not get this tool at all (ADR-0036 §6.4) — that is why the
    // callback is a constructor argument rather than a config flag.
    let project = std::env::current_dir()?;
    let shell = forge_harness::ShellTool::new(&project, |req| {
        eprintln!();
        eprintln!("  the assistant wants to run a shell command:");
        eprintln!("      {}", req.command);
        eprintln!("  in: {}", req.cwd.display());
        if let Some(reason) = req.verdict.reason() {
            eprintln!("  ! {reason}");
        }
        crate::agent::prompt_yes("  run it? [y/N] ").unwrap_or(false)
    })?;
    let skill_tools = forge_harness::SkillTools::new(skills);
    let mut providers: Vec<&dyn ToolProvider> = vec![&bus, &shell];
    if !skill_tools.is_empty() {
        providers.push(&skill_tools);
    }
    let report = forge_harness::forge_agent_with_tools(
        utterance,
        &project,
        &mut backend,
        &config,
        &providers,
        &mut observe,
    )?;
    println!("{}", report.summary);
    Ok(())
}

/// Append the skill catalog to a system prompt, or leave it alone.
///
/// An empty catalog stays out entirely: advertising `read_skill` with
/// nothing to read spends a turn on a tool that can only fail.
fn with_skill_catalog(prompt: &str, catalog: &str) -> String {
    if catalog.trim().is_empty() {
        return prompt.to_string();
    }
    format!(
        "{prompt}\n\nSkills — step-by-step workflows for specific jobs. Read the full \
         one with read_skill BEFORE starting a task it covers; these lines are only \
         names, and while a skill is loaded you may only use the tools it \
         declares:\n\n{catalog}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_catalog_leaves_the_prompt_untouched() {
        assert_eq!(with_skill_catalog("base", "  \n "), "base");
        assert!(!with_skill_catalog("base", "").contains("read_skill"));
    }

    #[test]
    fn a_catalog_is_appended_after_a_blank_line() {
        let p = with_skill_catalog("base.", "- demo: a demo");
        assert!(p.starts_with("base.\n\nSkills"), "{p}");
        assert!(p.ends_with("- demo: a demo"), "{p}");
    }
}
