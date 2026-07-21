//! lisa-modeld entrypoint. M0 scaffold: opens the store, reports its state,
//! and parses the seed catalog. The D-Bus service, hardware profiler, and
//! catalog refresh loop land in M1 (`docs/PLAN.md` §5.2).

use clap::Parser;
use lisa_modeld::{ModelStore, catalog, profile};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "lisa-modeld",
    about = "Lisa OS model catalog & store daemon (M0 scaffold)"
)]
struct Args {
    /// Store root. Production default is /var/lib/lisa/models; for
    /// development use a writable path or set LISA_MODELS_DIR.
    #[arg(long, env = "LISA_MODELS_DIR")]
    store: Option<PathBuf>,
}

fn default_store_root() -> PathBuf {
    let system = PathBuf::from("/var/lib/lisa/models");
    if system.is_dir() {
        return system;
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".local/share/lisa/models"))
        .unwrap_or(system)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let root = args.store.unwrap_or_else(default_store_root);
    let store = ModelStore::open(&root)?;

    let refs = store.list()?;
    let report = store.verify()?;
    println!("lisa-modeld {}", env!("CARGO_PKG_VERSION"));
    let hw = profile::profile();
    println!(
        "hardware: {}/{} ram={}GiB unified={} gpu_nodes={} npu_nodes={} => tier {}",
        hw.os, hw.arch, hw.total_ram_gb, hw.unified_memory, hw.gpu_nodes, hw.npu_nodes, hw.tier
    );
    println!("store: {}", store.root().display());
    println!(
        "refs: {}  blobs ok: {}  corrupt: {}",
        refs.len(),
        report.ok,
        report.corrupt.len()
    );
    for (path, expected, actual) in &report.corrupt {
        eprintln!(
            "CORRUPT {} expected {expected} got {actual}",
            path.display()
        );
    }

    let seed = include_str!("../../../models/catalog/catalog.toml");
    let cat = catalog::parse(seed)?;
    println!(
        "seed catalog: {} models (catalog_version {})",
        cat.models.len(),
        cat.catalog_version
    );
    println!("D-Bus service, hardware profiler, and refresh loop land in M1 (PLAN §5.2).");

    if !report.is_clean() {
        std::process::exit(1);
    }
    Ok(())
}
