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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

/// The manifests the image ships. Always searched, always first.
const SYSTEM_MANIFEST_DIR: &str = "/usr/share/lisa/manifests";

/// Manifest directories, in precedence order — the FIRST definition of
/// an app_id wins (issue #97), so a user-writable manifest can add a
/// new app but never redefine a system one.
fn manifest_dirs() -> Vec<PathBuf> {
    let user_data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|base| base.join("lisa/manifests"));
    search_path(
        Path::new(SYSTEM_MANIFEST_DIR),
        std::env::var_os("LISA_MANIFEST_DIRS").as_deref(),
        user_data,
    )
}

/// Assemble the search path. Split out so the precedence rule is
/// testable without a process environment.
///
/// `LISA_MANIFEST_DIRS` used to *replace* the whole list, which handed
/// the ordering — or the removal of the system directory outright — to
/// anyone who could set the daemon's environment (#134). That is the
/// same capability #97 already declares untrusted: a
/// `~/.config/systemd/user/lisa-agentd.service.d/` drop-in, or
/// `systemctl --user set-environment`, are ordinary same-user
/// operations, and `NoNewPrivileges=yes` does not touch either. So the
/// variable now only ever APPENDS; the system directory is prepended
/// unconditionally and cannot be displaced.
fn search_path(
    system: &Path,
    env: Option<&std::ffi::OsStr>,
    user_data: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut dirs = vec![system.to_path_buf()];
    if let Some(extra) = env {
        dirs.extend(std::env::split_paths(extra));
    }
    dirs.extend(user_data);
    // A directory listed twice would load twice and report the second
    // pass as a bogus "already defined" clash.
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
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
        // Loud, and naming the app and tool. The failure this replaces
        // was a 503 from the inference engine that named nobody, on a
        // device where the culprit was an unrelated app's schema
        // (#147).
        for (app, what) in &report.adjusted {
            warn!(app, what, "manifest adjusted at load");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// Issue #134. #97 made precedence positional — system first, user
    /// second, first definition wins — and `LISA_MANIFEST_DIRS`
    /// replaced the entire list, so setting it removed the system
    /// directory and a hostile manifest reusing a system `app_id` won
    /// again. The variable may add, never displace or remove.
    #[test]
    fn the_environment_cannot_displace_the_system_manifest_dir() {
        let system = Path::new("/usr/share/lisa/manifests");
        for hostile in [
            "/home/me/.local/share/evil",
            // Trying to get in FRONT of the system directory.
            "/home/me/evil:/usr/share/lisa/manifests",
            // Or to crowd it out entirely.
            "/tmp/a:/tmp/b:/tmp/c",
        ] {
            let dirs = search_path(system, Some(&OsString::from(hostile)), None);
            assert_eq!(
                dirs.first().map(PathBuf::as_path),
                Some(system),
                "`{hostile}` displaced the system manifest directory"
            );
            assert!(
                dirs.iter().filter(|d| d.as_path() == system).count() == 1,
                "the system directory is loaded twice for `{hostile}`"
            );
        }
    }

    #[test]
    fn the_user_directory_comes_last_and_the_env_still_works() {
        let system = Path::new("/sys");
        let dirs = search_path(
            system,
            Some(&OsString::from("/extra")),
            Some(PathBuf::from("/home/me/.local/share/lisa/manifests")),
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/sys"),
                PathBuf::from("/extra"),
                PathBuf::from("/home/me/.local/share/lisa/manifests"),
            ]
        );
        // No environment, no user dir: just the system directory.
        assert_eq!(search_path(system, None, None), vec![PathBuf::from("/sys")]);
    }
}
