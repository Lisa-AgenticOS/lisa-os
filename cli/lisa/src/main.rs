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
        /// Guided generation: path to a JSON Schema file; output is
        /// grammar-constrained to match it.
        #[arg(long)]
        json_schema: Option<PathBuf>,
        /// Run at background priority (preempted by interactive requests).
        #[arg(long)]
        background: bool,
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
    /// Read the append-only audit ledger (PLAN §5.7.6).
    Ledger {
        /// Show the most recent N entries.
        #[arg(long, default_value_t = 20)]
        tail: usize,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// Ledger DB path (default: /var/lib/lisa or ~/.local/share/lisa).
        #[arg(long, env = "LISA_LEDGER_DB")]
        db: Option<PathBuf>,
    },
    /// Embed text into a vector (reads stdin when piped).
    Embed {
        text: Vec<String>,
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
    },
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
    /// Print the hardware profile and PLAN §8 tier.
    Profile,
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
            json_schema,
            background,
        } => ask(prompt, &url, model, no_stream, json_schema, background),
        Command::Models { cmd, store } => models(cmd, store),
        Command::Tools | Command::Call | Command::Undo => {
            bail!("the Agent Bus lands in M5 — see docs/PLAN.md §5.4")
        }
        Command::Ledger { tail, json, db } => ledger_cmd(tail, json, db),
        Command::Embed { text, url } => embed(text, &url),
    }
}

fn ask(
    prompt: Vec<String>,
    url: &str,
    model: Option<String>,
    no_stream: bool,
    json_schema: Option<PathBuf>,
    background: bool,
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

    let mut body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": !no_stream,
    });
    if let Some(path) = json_schema {
        let schema: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&path)
                .with_context(|| format!("reading schema {}", path.display()))?,
        )
        .with_context(|| format!("parsing schema {}", path.display()))?;
        body["response_format"] = serde_json::json!({
            "type": "json_schema",
            "json_schema": { "name": "schema", "schema": schema, "strict": true },
        });
    }
    if background {
        body["lisa_priority"] = "background".into();
    }
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

fn ledger_cmd(tail: usize, json: bool, db: Option<PathBuf>) -> anyhow::Result<()> {
    let path = db.unwrap_or_else(lisa_ledger::Ledger::default_path);
    if !path.exists() {
        bail!(
            "no ledger at {} — it is created by lisa-inferenced on first start",
            path.display()
        );
    }
    let ledger = lisa_ledger::Ledger::open(&path)?;
    let entries = ledger.tail(tail)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    println!(
        "{} entries total — showing {} (ledger: {})",
        ledger.count()?,
        entries.len(),
        path.display()
    );
    for e in &entries {
        let secs = e.ts / 1000;
        let refmark = e.ref_id.map(|r| format!(" ->#{r}")).unwrap_or_default();
        println!(
            "#{:<5} {}  {:<19} {:<9} {:>5}tok {:>6}ms{}  {}",
            e.id, secs, e.kind, e.status, e.output_tokens, e.duration_ms, refmark, e.preview
        );
    }
    Ok(())
}

fn embed(text: Vec<String>, url: &str) -> anyhow::Result<()> {
    let mut text = text.join(" ");
    if !std::io::stdin().is_terminal() {
        let mut piped = String::new();
        std::io::stdin().read_to_string(&mut piped)?;
        if !piped.trim().is_empty() {
            text = piped;
        }
    }
    if text.trim().is_empty() {
        bail!("empty input — usage: lisa embed \"some text\"");
    }
    let endpoint = format!("{}/v1/embeddings", url.trim_end_matches('/'));
    let mut response = ureq::post(&endpoint)
        .send_json(serde_json::json!({ "input": text }))
        .with_context(|| format!("request to {endpoint} failed — is lisa-inferenced running?"))?;
    let json: serde_json::Value = response.body_mut().read_json()?;
    if let Some(err) = json["error"]["message"].as_str() {
        bail!("embeddings error: {err}");
    }
    let vector = &json["data"][0]["embedding"];
    println!("{vector}");
    Ok(())
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
        ModelsCmd::Profile => {
            let p = lisa_modeld::profile::profile();
            println!("{}", serde_json::to_string_pretty(&p)?);
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
