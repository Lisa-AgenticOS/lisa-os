//! `lisa` — the one command center (`docs/PLAN.md` §5.4, Appendix E rule 4:
//! everything under `lisa <verb>`, never scattered scripts).
//!
//! M0 surface: `ask` (streams from lisa-inferenced's OpenAI-compat
//! endpoint) and `models` (local store operations via the lisa-modeld
//! library). `tools`/`call`/`undo`/`ledger` are declared now and land with
//! the Agent Bus in M5.

mod agent;
mod apps;
mod guard;
mod skills;
mod terminal;
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
    /// Explain the last failed command (PLAN §5.8 Terminal):
    /// `lisa explain --exit 101 cargo build`, or pipe the output —
    /// `make 2>&1 | lisa explain`. A bare `lisa explain` uses what the
    /// terminal hooks stashed about the last failure.
    Explain {
        /// The command that failed (words joined; flags-first: put
        /// `--exit` before it, the command's own flags pass through).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        /// Its exit code.
        #[arg(long)]
        exit: Option<i32>,
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// Natural language → ONE shell command, printed for review — never
    /// executed (PLAN §5.8; the Ctrl+G terminal hook calls this).
    /// stdout is exactly the command; the explanation goes to stderr.
    Suggest {
        /// What you want, in plain words.
        request: Vec<String>,
        /// Emit the raw {command, explanation} JSON instead.
        #[arg(long)]
        json: bool,
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// Inspect and relax the action guard (ADR-0029, ADR-0030).
    ///
    /// This is the outside of the boundary: you set policy for the model
    /// running on your machine. Nothing the agent can invoke reaches it.
    Guard {
        #[command(subcommand)]
        cmd: GuardCmd,
    },
    /// Manage the local model store (PLAN §5.2).
    Models {
        #[command(subcommand)]
        cmd: ModelsCmd,
        /// Store root; production default is /var/lib/lisa/models.
        #[arg(long, env = "LISA_MODELS_DIR")]
        store: Option<PathBuf>,
    },
    /// Natural language → a confirmed tool action on the Agent Bus:
    /// `lisa do "add milk to my notes"` (ADR-0013 intent router).
    Do {
        /// What you want done, in plain words.
        utterance: Vec<String>,
        /// Inference endpoint for the intent router.
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
        #[arg(long)]
        model: Option<String>,
        /// Print the routed intent and exit without calling the bus.
        #[arg(long)]
        dry_run: bool,
        /// Auto-approve chip-level confirmations (modal ones still refuse).
        #[arg(long)]
        yes: bool,
    },
    /// List tools on the Agent Bus (PLAN §5.4).
    Tools,
    /// Call a tool on the Agent Bus directly:
    /// `lisa call app.lisaos.notes create_note '{"title":"milk"}'`.
    Call {
        /// App id (reverse-DNS, e.g. app.lisaos.notes).
        app_id: String,
        /// Tool name (e.g. create_note).
        tool: String,
        /// Arguments as a JSON object (default {}).
        args: Option<String>,
        /// Auto-approve chip-level confirmations.
        #[arg(long)]
        yes: bool,
    },
    /// Revert the last undoable agent action (PLAN §5.4).
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
    /// Update out-of-image payloads independently of the OS image
    /// (ADR-0020, ADR-0023): fetch, verify, and activate the newest shell
    /// tree and app payloads (Zen browser) — no reboot.
    Apps {
        #[command(subcommand)]
        cmd: AppsCmd,
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
        /// Forge a lisa_ui Flutter app: scaffolds the project (lisa_ui
        /// path dependency, LisaApp stub, smoke test) and verifies with
        /// `flutter analyze`. Needs the flutter SDK (see --setup).
        #[arg(long)]
        flutter: bool,
        /// Provision the pinned Flutter SDK to /var/lib/lisa/flutter
        /// (hash-pinned; Lisa devices, x86_64 + aarch64) and exit.
        #[arg(long)]
        setup: bool,
        /// Build --project for Linux and install it: bundle under the
        /// forge apps dir, plus a .desktop entry so it shows up in the
        /// app grid. No model runs.
        #[arg(long)]
        build: bool,
        /// --build, then launch the app.
        #[arg(long)]
        run: bool,
    },
    /// Skills: the SKILL.md workflows Lisa loads on demand (ADR-0025).
    Skills {
        #[command(subcommand)]
        cmd: SkillsCmd,
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
    /// Print a shell completion script to stdout, e.g.
    /// `lisa completions zsh > ~/.zfunc/_lisa`. Packages install these
    /// at the standard paths — see os/packages/lisa/PKGBUILD.
    Completions {
        /// Target shell (bash, zsh, fish, elvish, powershell).
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum GuardCmd {
    /// Every rule, what it stops, and whether you have relaxed it.
    List,
    /// Relax a rule: it warns instead of refusing. Never silent.
    Allow { rule: String },
    /// Enforce a rule again.
    Forbid { rule: String },
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// The catalog: one `name: description` line per skill.
    List,
    /// Print a skill's full workflow.
    Show { name: String },
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
enum AppsCmd {
    /// Fetch, verify (SHA256SUMS), install, and activate the newest payload
    /// for every channel, or just the named one (`shell`, `zen`).
    Update {
        /// Only this channel.
        channel: Option<String>,
    },
    /// Show the current and installed versions of every payload channel.
    Status,
    /// Flip back to the previously installed tree (or the baked image tree).
    Rollback {
        /// Only this channel.
        channel: Option<String>,
    },
    /// Install payloads the image no longer carries and this system does not
    /// have yet (Zen browser), leaving installed versions untouched. Run by
    /// lisa-apps-sync.timer and by `lisa update` before it stages a slot.
    Sync,
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
        Command::Explain {
            command,
            exit,
            url,
            model,
        } => terminal::explain_cmd(command, exit, &url, model),
        Command::Suggest {
            request,
            json,
            url,
            model,
        } => terminal::suggest_cmd(&request.join(" "), &url, model, json),
        Command::Guard { cmd } => match cmd {
            GuardCmd::List => guard::list_cmd(),
            GuardCmd::Allow { rule } => guard::allow_cmd(&rule),
            GuardCmd::Forbid { rule } => guard::forbid_cmd(&rule),
        },
        Command::Models { cmd, store } => models(cmd, store),
        Command::Do {
            utterance,
            url,
            model,
            dry_run,
            yes,
        } => agent::do_cmd(&utterance.join(" "), &url, model.as_deref(), dry_run, yes),
        Command::Tools => agent::tools_cmd(),
        Command::Call {
            app_id,
            tool,
            args,
            yes,
        } => agent::call_cmd(&app_id, &tool, args.as_deref(), yes),
        Command::Undo => agent::undo_cmd(),
        Command::Ledger { tail, json, db } => ledger_cmd(tail, json, db),
        Command::Embed { text, url } => embed(text, &url),
        Command::Completions { shell } => {
            completions(shell, &mut std::io::stdout());
            Ok(())
        }
        Command::Install { target, from, yes } => install_cmd(&target, from, yes),
        Command::Update { reboot } => update_cmd(reboot),
        Command::Apps { cmd } => match cmd {
            AppsCmd::Update { channel } => apps::update(channel.as_deref()),
            AppsCmd::Status => apps::status(),
            AppsCmd::Rollback { channel } => apps::rollback(channel.as_deref()),
            AppsCmd::Sync => apps::sync(),
        },
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
            flutter,
            setup,
            build,
            run,
        } => {
            if setup {
                forge_setup()
            } else if build || run {
                forge_build(&project, run)
            } else {
                forge_cmd(&task.join(" "), &project, model, &url, max_iters, flutter)
            }
        }
        Command::Skills { cmd } => match cmd {
            SkillsCmd::List => skills::list(),
            SkillsCmd::Show { name } => skills::show(&name),
        },
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

    // Remote (BYO cloud) models go to the egress broker, not the local
    // engine: `lisa ask --model remote:moonshot:kimi-k2 "…"`. The broker
    // (same user) holds the key, enforces per-scope consent, and ledgers
    // the egress — inferenced never gets network.
    if let Some(hint) = body["model"].as_str().map(str::to_owned)
        && let Some((provider, remote_model)) = parse_remote_model(&hint)
    {
        body["model"] = remote_model.into();
        return broker_chat(provider, &body);
    }

    print_chat(url, &body, no_stream)
}

/// POST a chat body to the local endpoint and print the reply — SSE
/// token deltas as they arrive, or in one shot with `no_stream`. The
/// single HTTP path the text verbs share (`ask`, `explain`) — no verb
/// grows HTTP machinery of its own.
pub(crate) fn print_chat(
    url: &str,
    body: &serde_json::Value,
    no_stream: bool,
) -> anyhow::Result<()> {
    if no_stream {
        println!("{}", chat_completion(url, body)?);
        return Ok(());
    }
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let mut response = ureq::post(&endpoint).send_json(body).with_context(|| {
        format!(
            "request to {endpoint} failed — is lisa-inferenced running? \
             Start it with `lisa-inferenced` (or `cargo run -p lisa-inferenced`)"
        )
    })?;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
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
            write!(out, "{}", sanitize_terminal(token))?;
            out.flush()?;
        }
    }
    writeln!(out)?;
    Ok(())
}

/// Strip terminal control characters and whole escape sequences from model
/// output before printing. The model's reply (and, via `lisa explain`,
/// whatever untrusted text was piped in to be explained) must not be able
/// to emit ESC/CSI/OSC sequences, carriage returns, or other C0 controls
/// into the user's terminal — issue #15. Newlines and tabs stay. CSI
/// (`ESC [ … final`) and OSC (`ESC ] … BEL/ST`) are dropped in full so no
/// printable residue like `[31m` leaks through; any other ESC drops with
/// its immediate follower.
pub(crate) fn sanitize_terminal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    // CSI: parameters/intermediates until a final byte @–~.
                    chars.next();
                    for f in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&f) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: until BEL or ST (ESC \). A lone trailing ESC ends it.
                    chars.next();
                    while let Some(f) = chars.next() {
                        if f == '\u{7}' {
                            break;
                        }
                        if f == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next(); // two-char sequence (ESC c, ESC 7, …)
                }
                None => {}
            }
            continue;
        }
        if !c.is_control() || c == '\n' || c == '\t' {
            out.push(c);
        }
    }
    out
}

