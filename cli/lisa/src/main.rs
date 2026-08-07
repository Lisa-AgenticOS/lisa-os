//! `lisa` — the one command center (`docs/PLAN.md` §5.4, Appendix E rule 4:
//! everything under `lisa <verb>`, never scattered scripts).
//!
//! M0 surface: `ask` (streams from lisa-inferenced's OpenAI-compat
//! endpoint) and `models` (local store operations via the lisa-modeld
//! library). `tools`/`call`/`undo`/`ledger` are declared now and land with
//! the Agent Bus in M5.

mod agent;
mod apps;
mod bus_tools;
mod dev;
mod devbox;
mod doctor;
mod guard;
mod install_plan;
mod mail;
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
        /// Attach an image or audio file (repeatable). Needs a model
        /// with the modality — a text-only engine refuses rather than
        /// answering about something it never saw.
        #[arg(long = "attach", value_name = "FILE")]
        attach: Vec<PathBuf>,
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
    /// Multi-turn assistant on the agent harness: read-tier Agent Bus
    /// tools in one loop (ADR-0025). Unlike `lisa do`, which routes one
    /// utterance to one tool call, this can look, read the answer, and
    /// look again.
    Assist {
        /// What you want, in plain words.
        utterance: Vec<String>,
        #[arg(
            long,
            default_value = "http://127.0.0.1:7777",
            env = "LISA_INFERENCE_URL"
        )]
        url: String,
        #[arg(long)]
        model: Option<String>,
        /// Turn budget: one tool call or done-signal each.
        #[arg(long, default_value_t = 12)]
        max_turns: usize,
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
    /// The proto-installer (a guided OOBE installer is M7). Run
    /// `lisa install --list` first: it shows every disk and, for the ones
    /// it will not write to, why.
    Install {
        /// Target block device (e.g. /dev/sda). Everything on it is lost.
        #[arg(required_unless_present = "list")]
        target: Option<PathBuf>,
        /// Show the disks on this machine and which of them are legal
        /// targets. Writes nothing.
        #[arg(long, conflicts_with_all = ["from", "yes"])]
        list: bool,
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
        #[arg(long, conflicts_with = "check")]
        reboot: bool,
        /// Report what is running, staged and available — download
        /// nothing, change nothing. This is what the Settings "Check for
        /// Updates" button runs.
        #[arg(long)]
        check: bool,
    },
    /// Join a connected mail account to the Maildir the Mail app reads
    /// (issue #155). Lisa does not speak IMAP itself — this writes the
    /// mbsync config and fetches the token mbsync authenticates with.
    Mail {
        #[command(subcommand)]
        cmd: MailCmd,
    },
    /// Collect the state of this machine into one shareable report:
    /// versions, services, storage, desktop, the Lisa units' warnings,
    /// and the Ledger tail. Credentials are removed and prompt text is
    /// withheld unless you ask for it.
    Doctor {
        /// Write to a file instead of stdout. Bare `--bundle` picks a
        /// path in the temp directory and prints it.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        bundle: Option<PathBuf>,
        /// Include Ledger prompt previews. They are the most useful
        /// thing in a diagnostic and the most private — read the file
        /// before sharing it.
        #[arg(long)]
        include_previews: bool,
        /// How many journal lines per unit.
        #[arg(long, default_value_t = 200)]
        journal_lines: usize,
    },
    /// Update out-of-image payloads independently of the OS image
    /// (ADR-0020, ADR-0023): fetch, verify, and activate the newest shell
    /// tree and CLI runtime payloads — no reboot.
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
    /// Record from the microphone and transcribe it (push-to-talk,
    /// §5.7.5). The capture half every other voice verb assumed.
    Listen {
        /// Stop after this many seconds. Push-to-talk holds the key for
        /// as long as somebody is talking, so this is a ceiling, not a
        /// setting: an unbounded recorder that is never stopped fills a
        /// disk quietly.
        #[arg(long, default_value_t = 15)]
        seconds: u32,
        #[arg(long)]
        model: Option<PathBuf>,
        /// Keep the recording at this path instead of discarding it.
        #[arg(long)]
        keep: Option<PathBuf>,
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
        /// Max plan→edit→check iterations before giving up.
        #[arg(long, default_value_t = 6)]
        max_iters: usize,
        /// Bring your own agent (PLAN §5.12.1, ADR-0061): run this
        /// program as the builder instead of the native loop — inside
        /// the same filesystem jail, with the same verifier gate after
        /// it exits, and the run in the Ledger either way. The task
        /// is appended as the final argument.
        #[arg(long)]
        byo: Option<String>,
        /// Arguments passed to the --byo program before the task
        /// (repeatable), e.g. --byo claude --byo-arg=-p
        #[arg(long = "byo-arg", allow_hyphen_values = true)]
        byo_args: Vec<String>,
    },
    /// Developer tooling for building on Lisa (ADR-0050).
    Dev {
        #[command(subcommand)]
        cmd: DevCmd,
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
enum MailCmd {
    /// Discover the connected account and write the sync config.
    Setup {
        /// Authenticate with a provider-issued app password from this
        /// file instead of an Online Accounts token. Needs no keyring
        /// and no SASL plugin, and is the only option some providers
        /// offer.
        #[arg(long, value_name = "FILE")]
        app_password: Option<PathBuf>,
        /// Where the Maildir goes. Defaults to ~/Mail, which is where
        /// the Mail app looks.
        #[arg(long, value_name = "DIR")]
        maildir: Option<PathBuf>,
        /// Replace a config this command did not write.
        #[arg(long)]
        force: bool,
    },
    /// Run one sync pass.
    Sync,
    /// Index the Maildir into the context store under `mail`
    /// provenance (#170), so retrieval can answer "the email about
    /// the parking permit". Runs automatically after `sync`; this
    /// verb exists for the first backfill, for re-runs, and for the
    /// reap that clears documents no message answers to (#224).
    Index {
        /// Walk and print what would be indexed and what the reap
        /// would remove, then write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// What is present, what is missing, and which layer is blocking.
    Status,
    /// Print a fresh access token. This is what mbsync's PassCmd runs;
    /// it writes a credential to stdout by design.
    Token {
        /// Which account, if more than one has mail enabled.
        #[arg(long, value_name = "ID")]
        account: Option<String>,
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
    /// Put a folder out of bounds for agent actions. TIGHTENING only —
    /// this can never permit anything, so it is always safe to run
    /// (#253).
    Protect { path: PathBuf },
    /// Take back a protection YOU added. Cannot reach a built-in rule:
    /// unprotecting /etc does not make /etc writable.
    Unprotect { path: PathBuf },
}

#[derive(Subcommand)]
enum DevCmd {
    /// Install a package into the dev box (rootless, in your home).
    Install {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Remove a package and the commands it put on PATH.
    Remove {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// What is installed, and how many commands each package publishes.
    List,
    /// A shell inside the dev box.
    Shell,
    /// Destroy the box and every shim. Real recovery, not a reinstall.
    Reset,
    /// Are rootless containers usable here? Checks the prerequisites
    /// that fail SILENTLY (#130): the subuid/subgid range, the uidmap
    /// helpers, podman, and room on the filesystem the container store
    /// actually lands on.
    Doctor {
        /// Space to plan for, in GiB (default: a modest first install).
        #[arg(long, default_value_t = 4)]
        needs: u64,
    },
    /// Check an app tree: the single authority on what a valid Lisa app
    /// is (ADR-0050 §4). Exits non-zero with findings; `lisa forge`
    /// runs it as its verifier.
    Check {
        /// The app directory (default: the current one).
        path: Option<PathBuf>,
    },
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
    /// Full loop: transcribe → classify → answer → say. Reads an audio
    /// file, or the microphone when none is given.
    Once {
        /// Audio file. Omit to record from the microphone instead —
        /// the loop was file-only until `lisa listen` existed, which
        /// meant the one thing it is for could not be demonstrated.
        audio: Option<PathBuf>,
        /// Recording ceiling when reading from the microphone.
        #[arg(long, default_value_t = 15)]
        seconds: u32,
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
    /// for every channel, or just the named one (`shell`, `runtime`).
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
    /// have yet, leaving installed versions untouched. Run by
    /// lisa-apps-sync.timer and by `lisa update` before it stages a slot.
    Sync,
    /// Install a payload tarball you already have, instead of fetching one.
    ///
    /// For offline media and for testing a build before it is released.
    /// The release path (`update`/`sync`) verifies every payload against
    /// the release SHA256SUMS; this one cannot — the file came from you,
    /// not from a manifest — and says so.
    Install {
        /// Channel to install into (`shell`, `runtime`).
        channel: String,
        /// The `.tar.zst` payload.
        tarball: PathBuf,
        /// Version to record for the tree (`YYYYMMDD.run`).
        #[arg(long)]
        version: String,
    },
    /// Print the directories a launcher searches for CHANNEL, best first
    /// — the one authority for where `update` installs a payload and
    /// where /usr/bin/lisa-app then reads it (issue #239).
    Path {
        /// Channel to resolve (`shell`, `runtime`).
        channel: String,
        /// Print the install root instead: the directory `update` writes
        /// `versions/<ver>` and `current` into.
        #[arg(long)]
        base: bool,
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
    /// Index the OS knowledge pack (/usr/share/lisa/knowledge) under
    /// the `system` provenance, if its content changed since the last
    /// sync (#175). Cheap when nothing changed; run after updates.
    SyncKnowledge {
        /// Read the pack from here instead of /usr/share/lisa/knowledge
        /// (dev hosts; tests).
        #[arg(long)]
        dir: Option<PathBuf>,
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
            attach,
        } => ask(
            prompt,
            &url,
            model,
            no_stream,
            json_schema,
            background,
            attach,
        ),
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
            GuardCmd::Protect { path } => guard::protect_cmd(&path),
            GuardCmd::Unprotect { path } => guard::unprotect_cmd(&path),
        },
        Command::Models { cmd, store } => models(cmd, store),
        Command::Do {
            utterance,
            url,
            model,
            dry_run,
            yes,
        } => agent::do_cmd(&utterance.join(" "), &url, model.as_deref(), dry_run, yes),
        Command::Assist {
            utterance,
            url,
            model,
            max_turns,
        } => bus_tools::assist_cmd(&utterance.join(" "), &url, model, max_turns),
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
        Command::Install {
            target,
            list,
            from,
            yes,
        } => {
            if list {
                list_install_targets()
            } else {
                install_cmd(
                    &target.expect("clap requires a target unless --list"),
                    from,
                    yes,
                )
            }
        }
        Command::Mail { cmd } => match cmd {
            MailCmd::Setup {
                app_password,
                maildir,
                force,
            } => mail::setup(app_password, maildir, force),
            MailCmd::Sync => mail::sync(),
            MailCmd::Index { dry_run } => mail::index(dry_run),
            MailCmd::Status => mail::status(),
            MailCmd::Token { account } => mail::token(account),
        },
        Command::Doctor {
            bundle,
            include_previews,
            journal_lines,
        } => doctor::run(bundle, include_previews, journal_lines),
        Command::Update { reboot, check } => {
            if check {
                update_check_cmd()
            } else {
                update_cmd(reboot)
            }
        }
        Command::Apps { cmd } => match cmd {
            AppsCmd::Update { channel } => apps::update(channel.as_deref()),
            AppsCmd::Status => apps::status(),
            AppsCmd::Rollback { channel } => apps::rollback(channel.as_deref()),
            AppsCmd::Sync => apps::sync(),
            AppsCmd::Install {
                channel,
                tarball,
                version,
            } => apps::install_local(&channel, &tarball, &version),
            AppsCmd::Path { channel, base } => apps::print_path(&channel, base),
        },
        Command::Remote { cmd } => remote_cmd(cmd),
        Command::Transcribe { audio, model } => {
            let m = voice::whisper_model(model)?;
            println!("{}", voice::transcribe(&audio, &m)?);
            Ok(())
        }
        Command::Listen {
            seconds,
            model,
            keep,
        } => {
            // Resolve the model BEFORE opening the microphone. Recording
            // somebody and then discovering there is nothing to
            // transcribe with wastes the one thing that cannot be
            // retried — they have already said it.
            let m = voice::whisper_model(model)?;
            let text = voice::listen(seconds, &m, keep.as_deref())?;
            if text.is_empty() {
                eprintln!("(nothing heard)");
            } else {
                println!("{text}");
            }
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
            byo,
            byo_args,
        } => match byo {
            Some(program) => forge_byo(&task.join(" "), &project, &program, &byo_args),
            None => forge_cmd(&task.join(" "), &project, model, &url, max_iters),
        },
        Command::Dev { cmd } => match cmd {
            DevCmd::Check { path } => dev::check_cmd(path),
            DevCmd::Doctor { needs } => dev::doctor_cmd(needs),
            DevCmd::Install { packages } => devbox::install_cmd(&packages),
            DevCmd::Remove { packages } => devbox::remove_cmd(&packages),
            DevCmd::List => devbox::list_cmd(),
            DevCmd::Shell => devbox::shell_cmd(),
            DevCmd::Reset => devbox::reset_cmd(),
        },
        Command::Skills { cmd } => match cmd {
            SkillsCmd::List => skills::list(),
            SkillsCmd::Show { name } => skills::show(&name),
        },
        Command::Context { cmd } => context_cmd(cmd),
        Command::Memory { cmd, app } => memory_cmd(cmd, &app),
    }
}

/// One attachment as an OpenAI content part: a data: URI, so the file
/// never has to be reachable by the provider and no temporary upload
/// exists to leak. The kind is decided by EXTENSION and the mime is
/// spelled explicitly — guessing "image/*" for a .wav would produce a
/// provider-side error that reads like our bug.
fn attachment_part(path: &std::path::Path) -> anyhow::Result<serde_json::Value> {
    use base64::Engine as _;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes =
        std::fs::read(path).with_context(|| format!("reading attachment {}", path.display()))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let image_mime = match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    };
    if let Some(mime) = image_mime {
        return Ok(serde_json::json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{mime};base64,{b64}")},
        }));
    }
    // Audio rides the OpenAI input_audio part, which takes a bare
    // base64 payload plus a format name (not a data: URI).
    let audio_format = match ext.as_str() {
        "wav" => Some("wav"),
        "mp3" => Some("mp3"),
        _ => None,
    };
    if let Some(format) = audio_format {
        return Ok(serde_json::json!({
            "type": "input_audio",
            "input_audio": {"data": b64, "format": format},
        }));
    }
    bail!(
        "cannot attach {} — supported: png, jpg, jpeg, webp, gif, wav, mp3",
        path.display()
    )
}

fn ask(
    prompt: Vec<String>,
    url: &str,
    model: Option<String>,
    no_stream: bool,
    json_schema: Option<PathBuf>,
    background: bool,
    attach: Vec<PathBuf>,
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

    // With attachments the message carries CONTENT PARTS, the shape
    // multimodal models take images and audio in; without them it stays
    // a plain string, which every engine understands.
    let content = if attach.is_empty() {
        serde_json::Value::String(prompt)
    } else {
        let mut parts = vec![serde_json::json!({"type": "text", "text": prompt})];
        for path in &attach {
            parts.push(attachment_part(path)?);
        }
        serde_json::Value::Array(parts)
    };
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": content}],
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

/// `lisa install --list` — every whole disk on the machine, and for the
/// ones that are not legal targets, the reason. Writes nothing.
///
/// This exists because the alternative to a picker is a person typing a
/// device node from memory, and the failure mode of that is the one this
/// whole module is built to prevent.
fn list_install_targets() -> anyhow::Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("disk topology is read with lsblk — Linux only. Boot the Lisa USB and run it there");
    }
    let disks = install_plan::read_topology()?;
    let facts = install_plan::read_facts();
    print!(
        "{}",
        install_plan::render_targets(&disks, &facts, install_plan::MIN_TARGET_BYTES)
    );
    Ok(())
}

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

    // Every decision about *which* disk lives in install_plan, where it
    // is a pure function with tests that have been made to go red. What
    // used to be here was a `starts_with` over /proc/mounts that could
    // not see the disk it booted from unless something from it happened
    // to be mounted, could not tell a partition from a disk, and could
    // not tell 16 GB from enough (see that module's header).
    //
    // Regular files are not planned: writing an image to a file is how
    // the image lane itself is tested, and there is nothing to destroy.
    let mut plan = None;
    // What was planned, and what must still be true when we open it.
    // `target` may be a `/dev/disk/by-id/*` symlink, and between the
    // plan and the write sits an unbounded wait for a human to type
    // ERASE — long enough for a hotplug to reassign `/dev/sdb` or for
    // the symlink to come to mean a different device (issue #301). So
    // the write opens the *planned* path, and then checks the device it
    // actually got against the device number the plan was made about.
    let mut planned: Option<(PathBuf, u64)> = None;
    if is_block {
        use std::os::unix::fs::MetadataExt;
        // The user may have typed a symlink (/dev/disk/by-id/...); lsblk
        // reports canonical kernel names.
        let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.clone());
        let rdev = std::fs::metadata(&canonical)?.rdev();
        let disks = install_plan::read_topology()?;
        let facts = install_plan::read_facts();
        match install_plan::plan(
            &canonical.to_string_lossy(),
            &disks,
            &facts,
            install_plan::MIN_TARGET_BYTES,
        ) {
            Ok(p) => {
                plan = Some(p);
                planned = Some((canonical, rdev));
            }
            Err(refusal) => bail!("{refusal}\n   `lisa install --list` shows the disks it will."),
        }
    }

    eprintln!(
        "!! {} will be COMPLETELY ERASED — every partition, every file.",
        target.display()
    );
    if let Some(p) = &plan {
        eprintln!("   {}", install_plan::human(p.size));
        if let Some(model) = &p.model {
            eprintln!("   {model}");
        }
        for line in &p.destroys {
            eprintln!("   erases {line}");
        }
        eprintln!(
            "   (this system is running from {}, not this disk)",
            p.boot_disks.join(", ")
        );
    }
    if !yes {
        eprint!("Type ERASE to continue: ");
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if answer.trim() != "ERASE" {
            println!("aborted — nothing written");
            return Ok(());
        }
    }

    // Open the path the plan was made about, not the one the user typed,
    // and then confirm the fd really is that device. A refusal here is
    // the topology having changed under us while the prompt was open;
    // re-run and re-read the plan.
    let sink_path = planned.as_ref().map_or(target, |(p, _)| p);
    let mut sink = std::fs::OpenOptions::new().write(true).open(sink_path)?;
    if let Some((path, planned_rdev)) = &planned {
        use std::os::unix::fs::MetadataExt;
        let opened = sink.metadata()?.rdev();
        if opened != *planned_rdev {
            bail!(
                "{} is not the device it was when the plan was made \
                 (device {planned_rdev} then, {opened} now) — a disk was added, removed or \
                 re-lettered. Nothing written; re-run `lisa install --list`",
                path.display()
            );
        }
    }
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

