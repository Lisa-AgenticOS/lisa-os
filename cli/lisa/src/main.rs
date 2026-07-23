//! `lisa` — the one command center (`docs/PLAN.md` §5.4, Appendix E rule 4:
//! everything under `lisa <verb>`, never scattered scripts).
//!
//! M0 surface: `ask` (streams from lisa-inferenced's OpenAI-compat
//! endpoint) and `models` (local store operations via the lisa-modeld
//! library). `tools`/`call`/`undo`/`ledger` are declared now and land with
//! the Agent Bus in M5.

mod voice;

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
    /// Context fabric: index and search your files (PLAN §5.3).
    Context {
        #[command(subcommand)]
        cmd: ContextCmd,
    },
    /// Per-app durable memory (PLAN §5.3).
    Memory {
        #[command(subcommand)]
        cmd: MemoryCmd,
        /// App namespace (per-app isolation is the point).
        #[arg(long, default_value = "host", global = true)]
        app: String,
    },
    /// Write the newest Lisa OS release to a whole disk — ERASES IT.
    /// The proto-installer (a guided OOBE installer is M7).
    Install {
        /// Target block device (e.g. /dev/sda). Everything on it is lost.
        target: PathBuf,
        /// Local .raw.zst to write instead of downloading the latest release.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Skip the typed confirmation (scripts/CI only).
        #[arg(long)]
        yes: bool,
    },
    /// Pull the newest OS release into the inactive A/B slot
    /// (systemd-sysupdate; Track I systems).
    Update {
        /// Reboot into the new version after a successful update.
        #[arg(long)]
        reboot: bool,
    },
    /// Manage BYO remote model providers (PLAN §5.11). Inference uses
    /// them via `lisa ask --model remote:<provider>:<model>`.
    Remote {
        #[command(subcommand)]
        cmd: RemoteCmd,
    },
    /// Transcribe an audio file with whisper.cpp (STT, §5.7.5).
    Transcribe {
        audio: PathBuf,
        #[arg(long)]
        model: Option<PathBuf>,
    },
    /// Speak text with the local voice (piper / say) (TTS, §5.7.5).
    Say { text: Vec<String> },
    /// Lisa Ambient: the voice loop (ADR-0011).
    Ambient {
        #[command(subcommand)]
        cmd: AmbientCmd,
    },
    /// LisaCode: talk an app into existence — the Forge harness drives a
    /// model to write + fix code until it passes analysis (PLAN §5.12.1).
    Forge {
        /// What to build, e.g. "a tip calculator".
        task: Vec<String>,
        /// Project directory (created/scaffolded if empty).
        #[arg(long, default_value = "./lisa-app")]
        project: PathBuf,
        /// Model — local (default) or remote:<provider>:<coder-model>.
        #[arg(long)]
        model: Option<String>,
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
        /// Max plan→edit→analyze iterations before giving up.
        #[arg(long, default_value_t = 6)]
        max_iters: usize,
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
enum AmbientCmd {
    /// Decide whether an utterance was addressed to Lisa (no wake word).
    Classify {
        text: Vec<String>,
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
    },
    /// Full loop on one audio file: transcribe → classify → answer → say.
    Once {
        audio: PathBuf,
        #[arg(long)]
        model: Option<PathBuf>,
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
        /// Speak the answer aloud.
        #[arg(long)]
        speak: bool,
        /// Phase-2: gate on the addressed-intent classifier instead of
        /// the "Hey Lisa" wake word (over-triggers on small models).
        #[arg(long)]
        classify: bool,
    },
}

#[derive(Subcommand)]
enum RemoteCmd {
    /// List providers and consent (may-offload) state.
    List,
    /// Add a custom OpenAI-compat provider.
    Add {
        id: String,
        display_name: String,
        url: String,
    },
    /// Store an API key for a provider (reads the key from stdin).
    Key { provider: String },
    /// Set per-scope offload consent (default: everything OFF).
    Consent {
        /// prompt | files | mail | calendar | screen | memory
        scope: String,
        /// on | off
        state: String,
    },
}

#[derive(Subcommand)]
enum ContextCmd {
    /// Index text files under a directory (incremental).
    Index {
        dir: PathBuf,
        /// Also embed chunks for hybrid (vector) search.
        #[arg(long)]
        embed: bool,
    },
    /// Search the index (lexical by default; --hybrid blends vectors).
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        /// Blend BM25 with vector similarity (needs indexed embeddings).
        #[arg(long)]
        hybrid: bool,
        /// Restrict to the provenance a granted scope permits, e.g.
        /// `--scope documents` (repeatable). A scoped search never
        /// returns a disallowed-provenance chunk (PLAN §5.3 ACL).
        #[arg(long)]
        scope: Vec<String>,
    },
}

