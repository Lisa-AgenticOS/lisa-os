//! lisa-remoted entry point: unix-socket HTTP server (+ optional D-Bus
//! registration) or the `--import-esp` oneshot (ADR-0008 §6).

use clap::Parser;
use lisa_remoted::provision;
use lisa_remoted::registry::Registry;
use lisa_remoted::secrets::SecretStore;
use lisa_remoted::service::Broker;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "lisa-remoted",
    about = "Lisa OS remote-provider egress broker (PLAN §5.11)"
)]
struct Args {
    /// Unix socket for the OpenAI-compatible proxy + management API.
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Broker state (registry, consent, 0600 credential store).
    #[arg(long)]
    state_dir: Option<PathBuf>,

    /// Ledger database path (defaults like the other daemons).
    #[arg(long)]
    ledger: Option<PathBuf>,

    /// Oneshot: import staged `lisa-provision/*.key` files from this
    /// ESP mountpoint into the credential store, scrub them from the
    /// ESP, and exit (field-test provisioning, superseded by M7 OOBE).
    #[arg(long, value_name = "ESP_MOUNTPOINT")]
    import_esp: Option<PathBuf>,

    /// Also register dev.lisaos.Remote1 on the session bus.
    #[arg(long)]
    dbus: bool,
}

fn default_state_dir() -> PathBuf {
    if let Some(state) = std::env::var_os("STATE_DIRECTORY") {
        return PathBuf::from(state);
    }
    let system = PathBuf::from("/var/lib/lisa/remoted");
    if system.is_dir() {
        return system;
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".local/share/lisa/remoted"))
        .unwrap_or(system)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    let state_dir = args.state_dir.unwrap_or_else(default_state_dir);

    if let Some(esp) = args.import_esp {
        let registry = Registry::open(&state_dir)?;
        let secrets = SecretStore::open(&state_dir)?;
        let outcomes = provision::import_esp(&esp, &registry, &secrets)?;
        for o in &outcomes {
            match o {
                provision::Outcome::Imported(id) => {
                    tracing::info!(provider = %id, "imported staged key; scrubbed from ESP")
                }
                provision::Outcome::UnknownProvider(id) => {
                    tracing::warn!(provider = %id, "unknown provider id; left on ESP")
                }
            }
        }
        tracing::info!(count = outcomes.len(), "ESP provisioning pass complete");
        return Ok(());
    }

    let ledger_path = args
        .ledger
        .unwrap_or_else(lisa_ledger::Ledger::default_path);
    let ledger = Arc::new(lisa_ledger::Ledger::open(&ledger_path)?);
    let broker = Broker::open(&state_dir, ledger)?;

    let _dbus_conn = if args.dbus {
        Some(lisa_remoted::dbus::serve(Arc::clone(&broker)).await?)
    } else {
        None
    };

    let socket = args
        .socket
        .unwrap_or_else(|| state_dir.join("remoted.sock"));
    if socket.exists() {
        std::fs::remove_file(&socket)?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = tokio::net::UnixListener::bind(&socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!(socket = %socket.display(), state = %state_dir.display(), "lisa-remoted up");

    let app = lisa_remoted::api::router(broker);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
