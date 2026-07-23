//! lisa-inferenced entrypoint — the one process that owns compute for
//! inference (`docs/PLAN.md` §5.1). M0 walking skeleton: OpenAI-compat HTTP
//! on loopback with the stub engine, llama-server supervision scaffold, and
//! an optional D-Bus liveness surface.

use clap::Parser;
use lisa_inferenced::config::{self, Config, EngineKind};
use lisa_inferenced::engine::StubEngine;
use lisa_inferenced::pool::{EngineProvider, ModelPool, SingleEngine};
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
    /// Serve every model in this store refs dir by name (implies llama).
    /// The "download in Settings, use it anywhere" path.
    #[arg(long)]
    models_dir: Option<PathBuf>,
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
    if let Some(dir) = args.models_dir {
        cfg.llama.models_dir = Some(dir);
        cfg.engine = EngineKind::Llama;
    }
    match args.engine.as_deref() {
        Some("stub") => cfg.engine = EngineKind::Stub,
        Some("llama") => cfg.engine = EngineKind::Llama,
        Some(other) => anyhow::bail!("unknown engine `{other}` (stub | llama)"),
        None => {}
    }

    // Decide the effective engine. `--engine llama` still falls back to the
    // stub when there's nothing to serve — no model in the store yet, or no
    // llama-server on PATH (the Track-L layer ships no engine; a fresh image
    // boots with an empty store). This keeps `lisa ask` a working round-trip
    // until a model is downloaded and the daemon restarts to pick it up.
    let llama_ready = cfg.engine == EngineKind::Llama
        && match llama_refs_and_default(&cfg.llama) {
            Ok((ref dir, _)) => {
                config::first_model_in(dir).is_some() && binary_on_path(&cfg.llama.server_bin)
            }
            Err(_) => false,
        };

    let (engines, model_name, engine_kind): (Arc<dyn EngineProvider>, String, String) =
        if llama_ready {
            let (refs_dir, default_model) = llama_refs_and_default(&cfg.llama)?;
            info!(
                dir = %refs_dir.display(),
                default = %default_model,
                "llama engine serving the model store"
            );
            let base = cfg.llama.clone();
            // One supervised llama-server child per resident model, lazily
            // spawned, LRU-evicted beyond max_resident (§5.1).
            let pool = ModelPool::new(
                default_model.clone(),
                refs_dir,
                base.port,
                base.max_resident,
                Box::new(move |_name, path, port| {
                    let mut child_cfg = base.clone();
                    child_cfg.model_path = Some(path);
                    child_cfg.port = port;
                    Ok(Arc::new(llama::LlamaEngine::new(child_cfg)))
                }),
            );
            (Arc::new(pool), default_model, "llama".to_string())
        } else {
            if cfg.engine == EngineKind::Llama {
                warn!(
                    "llama engine requested but no servable model / no llama-server \
                     on PATH — serving the stub (download a model in Settings, then \
                     `systemctl restart lisa-inferenced`)"
                );
            }
            (
                Arc::new(SingleEngine {
                    engine: Arc::new(StubEngine),
                    name: "lisa-system-stub".to_string(),
                }),
                "lisa-system-stub".to_string(),
                "stub".to_string(),
            )
        };
    // The BYO remote tier is OFF by default (CLAUDE.md rule 5: nothing
    // leaves the device). Only when it's enabled do we wrap the provider so
    // `remote:<provider>:<model>` names route to the lisa-remoted broker
    // (§5.11) — and honor the configured socket. Disabled → a `remote:` name
    // simply isn't found locally and is refused; inferenced stays
    // network-free either way (the broker owns egress).
    let engines: Arc<dyn EngineProvider> = if cfg.remote.enabled {
        let socket = cfg
            .remote
            .socket
            .clone()
            .unwrap_or_else(lisa_inferenced::remote::default_socket);
        info!(socket = %socket.display(), "BYO remote tier enabled — routing remote: names to the broker");
        Arc::new(lisa_inferenced::remote::RemoteRouter::new(engines, socket))
    } else {
        info!("BYO remote tier disabled — nothing leaves this device");
        engines
    };
    info!("engine provider initialized (remote routing enabled)");

    let scheduler = Arc::new(lisa_inferenced::scheduler::Scheduler::new(1));

    // D-Bus is opt-in until the portal lands; never fatal.
    let _dbus_conn = if cfg.dbus {
        match dbus::serve(Arc::clone(&engines), Arc::clone(&scheduler)).await {
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

    // No ledger, no inference (PLAN §4 rule 4): refuse to serve at all
    // if the audit log cannot be opened.
    let ledger_path = std::env::var_os("LISA_LEDGER_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(lisa_ledger::Ledger::default_path);
    let ledger = lisa_ledger::Ledger::open(&ledger_path)
        .map_err(|e| anyhow::anyhow!("cannot open ledger {}: {e}", ledger_path.display()))?;
    info!(ledger = %ledger_path.display(), "ledger open (append-only)");

    let state = api::AppState {
        engines,
        scheduler,
        engine_kind,
        model_name,
        ledger: Arc::new(ledger),
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

/// Is the given binary runnable — an absolute path that exists, or a bare
/// name found on `$PATH`? Used to decide whether the llama engine can serve
/// (Track-L layers ship no llama-server) before spawning a doomed child.
fn binary_on_path(bin: &std::path::Path) -> bool {
    if bin.is_absolute() || bin.components().count() > 1 {
        return bin.exists();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

/// Resolve the llama engine's (refs_dir, default_model) from config:
/// an explicit `model_path` (its parent is the dir, its filename the
/// default), or a `models_dir` store served by name (the default is the
/// configured one, else the first model present — so a fresh download is
/// usable with nothing set). Errors only when neither is configured.
fn llama_refs_and_default(cfg: &config::LlamaConfig) -> anyhow::Result<(PathBuf, String)> {
    if let Some(mp) = cfg.model_path.clone() {
        let refs = mp
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        let name = mp
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "lisa-system".to_string());
        return Ok((refs, name));
    }
    if let Some(dir) = cfg.models_dir.clone() {
        // Prefer the configured default — but only if it's actually in the
        // store; a stale/typo'd default would otherwise spawn a doomed
        // llama-server on a nonexistent path. Else use the first model
        // present.
        let default = cfg
            .default_model
            .clone()
            .filter(|m| dir.join(m).exists())
            .or_else(|| config::first_model_in(&dir))
            .unwrap_or_else(|| "lisa-system".to_string());
        return Ok((dir, default));
    }
    anyhow::bail!("engine llama needs --model, llama.model_path, or --models-dir/llama.models_dir")
}
