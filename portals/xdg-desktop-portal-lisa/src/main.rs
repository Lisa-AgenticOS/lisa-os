//! xdg-desktop-portal-lisa entrypoint (`docs/PLAN.md` §5.5, ADR-0008):
//! a per-user session-bus service (`dev.lisaos.Portal`) sitting between
//! apps and `dev.lisaos.Inference1`. Defaults are the safe ones: real
//! identity resolution via /proc, consent via the shell dialog service
//! (fail-closed when absent), grants and ledger on disk.

use clap::Parser;
use lisa_portal::consent::{ConsentUi, DbusConsentUi, StaticConsent};
use lisa_portal::grants::GrantStore;
use lisa_portal::identity::ProcResolver;
use lisa_portal::portal::{self, PortalState};
use lisa_portal::quota::QuotaConfig;
use lisa_portal::upstream::{InferenceUpstream, ZbusUpstream, stub::StubUpstream};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "xdg-desktop-portal-lisa",
    about = "Lisa OS trust-boundary portal (PLAN §5.5)"
)]
struct Args {
    /// Upstream inference daemon: dbus (dev.lisaos.Inference1) | stub.
    #[arg(long, default_value = "dbus")]
    upstream: String,
    /// Consent backend: ui (shell dialog, fail-closed when absent) |
    /// allow (dev only: grant everything, remembered) | deny.
    #[arg(long, default_value = "ui")]
    consent: String,
    /// Grant store path (default: ~/.local/share/lisa/grants.db, or
    /// $LISA_GRANTS_DB).
    #[arg(long)]
    grants_db: Option<PathBuf>,
    /// Ledger path (default: ~/.local/share/lisa/ledger.db, or
    /// $LISA_LEDGER_DB).
    #[arg(long)]
    ledger: Option<PathBuf>,
    /// Per-app requests/min quota.
    #[arg(long, default_value_t = QuotaConfig::default().requests_per_min)]
    requests_per_min: u32,
    /// Per-app tokens/day quota.
    #[arg(long, default_value_t = QuotaConfig::default().tokens_per_day)]
    tokens_per_day: i64,
}

/// The portal is per-user: its ledger lives in the user's data dir (the
/// system daemons write the system ledger under their StateDirectory).
fn default_ledger_path() -> PathBuf {
    if let Some(p) = std::env::var_os("LISA_LEDGER_DB") {
        return PathBuf::from(p);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".local/share/lisa/ledger.db"))
        .unwrap_or_else(|| PathBuf::from("lisa-ledger.db"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    let grants_path = args.grants_db.unwrap_or_else(GrantStore::default_path);
    let grants =
        Arc::new(GrantStore::open(&grants_path).map_err(|e| {
            anyhow::anyhow!("cannot open grant store {}: {e}", grants_path.display())
        })?);
    info!(grants = %grants_path.display(), "grant store open (append-only)");

    // No ledger, no portal (PLAN §4 rule 4).
    let ledger_path = args.ledger.unwrap_or_else(default_ledger_path);
    let ledger = Arc::new(
        lisa_ledger::Ledger::open(&ledger_path)
            .map_err(|e| anyhow::anyhow!("cannot open ledger {}: {e}", ledger_path.display()))?,
    );
    info!(ledger = %ledger_path.display(), "ledger open (append-only)");

    let session = zbus::Connection::session().await?;

    let upstream: Arc<dyn InferenceUpstream> = match args.upstream.as_str() {
        "dbus" => Arc::new(ZbusUpstream::new(session.clone())),
        "stub" => Arc::new(StubUpstream),
        other => anyhow::bail!("unknown upstream `{other}` (dbus | stub)"),
    };
    let consent: Arc<dyn ConsentUi> = match args.consent.as_str() {
        "ui" => Arc::new(DbusConsentUi::new(session.clone())),
        "allow" => {
            tracing::warn!("--consent allow grants every first-use request (dev mode)");
            Arc::new(StaticConsent::allow_always())
        }
        "deny" => Arc::new(StaticConsent::deny()),
        other => anyhow::bail!("unknown consent backend `{other}` (ui | allow | deny)"),
    };

    let state = PortalState::new(
        Arc::new(ProcResolver::new()),
        consent,
        upstream,
        grants,
        ledger,
        QuotaConfig {
            requests_per_min: args.requests_per_min,
            tokens_per_day: args.tokens_per_day,
        },
    );
    let _conn = portal::serve(state).await?;
    info!(
        "{} registered at {}",
        portal::PORTAL_BUS_NAME,
        portal::PORTAL_PATH
    );

    tokio::signal::ctrl_c().await?;
    info!("shutting down");
    Ok(())
}