/// One non-streaming completion against the local endpoint; returns the
/// reply content (`suggest` parses it, `print_chat --no-stream` prints
/// it).
pub(crate) fn chat_completion(url: &str, body: &serde_json::Value) -> anyhow::Result<String> {
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let mut response = ureq::post(&endpoint).send_json(body).with_context(|| {
        format!(
            "request to {endpoint} failed — is lisa-inferenced running? \
             Start it with `lisa-inferenced` (or `cargo run -p lisa-inferenced`)"
        )
    })?;
    let json: serde_json::Value = response.body_mut().read_json()?;
    if let Some(err) = json["error"]["message"].as_str() {
        bail!("inference error: {err}");
    }
    Ok(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}

use std::io::IsTerminal;

pub(crate) const RELEASES_API: &str =
    "https://api.github.com/repos/Lisa-AgenticOS/lisa-os/releases/latest";

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
    if is_block {
        individualize_copied_fsids(target);
    }
    println!(
        ">> wrote {:.1} GiB to {} — remove the USB stick and reboot; \
         first boot grows /var to fill the disk",
        written as f64 / (1u64 << 30) as f64,
        target.display()
    );
    Ok(())
}

/// The byte-copy leaves the target disk with btrfs filesystems whose fsids
/// are identical to the installer USB's — a state btrfs explicitly does not
/// support while both disks are visible (device-scan can associate devices
/// across the copies; issue #16). Regenerate each copied btrfs fsid with
/// `btrfstune -m`, the same tool the nightly A/B test trusts for exactly
/// this. Best-effort: a missing tool degrades to the old behavior plus a
/// loud warning, never a failed install.
fn individualize_copied_fsids(disk: &std::path::Path) {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    let disk_str = disk.to_string_lossy();
    // The kernel still has the pre-copy (empty) partition table cached.
    run("blockdev", &["--rereadpt", &disk_str]);
    run("udevadm", &["settle"]);
    let Some(out) = run("lsblk", &["-nro", "PATH,FSTYPE", &disk_str]) else {
        eprintln!(
            "!! could not enumerate {} — btrfs fsids on it still match the USB; \
             remove the stick before first boot",
            disk_str
        );
        return;
    };
    let mut done = 0;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut cols = line.split_whitespace();
        let (Some(path), Some(fstype)) = (cols.next(), cols.next()) else {
            continue;
        };
        if fstype != "btrfs" {
            continue;
        }
        if run("btrfstune", &["-f", "-m", path]).is_some() {
            done += 1;
        } else {
            eprintln!("!! btrfstune -m {path} failed — its fsid still matches the USB");
        }
    }
    if done > 0 {
        println!(">> individualized {done} btrfs fsid(s) on {disk_str}");
    }
}

/// The pinned Flutter SDK `lisa forge setup` installs on-device (issue
/// #37). Version, URL, and sha256 come from Google's releases manifest
/// (releases_linux.json, re-checked 2026-07-26) — never guessed.
const FLUTTER_SDK_VERSION: &str = "3.44.7";
const FLUTTER_SDK_URL: &str = "https://storage.googleapis.com/flutter_infra_release/releases/stable/linux/flutter_linux_3.44.7-stable.tar.xz";
const FLUTTER_SDK_SHA256: &str = "a0edd646c159c0e816788c0e46a4f071199c1320495898f5a679599b583a05a4";
/// The framework commit 3.44.7 is cut from. Two independent sources agree:
/// the `hash` field of the 3.44.7 entry in Google's releases_linux.json,
/// and the `3.44.7` tag on flutter/flutter (both read 2026-07-26). A git
/// commit id is a hash over the whole tree, so this pins the aarch64
/// install as tightly as the sha256 pins the x86_64 tarball (ADR-0027).
const FLUTTER_SDK_COMMIT: &str = "84fc5cbb223bc12f83d65b647ff8a56caf779ffd";
const FLUTTER_GIT_URL: &str = "https://github.com/flutter/flutter.git";
const FLUTTER_VAR_DIR: &str = "/var/lib/lisa/flutter";

/// How the pinned SDK is obtained for a given CPU architecture.
///
/// Google publishes the convenience **tarball** for linux-x64 only —
/// `releases_linux.json` carries `dart_sdk_arch: x64` and nothing else,
/// and `flutter_linux_arm64_*.tar.xz` is a 404 (checked 2026-07-26). The
/// *artifacts* an arm64 SDK needs do exist under the same pinned engine
/// revision (`dart-sdk-linux-arm64.zip`, `linux-arm64/artifacts.zip`,
/// `linux-arm64-release/linux-arm64-flutter-gtk.zip` — all HTTP 200), so
/// on aarch64 the SDK is a commit-pinned checkout that bootstraps itself
/// from those artifacts. See ADR-0027.
#[derive(Debug, PartialEq, Eq)]
enum FlutterInstall {
    /// sha256-pinned release tarball (linux-x64).
    Tarball {
        url: &'static str,
        sha256: &'static str,
    },
    /// Commit-pinned checkout of the release tag (linux-arm64).
    GitTag {
        url: &'static str,
        tag: &'static str,
        commit: &'static str,
    },
}

/// Pick the install route for `arch` (`std::env::consts::ARCH` values).
/// Unknown architectures refuse rather than guess an artifact URL.
fn flutter_install_plan(arch: &str) -> anyhow::Result<FlutterInstall> {
    match arch {
        "x86_64" => Ok(FlutterInstall::Tarball {
            url: FLUTTER_SDK_URL,
            sha256: FLUTTER_SDK_SHA256,
        }),
        "aarch64" => Ok(FlutterInstall::GitTag {
            url: FLUTTER_GIT_URL,
            tag: FLUTTER_SDK_VERSION,
            commit: FLUTTER_SDK_COMMIT,
        }),
        other => bail!(
            "no pinned Flutter SDK for {other} — Google publishes neither a \
             tarball nor engine artifacts for it (issue #37, ADR-0027); \
             install a Flutter SDK yourself and put it on PATH"
        ),
    }
}

