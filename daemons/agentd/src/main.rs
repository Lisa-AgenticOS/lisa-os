//! lisa-agentd — daemon entry point (`docs/PLAN.md` §5.4).
//!
//! Loads installed manifests, opens the Ledger (no ledger, no bus) and
//! the undo journal, and serves `dev.lisaos.Agent1` on the session bus.
//! No network access — ever (CLAUDE.md rule 5); the hardened systemd
//! unit enforces it on the image, and no dependency here may add it.

use lisa_agentd::bus::AgentBus;
use lisa_agentd::dbus;
use lisa_agentd::journal::UndoJournal;
use lisa_agentd::registry::Registry;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

/// Manifest directories, in load order (later wins on app_id clash):
/// system, then per-user; `LISA_MANIFEST_DIRS` (colon-separated)
/// overrides both for testing.
fn manifest_dirs() -> Vec<PathBuf> {
    if let Some(dirs) = std::env::var_os("LISA_MANIFEST_DIRS") {
        return std::env::split_paths(&dirs).collect();
    }
    let mut dirs = vec![PathBuf::from("/usr/share/lisa/manifests")];
    let user_data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(base) = user_data {
        dirs.push(base.join("lisa/manifests"));
    }
    dirs
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut registry = Registry::new();
    for dir in manifest_dirs() {
        let report = registry.load_dir(&dir);
        for app in &report.loaded {
            info!(dir = %dir.display(), app, "manifest loaded");
        }
        for (path, reason) in &report.skipped {
            warn!(path = %path.display(), reason, "manifest skipped");
        }
    }
    info!(apps = registry.len(), "registry ready");

    // No ledger, no bus (dataflow rule 4): refuse to start without it.
    let ledger = Arc::new(lisa_ledger::Ledger::open(
        lisa_ledger::Ledger::default_path(),
    )?);
    let journal = UndoJournal::open(UndoJournal::default_path())?;

    // Per-app unix-socket MCP transport (libs/mcp-bus, ADR-0013): tool
    // calls execute against the app's MCP server; a missing socket fails
    // cleanly and is ledgered, exactly as NullDispatcher did. Socket dir:
    // $LISA_MCP_DIR wins (the user units set %t/lisa/mcp), else the
    // session-private $XDG_RUNTIME_DIR/lisa/mcp, else the system default —
    // apps (e.g. lisa-notes) resolve their bind path the same way, so the
    // two sides always agree.
    let mcp_dir = std::env::var_os("LISA_MCP_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(|r| std::path::PathBuf::from(r).join("lisa/mcp"))
        });
    let dispatcher = match mcp_dir {
        Some(dir) => mcp_bus::McpDispatcher::new(dir),
        None => mcp_bus::McpDispatcher::default(),
    };
    let bus = Arc::new(AgentBus::new(
        registry,
        ledger,
        journal,
        Arc::new(dispatcher),
    ));

    let _connection = dbus::serve(bus).await?;
    info!("dev.lisaos.Agent1 up on the session bus");

    tokio::signal::ctrl_c().await?;
    info!("shutting down");
    Ok(())
}
