//! lisa-inferenced entrypoint — the one process that owns compute for
//! inference (`docs/PLAN.md` §5.1). M0 walking skeleton: OpenAI-compat HTTP
//! on loopback with the stub engine, llama-server supervision scaffold, and
//! an optional D-Bus liveness surface.

use clap::Parser;
use lisa_inferenced::config::{self, Config, EngineKind};
use lisa_inferenced::engine::{Engine, StubEngine};
use lisa_inferenced::{api, dbus, llama};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "lisa-inferenced",
    about = "Lisa OS model runtime & scheduler (M0 scaffold)"
)]
struct Args {
    /// TOML config path; defaults are used when omitted.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override the bind address (default 127.0.0.1:7777).
    #[arg(long)]
    bind: Option<String>,
    /// Override the engine: stub | llama.
    #[arg(long)]
    engine: Option<String>,
    /// Model path for the llama engine (implies --engine llama).
    #[arg(long)]
    model: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let mut cfg = Config::load(args.config.as_deref())?;
    if let Some(bind) = args.bind {
        cfg.bind = config::Bind(bind);
    }
    if let Some(model) = args.model {
        cfg.llama.model_path = Some(model);
        cfg.engine = EngineKind::Llama;
    }
    match args.engine.as_deref() {
        Some("stub") => cfg.engine = EngineKind::Stub,
        Some("llama") => cfg.engine = EngineKind::Llama,
        Some(other) => anyhow::bail!("unknown engine `{other}` (stub | llama)"),
        None => {}
    }

    let engine: Arc<dyn Engine> = match cfg.engine {
        EngineKind::Stub => Arc::new(StubEngine),
        EngineKind::Llama => {
            let llama = llama::LlamaEngine::new(cfg.llama.clone());
            if let Err(e) = llama.ensure_running().await {
                warn!("llama-server unavailable ({e}); requests will fail until M1 wiring");
            }
            Arc::new(llama)
        }
    };
    info!(engine = engine.name(), "engine initialized");

    let scheduler = Arc::new(lisa_inferenced::scheduler::Scheduler::new(1));

    // D-Bus is opt-in until the portal lands; never fatal.
    let _dbus_conn = if cfg.dbus {
        match dbus::serve(Arc::clone(&engine), Arc::clone(&scheduler)).await {
            Ok(conn) => {
                info!("org.lisa.Inference1 registered on the session bus");
                Some(conn)
            }
            Err(e) => {
                warn!("D-Bus unavailable, continuing HTTP-only: {e}");
                None
            }
        }
    } else {
        None
    };

    let model_name = match cfg.engine {
        EngineKind::Stub => "lisa-system-stub".to_string(),
        EngineKind::Llama => cfg
            .llama
            .model_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "lisa-system".to_string()),
    };
    let state = api::AppState {
        engine,
        scheduler,
        model_name,
    };
    let listener = tokio::net::TcpListener::bind(&cfg.bind.0).await?;
    info!("OpenAI-compat endpoint on http://{}", cfg.bind.0);
    axum::serve(listener, api::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("shutting down");
        })
        .await?;
    Ok(())
}