/// Resolve the flutter launcher: PATH first (dev hosts, distro installs),
/// then the on-device /var install `lisa forge setup` creates.
fn flutter_program() -> String {
    let on_path = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join("flutter").is_file()));
    if on_path {
        return "flutter".into();
    }
    let var_bin = std::path::Path::new(FLUTTER_VAR_DIR).join("bin/flutter");
    if var_bin.is_file() {
        return var_bin.to_string_lossy().into_owned();
    }
    "flutter".into() // let the spawn error carry the message
}

/// `lisa forge setup`: fetch the pinned Flutter SDK into /var (ADR-0020
/// spirit — the image stays lean, the durable partition carries the
/// toolchain). Streaming download with mandatory sha256 verification,
/// stage-then-rename so a partial fetch is never visible.
fn forge_setup() -> anyhow::Result<()> {
    let on_path = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join("flutter").is_file()));
    if on_path
        || std::path::Path::new(FLUTTER_VAR_DIR)
            .join("bin/flutter")
            .is_file()
    {
        println!("flutter is already available — nothing to do");
        return Ok(());
    }
    if !cfg!(target_os = "linux") {
        bail!("`lisa forge setup` provisions Lisa devices; install Flutter yourself on dev hosts");
    }
    let plan = flutter_install_plan(std::env::consts::ARCH)?;
    let dest = std::path::Path::new(FLUTTER_VAR_DIR);
    // A leftover dest without bin/flutter would survive the early "already
    // available" check and then fail the final rename after the whole
    // download (#43) — surface it now, and let the user remove it (we
    // never delete a directory we didn't just create).
    if dest.exists() {
        bail!(
            "{} exists but holds no usable SDK (bin/flutter missing) — \
             remove it (sudo rm -r {}) and rerun",
            dest.display(),
            dest.display()
        );
    }
    let parent = dest.parent().expect("FLUTTER_VAR_DIR has a parent");
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "creating {} — run `sudo lisa forge --setup` on a device",
            parent.display()
        )
    })?;
    match plan {
        FlutterInstall::Tarball { url, sha256 } => {
            install_flutter_tarball(parent, dest, url, sha256)?
        }
        FlutterInstall::GitTag { url, tag, commit } => {
            install_flutter_git(parent, dest, url, tag, commit)?
        }
    }
    // Bootstrap in place, after the rename: the first `flutter` run fetches
    // this architecture's Dart SDK (the whole aarch64 story) and caches
    // absolute paths, so it must see its final home.
    flutter_bootstrap(dest);
    println!(
        ">> Flutter {FLUTTER_SDK_VERSION} installed at {FLUTTER_VAR_DIR} — \
         `lisa forge --flutter` will find it automatically"
    );
    Ok(())
}

/// x86_64: stream the pinned tarball, verify sha256, unpack, rename.
fn install_flutter_tarball(
    parent: &std::path::Path,
    dest: &std::path::Path,
    url: &str,
    sha256: &str,
) -> anyhow::Result<()> {
    let part = parent.join("flutter.tar.xz.part");
    println!(">> downloading Flutter {FLUTTER_SDK_VERSION} (~1 GB, sha256-verified)");
    // Everything fallible runs inside; any error cleans up the partial
    // download instead of stranding ~1 GB on the durable partition (#43).
    let staging = parent.join(".flutter-staging");
    let fetch_and_unpack = || -> anyhow::Result<()> {
        use sha2::Digest;
        let mut resp = ureq::get(url)
            .call()
            .context("downloading the Flutter SDK")?;
        let mut reader = resp.body_mut().as_reader();
        let mut file = std::fs::File::create(&part).with_context(|| {
            format!(
                "creating {} — run `sudo lisa forge --setup` on a device",
                part.display()
            )
        })?;
        let mut hasher = sha2::Sha256::new();
        let mut buf = [0u8; 1 << 16];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
        }
        file.sync_all()?;
        let got: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if got != sha256 {
            bail!("sha256 mismatch on the Flutter SDK (got {got}) — refusing to unpack");
        }
        std::fs::create_dir_all(&staging)?;
        let status = std::process::Command::new("tar")
            .arg("-xJf")
            .arg(&part)
            .arg("-C")
            .arg(&staging)
            .status()
            .context("unpacking (tar with xz support required)")?;
        if !status.success() {
            bail!("unpacking the Flutter SDK failed ({status})");
        }
        std::fs::rename(staging.join("flutter"), dest)?;
        Ok(())
    };
    let result = fetch_and_unpack();
    let _ = std::fs::remove_file(&part);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// aarch64: clone the release tag and refuse anything but the pinned
/// commit. There is no arm64 tarball to hash, so the commit id *is* the
/// pin — and it is the same id Google's manifest publishes for 3.44.7
/// (ADR-0027). The SDK then downloads its own arm64 Dart SDK and engine
/// artifacts, which Google does publish, on first run.
fn install_flutter_git(
    parent: &std::path::Path,
    dest: &std::path::Path,
    url: &str,
    tag: &str,
    commit: &str,
) -> anyhow::Result<()> {
    let staging = parent.join(".flutter-staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let work = staging.join("flutter");
    // ~250 MB shallow, ~850 MB once bootstrapped (measured on the pinned tag).
    println!(">> cloning Flutter {tag} (~250 MB, pinned to commit {commit})");
    let clone_and_verify = || -> anyhow::Result<()> {
        let status = std::process::Command::new("git")
            .args(["clone", "--depth", "1", "--branch", tag, url])
            .arg(&work)
            .status()
            .context("running git — the aarch64 SDK install needs it (it ships in the image)")?;
        if !status.success() {
            bail!("git clone of the Flutter SDK failed ({status})");
        }
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["rev-parse", "HEAD"])
            .output()
            .context("reading the cloned Flutter revision")?;
        let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if head != commit {
            bail!("Flutter checkout is {head}, not the pinned {commit} — refusing to install");
        }
        std::fs::rename(&work, dest).context("moving the checkout into place")?;
        Ok(())
    };
    let result = clone_and_verify();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// First run of the freshly installed SDK: fetches this architecture's
/// Dart SDK and the Linux desktop engine artifacts `flutter build linux`
/// needs. Best effort — a device that is offline at this point still has
/// a usable SDK, it just pays the download on the first build.
fn flutter_bootstrap(dest: &std::path::Path) {
    println!(">> bootstrapping the SDK (Dart SDK + Linux desktop engine artifacts)");
    let ok = std::process::Command::new(dest.join("bin/flutter"))
        .args(["precache", "--linux"])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!(
            "!! `flutter precache --linux` did not finish — rerun it (or just \
             `lisa forge --build`, which fetches on demand) once the device is online"
        );
    }
}

/// Locate the lisa_ui package for a forged Flutter app's path dependency:
/// `LISA_UI_PATH` first (dev trees), then the installed location.
fn lisa_ui_path() -> anyhow::Result<PathBuf> {
    if let Some(p) = std::env::var_os("LISA_UI_PATH") {
        let p = PathBuf::from(p);
        if p.join("pubspec.yaml").exists() {
            return Ok(p);
        }
        bail!("LISA_UI_PATH={} has no pubspec.yaml", p.display());
    }
    let installed = PathBuf::from("/usr/share/lisa/lisa_ui");
    if installed.join("pubspec.yaml").exists() {
        return Ok(installed);
    }
    bail!(
        "lisa_ui not found — set LISA_UI_PATH to a lisa_ui checkout \
         (packaged path /usr/share/lisa/lisa_ui is absent)"
    );
}