/// The verifier the default (GJS) lane runs — `lisa dev check`, this
/// binary, ADR-0050 §4.
///
/// A function rather than an expression inside `forge_cmd` so a test can
/// assert what the Forge actually selects. `cli/lisa/tests/forge_verifier.rs`
/// executes the arm end to end but spells it itself, so it would keep
/// passing if this ever went back to `Verifier::Dart` — which is the
/// regression the whole of #243 is about.
fn default_verifier() -> forge_harness::Verifier {
    forge_harness::Verifier::Command {
        // `current_exe` rather than the string "lisa": the binary running
        // the loop IS the checker, so the verifier cannot pick up a
        // different `lisa` from `$PATH` — or fail to find one at all in a
        // dev checkout.
        program: std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "lisa".into()),
        args: vec!["dev".into(), "check".into()],
    }
}

/// PLAN §5.12.1's bring-your-own agent, made real (ADR-0061): any
/// agent CLI as the builder, inside Lisa's rails. The rails are the
/// point — the foreign agent gets the same Landlock jail as the native
/// loop's children (project read-write, caches read-only, nothing
/// else), the same verifier gate decides whether the result counts,
/// and the run is in the Ledger either way. What it does NOT get is
/// puppeteered: it runs its own loop, interactively if it wants —
/// stdio is inherited so a TUI works.
///
/// The program comes from the OWNER's flag, never from a model — this
/// is a capability handed in from outside the loop (ADR-0030), which
/// is why it may name programs the guard's allowlist would refuse.
/// Its network is its own: on a dev host that is the owner's business;
/// the on-device story routes through the broker or not at all, and
/// shipping this as a device default would need that answer first.
fn forge_byo(
    task: &str,
    project: &PathBuf,
    program: &str,
    byo_args: &[String],
) -> anyhow::Result<()> {
    std::fs::create_dir_all(project)?;
    // Ledger-mandatory, exactly like the native loop (#54): a machine
    // with an unwritable Ledger refuses to run an agent off the record.
    let ledger = lisa_ledger::Ledger::open(lisa_ledger::Ledger::default_path())?;
    let started = format!(
        "byo={program} args={byo_args:?} project={} task={task}",
        project.display()
    );
    ledger.append(&lisa_ledger::Event {
        kind: "forge.byo.start".into(),
        app_id: "host".into(),
        input_hash: blake3::hash(started.as_bytes()).to_hex().to_string(),
        status: "ok".into(),
        detail: started.clone(),
        ..Default::default()
    })?;

    let mut cmd = std::process::Command::new(program);
    cmd.args(byo_args);
    if !task.is_empty() {
        cmd.arg(task);
    }
    cmd.current_dir(project);
    let confinement = forge_harness::confine::confine_command(
        &mut cmd,
        project,
        &forge_harness::confine::user_home(),
    );
    // Said before the child runs, not after: whoever is watching the
    // terminal decides with this line in view.
    match confinement.note() {
        None => eprintln!(
            ">> {program} runs jailed to {} (filesystem; its network is its own)",
            project.display()
        ),
        Some(note) => eprintln!("!! {program} runs UNCONFINED: {note}"),
    }

    // The end event is written on EVERY exit (#337): "in the Ledger
    // either way" has to include the ways that go wrong — a spawn
    // failure and a verifier error are exactly the runs an audit trail
    // is for.
    let end = |status: &str, detail: String| {
        let _ = ledger.append(&lisa_ledger::Event {
            kind: "forge.byo.end".into(),
            app_id: "host".into(),
            input_hash: blake3::hash(started.as_bytes()).to_hex().to_string(),
            status: status.into(),
            detail,
            ..Default::default()
        });
    };
    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            end("error", format!("spawn failed: {e}"));
            anyhow::bail!("running `{program}`: {e} — is it installed and on PATH?");
        }
    };

    // The gate is not optional and not the foreign agent's opinion:
    // the same verifier the native loop answers to decides whether
    // this run produced a valid result (gauntlet discipline — a
    // foreign agent's own "done" is a DoneClaimed, not a verdict).
    let verifier = default_verifier();
    let findings = match verifier.check(project) {
        Ok(f) => f,
        Err(e) => {
            end("error", format!("verifier error after exit {status}: {e}"));
            return Err(e.into());
        }
    };
    let outcome = match (&status.success(), &findings) {
        (true, None) => "clean".to_string(),
        (true, Some(f)) => format!("exited ok, verifier findings: {}", truncate_str(f, 400)),
        (false, None) => format!("exit status {status}, verifier clean"),
        (false, Some(f)) => format!(
            "exit status {status}, verifier findings: {}",
            truncate_str(f, 400)
        ),
    };
    end(
        if status.success() && findings.is_none() { "ok" } else { "findings" },
        outcome.clone(),
    );
    match findings {
        None if status.success() => {
            println!(
                "byo run clean — verifier agrees. Source in {}",
                project.display()
            );
            Ok(())
        }
        None => {
            println!("byo agent exited {status}; verifier found nothing to flag.");
            Ok(())
        }
        Some(f) => {
            println!("byo run finished, but the verifier disagrees:\n{f}");
            anyhow::bail!("verifier findings — the run is not clean")
        }
    }
}