#[derive(Subcommand)]
enum MemoryCmd {
    Get {
        key: String,
    },
    Set {
        key: String,
        value: String,
    },
    List,
    /// Remove every key in this app's namespace (asks first).
    Wipe {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ModelsCmd {
    /// List installed models.
    List {
        /// Machine-readable JSON (for Settings / scripts).
        #[arg(long)]
        json: bool,
    },
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
    /// Show the model catalog annotated by what THIS machine can run
    /// locally (remote-provider models always run — see `lisa remote`).
    Catalog {
        /// Only show models that run (or run tight) on this machine.
        #[arg(long)]
        runnable: bool,
        /// Machine-readable JSON: {profile, models:[{…,fit,installed,
        /// available}]} — the data source for the Settings AI panel.
        #[arg(long)]
        json: bool,
    },
    /// Download a catalog model by id (resolves its pinned source+hash).
    Get { id: String },
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
        Command::Install { target, from, yes } => install_cmd(&target, from, yes),
        Command::Update { reboot } => update_cmd(reboot),
        Command::Remote { cmd } => remote_cmd(cmd),
        Command::Transcribe { audio, model } => {
            let m = voice::whisper_model(model)?;
            println!("{}", voice::transcribe(&audio, &m)?);
            Ok(())
        }
        Command::Say { text } => voice::say(&text.join(" ")),
        Command::Ambient { cmd } => ambient_cmd(cmd),
        Command::Forge {
            task,
            project,
            model,
            url,
            max_iters,
        } => forge_cmd(&task.join(" "), &project, model, &url, max_iters),
        Command::Context { cmd } => context_cmd(cmd),
        Command::Memory { cmd, app } => memory_cmd(cmd, &app),
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

const RELEASES_API: &str = "https://api.github.com/repos/Lisa-AgenticOS/lisa-os/releases/latest";

fn install_cmd(target: &PathBuf, from: Option<PathBuf>, yes: bool) -> anyhow::Result<()> {
    // Guards: block devices only on Linux and never the running disk;
    // regular-file targets are allowed anywhere (testing, image work).
    if !target.exists() {
        bail!("{} does not exist", target.display());
    }
    let is_block = {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata(target)?.file_type().is_block_device()
    };
    if is_block && !cfg!(target_os = "linux") {
        bail!("writing block devices is supported on Linux — boot the Lisa USB and run it there");
    }
    let target_str = target.to_string_lossy();
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts")
        && mounts.lines().any(|l| {
            l.split_whitespace()
                .next()
                .is_some_and(|d| d.starts_with(target_str.as_ref()))
        })
    {
        bail!(
            "{} has mounted partitions — it looks like the disk this system is running from. \
             Boot from the USB stick and install to the internal disk instead.",
            target.display()
        );
    }

    eprintln!(
        "!! {} will be COMPLETELY ERASED — every partition, every file.",
        target.display()
    );
    if !yes {
        eprint!("Type ERASE to continue: ");
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if answer.trim() != "ERASE" {
            println!("aborted — nothing written");
            return Ok(());
        }
    }

    let mut sink = std::fs::OpenOptions::new().write(true).open(target)?;
    let written = match from {
        Some(path) => {
            let file = std::fs::File::open(&path)?;
            let mut decoder = zstd::Decoder::new(std::io::BufReader::new(file))?;
            std::io::copy(&mut decoder, &mut sink)?
        }
        None => {
            // Resolve the newest release's .raw.zst asset and stream it
            // straight through zstd onto the disk — no scratch space.
            let mut resp = ureq::get(RELEASES_API)
                .header("User-Agent", "lisa-cli")
                .call()
                .context("querying latest release")?;
            let release: serde_json::Value = resp.body_mut().read_json()?;
            let asset = release["assets"]
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .find(|x| x["name"].as_str().is_some_and(|n| n.ends_with(".raw.zst")))
                })
                .ok_or_else(|| anyhow::anyhow!("no .raw.zst asset in the latest release"))?;
            let url = asset["browser_download_url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("asset has no download url"))?;
            let name = asset["name"].as_str().unwrap_or("image");
            eprintln!(">> streaming {name} to {}", target.display());
            let mut resp = ureq::get(url).call().context("downloading image")?;
            let reader = std::io::BufReader::new(resp.body_mut().as_reader());
            let mut decoder = zstd::Decoder::new(reader)?;
            std::io::copy(&mut decoder, &mut sink)?
        }
    };
    sink.sync_all()?;
    println!(
        ">> wrote {:.1} GiB to {} — remove the USB stick and reboot; \
         first boot grows /var to fill the disk",
        written as f64 / (1u64 << 30) as f64,
        target.display()
    );
    Ok(())
}

fn forge_cmd(
    task: &str,
    project: &PathBuf,
    model: Option<String>,
    url: &str,
    max_iters: usize,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(project)?;
    let pubspec = project.join("pubspec.yaml");
    if !pubspec.exists() {
        std::fs::write(
            &pubspec,
            "name: lisa_app\ndescription: An app forged by LisaCode.\nenvironment:\n  sdk: ^3.0.0\n",
        )?;
        std::fs::create_dir_all(project.join("bin"))?;
    }
    println!("LisaCode: building \"{task}\" in {}", project.display());
    let mut backend = forge_harness::OpenAiBackend {
        url: url.to_string(),
        model,
    };
    match forge_harness::forge(task, project, &mut backend, max_iters) {
        Ok(report) => {
            println!(
                "converged in {} iteration(s) — code passes `dart analyze`. Source in {}",
                report.iterations,
                project.display()
            );
            Ok(())
        }
        Err(e) => bail!("forge did not finish: {e}"),
    }
}

fn ambient_cmd(cmd: AmbientCmd) -> anyhow::Result<()> {
    match cmd {
        AmbientCmd::Classify { text, url } => {
            let a = voice::classify_addressed(&text.join(" "), &url)?;
            println!(
                "addressed={} confidence={:.2} intent={:?}",
                a.addressed, a.confidence, a.intent
            );
        }
        AmbientCmd::Once {
            audio,
            model,
            url,
            speak,
            classify,
        } => {
            let m = voice::whisper_model(model)?;
            let transcript = voice::transcribe(&audio, &m)?;
            println!("heard:  {transcript}");
            // Default: "Hey Lisa" wake word (reliable). --classify uses
            // the Phase-2 addressed-intent model gate.
            let query = if classify {
                let a = voice::classify_addressed(&transcript, &url)?;
                println!(
                    "decide: addressed={} confidence={:.2} intent={:?}",
                    a.addressed, a.confidence, a.intent
                );
                if a.addressed {
                    Some(transcript.clone())
                } else {
                    None
                }
            } else {
                match voice::wake_word(&transcript) {
                    Some(q) => {
                        println!("decide: wake word \"Hey Lisa\" heard");
                        Some(q)
                    }
                    None => None,
                }
            };
            let Some(query) = query else {
                println!("(not addressed to Lisa — staying quiet)");
                return Ok(());
            };
            let reply = voice::answer(&query, &url)?;
            println!("Lisa:   {reply}");
            if speak {
                voice::say(&reply)?;
            }
        }
    }
    Ok(())
}

fn remoted_socket() -> PathBuf {
    std::env::var_os("LISA_REMOTED_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/lisa/remoted/remoted.sock"))
}

/// Minimal sync HTTP/1.1 over the broker's unix socket (Connection:
/// close). The broker is loopback-only; the CLI never touches the
/// network — egress stays the broker's job (rule 5).
fn broker_request(method: &str, path: &str, body: Option<&str>) -> anyhow::Result<(u16, String)> {
    use std::os::unix::net::UnixStream;
    let sock = remoted_socket();
    let mut stream = UnixStream::connect(&sock).with_context(|| {
        format!(
            "lisa-remoted socket {} — is the broker running? (systemctl start lisa-remoted)",
            sock.display()
        )
    })?;
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: lisa-remoted\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(req.as_bytes())?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let (head, resp) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("malformed broker response"))?;
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Ok((status, resp.trim().to_string()))
}