/// A forged app's Dart package name, derived from the project directory:
/// lowercased, non-alphanumerics folded to `_`, leading digits prefixed —
/// the pub naming rules. It becomes the pubspec `name`, the built binary
/// (CMake `BINARY_NAME`), and the tail of the desktop id, so the app that
/// lands in the app grid is named after what the user asked for instead of
/// every forged app being `lisa_app`.
fn dart_package_name(project: &std::path::Path) -> String {
    let raw = project
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    match out.chars().next() {
        None => "lisa_app".to_string(),
        Some(c) if c.is_ascii_digit() => format!("app_{out}"),
        _ => out,
    }
}

/// Read the `name:` field out of a pubspec (top level, first match) —
/// the built binary and the desktop id follow it.
fn pubspec_name(pubspec: &str) -> Option<String> {
    pubspec.lines().find_map(|l| {
        let rest = l.strip_prefix("name:")?;
        let name = rest.trim();
        (!name.is_empty()).then(|| name.trim_matches(['"', '\'']).to_string())
    })
}

/// Scaffold a runnable lisa_ui Flutter app: pubspec with the lisa_ui path
/// dependency, a LisaApp main stub the analyzer accepts, and one smoke
/// test — so the verifier judges real app code from turn one.
fn scaffold_flutter_app(project: &std::path::Path) -> anyhow::Result<()> {
    let ui = lisa_ui_path()?;
    let pkg = dart_package_name(project);
    std::fs::create_dir_all(project.join("lib"))?;
    std::fs::create_dir_all(project.join("test"))?;
    // Write-if-absent: a rerun after a failed `pub get` (#38) must retry
    // resolution without clobbering files the model (or user) edited.
    let write_if_absent = |path: PathBuf, content: String| -> anyhow::Result<()> {
        if !path.exists() {
            std::fs::write(path, content)?;
        }
        Ok(())
    };
    write_if_absent(
        project.join("pubspec.yaml"),
        format!(
            "name: {pkg}\ndescription: An app forged by LisaCode.\n\
             publish_to: none\nversion: 0.1.0\n\
             environment:\n  sdk: ^3.9.0\n\
             dependencies:\n  flutter:\n    sdk: flutter\n  lisa_ui:\n    path: {}\n\
             dev_dependencies:\n  flutter_test:\n    sdk: flutter\n",
            ui.display()
        ),
    )?;
    write_if_absent(
        project.join("lib/main.dart"),
        "import 'package:lisa_ui/lisa_ui.dart';\n\n\
         void main() {\n  runApp(\n    const LisaApp(\n      title: 'Lisa App',\n      \
         home: LisaScaffold(\n        title: 'Lisa App',\n        \
         body: Center(child: Text('Forged by LisaCode')),\n      ),\n    ),\n  );\n}\n"
            .into(),
    )?;
    write_if_absent(
        project.join("test/smoke_test.dart"),
        format!(
            "import 'package:flutter_test/flutter_test.dart';\nimport 'package:{pkg}/main.dart' as app;\n\n\
             void main() {{\n  testWidgets('app builds', (tester) async {{\n    app.main();\n    \
             await tester.pump();\n  }});\n}}\n"
        ),
    )?;
    let status = std::process::Command::new(flutter_program())
        .args(["pub", "get"])
        .current_dir(project)
        .status()
        .context("running flutter pub get (is the flutter SDK installed?)")?;
    if !status.success() {
        bail!("flutter pub get failed in {}", project.display());
    }
    Ok(())
}

fn forge_cmd(
    task: &str,
    project: &PathBuf,
    model: Option<String>,
    url: &str,
    max_iters: usize,
    flutter: bool,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(project)?;
    let verifier = if flutter {
        // "Scaffolded" means pub get SUCCEEDED (its package_config is the
        // marker) — gating on pubspec existence let a failed pub get leave
        // a half-scaffold that reruns silently skipped (#38).
        let pubspec = project.join("pubspec.yaml");
        if pubspec.exists()
            && !std::fs::read_to_string(&pubspec).is_ok_and(|p| p.contains("sdk: flutter"))
        {
            bail!(
                "{} already holds a non-Flutter project — pick a fresh --project \
                 directory for --flutter",
                project.display()
            );
        }
        if !project.join(".dart_tool/package_config.json").exists() {
            scaffold_flutter_app(project)?;
            println!(
                ">> scaffolded a lisa_ui Flutter app in {}",
                project.display()
            );
        }
        forge_harness::Verifier::Command {
            program: flutter_program(),
            args: vec!["analyze".into(), "--no-pub".into()],
        }
    } else {
        let pubspec = project.join("pubspec.yaml");
        if !pubspec.exists() {
            std::fs::write(
                &pubspec,
                format!(
                    "name: {}\ndescription: An app forged by LisaCode.\nenvironment:\n  sdk: ^3.0.0\n",
                    dart_package_name(project)
                ),
            )?;
            std::fs::create_dir_all(project.join("bin"))?;
        }
        forge_harness::Verifier::Dart
    };
    println!("LisaCode: building \"{task}\" in {}", project.display());
    let mut backend = forge_harness::OpenAiBackend {
        url: url.to_string(),
        model,
    };
    // Issue #54: the forge loop edits your files unattended, so it is
    // exactly the thing VISION.md's "every action it took is in the
    // Ledger" has to be true of. Opening it here also means a machine
    // with an unwritable Ledger refuses to forge rather than acting off
    // the record.
    let ledger = std::sync::Arc::new(lisa_ledger::Ledger::open(
        lisa_ledger::Ledger::default_path(),
    )?);
    let config = forge_harness::AgentConfig {
        max_turns: max_iters.saturating_mul(8).max(8),
        verifier,
        ledger: Some(ledger),
        ..Default::default()
    };
    // Narrate the loop live — an agent you can watch, not a spinner.
    let mut observe = |ev: forge_harness::AgentEvent| {
        use forge_harness::AgentEvent as E;
        match ev {
            E::Turn { n, max } => eprintln!("[turn {n}/{max}]"),
            E::Call { name, detail } => eprintln!("  · {name} {detail}"),
            E::CallResult { ok, chars } => {
                if !ok {
                    eprintln!("    ! tool error ({chars} chars)");
                }
            }
            E::VerifierFindings { chars } => eprintln!("  ! verifier findings ({chars} chars)"),
            E::VerifierClean => eprintln!("  ✓ verifier clean"),
            E::DoneClaimed => eprintln!("  ∴ model claims done — checking"),
        }
    };
    match forge_harness::forge_agent_observed(task, project, &mut backend, &config, &mut observe) {
        Ok(report) => {
            println!(
                "converged in {} turn(s) — verifier clean. Source in {}",
                report.turns,
                project.display()
            );
            if flutter {
                println!(
                    "next: `lisa forge --run --project {}` builds it and puts it in the app grid",
                    project.display()
                );
            }
            Ok(())
        }
        Err(e) => bail!("forge did not finish: {e}"),
    }
}

// ---------------------------------------------------------------------
// `lisa forge --build` / `--run`: from source to an app in the grid.
// ---------------------------------------------------------------------

/// Reverse-DNS org for locally forged apps (ADR-0016 app namespace). The
/// Flutter Linux template turns `--org X --project-name p` into
/// `APPLICATION_ID = X.p`, and `my_application.cc` calls
/// `g_set_prgname(APPLICATION_ID)` — so this is simultaneously the GTK
/// app id, the WM class, and the `.desktop` basename. Keeping the three
/// equal is what makes GNOME match the window to its launcher entry.
const FORGED_APP_ORG: &str = "app.lisaos.forge";

/// Durable, non-image home for forged app bundles (ADR-0023: the image
/// carries the OS contract, /var carries what the user grows). Same
/// shape as `default_store_root`: the group-writable system dir when the
/// device has one (tmpfiles.d/lisa-forge.conf), else the user's own data
/// dir — which on Lisa is its own partition too (ADR-0019), so the image
/// stays slim either way.
fn forge_apps_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("LISA_FORGE_APPS_DIR") {
        return PathBuf::from(p);
    }
    let system = PathBuf::from("/var/lib/lisa/forge/apps");
    if system.is_dir() {
        return system;
    }
    user_data_dir().join("lisa/forge/apps")
}