/// First `n` chars, honestly marked when cut.
fn truncate_str(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n).collect();
        format!("{cut}… [truncated]")
    }
}

fn forge_cmd(
    task: &str,
    project: &PathBuf,
    model: Option<String>,
    url: &str,
    max_iters: usize,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(project)?;
    // The lane is GJS (ADR-0047 §4), and its checker is
    // `lisa dev check` (ADR-0050 §4) — not a `Verifier::Gjs` arm.
    //
    // The arm would have been smaller today and wrong tomorrow: it
    // would be a second implementation of "what a valid Lisa app
    // is", living where only a forge run can reach it, while the
    // verb has to exist anyway for CI and for the person who edits
    // the file after the loop ends. This repo's two most expensive
    // defects are one truth in two places (#218 in triplicate, #239
    // spelled two ways); adding a second copy on purpose, with a
    // migration promised later, is how the interval that produces
    // drift gets created deliberately.
    //
    // No scaffold is written. GJS is interpreted, so there is
    // nothing to fetch and nothing to build (ADR-0050 §6): an
    // empty directory is a legitimate start, and `lisa dev check`
    // reports "no sources" until the model writes some — which is
    // #29's gate, and the reason a bare "done" cannot converge on an
    // empty tree.
    //
    // `current_exe` rather than the string "lisa": the binary
    // running the loop is the checker, so the verifier cannot pick
    // up a different `lisa` from `$PATH` — or fail to find one at
    // all in a dev checkout.
    let verifier = default_verifier();
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
        ..forge_harness::AgentConfig::new(ledger)
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
            E::Ambient { chars } => eprintln!("  + ambient context joined ({chars} chars)"),
            // forge narrates turns, not prose: a build loop printing the
            // model's thinking token by token buries the tool calls that
            // matter. The chat surfaces render these.
            E::Delta(_) => {}
        }
    };
    match forge_harness::forge_agent_observed(task, project, &mut backend, &config, &mut observe) {
        Ok(report) => {
            println!(
                "converged in {} turn(s) — verifier clean. Source in {}",
                report.turns,
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
            seconds,
            model,
            url,
            speak,
            classify,
        } => {
            let m = voice::whisper_model(model)?;
            let transcript = match audio {
                Some(path) => voice::transcribe(&path, &m)?,
                None => voice::listen(seconds, &m, None)?,
            };
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

/// Refuse to call an update "staged" when the boot loader will ignore it.
///
/// sd-boot honours an explicit `LoaderEntryDefault` over its own version
/// sort. Pinning a known-good version after a successful boot is a
/// reasonable thing to do once — and a permanent trap afterwards, because
/// every later update then stages correctly, reports success, and is
/// never booted. On the reference iMac that cost a full debugging session
/// aimed at the wrong subsystem: the audio fix under test had never been
/// installed, and nothing said so (issue #141).
///
/// Advisory by design: this reports, it does not silently repoint
/// somebody's boot loader. A pin is a deliberate act and un-pinning is
/// the owner's call, so the failure mode we remove is the SILENT one.
/// `IMAGE_VERSION` of the *booted* slot — the version actually running,
/// which is not the same question as "what is installed".
///
/// This is the value sysupdate keys slots on (`root_@v`) and the one
/// release.yml bakes into the image, so it compares directly against a
/// version reported over the sysupdate1 bus.
///
/// `/etc` is per-slot, so /etc/os-release always describes the running
/// system; `/usr/lib/os-release` is the fallback for images that ship it
/// only there.
/// Record a `lisa dev` action in the Ledger (#130 phase 2).
///
/// "You can read exactly what it did" covers the toolchain too — a
/// developer front door that installs software without an audit line is
/// the one place on the machine you cannot account for.
///
/// Best-effort and non-fatal: a missing Ledger must not stop somebody
/// installing a database, but it IS reported, because a silent failure
/// here means the audit trail has a hole nobody knows about.
pub fn ledger_note(kind: &str, detail: &str) {
    let record = || -> anyhow::Result<()> {
        let ledger = lisa_ledger::Ledger::open(lisa_ledger::Ledger::default_path())?;
        ledger.append(&lisa_ledger::Event {
            kind: kind.into(),
            app_id: "host".into(),
            input_hash: blake3::hash(detail.as_bytes()).to_hex().to_string(),
            status: "ok".into(),
            detail: detail.into(),
            ..Default::default()
        })?;
        Ok(())
    };
    if let Err(e) = record() {
        eprintln!("warning: could not write the Ledger entry for {kind}: {e}");
    }
}

fn running_image_version() -> Option<String> {
    for path in ["/etc/os-release", "/usr/lib/os-release"] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Some(v) = os_release_value(&text, "IMAGE_VERSION") {
            return Some(v);
        }
    }
    None
}

/// Pull one key out of os-release(5) text.
///
/// Values are shell-quoted there — `IMAGE_VERSION="20260728.49"` — and an
/// unstripped quote turns every version comparison into a mismatch, which
/// would make this report "staged" forever.
pub(crate) fn version_line() -> String {
    format!("lisa {}", env!("CARGO_PKG_VERSION"))
}

pub(crate) fn os_release_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(|line| {
            let rest = line.strip_prefix(key)?.strip_prefix('=')?;
            let rest = rest.trim();
            let unquoted = rest
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .or_else(|| rest.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
                .unwrap_or(rest);
            (!unquoted.is_empty()).then(|| unquoted.to_string())
        })
}

fn warn_if_boot_default_is_pinned(installed: Option<&str>) {
    let out = match std::process::Command::new("bootctl")
        .arg("status")
        .env("SYSTEMD_COLORS", "0")
        .output()
    {
        Ok(o) if o.status.success() => o,
        // No bootctl, or not a sd-boot system (Track L): nothing to say.
        _ => return,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let pinned = text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("Default Entry:")?;
        Some(rest.trim().to_string())
    });
    let Some(pinned) = pinned.filter(|p| !p.is_empty()) else {
        return; // No pin: sd-boot picks the newest, which is what we want.
    };

    // Which version SHOULD boot? When the caller knows what it installed,
    // that. Otherwise ask the boot loader for the highest version it can
    // see — the exec fallback path does not learn a version, and the
    // question that matters is the same either way: will the newest thing
    // on this machine actually boot?
    let newest = match installed {
        Some(v) => v.to_string(),
        None => {
            let listed = match std::process::Command::new("bootctl")
                .arg("list")
                .env("SYSTEMD_COLORS", "0")
                .output()
            {
                Ok(o) if o.status.success() => o,
                _ => return,
            };
            let text = String::from_utf8_lossy(&listed.stdout);
            let mut versions: Vec<String> = text
                .lines()
                .filter_map(|l| l.trim().strip_prefix("version:"))
                .map(|v| v.trim().to_string())
                .collect();
            // Version strings here are YYYYMMDD.run, so lexicographic
            // ordering is chronological.
            versions.sort();
            match versions.pop() {
                Some(v) => v,
                None => return,
            }
        }
    };
    if pinned.contains(&newest) {
        return; // Pinned to exactly the version that should boot. Fine.
    }
    let installed = newest.as_str();
    eprintln!();
    eprintln!("!! the boot loader is PINNED to `{pinned}`, not to {installed}");
    eprintln!("!! rebooting will come back on the OLD version and this update");
    eprintln!("!! will look like it did nothing. Clear the pin with:");
    eprintln!("!!     sudo bootctl set-default \"\"");
    eprintln!("!! (then sd-boot boots the newest entry), or pin the new one:");
    eprintln!("!!     sudo bootctl set-default lisa_{installed}.efi");
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
/// Is this failure "you are not root", as opposed to a real problem?
///
/// Walks the error chain rather than matching on the message: the
/// payload tree is root-owned and a desktop user hits EACCES on the
/// first mkdir, which is a different situation from an unreachable app
/// channel and deserves a different sentence (#140).
fn is_permission_denied(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
    })
}