fn remote_cmd(cmd: RemoteCmd) -> anyhow::Result<()> {
    match cmd {
        RemoteCmd::List => {
            let (st, body) = broker_request("GET", "/v1/providers", None)?;
            if st != 200 {
                bail!("broker: {body}");
            }
            let v: serde_json::Value = serde_json::from_str(&body)?;
            println!("providers:");
            for p in v["providers"].as_array().cloned().unwrap_or_default() {
                println!(
                    "  {:<14} {}",
                    p["id"].as_str().unwrap_or("?"),
                    p["base_url"].as_str().unwrap_or("(unset)")
                );
            }
            if let Ok((_, c)) = broker_request("GET", "/v1/consent", None) {
                println!("\nconsent (may offload — default off):\n  {c}");
            }
            println!(
                "\nSet a key:   lisa remote key <provider>\n\
                 Allow scope: lisa remote consent prompt on\n\
                 Use it:      lisa ask --model remote:<provider>:<model>"
            );
        }
        RemoteCmd::Add {
            id,
            display_name,
            url,
        } => {
            let b = serde_json::json!({"id": id, "display_name": display_name, "base_url": url})
                .to_string();
            let (st, body) = broker_request("POST", "/v1/providers", Some(&b))?;
            if st != 200 {
                bail!("broker: {body}");
            }
            println!("added provider `{id}` -> {url}");
        }
        RemoteCmd::Key { provider } => {
            eprintln!(
                "paste the API key for `{provider}` and press Enter (input is stored encrypted, write-only):"
            );
            let mut key = String::new();
            std::io::stdin().read_line(&mut key)?;
            let key = key.trim();
            if key.is_empty() {
                bail!("empty key");
            }
            let b = serde_json::json!({ "key": key }).to_string();
            let (st, body) =
                broker_request("PUT", &format!("/v1/providers/{provider}/key"), Some(&b))?;
            if st != 200 {
                bail!("broker: {body}");
            }
            println!("key stored for `{provider}`");
        }
        RemoteCmd::Consent { scope, state } => {
            let allowed = matches!(state.as_str(), "on" | "yes" | "true" | "allow");
            let b = serde_json::json!({"scope": scope, "allowed": allowed}).to_string();
            let (st, body) = broker_request("PUT", "/v1/consent", Some(&b))?;
            if st != 200 {
                bail!("broker: {body}");
            }
            println!(
                "{scope} offload: {}",
                if allowed { "ALLOWED" } else { "denied" }
            );
        }
    }
    Ok(())
}

