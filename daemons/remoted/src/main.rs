//! lisa-remoted entry point: unix-socket HTTP server, optionally also
//! registering dev.lisaos.Remote1 on the session bus.

use clap::Parser;
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

    // `into_make_service_with_connect_info` is what carries the kernel's
    // answer about each peer into the handlers (issue #99). Without it
    // every management route refuses — which is the right way round for
    // a wiring mistake, but it does mean this line is load-bearing.
    let app = lisa_remoted::api::router(broker);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<lisa_remoted::api::PeerInfo>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