/// Will anything retry the payload fetch on its own?
///
/// The deferral message is only honest if the timer exists and is
/// active. Reporting "the timer will handle it" on a system where it is
/// masked would be a more comfortable lie than the sudo instruction it
/// replaces.
fn apps_sync_timer_active() -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "lisa-apps-sync.timer"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

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
            // NoCandidate means "nothing NEWER to fetch" — NOT "you are
            // running the newest thing on this machine". Those differ for
            // the entire window between staging and the next boot, which
            // is exactly when someone runs `lisa update` again to see
            // where they stand. Answering "already up to date" there is a
            // lie shaped like a no-op: the new version is on disk waiting
            // for a reboot, and the message says there is nothing to wait
            // for. Observed on the reference iMac (#144): .49 staged, .47
            // booted, and the update looked like it had never run.
            let detail = msg
                .as_deref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default();
            let running = running_image_version();
            let installed: Option<String> = target.call("GetVersion", &()).ok();
            match (running.as_deref(), installed.as_deref()) {
                // Newest installed set is not the one we booted: staged.
                // Version strings are YYYYMMDD.run, so `>` is chronological.
                (Some(run), Some(have)) if have > run => {
                    println!("{have} is staged — reboot to use it (running {run})");
                    // The pin check belongs here for the same reason it
                    // belongs after a fresh staging: a pinned loader makes
                    // the reboot a no-op, and this path is the one a person
                    // hits when they are *already* wondering why nothing
                    // changed.
                    warn_if_boot_default_is_pinned(Some(have));
                }
                (Some(run), _) => println!("already up to date (running {run}){detail}"),
                (None, _) => println!("already up to date{detail}"),
            }
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
        warn_if_boot_default_is_pinned(Some(&version));
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