fn update_cmd(reboot: bool) -> anyhow::Result<()> {
    let sysupdate = std::path::Path::new("/usr/lib/systemd/systemd-sysupdate");
    if !sysupdate.exists() {
        bail!(
            "systemd-sysupdate not found — OS self-update runs on Lisa (Track I) systems; \
             updates are published at https://github.com/Lisa-AgenticOS/lisa-os/releases"
        );
    }
    let status = std::process::Command::new(sysupdate)
        .arg("update")
        .status()?;
    if !status.success() {
        bail!("systemd-sysupdate failed ({status})");
    }
    if reboot {
        std::process::Command::new(sysupdate)
            .arg("reboot")
            .status()?;
    } else {
        println!(
            "update staged in the inactive slot — reboot to use it (rollback is automatic on boot failure)"
        );
    }
    Ok(())
}

fn context_store() -> anyhow::Result<lisa_contextd::ContextStore> {
    let path = std::env::var_os("LISA_CONTEXT_DB")
        .map(PathBuf::from)
        .unwrap_or_else(lisa_contextd::ContextStore::default_path);
    Ok(lisa_contextd::ContextStore::open(path)?)
}

fn context_cmd(cmd: ContextCmd) -> anyhow::Result<()> {
    let store = context_store()?;
    match cmd {
        ContextCmd::Index { dir, embed } => {
            let report = store.index_dir(&dir)?;
            println!(
                "indexed {} file(s) ({} chunks), {} unchanged",
                report.indexed, report.chunks, report.skipped_unchanged
            );
            if embed {
                let n = store.embed_pending(&lisa_contextd::embed::HashEmbedder::default())?;
                println!("embedded {n} new chunk(s) for hybrid search");
            }
        }
        ContextCmd::Search {
            query,
            limit,
            hybrid,
            scope,
        } => {
            let query = query.join(" ");
            // Every retrieval is ledgered (PLAN §5.3) — query hash, not text.
            let ledger = lisa_ledger::Ledger::open(lisa_ledger::Ledger::default_path())?;
            ledger.append(&lisa_ledger::Event {
                kind: if !scope.is_empty() {
                    "context.search.scoped"
                } else if hybrid {
                    "context.search.hybrid"
                } else {
                    "context.search"
                }
                .into(),
                app_id: "host".into(),
                input_hash: blake3::hash(query.as_bytes()).to_hex().to_string(),
                status: "ok".into(),
                ..Default::default()
            })?;
            let hits = if !scope.is_empty() {
                let scopes: Vec<&str> = scope.iter().map(String::as_str).collect();
                store.search_scoped(&query, &scopes, limit)?
            } else if hybrid {
                store.search_hybrid(
                    &query,
                    &lisa_contextd::embed::HashEmbedder::default(),
                    limit,
                )?
            } else {
                store.search(&query, limit)?
            };
            for hit in hits {
                println!(
                    "[{}] {}
    {}",
                    hit.provenance, hit.source, hit.snippet
                );
            }
        }
    }
    Ok(())
}