/// `$XDG_DATA_HOME`, or the spec default `~/.local/share`.
fn user_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|h| h.join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The `.desktop` file a forged app is launched by. Names and ids all
/// derive from the app id so GNOME can tie window → icon → entry.
fn desktop_entry(app_id: &str, name: &str, exec: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         Comment=Forged on this device by LisaCode\n\
         Exec=\"{}\"\n\
         Icon=application-x-executable\n\
         Terminal=false\n\
         Categories=Utility;\n\
         StartupNotify=true\n\
         StartupWMClass={app_id}\n\
         X-Lisa-Forged=true\n",
        exec.display()
    )
}

/// `tip_calc` → `Tip Calc`: the human-facing name in the app grid.
fn display_name(pkg: &str) -> String {
    pkg.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read `APPLICATION_ID` back out of a generated `linux/CMakeLists.txt` —
/// on a rebuild the runner already exists, and the id baked into it is the
/// truth, not whatever we would generate now.
fn cmake_application_id(cmake: &str) -> Option<String> {
    cmake.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("set(APPLICATION_ID")?;
        let inner = rest.trim().trim_end_matches(')').trim();
        Some(inner.trim_matches('"').to_string()).filter(|s| !s.is_empty())
    })
}

/// `flutter build linux` writes to `build/linux/<arch>/<mode>/bundle`.
/// Prefer the host arch's directory, then accept any that exists — the
/// arch spelling is the SDK's, not ours.
fn built_bundle(project: &std::path::Path) -> Option<PathBuf> {
    let host = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let root = project.join("build/linux");
    let candidate = |arch: &str| {
        let p = root.join(arch).join("release/bundle");
        p.is_dir().then_some(p)
    };
    candidate(host).or_else(|| {
        let mut found: Vec<PathBuf> = std::fs::read_dir(&root)
            .ok()?
            .flatten()
            .filter_map(|e| candidate(&e.file_name().to_string_lossy()))
            .collect();
        found.sort();
        found.pop()
    })
}

/// Recursive copy, permissions included (`fs::copy` carries the mode, so
/// the built executable stays executable).
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)
                .with_context(|| format!("copying {} → {}", src.display(), dst.display()))?;
        }
    }
    Ok(())
}

