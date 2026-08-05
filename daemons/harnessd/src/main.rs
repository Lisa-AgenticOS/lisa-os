//! lisa-harnessd — the one agent loop, as a service (ADR-0025, #59).
//!
//! Every surface that wants an assistant — the Assistant window, the
//! overlay, `lisa assist`, and later a schedule or an event trigger
//! (ADR-0036) — drives THIS, rather than each growing a loop of its own.
//! Two loops would mean two answers to "which tools may the model use"
//! and two places for the provenance rules to drift apart; the second
//! copy is always the one that forgets.
//!
//! # Where this sits
//!
//! Three processes, three jobs, and none of them able to approve its own
//! work:
//!
//! - **harnessd** hosts the MODEL and runs the loop.
//! - **agentd** owns policy: tiers, provenance, the Ledger, the undo
//!   journal. It never sees a model.
//! - **lisa-consentd** raises the human's dialog and nothing else.
//!
//! That split is issue #145's whole point. Before it, the process
//! hosting the model also owned the consent surface, so a call it
//! originated came back to `Confirm` from the same peer that asked, and
//! the model approved itself. Keeping the model here — outside both the
//! policy engine and the dialog — is what makes the separation real
//! rather than a diagram.
//!
//! # Trust comes from the caller, never the message
//!
//! `Run` takes a trigger class, and it is NOT taken at face value
//! (ADR-0033, ADR-0036 §1). A client that could name its own trigger
//! could launder an event — attacker-supplied content — into the
//! `prompt` class that a human typed. So the class is *resolved* from
//! the caller's transport identity, and a caller may only ever narrow
//! its own trust, never widen it.
//!
//! No network access: the model endpoint is inferenced on loopback, and
//! egress belongs to lisa-remoted (CLAUDE.md rule 5).

mod caller;
mod dbus;
mod loop_runner;
mod memory;
mod skills;
mod workspace;

use std::sync::Arc;
use tracing::{info, warn};

/// This user's own Ledger. `STATE_DIRECTORY` wins when systemd sets it
/// (a user unit gets a per-user one); otherwise `$HOME`. Never the
/// shared system path — see the note in `main`.
fn ledger_path() -> std::path::PathBuf {
    if let Some(state) = std::env::var_os("STATE_DIRECTORY") {
        return std::path::PathBuf::from(state).join("ledger.db");
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".local/share/lisa/ledger.db")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // No ledger, no loop. The harness records every tool call before it
    // runs (#129), so a machine that cannot write the record must not
    // act — the same rule agentd applies to the bus.
    //
    // PER-USER, explicitly. `Ledger::default_path()` prefers the shared
    // /var/lib/lisa when it exists, which is right for a system daemon
    // and wrong for this one: harnessd runs one instance per logged-in
    // user, and its Ledger holds what THAT person asked and what was
    // done about it. Two users sharing one file would mean each reading
    // the other's assistant history — a privacy failure, not an
    // inconvenience. (On the reference machine /var/lib/lisa is
    // root-owned and unwritable anyway, so the shared path would simply
    // have refused to open; that is luck, not design.)
    let path = ledger_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ledger = Arc::new(lisa_ledger::Ledger::open(&path)?);
    info!("ledger open at {}", path.display());

    let conn = dbus::serve(ledger).await?;
    info!("serving dev.lisaos.Harness1 on the session bus");
    let _ = conn;

    tokio::signal::ctrl_c().await?;
    warn!("interrupted — exiting");
    Ok(())
}