fn memory_cmd(cmd: MemoryCmd, app: &str) -> anyhow::Result<()> {
    let store = context_store()?;
    match cmd {
        MemoryCmd::Get { key } => match store.memory_get(app, &key)? {
            Some(v) => println!("{v}"),
            None => bail!("no value for `{key}` in namespace `{app}`"),
        },
        MemoryCmd::Set { key, value } => store.memory_set(app, &key, &value)?,
        MemoryCmd::List => {
            for (k, v) in store.memory_list(app)? {
                println!("{k}	{v}");
            }
        }
        MemoryCmd::Wipe { yes } => {
            if !yes {
                eprint!("wipe ALL memory for namespace `{app}`? [y/N] ");
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim(), "y" | "Y" | "yes") {
                    println!("aborted");
                    return Ok(());
                }
            }
            let removed = store.memory_wipe(app)?;
            println!("wiped {removed} key(s) from `{app}`");
        }
    }
    Ok(())
}

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
    // Shared system store: the login user (member of group `lisa`)
    // downloads here and lisa-inferenced reads it (world-readable). Kept
    // OUTSIDE /var/lib/lisa — that's inferenced's private StateDirectory,
    // and a DynamicUser would clash over ownership. Created 2775 root:lisa
    // by tmpfiles.d/lisa-models.conf on the image; on a dev host (no such
    // dir) we fall back to the user's home store.
    let system = PathBuf::from("/var/lib/lisa-models");
    if system.is_dir() {
        return system;
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".local/share/lisa/models"))
        .unwrap_or(system)
}