/// Install a built bundle as `<apps dir>/<app id>/bundle`, staging then
/// renaming so a half-copied tree is never launchable, and keeping the
/// previous build beside it as this payload's own rollback (ADR-0023
/// delivery rule 2). Returns the installed executable.
fn install_forged_bundle(
    app_id: &str,
    exe_name: &str,
    src: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let root = forge_apps_dir().join(app_id);
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    let staging = root.join(format!(".stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let result = copy_tree(src, &staging);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result?;
    let live = root.join("bundle");
    let previous = root.join("bundle.previous");
    if live.exists() {
        let _ = std::fs::remove_dir_all(&previous);
        std::fs::rename(&live, &previous).context("rotating the previous build out of the way")?;
    }
    std::fs::rename(&staging, &live).context("moving the new build into place")?;
    Ok(live.join(exe_name))
}

/// `flutter build linux` needs the CMake/GTK runner under `linux/`, which
/// the forge scaffold deliberately does not hand-write — it comes from the
/// SDK's own template. Generated into a scratch directory and copied in,
/// so an existing `lib/`, pubspec or test can never be clobbered.
fn ensure_linux_runner(project: &std::path::Path, pkg: &str) -> anyhow::Result<()> {
    if project.join("linux/CMakeLists.txt").is_file() {
        return Ok(());
    }
    let scratch = std::env::temp_dir().join(format!("lisa-forge-runner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    println!(">> generating the Linux runner (flutter create --platforms=linux)");
    let generate = || -> anyhow::Result<()> {
        let status = std::process::Command::new(flutter_program())
            .args([
                "create",
                "--platforms=linux",
                "--template=app",
                "--no-pub",
                "--org",
                FORGED_APP_ORG,
                "--project-name",
                pkg,
            ])
            .arg(&scratch)
            .status()
            .context(
                "running flutter create (is the flutter SDK installed? see `lisa forge --setup`)",
            )?;
        if !status.success() {
            bail!("flutter create failed ({status})");
        }
        copy_tree(&scratch.join("linux"), &project.join("linux"))
    };
    let result = generate();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// `lisa forge --build` / `--run`: compile the forged app for Linux and
/// install it where the shell can launch it. No model runs here — this is
/// the step that turns source into something you can click.
fn forge_build(project: &std::path::Path, launch: bool) -> anyhow::Result<()> {
    let pubspec_path = project.join("pubspec.yaml");
    let pubspec = std::fs::read_to_string(&pubspec_path).with_context(|| {
        format!(
            "{} is not a Flutter project — forge one first with \
             `lisa forge --flutter --project {}`",
            project.display(),
            project.display()
        )
    })?;
    if !pubspec.contains("sdk: flutter") {
        bail!(
            "{} is a plain Dart project — only Flutter apps have a Linux build",
            project.display()
        );
    }
    let pkg = pubspec_name(&pubspec)
        .with_context(|| format!("{} has no `name:` field", pubspec_path.display()))?;
    ensure_linux_runner(project, &pkg)?;
    let app_id = std::fs::read_to_string(project.join("linux/CMakeLists.txt"))
        .ok()
        .and_then(|c| cmake_application_id(&c))
        .unwrap_or_else(|| format!("{FORGED_APP_ORG}.{pkg}"));

    println!(">> flutter build linux --release ({})", project.display());
    let status = std::process::Command::new(flutter_program())
        .args(["build", "linux", "--release"])
        .current_dir(project)
        .status()
        .context("running flutter build linux (see `lisa forge --setup`)")?;
    if !status.success() {
        bail!(
            "flutter build linux failed in {} — it runs on a Linux host only, and \
             needs clang, cmake, ninja, pkg-config and gtk3 installed there",
            project.display()
        );
    }
    let bundle = built_bundle(project).with_context(|| {
        format!(
            "the build produced no bundle under {}/build/linux",
            project.display()
        )
    })?;
    let exe = install_forged_bundle(&app_id, &pkg, &bundle)?;

    let name = display_name(&pkg);
    let apps = user_data_dir().join("applications");
    std::fs::create_dir_all(&apps).with_context(|| format!("creating {}", apps.display()))?;
    let entry = apps.join(format!("{app_id}.desktop"));
    std::fs::write(&entry, desktop_entry(&app_id, &name, &exe))
        .with_context(|| format!("writing {}", entry.display()))?;
    println!(
        ">> installed {} — \"{name}\" is in the app grid ({})",
        exe.display(),
        entry.display()
    );

    if launch {
        std::process::Command::new(&exe)
            .spawn()
            .with_context(|| format!("launching {}", exe.display()))?;
        println!(">> launched {name}");
    }
    Ok(())
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

/// The broker's unix socket. LISA_REMOTED_SOCKET wins; otherwise prefer the
/// per-user runtime socket the user unit binds ($XDG_RUNTIME_DIR/lisa/…),
/// falling back to the legacy system path for a system-scope broker.
fn remoted_socket() -> PathBuf {
    if let Some(s) = std::env::var_os("LISA_REMOTED_SOCKET") {
        return PathBuf::from(s);
    }
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(rt).join("lisa/remoted.sock");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("/var/lib/lisa/remoted/remoted.sock")
}

/// Split a `remote:<provider>:<model>` hint into (provider, model). The
/// model tail may itself contain colons/slashes (openrouter vendor/model,
/// tinker:// URIs), so only the first two segments are fixed.
fn parse_remote_model(model: &str) -> Option<(&str, &str)> {
    let rest = model.strip_prefix("remote:")?;
    let (provider, tail) = rest.split_once(':')?;
    if provider.is_empty() || tail.is_empty() {
        return None;
    }
    Some((provider, tail))
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

/// Route a chat request to the remote-provider broker over its unix socket
/// (non-streaming for now). `body["model"]` must already be the
/// provider-side id (the `remote:<provider>:` prefix stripped). The broker
/// enforces `prompt` consent + ledgers the egress; `provider` rides the
/// `x-lisa-provider` header, matching `lisa-remoted`'s `/v1/chat/completions`.
fn broker_chat(provider: &str, body: &serde_json::Value) -> anyhow::Result<()> {
    use std::os::unix::net::UnixStream;
    let mut body = body.clone();
    body["stream"] = false.into(); // streaming over the socket is a follow-up
    let payload = body.to_string();
    let sock = remoted_socket();
    let mut stream = UnixStream::connect(&sock).with_context(|| {
        format!(
            "lisa-remoted socket {} — is the broker running? \
             (systemctl --user start lisa-remoted)",
            sock.display()
        )
    })?;
    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: lisa-remoted\r\n\
         Content-Type: application/json\r\nx-lisa-provider: {provider}\r\n\
         x-lisa-scopes: prompt\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    );
    stream.write_all(req.as_bytes())?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let (head, resp) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("malformed broker response"))?;
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let json: serde_json::Value =
        serde_json::from_str(resp.trim()).unwrap_or_else(|_| serde_json::json!({}));
    if status >= 400 || json.get("error").is_some() {
        let msg = json["error"]["message"]
            .as_str()
            .or_else(|| json["error"].as_str())
            .unwrap_or_else(|| resp.trim());
        bail!(
            "remote provider {provider}: {msg}\n\
             (add an API key and enable the 'prompt' scope in \
             Settings › Intelligence › Providers)"
        );
    }
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    println!("{content}");
    Ok(())
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

/// Defense-in-depth for the issue-#20 class: sysupdate's vacuum evicts the
/// oldest instance of each transfer and never checks what backs `/` —
/// `ProtectVersion=%A` in every transfer is the only thing standing between
/// an update and erasing the booted slot. Refuse to update through a config
/// that lost that guard. `/etc/sysupdate.d` overrides same-named files in
/// `/usr/lib/sysupdate.d`, so collect with that precedence.
fn assert_transfers_protect_booted() -> anyhow::Result<()> {
    let mut transfers: std::collections::BTreeMap<String, PathBuf> = Default::default();
    for dir in ["/usr/lib/sysupdate.d", "/etc/sysupdate.d"] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "transfer")
                && let Some(name) = path.file_name()
            {
                transfers.insert(name.to_string_lossy().into_owned(), path);
            }
        }
    }
    let unguarded: Vec<String> = transfers
        .into_iter()
        .filter(|(_, path)| {
            std::fs::read_to_string(path).is_ok_and(|text| {
                !text
                    .lines()
                    .any(|l| l.trim().starts_with("ProtectVersion="))
            })
        })
        .map(|(name, _)| name)
        .collect();
    if !unguarded.is_empty() {
        if std::env::var_os("LISA_UPDATE_ALLOW_UNPROTECTED").is_some() {
            eprintln!(
                "!! proceeding despite unguarded transfer config ({}) — \
                 LISA_UPDATE_ALLOW_UNPROTECTED is set",
                unguarded.join(", ")
            );
            return Ok(());
        }
        bail!(
            "refusing to update: transfer config without ProtectVersion= ({}) — \
             sysupdate's vacuum could erase the slot this system is booted from \
             (issue #20). v27+ images ship the guard; to update anyway (e.g. to \
             reach the fixed release from an old image), set \
             LISA_UPDATE_ALLOW_UNPROTECTED=1.",
            unguarded.join(", ")
        );
    }
    Ok(())
}

/// Stage an update through systemd-sysupdated's D-Bus surface
/// (`org.freedesktop.sysupdate1`, systemd ≥257). Unlike execing
/// systemd-sysupdate — root-only, because the ESP and partition writes
/// need privilege — this path is polkit-mediated, so a desktop user (the
/// Settings Update button, issue #19) gets an auth prompt instead of a
/// permission error. Returns Ok(false) when the service isn't on the bus
/// (older image, no sysupdated build) — the caller falls back to the
/// exec path. A real update failure is an error: falling back would just
/// fail again with a worse message.
fn update_via_sysupdated(reboot: bool) -> anyhow::Result<bool> {
    use zbus::blocking::{Connection, Proxy};

    let Ok(conn) = Connection::system() else {
        return Ok(false);
    };
    let manager: Proxy = match Proxy::new(
        &conn,
        "org.freedesktop.sysupdate1",
        "/org/freedesktop/sysupdate1",
        "org.freedesktop.sysupdate1.Manager",
    ) {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    // Subscribe before starting the job so completion can't race us.
    let mut removals = match manager.receive_signal("JobRemoved") {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };
    let targets: Vec<(String, String, zbus::zvariant::OwnedObjectPath)> =
        match manager.call("ListTargets", &()) {
            Ok(t) => t,
            // Name exists but the call failed → service not usable here.
            Err(_) => return Ok(false),
        };
    let Some((_, name, path)) = targets.into_iter().find(|(class, _, _)| class == "host") else {
        return Ok(false);
    };

    let target: Proxy = Proxy::new(
        &conn,
        "org.freedesktop.sysupdate1",
        path,
        "org.freedesktop.sysupdate1.Target",
    )?;
    println!(">> staging via systemd-sysupdated (target {name})");

    // JobRemoved(t id, o path, i status): status 0 = success. Subscribed
    // before any job starts so completion cannot race us; one reader
    // thread serves every job this run starts. The wait is bounded, and
    // a dead bus is an error, never success (#31) — with --reboot a
    // false success would reboot on unfinished staging.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for signal in removals.by_ref() {
            if let Ok(parsed) = signal
                .body()
                .deserialize::<(u64, zbus::zvariant::OwnedObjectPath, i32)>()
                && tx.send(parsed).is_err()
            {
                return;
            }
        }
        // Iterator exhausted: the connection died. Dropping tx reports it.
    });

    // Without AllowInteractiveAuth, polkit can only consult static policy
    // — it never shows the auth prompt this whole path exists for (#32).
    let interactive = zbus::proxy::MethodFlags::AllowInteractiveAuth.into();

    let wait_for = |job: zbus::zvariant::OwnedObjectPath, what: &str| -> anyhow::Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60 * 60);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(remaining) {
                Ok((_, done_path, _)) if done_path != job => continue,
                Ok((_, _, 0)) => return Ok(()),
                Ok((_, _, status)) => {
                    bail!("sysupdated {what} job failed (status {status})")
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => bail!(
                    "timed out waiting for the sysupdated {what} job (60 min) — \
                     check `journalctl -u systemd-sysupdated`"
                ),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => bail!(
                    "bus connection lost while waiting for the sysupdated {what} job — \
                     staging state unknown; check `journalctl -u systemd-sysupdated`"
                ),
            }
        }
    };

    type JobReply = (String, u64, zbus::zvariant::OwnedObjectPath);
    let call = |member: &str, version: &str| -> zbus::Result<Option<JobReply>> {
        target.call_with_flags(member, interactive, &(version, 0u64))
    };

    // systemd 261 replaced the one-shot `Target.Update` with `Acquire`
    // (fetch into the inactive slot) followed by `Install` (make it
    // bootable). On such a system `Update` does not merely require
    // privilege — it does not exist, and because the bus policy denies
    // unknown members by default the call comes back as
    //
    //     org.freedesktop.DBus.Error.AccessDenied:
    //     Sender is not authorized to send message
    //
    // which reads as a permission problem and is not one. That is why
    // the Settings Update button appeared to do nothing (#19): polkit
    // already grants `sysupdate1.update` to the active local session
    // (allow_active=yes), so there was never a prompt to miss.
    let acquired = match call("Acquire", "") {
        Ok(Some(reply)) => Some(reply),
        Ok(None) => bail!("sysupdated returned no reply to Acquire"),
        Err(zbus::Error::MethodError(ref err_name, ref msg, _))
            if err_name.as_str().ends_with("NoCandidate") =>
        {
            println!(
                "already up to date{}",
                msg.as_deref()
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default()
            );
            return Ok(true);
        }
        // Older sysupdated (systemd < 261). The CLI ships through the
        // runtime channel independently of the image, so a new binary
        // can land on an older system.
        Err(zbus::Error::MethodError(ref err_name, _, _))
            if err_name.as_str().ends_with("UnknownMethod") =>
        {
            None
        }
        Err(e) => bail!("sysupdated refused to acquire the update: {e}"),
    };

    let version = match acquired {
        Some((version, _, job)) => {
            println!(">> downloading {version} into the inactive slot…");
            wait_for(job, "acquire")?;
            let (_, _, job) = match call("Install", &version) {
                Ok(Some(r)) => r,
                Ok(None) => bail!("sysupdated returned no reply to Install"),
                Err(e) => bail!("sysupdated refused to install {version}: {e}"),
            };
            println!(">> installing {version}…");
            wait_for(job, "install")?;
            version
        }
        None => {
            let (version, _, job) = match call("Update", "") {
                Ok(Some(r)) => r,
                Ok(None) => bail!("sysupdated returned no reply to Update"),
                Err(e) => bail!("sysupdated refused the update: {e}"),
            };
            println!(">> update to {version} started — waiting for the job to finish");
            wait_for(job, "update")?;
            version
        }
    };
    let _ = &version;
    if reboot {
        let login1: Proxy = Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        login1
            .call_with_flags::<_, _, ()>("Reboot", interactive, &(false))?
            .unwrap_or(());
    } else {
        println!(
            "update staged in the inactive slot — reboot to use it (rollback is automatic on boot failure)"
        );
    }
    Ok(true)
}

