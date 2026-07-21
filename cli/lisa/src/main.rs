//! `lisa` — the one command center (`docs/PLAN.md` §5.4, Appendix E rule 4:
//! everything under `lisa <verb>`, never scattered scripts).
//!
//! M0 surface: `ask` (streams from lisa-inferenced's OpenAI-compat
//! endpoint) and `models` (local store operations via the lisa-modeld
//! library). `tools`/`call`/`undo`/`ledger` are declared now and land with
//! the Agent Bus in M5.

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use lisa_modeld::{ModelStore, fetch};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "lisa", version, about = "Lisa OS command center")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ask the system model. Reads stdin when piped, e.g.
    /// `git log | lisa ask "changelog, markdown"`.
    Ask {
        /// The prompt (joined if given as multiple words).
        prompt: Vec<String>,
        /// Inference endpoint.
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
        #[arg(long)]
        model: Option<String>,
        /// Wait for the full response instead of streaming tokens.
        #[arg(long)]
        no_stream: bool,
    },
    /// Manage the local model store (PLAN §5.2).
    Models {
        #[command(subcommand)]
        cmd: ModelsCmd,
        /// Store root; production default is /var/lib/lisa/models.
        #[arg(long, env = "LISA_MODELS_DIR")]
        store: Option<PathBuf>,
    },
    /// List tools on the Agent Bus (lands in M5, PLAN §5.4).
    Tools,
    /// Call a tool on the Agent Bus (lands in M5, PLAN §5.4).
    Call,
    /// Revert the last agent action (lands in M5, PLAN §5.4).
    Undo,
    /// Query the audit ledger (lands in M2, PLAN §5.7.6).
    Ledger,
}

#[derive(Subcommand)]
enum ModelsCmd {
    /// List installed models.
    List,
    /// Recompute hashes for every stored blob.
    Verify,
    /// Remove blobs no model name references anymore.
    Gc,
    /// Remove a model name (its blob survives until `gc`).
    Rm {
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Download a model with a mandatory pinned blake3 hash.
    Pull {
        url: String,
        name: String,
        #[arg(long)]
        blake3: String,
    },
    /// Print the blake3 of a local file (for catalog pinning).
    Hash { file: PathBuf },
    /// Import a local file into the store (copied, source untouched).
    Add {
        file: PathBuf,
        name: String,
        /// Refuse unless the file's blake3 matches.
        #[arg(long)]
        blake3: Option<String>,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Ask {
            prompt,
            url,
            model,
            no_stream,
        } => ask(prompt, &url, model, no_stream),
        Command::Models { cmd, store } => models(cmd, store),
        Command::Tools | Command::Call | Command::Undo => {
            bail!("the Agent Bus lands in M5 — see docs/PLAN.md §5.4")
        }
        Command::Ledger => bail!("the Ledger lands in M2 — see docs/PLAN.md §5.7.6"),
    }
}

fn ask(
    prompt: Vec<String>,
    url: &str,
    model: Option<String>,
    no_stream: bool,
) -> anyhow::Result<()> {
    let mut prompt = prompt.join(" ");
    // Piped stdin becomes context, shell-pipeline style (PLAN §5.4).
    if !std::io::stdin().is_terminal() {
        let mut piped = String::new();
        std::io::stdin().read_to_string(&mut piped)?;
        if !piped.trim().is_empty() {
            prompt = format!("{prompt}\n\n---\nInput:\n{piped}");
        }
    }
    if prompt.trim().is_empty() {
        bail!("empty prompt — usage: lisa ask \"your question\"");
    }

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": !no_stream,
    });
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let mut response = ureq::post(&endpoint).send_json(&body).with_context(|| {
        format!(
            "request to {endpoint} failed — is lisa-inferenced running? \
             Start it with `lisa-inferenced` (or `cargo run -p lisa-inferenced`)"
        )
    })?;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if no_stream {
        let json: serde_json::Value = response.body_mut().read_json()?;
        if let Some(err) = json["error"]["message"].as_str() {
            bail!("inference error: {err}");
        }
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default();
        writeln!(out, "{content}")?;
        return Ok(());
    }

    // SSE: print token deltas as they arrive.
    let reader = BufReader::new(response.body_mut().as_reader());
    for line in reader.lines() {
        let line = line?;
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let chunk: serde_json::Value = serde_json::from_str(data)?;
        if let Some(err) = chunk["error"]["message"].as_str() {
            bail!("inference error: {err}");
        }
        if let Some(token) = chunk["choices"][0]["delta"]["content"].as_str() {
            write!(out, "{token}")?;
            out.flush()?;
        }
    }
    writeln!(out)?;
    Ok(())
}

use std::io::IsTerminal;

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

fn models(cmd: ModelsCmd, store_root: Option<PathBuf>) -> anyhow::Result<()> {
    let root = store_root.unwrap_or_else(default_store_root);
    let store = ModelStore::open(&root)?;
    match cmd {
        ModelsCmd::List => {
            let refs = store.list()?;
            if refs.is_empty() {
                println!("no models installed (store: {})", store.root().display());
            }
            for r in refs {
                println!(
                    "{}\t{:.2} GiB\t{}",
                    r.name,
                    r.size as f64 / (1 << 30) as f64,
                    r.blake3
                );
            }
        }
        ModelsCmd::Verify => {
            let report = store.verify()?;
            println!("{} blob(s) ok, {} corrupt", report.ok, report.corrupt.len());
            for (path, expected, actual) in &report.corrupt {
                eprintln!(
                    "CORRUPT {} expected {expected} got {actual}",
                    path.display()
                );
            }
            if !report.is_clean() {
                bail!("store verification failed — re-pull the corrupt model(s)");
            }
        }
        ModelsCmd::Gc => {
            let removed = store.gc()?;
            println!("removed {} unreferenced blob(s)", removed.len());
        }
        ModelsCmd::Rm { name, yes } => {
            if !yes {
                eprint!(
                    "remove model ref `{name}`? Its data is reclaimed on the next `lisa models gc`. [y/N] "
                );
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim(), "y" | "Y" | "yes") {
                    println!("aborted");
                    return Ok(());
                }
            }
            store.remove_ref(&name)?;
            println!("removed ref `{name}` (blob reclaimed on next gc)");
        }
        ModelsCmd::Pull { url, name, blake3 } => {
            let entry = fetch::pull(&store, &url, &name, &blake3)?;
            println!(
                "pulled `{}` ({:.2} GiB, blake3 {})",
                entry.name,
                entry.size as f64 / (1 << 30) as f64,
                entry.blake3
            );
        }
        ModelsCmd::Hash { file } => {
            println!("{}", ModelStore::hash_file(&file)?);
        }
        ModelsCmd::Add { file, name, blake3 } => {
            let entry = match blake3 {
                Some(expected) => store.add_file_verified(&file, &name, &expected)?,
                None => store.add_file(&file, &name)?,
            };
            println!(
                "added `{}` ({:.2} GiB, blake3 {})",
                entry.name,
                entry.size as f64 / (1 << 30) as f64,
                entry.blake3
            );
        }
    }
    Ok(())
}
