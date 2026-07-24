//! lisa-contextd entrypoint — the context fabric's serving surface
//! (`docs/PLAN.md` §5.3). Opens the per-user store and the Ledger (no
//! ledger, no retrieval — dataflow rule 4) and owns
//! `dev.lisaos.Context1` on the session bus: scoped/hybrid search over
//! the user's index and per-app durable memory, every retrieval
//! ledgered before results return. No network access — ever
//! (CLAUDE.md rule 5): the store and ledger are local SQLite files and
//! the only surface is D-Bus; the hardened user unit
//! (`os/packages/lisa/lisa-contextd-user.service`) enforces it on the
//! image, and no dependency here may add it.

use clap::Parser;
use lisa_contextd::{ContextStore, dbus};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "lisa-contextd",
    about = "Lisa OS context fabric daemon (PLAN §5.3)"
)]
struct Args {
    /// Context store path; defaults to $LISA_CONTEXT_DB, else
    /// ~/.local/share/lisa/context/context.db (the CLI's resolution,
    /// so daemon and `lisa context` always see the same index).
    #[arg(long)]
    db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let db = args
        .db
        .or_else(|| std::env::var_os("LISA_CONTEXT_DB").map(PathBuf::from))
        .unwrap_or_else(ContextStore::default_path);
    let store = Arc::new(ContextStore::open(&db)?);
    info!(db = %db.display(), "context store open");

    // No ledger, no retrieval (dataflow rule 4): refuse to serve at
    // all if the audit log cannot be opened.
    let ledger_path = std::env::var_os("LISA_LEDGER_DB")
        .map(PathBuf::from)
        .unwrap_or_else(lisa_ledger::Ledger::default_path);
    let ledger = Arc::new(
        lisa_ledger::Ledger::open(&ledger_path)
            .map_err(|e| anyhow::anyhow!("cannot open ledger {}: {e}", ledger_path.display()))?,
    );
    info!(ledger = %ledger_path.display(), "ledger open (append-only)");

    let conn = dbus::serve(store, ledger).await?;
    info!("dev.lisaos.Context1 up on the session bus");

    // A dead bus connection silently drops the name while the process
    // keeps serving (seen live on inferenced: session restart → bus
    // socket gone → the name vanished with the daemon still up). Exit
    // and let systemd restart us onto the live bus instead of serving
    // a ghost.
    let watchdog = conn.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if watchdog.is_closed() {
                tracing::error!("D-Bus connection lost — exiting so systemd re-registers the name");
                std::process::exit(1);
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("shutting down");
    Ok(())
}