/// `lisa update --check`: report state, touch nothing.
///
/// Three distinct facts, which today's UI kept conflating (#144):
///
/// - **running** — the booted slot's `IMAGE_VERSION`.
/// - **staged** — the newest COMPLETE update set on disk. Different from
///   `running` for the whole window between staging and the next reboot.
/// - **available** — what the channel offers, from `Target.CheckNew`.
///
/// Output is `key: value` lines so the Settings row can parse it without
/// a second D-Bus client in C, and a human can still read it. Absent
/// facts are omitted rather than printed as "unknown": a caller must be
/// able to tell "there is no staged update" from "I could not find out".
///
/// `CheckNew` needs polkit `sysupdate1.check`, which is `allow_active=yes`
/// — free from a local Settings window, refused from SSH. That refusal is
/// reported, never silently rendered as "no update available".
///
/// Reported to a MACHINE as well as to a person: a failed check exits
/// non-zero. It used to print `check-failed: …` and exit 0, so on the
/// reference device
///
///   $ lisa update --check; echo $?
///   check-failed: …InteractiveAuthorizationRequired…
///   0
///
/// — and the Settings "Check for Updates" button runs exactly this. Any
/// caller that branched on the exit status was told the check succeeded
/// and found nothing, which is the difference between "you are up to
/// date" and "I could not look".
fn update_check_cmd() -> anyhow::Result<()> {
    use zbus::blocking::{Connection, Proxy};

    let running = running_image_version();
    if let Some(ref v) = running {
        println!("running: {v}");
    }

    let mut checked = false;
    let mut why: Option<String> = None;
    if let Ok(conn) = Connection::system()
        && let Ok(manager) = Proxy::new(
            &conn,
            "org.freedesktop.sysupdate1",
            "/org/freedesktop/sysupdate1",
            "org.freedesktop.sysupdate1.Manager",
        )
        && let Ok(targets) = manager
            .call::<_, _, Vec<(String, String, zbus::zvariant::OwnedObjectPath)>>(
                "ListTargets",
                &(),
            )
        && let Some((_, _, path)) = targets.into_iter().find(|(class, _, _)| class == "host")
        && let Ok(target) = Proxy::new(
            &conn,
            "org.freedesktop.sysupdate1",
            path,
            "org.freedesktop.sysupdate1.Target",
        ) as zbus::Result<Proxy>
    {
        // GetVersion is the newest COMPLETE set — a slot whose root
        // landed but whose UKI did not must not be announced as ready.
        if let Ok(installed) = target.call::<_, _, String>("GetVersion", &())
            && !installed.is_empty()
            && running.as_deref().is_some_and(|r| installed.as_str() > r)
        {
            println!("staged: {installed}");
        }
        match target.call::<_, _, String>("CheckNew", &()) {
            Ok(newest) if !newest.is_empty() => {
                checked = true;
                println!("available: {newest}");
            }
            Ok(_) => checked = true, // Checked; nothing newer offered.
            Err(e) => {
                println!("check-failed: {e}");
                why = Some(e.to_string());
            }
        }
    }
    if !checked {
        println!(
            "note: could not reach the update channel — \
             `available:` above is absent, not empty"
        );
        // Non-zero, so a script or the Settings button can tell "I could
        // not look" from "nothing to install". The human-readable lines
        // above are already printed; this only adds the machine-readable
        // half that was missing.
        bail!(
            "update check did not run: {}",
            why.unwrap_or_else(
                || "sysupdated (org.freedesktop.sysupdate1) was not reachable".into()
            )
        );
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
    assert_transfers_protect_booted()?;
    // ADR-0023 phase 1: the slot we are about to stage may not carry
    // payloads this one does. Pull what is missing onto the PERSISTENT
    // /var first, while the current slot's baked copy is still there to
    // fall back on, so the reboot never lands on a system missing a
    // surface. Best-effort on purpose: an unreachable app channel must
    // not block an OS security update, but it must be loud, because the
    // silent version of this failure is a user who reboots into a
    // desktop with something missing.
    if let Err(e) = apps::sync() {
        // Two different failures wearing one message (#140).
        //
        // The payload tree lives on root-owned /var, so a desktop user
        // running `lisa update` cannot write it — while the OS half of
        // the same command works fine, because sysupdated is
        // polkit-mediated and grants the active local session. Telling
        // that user to run `sudo lisa apps sync` is precisely the shape
        // ADR-0034 exists to forbid: nothing user-facing should need
        // sudo, and `escalate.privilege` is an unoverridable Deny in our
        // own guard. So it is not said.
        //
        // Nothing is lost by not saying it: lisa-apps-sync.timer already
        // runs the same fetch as root, hourly, and exists for exactly
        // this case. The honest report is that the work is deferred, not
        // that the user must escalate — but ONLY if that timer is
        // actually going to run, so that is checked rather than assumed.
        if is_permission_denied(&e) {
            if apps_sync_timer_active() {
                eprintln!(
                    "-- app payloads need root to stage; leaving them to \
                     lisa-apps-sync.timer, which fetches them within the hour.\n\
                     -- the next boot may briefly lack them."
                );
            } else {
                eprintln!(
                    "!! app payloads could not be staged and lisa-apps-sync.timer \
                     is not running, so nothing will retry.\n\
                     !! enable it with: systemctl enable --now lisa-apps-sync.timer"
                );
            }
        } else {
            eprintln!(
                "!! could not pre-fetch app payloads: {e:#}\n\
                 !! the new slot may boot without them; lisa-apps-sync.timer retries hourly"
            );
        }
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
        warn_if_boot_default_is_pinned(None);
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
                // Say which embedder wrote these vectors. A store filled
                // by the hash fallback and one filled by the model look
                // identical afterwards, and mixing them silently makes
                // cosine meaningless (#163).
                let chosen = lisa_contextd::embed::resolve();
                let n = store.embed_pending(chosen.embedder.as_ref())?;
                let model = chosen.model.as_deref().unwrap_or("daemon default");
                println!(
                    "embedded {n} new chunk(s) for hybrid search \
                     (embedder: {}, model: {model})",
                    chosen.kind
                );
                if chosen.kind == "inferenced" && chosen.model.is_none() {
                    eprintln!(
                        "note: embedding with the daemon's default model, not \
                         {} — `lisa models get {}` improves retrieval quality",
                        lisa_contextd::embed::EMBEDDING_MODEL,
                        lisa_contextd::embed::EMBEDDING_MODEL
                    );
                }
                if chosen.kind == "hash" {
                    eprintln!(
                        "note: no model-backed embedder was reachable, so these vectors carry no \
                         semantic meaning — hybrid search will rank lexically (#163)"
                    );
                }
            }
        }
        ContextCmd::SyncKnowledge { dir } => {
            let dir = dir.unwrap_or_else(|| PathBuf::from("/usr/share/lisa/knowledge"));
            if !dir.is_dir() {
                // A machine without a pack is not broken — the pack
                // ships with the image and older images predate it.
                println!("no knowledge pack at {} — nothing to sync", dir.display());
                return Ok(());
            }
            // Change detection by content, not by version string: the
            // pack is bytes on disk, and hashing what is actually there
            // cannot disagree with what is actually there. app_memory
            // under a reserved app id holds the last-synced hash.
            let mut names: Vec<PathBuf> = walkdir::WalkDir::new(&dir)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
                .collect();
            names.sort();
            let mut hasher = blake3::Hasher::new();
            for p in &names {
                hasher.update(p.to_string_lossy().as_bytes());
                hasher.update(&std::fs::read(p)?);
            }
            let pack_hash = hasher.finalize().to_hex().to_string();

            // The hash gates the INDEXING only. Embedding runs on every
            // sync regardless — the review (#177) caught the first
            // version early-returning on an unchanged hash, which made
            // its own printed advice ("rerun after lisa models get") a
            // no-op: the rerun hit the stamp and left the chunks
            // pending until the next image happened to change the pack.
            const STAMP_APP: &str = "dev.lisaos.knowledge";
            if store.memory_get(STAMP_APP, "pack_hash")? == Some(pack_hash.clone()) {
                println!("knowledge pack unchanged — already indexed");
            } else {
                let report = store.index_dir_as(&dir, "system")?;
                // A pack is a MIRROR of its directory: what an upgrade
                // renamed or removed must leave the store too, or the
                // model keeps answering from the previous image's docs
                // (#178).
                let pruned = store.prune_missing("system")?;
                println!(
                    "knowledge pack: indexed {} file(s) ({} chunks), {} unchanged, {} pruned",
                    report.indexed, report.chunks, report.skipped_unchanged, pruned
                );
                store.memory_set(STAMP_APP, "pack_hash", &pack_hash)?;
            }
            // Embed ONLY with the model-backed embedder. Backfilling
            // with the hash fallback would mix vector spaces in one
            // store — cosine between a hash vector and a model vector
            // is noise that LOOKS like ranking — so a machine without
            // the model leaves the chunks pending, and THIS line is the
            // recovery path: the first sync after `lisa models get`
            // embeds them, unchanged hash or not.
            // The SHORT request timeout, not the default: this runs
            // inside `TimeoutStartSec=120`, and a socket deadline the
            // unit outlives is not a deadline. Everything else that
            // embeds — `lisa context index --embed`, the mail backfill
            // — keeps `DEFAULT_REQUEST_TIMEOUT`, because a legitimate
            // slow batch there must not fail.
            let chosen = lisa_contextd::embed::resolve_with_timeout(
                lisa_contextd::embed::BOOT_REQUEST_TIMEOUT,
            );
            if chosen.kind == "inferenced" {
                // SCOPED to `system` (#192). The unscoped call that
                // stood here embedded every pending chunk in the store,
                // so a unit whose job is a 28-chunk knowledge pack
                // inherited the mail backfill's 90,000 and was killed
                // at TimeoutStartSec on every boot. The recovery path
                // above is unchanged: this still runs on an unchanged
                // hash, and still embeds the pack's own pending chunks.
                //
                // RETRIED (#192) because `After=` orders start, not
                // readiness: llama-server may still be loading
                // nomic-embed when this fires.
                //
                // THREE attempts, not five. The budget now has to cover
                // the socket timeout as well as the sleeping: worst
                // case is `max_duration` = 3 × 30s + (2+4)s = 96s,
                // inside the unit's 120 with 24s to spare for the 5s of
                // hashing and indexing above. Five attempts would be
                // 5 × 30 + 30 = 180s, which is the unit's timeout
                // again — i.e. exactly the bound this change exists to
                // stop relying on. What the two dropped attempts cost
                // is 24s of extra backoff, and backoff was never what
                // rescued a cold start anyway: a nomic-embed load takes
                // tens of seconds, so neither 6s nor 30s reliably
                // covers it. What covers it is the designed outcome
                // below — leave the chunks pending, exit 0, embed on
                // the next sync.
                let embedder = lisa_contextd::embed::RetryingEmbedder::new(
                    chosen.embedder.as_ref(),
                    3,
                    std::time::Duration::from_secs(2),
                );
                match store.embed_pending_provenance(&embedder, "system") {
                    Ok(n) if n > 0 => println!(
                        "embedded {n} chunk(s) (model: {})",
                        chosen.model.as_deref().unwrap_or("daemon default")
                    ),
                    Ok(_) => {}
                    // A cold or unreachable embedder is a condition of
                    // the machine, not a defect: the chunks stay
                    // pending and the next run picks them up, which is
                    // strictly better than a failed unit that says
                    // nothing about what to do. A store error is a real
                    // fault and still propagates.
                    Err(e @ lisa_contextd::StoreError::Io(_)) => println!(
                        "not embedding: the embedder did not answer ({e}); chunks stay \
                         pending and the next sync picks them up"
                    ),
                    Err(e) => return Err(e.into()),
                }
            } else {
                println!(
                    "not embedding: no model-backed embedder (chunks stay pending; \
                     rerun after `lisa models get {}`)",
                    lisa_contextd::embed::EMBEDDING_MODEL
                );
            }
        }
        ContextCmd::Search {
            query,
            limit,
            hybrid,
            scope,
        } => {
            let query = query.join(" ");
            let chosen = hybrid.then(lisa_contextd::embed::resolve);
            let embedder_kind = chosen.as_ref().map_or("none", |c| c.kind);
            let embedder_model = chosen
                .as_ref()
                .and_then(|c| c.model.clone())
                .unwrap_or_else(|| "(daemon default)".into());
            // Every retrieval is ledgered (PLAN §5.3) — query hash, not text.
            let ledger = lisa_ledger::Ledger::open(lisa_ledger::Ledger::default_path())?;
            ledger.append(&lisa_ledger::Event {
                kind: match (!scope.is_empty(), hybrid) {
                    (true, true) => "context.search.hybrid_scoped",
                    (true, false) => "context.search.scoped",
                    (false, true) => "context.search.hybrid",
                    (false, false) => "context.search",
                }
                .into(),
                app_id: "host".into(),
                input_hash: blake3::hash(query.as_bytes()).to_hex().to_string(),
                status: "ok".into(),
                detail: serde_json::json!({
                    "embedder": embedder_kind,
                    "embedder_model": embedder_model,
                })
                .to_string(),
                ..Default::default()
            })?;
            // Scope and hybrid are orthogonal — visibility vs ranking —
            // and the daemon's D-Bus path already learned this the hard
            // way (see search_hybrid_scoped's doc comment). The first
            // version of THIS branch had the same bug: --scope silently
            // won over --hybrid, so the caller got the worse ranking,
            // an "ok" exit, and a ledger row that erased the request.
            let hits = if !scope.is_empty() && hybrid {
                let scopes: Vec<&str> = scope.iter().map(String::as_str).collect();
                store.search_hybrid_scoped(
                    &query,
                    &scopes,
                    chosen
                        .as_ref()
                        .map(|c| c.embedder.as_ref())
                        .expect("hybrid implies an embedder"),
                    limit,
                )?
            } else if !scope.is_empty() {
                let scopes: Vec<&str> = scope.iter().map(String::as_str).collect();
                store.search_scoped(&query, &scopes, limit)?
            } else if hybrid {
                store.search_hybrid(
                    &query,
                    chosen
                        .as_ref()
                        .map(|c| c.embedder.as_ref())
                        .expect("hybrid implies an embedder"),
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
            // A piper voice is weights plus an .onnx.json config, and the
            // weights alone will not synthesize. Pulled under `<id>.json`
            // so the engine can find it next to the ref it was told about.
            //
            // Half-pinned is refused rather than half-fetched: an entry
            // naming a config with no hash would otherwise install an
            // unverified file beside a verified one.
            match (&entry.config_source, &entry.config_blake3) {
                (Some(cfg_src), Some(cfg_hash)) => {
                    let c = fetch::pull(&store, cfg_src, &format!("{id}.json"), cfg_hash)?;
                    println!("installed `{}` (config, {} bytes)", c.name, c.size);
                }
                (None, None) => {}
                _ => bail!(
                    "`{id}` pins only half of its config artifact — \
                     config_source and config_blake3 must both be set"
                ),
            }
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
mod attachment_tests {
    use super::attachment_part;

    /// Write a temp file with the given extension and bytes.
    fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        // Per-process: a fixed name under the shared temp dir has two
        // test binaries writing the same file at once, and a torn read
        // here would look like an attachment-encoding bug.
        let dir = std::env::temp_dir().join(format!("lisa-attach-tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn an_image_becomes_a_data_uri_image_part() {
        // The shape a provider expects, and the one inferenced's
        // `Content::Parts` forwards verbatim (#209).
        let p = tmp("shot.png", b"\x89PNG\r\n\x1a\n");
        let v = attachment_part(&p).unwrap();
        assert_eq!(v["type"], "image_url");
        let url = v["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        assert!(
            !url.ends_with("base64,"),
            "the payload must actually be there"
        );
    }

    #[test]
    fn audio_is_a_bare_payload_and_a_format_name_not_a_data_uri() {
        // The asymmetry that makes this worth a test: `image_url` takes
        // a data: URI and `input_audio` takes raw base64 plus a format.
        // Sending one shape where the other is expected produces a
        // provider-side error that reads like our bug.
        let p = tmp("clip.wav", b"RIFF....WAVE");
        let v = attachment_part(&p).unwrap();
        assert_eq!(v["type"], "input_audio");
        assert_eq!(v["input_audio"]["format"], "wav");
        let data = v["input_audio"]["data"].as_str().unwrap();
        assert!(
            !data.starts_with("data:"),
            "audio takes no data: URI: {data}"
        );
        assert!(!data.is_empty());
    }

    #[test]
    fn the_extension_decides_and_is_case_insensitive() {
        for (name, want) in [
            ("a.PNG", "image_url"),
            ("a.JPEG", "image_url"),
            ("a.WebP", "image_url"),
            ("a.gif", "image_url"),
            ("a.MP3", "input_audio"),
        ] {
            let p = tmp(name, b"x");
            assert_eq!(attachment_part(&p).unwrap()["type"], want, "{name}");
        }
    }

    #[test]
    fn an_unsupported_file_is_refused_by_name_and_lists_what_works() {
        // Guessing `image/*` for an unknown type is what the doc comment
        // above the function warns against: it produces a provider error
        // that reads like our bug rather than like the user's typo.
        let p = tmp("notes.pdf", b"%PDF-1.4");
        let e = attachment_part(&p).unwrap_err().to_string();
        assert!(e.contains("notes.pdf"), "{e}");
        assert!(
            e.contains("png"),
            "the message must say what IS supported: {e}"
        );
        assert!(e.contains("wav"), "{e}");
    }

    #[test]
    fn a_file_with_no_extension_is_refused_rather_than_sniffed() {
        let p = tmp("screenshot", b"\x89PNG\r\n\x1a\n");
        assert!(
            attachment_part(&p).is_err(),
            "content sniffing is not the contract"
        );
    }

    #[test]
    fn a_missing_file_names_itself_in_the_error() {
        let e = attachment_part(std::path::Path::new("/nonexistent/x.png"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("/nonexistent/x.png"), "{e}");
    }
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

    /// **No user-facing verb tells you to escalate** (#243, CLAUDE.md 7b,
    /// ADR-0034 §3).
    ///
    /// The removed Flutter lane's `lisa forge --setup` once printed
    /// *"run `sudo lisa forge --setup`"* when its `/var` create failed.
    /// `escalate.privilege` is an unoverridable `Deny` in our own guard,
    /// so the CLI advising it is the machine arguing with itself. The
    /// lane is gone; the rule outlives it.
    ///
    /// Asserted over the source rather than by running it, because the
    /// failure is a string a person only sees when the path is already
    /// broken — the one place nobody looks.
    #[test]
    fn no_verb_ever_advises_sudo() {
        let src = include_str!("main.rs");
        // Nowhere in the CLI does anything tell you to run one of
        // our own verbs as root. `lisa update` may still print a
        // `bootctl` recovery line — that is a bootloader command, not
        // this program asking to be re-run with privilege it refuses to
        // request.
        // Built at runtime so this assertion is not itself a hit.
        let needle = format!("{}{} lisa", "su", "do");
        for (i, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            assert!(
                !code.contains(&needle),
                "line {} tells the user to run a lisa verb with elevated \
                 privilege: {line}",
                i + 1
            );
        }
    }

    /// **The Forge's default lane runs `lisa dev check`** (#243,
    /// ADR-0047 §4, ADR-0050 §4).
    ///
    /// It ran `Verifier::Dart`, which on a directory of `.js` files
    /// reports "the project contains no Dart source files yet" and can
    /// never converge — so the headline feature could not produce the
    /// kind of app Lisa ships. `tests/forge_verifier.rs` proves the arm
    /// works; this proves it is the arm chosen.
    #[test]
    fn the_default_lane_verifies_with_lisa_dev_check() {
        let forge_harness::Verifier::Command { program, args } = default_verifier() else {
            panic!(
                "the default lane must run a command verifier — a \
                 language-specific arm is a second opinion about what a \
                 valid Lisa app is (ADR-0050 §4)"
            );
        };
        assert_eq!(args, ["dev", "check"]);
        assert!(
            std::path::Path::new(&program).is_absolute(),
            "the verifier resolves `lisa` through $PATH: {program}"
        );
    }
}

#[cfg(test)]
mod remote_tests {
    use super::{os_release_value, parse_remote_model};

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

    /// The real thing, verbatim from the reference iMac running .47.
    const OS_RELEASE: &str = r#"NAME="Lisa OS"
ID=lisa
VERSION_ID=50
IMAGE_VERSION="20260727.47"
PRETTY_NAME="Lisa OS 20260727.47"
"#;

    #[test]
    fn os_release_value_strips_the_shell_quotes() {
        // Unstripped, this returns `"20260727.47"` with quotes, which
        // never equals the bus's `20260727.47` — so `lisa update` would
        // claim an update is staged on every single run.
        assert_eq!(
            os_release_value(OS_RELEASE, "IMAGE_VERSION").as_deref(),
            Some("20260727.47")
        );
        assert_eq!(
            os_release_value("IMAGE_VERSION='20260728.49'", "IMAGE_VERSION").as_deref(),
            Some("20260728.49")
        );
        // Unquoted is legal in os-release(5) too.
        assert_eq!(
            os_release_value("IMAGE_VERSION=20260728.49", "IMAGE_VERSION").as_deref(),
            Some("20260728.49")
        );
    }

    #[test]
    fn os_release_value_does_not_match_a_longer_key() {
        // `VERSION` must not be answered by `VERSION_ID` or by
        // `IMAGE_VERSION`: a suffix/prefix collision here would report
        // GNOME's 50 as the image version and compare nonsense.
        assert_eq!(os_release_value(OS_RELEASE, "VERSION"), None);
        assert_eq!(
            os_release_value(OS_RELEASE, "VERSION_ID").as_deref(),
            Some("50")
        );
    }

    #[test]
    fn os_release_value_ignores_comments_and_absent_keys() {
        assert_eq!(
            os_release_value("# IMAGE_VERSION=nope\n", "IMAGE_VERSION"),
            None
        );
        assert_eq!(os_release_value(OS_RELEASE, "BUILD_ID"), None);
        // An empty value is absent, not an empty version string — an
        // empty one would sort below every real version and read as
        // "something newer is staged" forever.
        assert_eq!(
            os_release_value("IMAGE_VERSION=\"\"", "IMAGE_VERSION"),
            None
        );
    }

    /// The staged-vs-current comparison itself. Versions are YYYYMMDD.run,
    /// so lexicographic ordering is chronological — this pins that, because
    /// the whole fix rests on it.
    #[test]
    fn version_strings_order_chronologically() {
        assert!("20260728.49" > "20260727.47");
        assert!("20260727.47" > "20260727.44");
        // Same day, later run.
        assert!("20260727.47" > "20260727.46");
        // Not newer than itself: this is the "truly up to date" case.
        assert!(!("20260728.49" > "20260728.49"));
    }
}

#[cfg(test)]
mod prefetch_tests {
    use super::is_permission_denied;

    /// Issue #140: `lisa update` told the desktop user to run
    /// `sudo lisa apps sync`, which is the exact shape ADR-0034 forbids
    /// — nothing user-facing should need sudo, and `escalate.privilege`
    /// is an unoverridable Deny in our own guard. Distinguishing "you
    /// are not root" from "the channel is unreachable" is what lets the
    /// message be honest without escalating.
    #[test]
    fn a_permission_error_is_told_apart_from_a_real_failure() {
        let denied = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Permission denied (os error 13)",
        ));
        assert!(is_permission_denied(&denied));

        // …including when it is wrapped, which is how it actually
        // arrives: apps::sync adds context about which payload and
        // which directory before it reaches the caller.
        let wrapped = denied.context("creating /var/lib/lisa/apps/payloads/shell/versions");
        assert!(is_permission_denied(&wrapped));
    }

    #[test]
    fn an_unreachable_channel_is_not_mistaken_for_a_permission_problem() {
        // Reporting "leave it to the timer" for a network failure would
        // be a comfortable lie: the timer will fail the same way.
        let offline = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        assert!(!is_permission_denied(&offline));
        assert!(!is_permission_denied(&anyhow::anyhow!("checksum mismatch")));
    }
}