/// `systemd-run` argv that stages the update inside its own transient
/// unit instead of as a child of the caller's shell.
///
/// Why this is not a nicety (issue #45): sysupdate relabels and retypes
/// the TARGET partition before it downloads a single byte
/// (systemd src/sysupdate/sysupdate-transfer.c, transfer_acquire_instance:
/// "Set the partition label and change the partition type to the derived
/// 'partial' type UUID"). A staging run killed part way therefore leaves a
/// slot advertising the new version's PARTLABEL over old or half-written
/// bytes. On the field iMac exactly that happened — a debug rerun died
/// with the SSH session and both root slots stopped switch-rooting. A
/// transient unit is not in the login session's scope, so a dropped
/// connection, a Ctrl-C or a closed lid cannot SIGHUP the transfer any
/// more; `--wait` only makes *us* wait for it.
///
/// Progress goes to the journal rather than this terminal — the follow
/// command is printed, and it is the same journal that carries the
/// `SYSTEMD_LOG_LEVEL=debug` detail when something goes wrong.
fn staging_unit_argv(unit: &str, sysupdate: &str, debug: bool) -> Vec<String> {
    let mut argv = vec![
        format!("--unit={unit}"),
        "--collect".into(),
        "--wait".into(),
        "--service-type=oneshot".into(),
        "--description=Lisa OS update staging".into(),
    ];
    if debug {
        // The ONLY place the real reason for "Failed to allocate puller:
        // Operation not supported" is written: systemd logs the underlying
        // dlopen error at debug level ("Shared library 'libcurl.so.4' is
        // not available: …", src/basic/dlfcn-util.c) and returns a bare
        // EOPNOTSUPP to its caller.
        argv.push("--setenv=SYSTEMD_LOG_LEVEL=debug".into());
    }
    argv.push(sysupdate.into());
    argv.push("update".into());
    argv
}

/// What to tell an operator whose staging run just failed. The two things
/// that cost the field device a USB reinstall were (a) not knowing that a
/// failed run leaves the target slot mislabelled and (b) rerunning the
/// diagnosis in a foreground SSH session that then dropped.
fn staging_failure_help(unit: &str) -> String {
    format!(
        "the target slot may be mid-transfer: sysupdate relabels and retypes it \
         BEFORE downloading, so its PARTLABEL can now advertise the new version \
         over stale bytes (issue #45). Do NOT reboot into it and do NOT interrupt \
         a retry.\n  logs:  journalctl -u {unit} --no-pager\n  \
         retry with full detail, detached from this session (survives an SSH drop):\n    \
         sudo systemd-run --unit=lisa-update-debug --collect \
         --setenv=SYSTEMD_LOG_LEVEL=debug \
         /usr/lib/systemd/systemd-sysupdate update\n    \
         journalctl -fu lisa-update-debug\n  \
         a line of the form \"Shared library '…' is not available: …\" names the \
         real cause of a puller failure; nothing else does."
    )
}

fn update_cmd(reboot: bool) -> anyhow::Result<()> {
    let sysupdate = std::path::Path::new("/usr/lib/systemd/systemd-sysupdate");
    if !sysupdate.exists() {
        bail!(
            "systemd-sysupdate not found — OS self-update runs on Lisa (Track I) systems; \
             updates are published at https://github.com/Lisa-AgenticOS/lisa-os/releases"
        );
    }
    assert_transfers_protect_booted()?;
    // ADR-0023 phase 1: the slot we are about to stage may not carry
    // payloads this one does — the Zen browser left the image and now
    // arrives through the apps channel. Pull what is missing onto the
    // PERSISTENT /var first, while the current slot's baked copy is still
    // there to fall back on, so the reboot never lands on a system whose
    // browser is gone. Best-effort on purpose: an unreachable app channel
    // must not block an OS security update, but it must be loud, because
    // the silent version of this failure is a user who reboots into a
    // desktop with no browser.
    if let Err(e) = apps::sync() {
        eprintln!(
            "!! could not pre-fetch app payloads: {e:#}\n\
             !! the new slot may boot without them — run `sudo lisa apps sync` \
             once online, before or after rebooting"
        );
    }
    // Preferred: the polkit-mediated D-Bus path (works unprivileged). Its
    // work runs inside systemd-sysupdated.service, so it already survives a
    // dropped session.
    match update_via_sysupdated(reboot) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => {
            eprintln!("!! {}", staging_failure_help("systemd-sysupdated"));
            return Err(e);
        }
    }
    // Fallback: direct exec — needs root, kept for images without
    // sysupdated. Staged inside a transient unit so an interrupted session
    // cannot leave a slot half written (issue #45); if systemd-run is not
    // usable we still run it directly rather than refuse to update.
    let sysupdate_str = sysupdate.to_string_lossy().into_owned();
    let unit = format!("lisa-update-{}", std::process::id());
    let debug = std::env::var_os("LISA_UPDATE_DEBUG").is_some();
    println!(">> staging in transient unit {unit} — follow with `journalctl -fu {unit}`");
    let status = match std::process::Command::new("systemd-run")
        .args(staging_unit_argv(&unit, &sysupdate_str, debug))
        .status()
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "!! systemd-run unavailable ({e}) — staging in this session instead; \
                       do not interrupt it"
            );
            std::process::Command::new(sysupdate)
                .arg("update")
                .status()?
        }
    };
    if !status.success() {
        bail!(
            "systemd-sysupdate failed ({status}) — {}",
            staging_failure_help(&unit)
        );
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

/// Generate a completion script for `shell` into `out` (issue #10).
/// Split from the match arm so tests can capture the output in-process.
fn completions(shell: clap_complete::Shell, out: &mut dyn Write) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "lisa", out);
}

#[cfg(test)]
mod completions_tests {
    use clap::CommandFactory;

    #[test]
    fn subcommand_exists() {
        assert!(
            super::Cli::command()
                .find_subcommand("completions")
                .is_some(),
            "`lisa completions` must exist"
        );
    }

    #[test]
    fn zsh_script_is_nonempty_and_names_the_function() {
        let mut buf = Vec::new();
        super::completions(clap_complete::Shell::Zsh, &mut buf);
        let script = String::from_utf8(buf).expect("zsh completions are UTF-8");
        assert!(!script.is_empty());
        assert!(script.contains("_lisa"), "zsh script defines _lisa");
    }
}