fn models(cmd: ModelsCmd, store_root: Option<PathBuf>) -> anyhow::Result<()> {
    use serde_json::{Value, json};
    let root = store_root.unwrap_or_else(default_store_root);
    let store = ModelStore::open(&root)?;
    match cmd {
        ModelsCmd::List { json } => {
            let refs = store.list()?;
            if json {
                let installed: Vec<Value> = refs
                    .iter()
                    .map(|r| {
                        json!({
                            "name": r.name,
                            "size_bytes": r.size,
                            "size_gib": r.size as f64 / (1 << 30) as f64,
                            "blake3": r.blake3,
                        })
                    })
                    .collect();
                println!("{}", json!({"installed": installed}));
            } else {
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
        ModelsCmd::Catalog { runnable, json } => {
            use lisa_modeld::recommend::Fit;
            let hw = lisa_modeld::profile::profile();
            let catalog = lisa_modeld::seed_catalog();
            let recs = lisa_modeld::recommend::recommend(&catalog, &hw);
            if json {
                // Installed = a store ref named after the catalog id
                // (how `lisa models get` names it).
                let installed: std::collections::HashSet<String> =
                    store.list()?.into_iter().map(|r| r.name).collect();
                let models: Vec<Value> = recs
                    .iter()
                    .filter(|r| !(runnable && r.fit == Fit::TooBig))
                    .map(|r| {
                        let entry = catalog.models.iter().find(|m| m.id == r.id);
                        json!({
                            "id": r.id,
                            "task": r.task,
                            "license": r.license,
                            "engine": entry.map(|e| e.engine.clone()),
                            "min_ram_gb": r.min_ram_gb,
                            "fit": r.fit,
                            "fit_label": r.fit.label(),
                            "note": r.note,
                            "installed": installed.contains(&r.id),
                            // Downloadable now: a pinned source+hash exists
                            // and it isn't revoked.
                            "available": entry
                                .map(|e| !e.revoked && e.source.is_some() && e.blake3.is_some())
                                .unwrap_or(false),
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    json!({
                        "profile": hw,
                        "models": models,
                    })
                );
                return Ok(());
            }
            println!(
                "your machine: {} GiB RAM, tier {} — local model fit:\n",
                hw.total_ram_gb, hw.tier
            );
            for r in recs {
                if runnable && r.fit == Fit::TooBig {
                    continue;
                }
                let mark = match r.fit {
                    Fit::Runs => "OK  ",
                    Fit::Tight => "TIGHT",
                    Fit::TooBig => "REMOTE",
                };
                println!("  [{mark}] {:<28} {:<10} {}", r.id, r.task, r.fit.label());
            }
            println!(
                "\nBig models that say REMOTE run fine through a provider: \
                 `lisa remote` (HuggingFace, OpenAI, ...)."
            );
        }
        ModelsCmd::Get { id } => {
            let catalog = lisa_modeld::seed_catalog();
            let entry = catalog.models.iter().find(|m| m.id == id).ok_or_else(|| {
                anyhow::anyhow!("no catalog model `{id}` (see `lisa models catalog`)")
            })?;
            if entry.revoked {
                bail!("`{id}` is revoked and must not be installed");
            }
            let (Some(source), Some(hash)) = (&entry.source, &entry.blake3) else {
                bail!("`{id}` has no pinned source yet (catalog entry not finalized)");
            };
            println!(
                "pulling `{id}` ({}) — license: {}",
                entry.task, entry.license
            );
            let e = fetch::pull(&store, source, &id, hash)?;
            let ref_path = store.root().join("refs").join(&e.name);
            println!(
                "installed `{}` ({:.2} GiB) at {}",
                e.name,
                e.size as f64 / (1 << 30) as f64,
                ref_path.display()
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
