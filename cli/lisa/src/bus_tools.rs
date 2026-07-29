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
    let config = forge_harness::AgentConfig {
        max_turns,
        verifier: forge_harness::Verifier::None,
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
    let providers: [&dyn ToolProvider; 2] = [&bus, &shell];
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