#[cfg(test)]
mod update_staging_tests {
    use super::{staging_failure_help, staging_unit_argv};

    #[test]
    fn staging_runs_detached_in_its_own_unit() {
        let argv = staging_unit_argv("lisa-update-7", "/usr/lib/systemd/systemd-sysupdate", false);
        // The unit is the point: a transient unit outlives the login
        // session, so an SSH drop cannot kill a partition write half way
        // through (issue #45).
        assert_eq!(argv[0], "--unit=lisa-update-7");
        assert!(argv.iter().any(|a| a == "--wait"));
        assert_eq!(
            argv[argv.len() - 2..],
            ["/usr/lib/systemd/systemd-sysupdate", "update"]
        );
        // Debug detail is opt-in and must not leak into normal runs.
        assert!(!argv.iter().any(|a| a.contains("SYSTEMD_LOG_LEVEL")));
    }

    #[test]
    fn debug_staging_asks_for_the_only_log_level_that_names_the_cause() {
        let argv = staging_unit_argv("u", "/s", true);
        assert!(argv.iter().any(|a| a == "--setenv=SYSTEMD_LOG_LEVEL=debug"));
        assert_eq!(argv[argv.len() - 2..], ["/s", "update"]);
    }

    #[test]
    fn failure_help_names_the_unit_and_warns_about_the_half_labelled_slot() {
        let help = staging_failure_help("lisa-update-7");
        assert!(help.contains("journalctl -u lisa-update-7"));
        assert!(help.contains("mid-transfer"));
        assert!(help.contains("Shared library"));
        assert!(help.contains("systemd-run"));
    }
}

#[cfg(test)]
mod forge_tests {
    use super::*;

    #[test]
    fn every_supported_arch_has_a_pinned_artifact_and_the_rest_refuse() {
        // x86_64 keeps the sha256-pinned tarball Google publishes.
        assert_eq!(
            flutter_install_plan("x86_64").unwrap(),
            FlutterInstall::Tarball {
                url: FLUTTER_SDK_URL,
                sha256: FLUTTER_SDK_SHA256,
            }
        );
        // aarch64 has no tarball at all, so the pin is the release commit —
        // the same id Google's manifest publishes for this version.
        assert_eq!(
            flutter_install_plan("aarch64").unwrap(),
            FlutterInstall::GitTag {
                url: FLUTTER_GIT_URL,
                tag: FLUTTER_SDK_VERSION,
                commit: FLUTTER_SDK_COMMIT,
            }
        );
        assert_eq!(FLUTTER_SDK_COMMIT.len(), 40);
        // Anything else refuses rather than guessing a URL (CLAUDE.md rule 8).
        let err = flutter_install_plan("riscv64").unwrap_err().to_string();
        assert!(err.contains("riscv64"), "{err}");
        assert!(err.contains("issue #37"), "{err}");
    }

    #[test]
    fn package_names_follow_the_project_directory_within_pub_rules() {
        assert_eq!(
            dart_package_name(std::path::Path::new("./tip-calc")),
            "tip_calc"
        );
        assert_eq!(
            dart_package_name(std::path::Path::new("/x/My App!")),
            "my_app"
        );
        // A leading digit is not a legal Dart identifier.
        assert_eq!(dart_package_name(std::path::Path::new("2048")), "app_2048");
        assert_eq!(dart_package_name(std::path::Path::new("---")), "lisa_app");
    }

    #[test]
    fn pubspec_name_and_display_name_round_trip() {
        let pubspec = "name: tip_calc\ndescription: An app forged by LisaCode.\n";
        assert_eq!(pubspec_name(pubspec).as_deref(), Some("tip_calc"));
        assert_eq!(pubspec_name("description: x\n"), None);
        assert_eq!(display_name("tip_calc"), "Tip Calc");
    }

    #[test]
    fn the_application_id_is_read_back_from_the_generated_runner() {
        // Verbatim shape of flutter's linux/CMakeLists.txt (SDK 3.44.7).
        let cmake = "cmake_minimum_required(VERSION 3.13)\n\
                     set(BINARY_NAME \"tip_calc\")\n\
                     set(APPLICATION_ID \"app.lisaos.forge.tip_calc\")\n";
        assert_eq!(
            cmake_application_id(cmake).as_deref(),
            Some("app.lisaos.forge.tip_calc")
        );
        assert_eq!(cmake_application_id("set(BINARY_NAME \"x\")\n"), None);
    }

    #[test]
    fn the_desktop_entry_ties_window_icon_and_launcher_together() {
        let entry = desktop_entry(
            "app.lisaos.forge.tip_calc",
            "Tip Calc",
            std::path::Path::new(
                "/var/lib/lisa/forge/apps/app.lisaos.forge.tip_calc/bundle/tip_calc",
            ),
        );
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("Name=Tip Calc\n"));
        assert!(entry.contains("Type=Application\n"));
        // GNOME matches a Wayland window to its entry by app id; the
        // Flutter runner sets prgname to APPLICATION_ID, so these agree.
        assert!(entry.contains("StartupWMClass=app.lisaos.forge.tip_calc\n"));
        assert!(entry.contains("Exec=\"/var/lib/lisa/forge/apps/"));
    }

    #[test]
    fn the_built_bundle_is_found_whatever_the_sdk_calls_the_arch() {
        let dir = tempfile::tempdir().unwrap();
        assert!(built_bundle(dir.path()).is_none());
        // An arch directory we do not run on is still discovered — the
        // spelling belongs to the SDK, not to us.
        let bundle = dir.path().join("build/linux/riscv64/release/bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        assert_eq!(built_bundle(dir.path()).unwrap(), bundle);
    }

    #[test]
    fn installing_a_build_stages_then_swaps_and_keeps_one_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("apps");
        // SAFETY: test-scoped env; no other test reads this variable.
        unsafe { std::env::set_var("LISA_FORGE_APPS_DIR", &apps) };

        let build = |marker: &str| {
            let src = dir.path().join(format!("build-{marker}"));
            std::fs::create_dir_all(src.join("data")).unwrap();
            std::fs::write(src.join("tip_calc"), marker).unwrap();
            std::fs::write(src.join("data/flutter_assets"), marker).unwrap();
            src
        };

        let exe =
            install_forged_bundle("app.lisaos.forge.tip_calc", "tip_calc", &build("v1")).unwrap();
        assert!(exe.ends_with("bundle/tip_calc"));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "v1");
        // Nested files come along.
        assert!(exe.parent().unwrap().join("data/flutter_assets").exists());
        // No staging directory survives a successful install.
        let root = apps.join("app.lisaos.forge.tip_calc");
        assert!(
            std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().starts_with(".stage")),
        );

        // A rebuild swaps in place and keeps exactly one previous build.
        let exe =
            install_forged_bundle("app.lisaos.forge.tip_calc", "tip_calc", &build("v2")).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "v2");
        assert_eq!(
            std::fs::read_to_string(root.join("bundle.previous/tip_calc")).unwrap(),
            "v1"
        );

        unsafe { std::env::remove_var("LISA_FORGE_APPS_DIR") };
    }
}

#[cfg(test)]
mod remote_tests {
    use super::parse_remote_model;

    #[test]
    fn parses_provider_and_model_including_colonful_tails() {
        assert_eq!(
            parse_remote_model("remote:moonshot:kimi-k2"),
            Some(("moonshot", "kimi-k2"))
        );
        // openrouter vendor/model and colon-bearing tails stay intact.
        assert_eq!(
            parse_remote_model("remote:openrouter:anthropic/claude-3.5-sonnet"),
            Some(("openrouter", "anthropic/claude-3.5-sonnet"))
        );
        assert_eq!(
            parse_remote_model("remote:tinker:tinker://run/w/5"),
            Some(("tinker", "tinker://run/w/5"))
        );
        // Non-remote and malformed hints route locally / are rejected.
        assert_eq!(parse_remote_model("qwen3-0.6b"), None);
        assert_eq!(parse_remote_model("remote:openai"), None);
        assert_eq!(parse_remote_model("remote:openai:"), None);
    }
}
