//! `lisa apps` — the out-of-image payload channel (ADR-0020, widened by
//! ADR-0023 phase 1): refresh what the image no longer carries without
//! touching a boot slot.
//!
//! Two kinds of payload ride the same release channel today:
//!
//! * **`shell`** — the interpreted GJS surfaces (ADR-0020). Arch-independent,
//!   and the image still bakes a copy, so this channel is opt-in: it only
//!   moves when the user runs `lisa apps update`.
//! * **`zen`** — the Zen browser tree that used to be `/opt/zen` in the image
//!   (ADR-0023 phase 1). Per-architecture, and after the migration release
//!   the image carries NO copy — so this channel is auto-synced
//!   (`lisa apps sync`, run by lisa-apps-sync.timer and by `lisa update`
//!   before it stages a new slot). Losing the browser to an OS update is the
//!   failure this exists to prevent.
//!
//! Layout, per channel: `<base>/versions/<ver>/` holds one full tree and
//! `<base>/current` is a symlink flipped atomically (symlink + rename).
//! `<base>` is `/var/lib/lisa-apps/payloads/<name>`. It sits beside the
//! model store rather than inside `/var/lib/lisa`, which is a DynamicUser
//! StateDirectory whose real path (`/var/lib/private/lisa`) no ordinary
//! user can traverse — see APPS_DIR. Older locations are still read, never
//! written, so a device mid-upgrade keeps resolving what it already has.
//! Integrity: sha256 against the release's SHA256SUMS manifest (same trust
//! level as the sysupdate transfer set; GPG signing lands with the M1
//! signed repo).
//!
//! **`resolve` is the single authority for where a payload is** (issue
//! #239). `place_tree` writes through `Channel::base`, `resolve` reads
//! through it, and every launcher that is not this program asks —
//! `/usr/bin/lisa-app` runs `lisa apps path shell` rather than spelling a
//! `/var` path of its own. It used to spell one, the two spellings drifted
//! apart, and for three releases `lisa apps update` unpacked a tree that
//! nothing ever launched while `lisa apps status` called it current.

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{IsTerminal, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Where payloads live.
///
/// **Beside the model store, not inside `/var/lib/lisa`.** That path is
/// lisa-inferenced's `StateDirectory` with `DynamicUser=`, so systemd
/// puts the real tree under `/var/lib/private/lisa`, and both
/// `/var/lib/private` (0700 root) and `/var/lib/private/lisa` (0700
/// nobody) are closed to everyone. Payloads written there are unreadable
/// by the user who has to execute them.
///
/// That was not theoretical: on the reference iMac the zen payload
/// installed correctly, `lisa apps status` reported it current as root,
/// and `[ -x .../current/zen ]` failed as the desktop user — so
/// `/usr/bin/zen-browser` fell through to the baked `/opt/zen` every
/// time. The fallback hid it, and deleting `/opt/zen` (issue #89) would
/// have shipped a browser that could not start.
///
/// `/var/lib/lisa-models` already solved exactly this for models, with a
/// tmpfiles rule making it 2775 root:lisa. Payloads use the same shape.
const APPS_DIR: &str = "/var/lib/lisa-apps";

/// Mode for every directory the channel machinery creates: setgid so
/// unpacked files inherit group `lisa`, group-writable so the DESKTOP USER
/// can install into a tree root that root created first.
///
/// lisa-apps-sync.service runs as root and gets there first on most
/// devices. Without this, `create_dir_all` left `payloads/zen` and its
/// `versions/` at 0755 root:lisa and `lisa apps update` failed for the
/// user with `Permission denied (os error 13)` — ADR-0034 §7b says
/// nothing user-facing needs sudo, and `escalate.privilege` is an
/// unoverridable Deny in our own guard, so "run it with sudo" is not an
/// answer (issue #239 defect 4).
const DIR_MODE: u32 = 0o2775;

/// Prefix for the absolute system paths this module reads (baked trees,
/// pre-migration payload locations). Empty in production; a tempdir in
/// tests, so a test's answer never depends on what the dev host happens
/// to have in `/usr` or `/var`.
fn sys_root() -> Option<PathBuf> {
    std::env::var_os("LISA_APPS_ROOT").map(PathBuf::from)
}

/// An absolute system path, under `sys_root()` when one is set.
fn abs(p: &str) -> PathBuf {
    match sys_root() {
        Some(r) => r.join(p.trim_start_matches('/')),
        None => PathBuf::from(p),
    }
}

fn apps_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("LISA_APPS_STATE") {
        return PathBuf::from(p);
    }
    abs(APPS_DIR)
}

/// The architecture whose payloads this system takes. Overridable so the
/// packaging lane can build/inspect the other arch's channel from CI.
fn payload_arch() -> String {
    std::env::var("LISA_PAYLOAD_ARCH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

/// One payload channel: a named tree on /var that the release channel
/// refreshes independently of the OS image.
struct Channel {
    /// Selector for `lisa apps <verb> <name>`.
    name: &'static str,
    /// One line for `lisa apps status`.
    what: &'static str,
    /// Release-asset name is `<prefix><ver>.tar.zst`, or
    /// `<prefix><ver>_<arch>.tar.zst` when `per_arch`.
    prefix: &'static str,
    per_arch: bool,
    /// State dir under `apps_dir()`. Every channel has one — a channel
    /// that lived at the root of `apps_dir()` is how the shell tree ended
    /// up somewhere no launcher looked (#239).
    subdir: &'static str,
    /// A path inside a tree that must exist for it to count as a tree at
    /// all. `has_tree`-by-symlink alone reports success for an empty
    /// directory, which is the shape of defect #239: bookkeeping that is
    /// true of a state record and false of the running system.
    probe: &'static str,
    /// Installed version trees to keep (including the current one). Payload
    /// trees are hundreds of MiB; /var is finite.
    keep: usize,
    /// Fetch automatically when no tree is installed yet. True only for
    /// payloads with NO baked fallback in the image — otherwise a fresh
    /// install would pull a copy of something it already has, and skew the
    /// tree ahead of the image for no reason (ADR-0020 consequences).
    auto_sync: bool,
    /// Directories an EARLIER release installed this channel to. Read when
    /// this channel's own tree is absent; never written. A device
    /// mid-upgrade keeps launching what it already has.
    stale: &'static [&'static str],
    /// The tree the IMAGE bakes, if any — the last thing `resolve`
    /// returns and the floor `rollback` falls back to.
    baked: Option<&'static str>,
    /// Whether that baked tree is a permanent floor (`shell`, `runtime`:
    /// every image carries one) or a pre-migration leftover the current
    /// image no longer ships (`zen`). Only the wording differs, but the
    /// difference is the whole reason `zen` auto-syncs and `shell` does
    /// not.
    baked_is_floor: bool,
}

const CHANNELS: &[Channel] = &[
    Channel {
        name: "shell",
        what: "GJS surfaces and apps — assistant, Ledger, Mail, Surfer, Preview",
        prefix: "lisa-apps_",
        per_arch: false,
        subdir: "payloads/shell",
        // Any file under the tree would do; the assistant is the surface
        // whose absence is noticed first.
        probe: "assistant/lisa-assistant.js",
        keep: 3,
        auto_sync: false,
        // Where releases up to 20260804.76 unpacked it (the root of the
        // state dir, which no launcher ever read — #239), and the
        // original ADR-0020 location before the DynamicUser move.
        stale: &["/var/lib/lisa-apps", "/var/lib/lisa/apps"],
        // The image always bakes this, so it is a permanent floor.
        baked: Some("/usr/share/lisa/shell"),
        baked_is_floor: true,
    },
    Channel {
        name: "runtime",
        what: "the lisa CLI and its skills — updated without a reboot (issue #52)",
        prefix: "lisa-runtime_",
        // A compiled binary: the payload must match the machine.
        per_arch: true,
        subdir: "payloads/runtime",
        probe: "bin/lisa",
        keep: 2,
        // The image always bakes the CLI at /usr/lib/lisa/bin/lisa and the
        // resolver falls back to it, so this is a permanent floor, not a
        // migration — nothing to auto-pull on a fresh install.
        auto_sync: false,
        stale: &["/var/lib/lisa/apps/payloads/runtime"],
        baked: Some("/usr/lib/lisa"),
        baked_is_floor: true,
    },
    Channel {
        name: "zen",
        what: "Zen browser — the tree that used to be baked as /opt/zen",
        prefix: "lisa-zen_",
        per_arch: true,
        subdir: "payloads/zen",
        probe: "zen",
        keep: 2,
        auto_sync: true,
        stale: &["/var/lib/lisa/apps/payloads/zen"],
        // Pre-migration images baked this; new ones do not, which is why
        // this is the one channel that auto-syncs.
        baked: Some("/opt/zen"),
        baked_is_floor: false,
    },
];

/// Where a tree that `resolve` returned came from.
#[derive(Debug, PartialEq, Eq)]
enum Source {
    /// This channel's own tree on /var, at the version `current` records.
    Channel(String),
    /// A location an older release installed to. Read, never written.
    Stale,
    /// The copy the image bakes.
    Baked,
}

/// A directory a launcher would actually run code out of.
struct Resolved {
    dir: PathBuf,
    source: Source,
}

impl Resolved {
    fn what(&self) -> String {
        match &self.source {
            Source::Channel(v) => format!("the {v} tree this channel installed"),
            Source::Stale => "a tree an older release left behind".to_string(),
            Source::Baked => "the tree the image bakes".to_string(),
        }
    }
}

impl Channel {
    fn base(&self) -> PathBuf {
        apps_dir().join(self.subdir)
    }

    /// `<base>/current` — the symlink `place_tree` flips and every reader
    /// follows. One spelling, one function.
    fn current_link(&self) -> PathBuf {
        self.base().join("current")
    }

    /// Whether `dir` holds a real tree of this channel, rather than an
    /// empty directory or a dangling symlink. Metadata follows symlinks.
    fn is_tree(&self, dir: &Path) -> bool {
        dir.join(self.probe).exists()
    }

    /// Every directory a launcher should try, best first. This — not a
    /// path list in a shell script — is what `/usr/bin/lisa-app` and
    /// `lisa apps status` both go through (#239).
    fn resolution(&self) -> Vec<Resolved> {
        let mut out = Vec::new();
        let cur = self.current_link();
        if self.is_tree(&cur) {
            let v = current_version(self).unwrap_or_else(|| "?".into());
            out.push(Resolved {
                dir: cur,
                source: Source::Channel(v),
            });
        }
        for s in self.stale {
            let d = abs(s).join("current");
            if self.is_tree(&d) {
                out.push(Resolved {
                    dir: d,
                    source: Source::Stale,
                });
            }
        }
        if let Some(b) = self.baked {
            let d = abs(b);
            if self.is_tree(&d) {
                out.push(Resolved {
                    dir: d,
                    source: Source::Baked,
                });
            }
        }
        out
    }

    /// The one directory a launch would use, if any.
    fn resolve(&self) -> Option<Resolved> {
        self.resolution().into_iter().next()
    }

    /// The version an asset name carries, if it belongs to this channel and
    /// this architecture.
    fn version_of(&self, asset: &str, arch: &str) -> Option<String> {
        let rest = asset.strip_prefix(self.prefix)?;
        let suffix = if self.per_arch {
            format!("_{arch}.tar.zst")
        } else {
            ".tar.zst".to_string()
        };
        rest.strip_suffix(&suffix).map(str::to_string)
    }
}

fn channels_for(only: Option<&str>) -> anyhow::Result<Vec<&'static Channel>> {
    let Some(name) = only else {
        return Ok(CHANNELS.iter().collect());
    };
    match CHANNELS.iter().find(|c| c.name == name) {
        Some(c) => Ok(vec![c]),
        None => {
            let known: Vec<&str> = CHANNELS.iter().map(|c| c.name).collect();
            bail!(
                "unknown apps channel {name:?} — known channels: {}",
                known.join(", ")
            )
        }
    }
}

/// The newest release, fetched once and shared across channels.
struct Release {
    tag: String,
    /// (asset name, download URL)
    assets: Vec<(String, String)>,
    sums_url: Option<String>,
    sums: Option<String>,
}

impl Release {
    fn latest() -> anyhow::Result<Self> {
        let mut resp = ureq::get(crate::RELEASES_API)
            .call()
            .context("fetching the latest release")?;
        let json: serde_json::Value = resp.body_mut().read_json().context("parsing the release")?;
        let tag = json["tag_name"].as_str().unwrap_or("?").to_string();
        let assets: Vec<(String, String)> = json["assets"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|a| {
                        Some((
                            a["name"].as_str()?.to_string(),
                            a["browser_download_url"].as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sums_url = assets
            .iter()
            .find(|(n, _)| n == "SHA256SUMS")
            .map(|(_, u)| u.clone());
        Ok(Self {
            tag,
            assets,
            sums_url,
            sums: None,
        })
    }

    /// The newest asset of `ch` for `arch`: (asset name, version, URL).
    fn asset_for(&self, ch: &Channel, arch: &str) -> Option<(String, String, String)> {
        self.assets
            .iter()
            .filter_map(|(n, u)| ch.version_of(n, arch).map(|v| (n.clone(), v, u.clone())))
            .max_by(|a, b| ver_key(&a.1).cmp(&ver_key(&b.1)))
    }

    /// The sha256 the manifest records for `asset`, fetching the manifest on
    /// first use.
    fn expected_sha256(&mut self, asset: &str) -> anyhow::Result<String> {
        if self.sums.is_none() {
            let url = self
                .sums_url
                .clone()
                .with_context(|| format!("release {} has no SHA256SUMS manifest", self.tag))?;
            self.sums = Some(
                ureq::get(&url)
                    .call()
                    .context("fetching SHA256SUMS")?
                    .body_mut()
                    .read_to_string()?,
            );
        }
        let sums = self.sums.as_deref().unwrap_or_default();
        sums.lines()
            .find_map(|l| l.strip_suffix(asset).map(|h| h.trim().to_string()))
            .with_context(|| format!("{asset} is not in SHA256SUMS"))
    }
}

enum Outcome {
    Installed(String),
    AlreadyCurrent(String),
    /// This release publishes nothing for this channel/arch.
    NotPublished,
}

/// Fetch, verify, unpack, and activate `ch`'s newest asset.
fn install(ch: &Channel, rel: &mut Release, arch: &str) -> anyhow::Result<Outcome> {
    let Some((asset, ver, url)) = rel.asset_for(ch, arch) else {
        return Ok(Outcome::NotPublished);
    };
    // The tree as well as the version, deliberately: if `current` points
    // at a version whose tree is gone (hand-deleted, a truncated /var, a
    // failed rename), the symlink alone would say "already current" and
    // this would refuse to repair the very thing sync() was asked to
    // repair — a browser that stays broken forever.
    if current_version(ch).as_deref() == Some(ver.as_str()) && ch.is_tree(&ch.current_link()) {
        return Ok(Outcome::AlreadyCurrent(ver));
    }
    let expected = rel.expected_sha256(&asset)?;
    println!(">> {}: downloading {asset} ({})…", ch.name, rel.tag);
    place_tree(ch, &ver, &|dest: &Path| {
        // Streamed, not buffered: the zen payload is ~100 MiB compressed and
        // reading it into a Vec before writing it out doubles the peak RSS
        // on a device that may only have a few GiB.
        let got = download_verified(&url, dest)?;
        if got != expected {
            bail!("checksum mismatch for {asset}: expected {expected}, got {got}");
        }
        Ok(())
    })?;
    Ok(Outcome::Installed(ver))
}

/// Put a payload tarball on disk as `<base>/versions/<ver>` and make it
/// current. `fetch` writes the (verified) tarball to the path it is given.
///
/// Stage-then-rename (issue #17): the tarball is fetched and unpacked inside
/// a hidden staging dir and only renamed into place once complete, so a
/// kill/power-loss mid-transfer never leaves a partial tree that
/// status/rollback would trust. Hidden names are ignored by
/// installed_versions. The pid in the name keeps concurrent runs from
/// clobbering each other's staging; the final flip stays atomic either way.
fn place_tree(
    ch: &Channel,
    ver: &str,
    fetch: &dyn Fn(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let base = ch.base();
    let versions = base.join("versions");
    std::fs::create_dir_all(&versions)
        .with_context(|| format!("creating {}", versions.display()))?;
    // Whoever gets here first — root from lisa-apps-sync.service, or the
    // desktop user from `lisa apps update` — leaves the tree writable by
    // group `lisa` for the other (#239 defect 4). Best-effort: only the
    // owner may chmod, and a directory root already created with the
    // wrong mode is repaired by tmpfiles at boot, not from here.
    for d in [versions.as_path(), base.as_path()] {
        let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(DIR_MODE));
    }
    let staging = versions.join(format!(".staging-{}-{}", ver, std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let dest = versions.join(ver);
    let out = (|| -> anyhow::Result<()> {
        let tmp_tar = staging.join("payload.tar.zst");
        fetch(&tmp_tar)?;
        let unpack_dir = staging.join("tree");
        std::fs::create_dir_all(&unpack_dir)?;
        let status = std::process::Command::new("tar")
            .arg("--zstd")
            .arg("-xf")
            .arg(&tmp_tar)
            .arg("-C")
            .arg(&unpack_dir)
            .status()
            .context("running tar (needs zstd support)")?;
        if !status.success() {
            bail!("unpacking the {} payload failed ({status})", ch.name);
        }
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        std::fs::rename(&unpack_dir, &dest).context("moving the unpacked tree into place")?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staging);
    out?;

    flip_current(ch, ver)?;
    // Housekeeping, not the install. A tree root another uid unpacked
    // cannot be removed by this one, and failing an update over that
    // would leave the device unable to take the payload it has already
    // verified, unpacked and activated (#239 defect 4, on a device whose
    // zen trees root installed first).
    if let Err(e) = prune(ch) {
        eprintln!("-- {}: could not prune older versions: {e:#}", ch.name);
    }
    Ok(())
}

/// `lisa apps install <channel> <tarball> --version <ver>`: put a payload
/// you already have on disk into a channel.
///
/// The release path verifies every payload against the release manifest.
/// This one has no manifest to check against — offline media, or a build
/// under test — so it says that instead of implying a verification it did
/// not do.
pub fn install_local(channel: &str, tarball: &Path, ver: &str) -> anyhow::Result<()> {
    let ch = channels_for(Some(channel))?[0];
    if ver.is_empty() || ver.contains('/') {
        bail!("{ver:?} is not a version — expected YYYYMMDD.run");
    }
    if !tarball.is_file() {
        bail!("{} is not a file", tarball.display());
    }
    let src = std::fs::canonicalize(tarball)
        .with_context(|| format!("resolving {}", tarball.display()))?;
    println!(
        ">> {}: installing {} as {ver} — NOT verified against a release manifest",
        ch.name,
        src.display()
    );
    place_tree(ch, ver, &|dest: &Path| {
        std::fs::copy(&src, dest)
            .with_context(|| format!("copying {} to {}", src.display(), dest.display()))?;
        Ok(())
    })?;
    println!(
        "{}: {ver} is current — it takes effect on the next launch, no reboot",
        ch.name
    );
    Ok(())
}

/// `lisa apps path <channel>`: the directories a launcher searches, best
/// first, one per line. `/usr/bin/lisa-app` reads this instead of
/// carrying a path list of its own (#239 defect 1).
pub fn print_path(channel: &str, base_only: bool) -> anyhow::Result<()> {
    let ch = channels_for(Some(channel))?[0];
    if base_only {
        println!("{}", ch.base().display());
        return Ok(());
    }
    let found = ch.resolution();
    if found.is_empty() {
        bail!(
            "no {} tree on this system — not on the channel ({}), \
             not baked by the image",
            ch.name,
            ch.base().display()
        );
    }
    for r in found {
        println!("{}", r.dir.display());
    }
    Ok(())
}

/// Stream `url` into `dest`, hashing as it goes; returns the hex sha256.
fn download_verified(url: &str, dest: &Path) -> anyhow::Result<String> {
    let mut resp = ureq::get(url).call().context("downloading the payload")?;
    let mut reader = resp.body_mut().as_reader();
    let mut file =
        std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done: u64 = 0;
    let mut marked: u64 = 0;
    let tty = std::io::stdout().is_terminal();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        if tty && done - marked >= 8 << 20 {
            marked = done;
            print!("\r   {} MiB", done >> 20);
            let _ = std::io::stdout().flush();
        }
    }
    file.sync_all()?;
    if tty && marked > 0 {
        println!("\r   {} MiB", done >> 20);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// `lisa apps update [channel]`: fetch, verify, install, and activate the
/// newest payload for every channel (or just one).
pub fn update(only: Option<&str>) -> anyhow::Result<()> {
    let chans = channels_for(only)?;
    let explicit = only.is_some();
    let arch = payload_arch();
    let mut rel = Release::latest()?;
    let mut failed = 0usize;
    for ch in chans {
        match install(ch, &mut rel, &arch) {
            Ok(Outcome::Installed(v)) => println!(
                "{}: {v} is current — it takes effect on the next launch, no reboot",
                ch.name
            ),
            Ok(Outcome::AlreadyCurrent(v)) => println!("{}: {v} is already current", ch.name),
            Ok(Outcome::NotPublished) => {
                let msg = format!(
                    "release {} publishes no {} payload for {arch}",
                    rel.tag, ch.name
                );
                if explicit {
                    bail!("{msg} — nothing to install");
                }
                println!("{}: skipped — {msg}", ch.name);
            }
            Err(e) => {
                eprintln!("!! {}: {e:#}", ch.name);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("{failed} channel(s) failed to update");
    }
    Ok(())
}

/// `lisa apps sync`: install auto-sync channels that have NO tree on /var
/// yet, and leave everything already installed alone.
///
/// This is the "never lose the browser" path (ADR-0023 phase 1). It runs
/// from lisa-apps-sync.timer once the machine is online, and from
/// `lisa update` before a new root slot is staged — so a device that is
/// about to boot an image without `/opt/zen` already has the browser on its
/// persistent /var. Deliberately NOT an upgrade: a payload the user already
/// has never moves version behind their back.
pub fn sync() -> anyhow::Result<()> {
    let arch = payload_arch();
    let missing: Vec<&Channel> = CHANNELS
        .iter()
        .filter(|c| c.auto_sync && !c.is_tree(&c.current_link()))
        .collect();
    if missing.is_empty() {
        println!("apps sync: every auto-synced payload is already installed");
        return Ok(());
    }
    let mut rel = Release::latest()?;
    let mut failed = 0usize;
    for ch in missing {
        match install(ch, &mut rel, &arch) {
            Ok(Outcome::Installed(v)) => println!("{}: installed {v}", ch.name),
            Ok(Outcome::AlreadyCurrent(v)) => println!("{}: {v} is already current", ch.name),
            Ok(Outcome::NotPublished) => println!(
                "{}: release {} publishes no payload for {arch} — the image copy still applies",
                ch.name, rel.tag
            ),
            Err(e) => {
                eprintln!("!! {}: {e:#}", ch.name);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("{failed} payload(s) could not be synced");
    }
    Ok(())
}

/// `lisa apps status`: what is installed, and — the part that matters —
/// what a launch would ACTUALLY run.
///
/// It used to print the state record alone: `shell — current: 20260804.76,
/// installed: 20260804.76`, which was true of a symlink and false of the
/// running system, and would have printed the same for a tree that was
/// empty, absent, or unpacked where no launcher looks (#239 defect 3).
/// That is the shape of #219 (a socket treated as a working tool) and
/// #192 (a comment asserting a bound nothing enforced): a report about
/// bookkeeping presented as a report about reality.
pub fn status() -> anyhow::Result<()> {
    print!("{}", report()?);
    Ok(())
}

fn report() -> anyhow::Result<String> {
    let mut out = String::new();
    writeln!(out, "payload arch: {}", payload_arch())?;
    for ch in CHANNELS {
        let base = ch.base();
        let recorded = current_version(ch);
        let resolved = ch.resolve();
        writeln!(out, "\n{} — {}", ch.name, ch.what)?;
        match &recorded {
            Some(v) => writeln!(out, "  current: {v}")?,
            None => writeln!(out, "  current: (none installed on /var)")?,
        }
        for v in installed_versions(&base)? {
            writeln!(out, "  installed: {v}")?;
        }
        match &resolved {
            Some(r) => writeln!(out, "  resolves: {} — {}", r.dir.display(), r.what())?,
            None => writeln!(
                out,
                "  resolves: NOTHING — no usable tree on the channel or in the image"
            )?,
        }
        // The line #239 existed for: say plainly when the recorded
        // version is not what launches.
        let running = match &resolved {
            Some(r) => match &r.source {
                Source::Channel(v) => Some(v.clone()),
                _ => None,
            },
            None => None,
        };
        if let Some(v) = &recorded
            && running.as_deref() != Some(v.as_str())
        {
            writeln!(
                out,
                "  !! {v} is recorded but does NOT launch — nothing usable at {}",
                ch.current_link().display()
            )?;
        }
    }
    Ok(out)
}

/// `lisa apps rollback [channel]`: flip to the newest installed version
/// strictly OLDER than the current one; with none older, drop back to the
/// baked tree. Already on the baked tree → nothing to roll back to.
pub fn rollback(only: Option<&str>) -> anyhow::Result<()> {
    for ch in channels_for(only)? {
        let base = ch.base();
        let Some(current) = current_version(ch) else {
            println!(
                "{}: already on the baked image tree — nothing to roll back",
                ch.name
            );
            continue;
        };
        let mut versions = installed_versions(&base)?;
        versions.retain(|v| ver_key(v) < ver_key(&current));
        match versions.last() {
            Some(prev) => {
                flip_current(ch, prev)?;
                println!("{}: rolled back to {prev}", ch.name);
            }
            None => {
                let _ = std::fs::remove_file(ch.current_link());
                println!(
                    "{}: no older installed version — {}",
                    ch.name,
                    if ch.baked_is_floor {
                        "the baked image tree is active again"
                    } else {
                        "nothing on /var now; run `lisa apps update` to reinstall"
                    }
                );
            }
        }
    }
    Ok(())
}

/// Order key for `YYYYMMDD.run` versions: numeric per component, so
/// `.30` sorts after `.9` (plain string compare gets that wrong).
fn ver_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

/// The version `current` points at, if any. Does not require the tree to
/// still exist — see `Channel::is_tree` for that.
fn current_version(ch: &Channel) -> Option<String> {
    ch.current_link()
        .read_link()
        .ok()?
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
}

fn installed_versions(base: &Path) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base.join("versions")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // Hidden entries are in-flight staging dirs (issue #17) —
            // never report them as installed.
            if e.path().is_dir() && !name.starts_with('.') {
                out.push(name);
            }
        }
    }
    out.sort_by_key(|v| ver_key(v));
    Ok(out)
}

/// Drop the oldest trees beyond `ch.keep`, never the current one. Payload
/// channels put hundreds of MiB per version on a /var that is also the
/// model store — unbounded history is not an option there.
fn prune(ch: &Channel) -> anyhow::Result<()> {
    let base = ch.base();
    let current = current_version(ch);
    let versions = installed_versions(&base)?;
    let excess = versions.len().saturating_sub(ch.keep);
    for v in versions.iter().take(excess) {
        if Some(v.as_str()) == current.as_deref() {
            continue;
        }
        std::fs::remove_dir_all(base.join("versions").join(v))
            .with_context(|| format!("pruning {}", base.join("versions").join(v).display()))?;
        println!("{}: pruned {v}", ch.name);
    }
    Ok(())
}

/// Atomic flip: build `current.new` then rename over `current`.
fn flip_current(ch: &Channel, ver: &str) -> anyhow::Result<()> {
    let staging = ch.base().join("current.new");
    let _ = std::fs::remove_file(&staging);
    std::os::unix::fs::symlink(format!("versions/{ver}"), &staging)
        .with_context(|| format!("creating {}", staging.display()))?;
    std::fs::rename(&staging, ch.current_link()).context("flipping the current symlink")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests that set LISA_APPS_STATE.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn chan(name: &str) -> &'static Channel {
        CHANNELS.iter().find(|c| c.name == name).unwrap()
    }

    /// A version tree that counts as a tree: the channel's probe file is
    /// what makes it one, so tests build the same thing a payload does.
    fn make_tree(ch: &Channel, ver: &str) -> PathBuf {
        let dir = ch.base().join("versions").join(ver);
        let probe = dir.join(ch.probe);
        std::fs::create_dir_all(probe.parent().unwrap()).unwrap();
        std::fs::write(&probe, "// tree\n").unwrap();
        dir
    }

    #[test]
    fn flip_and_rollback_are_atomic_symlink_swaps() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path()) };
        let ch = chan("shell");
        let base = ch.base();
        make_tree(ch, "1");
        make_tree(ch, "2");

        flip_current(ch, "2").unwrap();
        assert_eq!(
            base.join("current").read_link().unwrap(),
            Path::new("versions/2")
        );
        // Rolling back with 1 installed flips to 1.
        rollback(Some("shell")).unwrap();
        assert_eq!(
            base.join("current").read_link().unwrap(),
            Path::new("versions/1")
        );
        // Rolling back again drops to the baked tree, and stays there.
        rollback(Some("shell")).unwrap();
        assert!(base.join("current").read_link().is_err());
        rollback(Some("shell")).unwrap();
        assert!(base.join("current").read_link().is_err());
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    /// Every channel owns a directory under `payloads/`, and no channel
    /// owns the root of the state dir. `shell` used to, and its
    /// `versions/` then sat next to `payloads/` where nothing looked for
    /// it (#239).
    #[test]
    fn every_channel_owns_a_directory_under_payloads() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path()) };
        for c in CHANNELS {
            assert_eq!(
                c.base(),
                dir.path().join("payloads").join(c.name),
                "{} is not under payloads/",
                c.name
            );
        }
        let shell = chan("shell");
        let zen = chan("zen");
        make_tree(shell, "20260101.1");
        make_tree(zen, "20260102.2");
        flip_current(shell, "20260101.1").unwrap();
        flip_current(zen, "20260102.2").unwrap();
        assert_eq!(
            installed_versions(&shell.base()).unwrap(),
            vec!["20260101.1".to_string()]
        );
        assert_eq!(
            installed_versions(&zen.base()).unwrap(),
            vec!["20260102.2".to_string()]
        );
        assert!(shell.is_tree(&shell.current_link()) && zen.is_tree(&zen.current_link()));
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    /// Asset naming is the contract between release.yml and the CLI: the zen
    /// payload is per-arch and an x86_64 box must never take the arm64 tree.
    #[test]
    fn asset_names_select_channel_and_arch() {
        let zen = chan("zen");
        let shell = chan("shell");
        assert_eq!(
            zen.version_of("lisa-zen_20260726.5_x86_64.tar.zst", "x86_64"),
            Some("20260726.5".into())
        );
        assert_eq!(
            zen.version_of("lisa-zen_20260726.5_aarch64.tar.zst", "x86_64"),
            None
        );
        assert_eq!(
            zen.version_of("lisa-zen_20260726.5_aarch64.tar.zst", "aarch64"),
            Some("20260726.5".into())
        );
        // Channels never claim each other's assets.
        assert_eq!(
            zen.version_of("lisa-apps_20260726.5.tar.zst", "x86_64"),
            None
        );
        assert_eq!(
            shell.version_of("lisa-zen_20260726.5_x86_64.tar.zst", "x86_64"),
            None
        );
        assert_eq!(
            shell.version_of("lisa-apps_20260726.5.tar.zst", "x86_64"),
            Some("20260726.5".into())
        );
    }

    /// The newest published version wins, numerically — `.30` beats `.9`.
    #[test]
    fn asset_pick_is_newest_by_numeric_version() {
        let assets = [
            "lisa-zen_20260726.9_x86_64.tar.zst",
            "lisa-zen_20260726.30_x86_64.tar.zst",
        ];
        let rel = Release {
            tag: "v20260726.30".into(),
            assets: assets
                .iter()
                .map(|n| (n.to_string(), format!("https://example/{n}")))
                .collect(),
            sums_url: None,
            sums: None,
        };
        let (name, ver, _) = rel.asset_for(chan("zen"), "x86_64").unwrap();
        assert_eq!(ver, "20260726.30");
        assert_eq!(name, "lisa-zen_20260726.30_x86_64.tar.zst");
        // Nothing for an arch we do not publish.
        assert!(rel.asset_for(chan("zen"), "riscv64").is_none());
    }

    /// Prune keeps `keep` trees and never removes the running one.
    #[test]
    fn prune_keeps_current_and_the_newest() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path()) };
        let ch = chan("zen"); // keep = 2
        let base = ch.base();
        for v in ["20260101.1", "20260102.2", "20260103.3", "20260104.4"] {
            make_tree(ch, v);
        }
        flip_current(ch, "20260104.4").unwrap();
        prune(ch).unwrap();
        assert_eq!(
            installed_versions(&base).unwrap(),
            vec!["20260103.3".to_string(), "20260104.4".to_string()]
        );

        // Current is the OLDEST: it survives the prune even though it is
        // over the keep line, because deleting the running tree would take
        // the browser out from under the user.
        for v in ["20260105.5", "20260106.6"] {
            make_tree(ch, v);
        }
        flip_current(ch, "20260103.3").unwrap();
        prune(ch).unwrap();
        let left = installed_versions(&base).unwrap();
        assert!(
            left.contains(&"20260103.3".to_string()),
            "current pruned: {left:?}"
        );
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    /// The end of the pipeline, on real files: a payload tarball shaped the
    /// way os/repo-tools/build-zen-payload.sh shapes it must land as
    /// `<base>/current/zen` — the exact path /usr/bin/zen-browser execs.
    /// A layout change on either side takes the browser out, so the two
    /// sides are pinned against each other here.
    #[test]
    fn zen_payload_unpacks_where_the_launcher_looks() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("browser")).unwrap();
        std::fs::write(src.join("zen"), "#!/bin/sh\n").unwrap();
        std::fs::write(src.join("browser/omni.ja"), "x").unwrap();
        let tarball = dir.path().join("payload.tar.zst");
        // Same invocation as the release tool: contents of the tree at the
        // tarball root, no wrapping directory.
        let made = std::process::Command::new("tar")
            .args(["--zstd", "-cf"])
            .arg(&tarball)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .status();
        match made {
            Ok(s) if s.success() => {}
            // A host tar without zstd cannot exercise this; the device and
            // CI both have one, and `lisa apps update` says so at runtime.
            _ => {
                eprintln!("skipping: no tar with --zstd on this host");
                return;
            }
        }

        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path().join("state")) };
        let ch = chan("zen");
        place_tree(ch, "20260726.1", &|dest| {
            std::fs::copy(&tarball, dest)?;
            Ok(())
        })
        .unwrap();

        let base = ch.base();
        assert_eq!(base, dir.path().join("state/payloads/zen"));
        // What zen-browser.sh checks, verbatim.
        assert!(base.join("current/zen").is_file(), "no current/zen");
        assert!(base.join("current/browser/omni.ja").is_file());
        assert_eq!(current_version(ch).as_deref(), Some("20260726.1"));
        // …and nothing is left behind in staging.
        let leftovers: Vec<_> = std::fs::read_dir(base.join("versions"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["20260726.1".to_string()]);
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    /// A `current` symlink pointing at a tree that is gone must read as
    /// "nothing installed", so sync/install repair it instead of trusting
    /// the link and leaving the user without a browser.
    #[test]
    fn a_dangling_current_is_not_mistaken_for_an_installed_tree() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path()) };
        let ch = chan("zen");
        let base = ch.base();
        make_tree(ch, "20260726.1");
        flip_current(ch, "20260726.1").unwrap();
        assert!(ch.is_tree(&ch.current_link()));

        std::fs::remove_dir_all(base.join("versions/20260726.1")).unwrap();
        assert_eq!(current_version(ch).as_deref(), Some("20260726.1"));
        assert!(
            !ch.is_tree(&ch.current_link()),
            "dangling current read as a live tree"
        );
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    #[test]
    fn unknown_channel_is_an_error_not_a_silent_no_op() {
        assert!(channels_for(Some("nope")).is_err());
        assert_eq!(channels_for(Some("zen")).unwrap().len(), 1);
        assert_eq!(channels_for(None).unwrap().len(), CHANNELS.len());
    }

    fn repo(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    /// Resolution order, on real directories: the channel's own tree wins,
    /// a tree an older release left behind is next, the baked copy is the
    /// floor. Every launcher goes through this, so getting the order wrong
    /// silently downgrades a device to older code.
    #[test]
    fn resolution_prefers_the_channel_tree_then_stale_then_baked() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sysroot");
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("LISA_APPS_STATE", dir.path().join("state"));
            std::env::set_var("LISA_APPS_ROOT", &root);
        }
        let ch = chan("shell");
        let write = |dir: &Path, tag: &str| {
            let p = dir.join(ch.probe);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, tag).unwrap();
        };
        let read = |r: &Resolved| std::fs::read_to_string(r.dir.join(ch.probe)).unwrap();

        // Only the image copy exists.
        write(&root.join("usr/share/lisa/shell"), "baked");
        let r = ch.resolve().unwrap();
        assert_eq!(r.source, Source::Baked);
        assert_eq!(read(&r), "baked");

        // A tree an older release unpacked at the root of the state dir —
        // the exact leftover #239 produced on every updated device.
        let stale = root.join("var/lib/lisa-apps/versions/20260804.76");
        write(&stale, "stale");
        std::os::unix::fs::symlink(
            "versions/20260804.76",
            root.join("var/lib/lisa-apps/current"),
        )
        .unwrap();
        let r = ch.resolve().unwrap();
        assert_eq!(r.source, Source::Stale);
        assert_eq!(read(&r), "stale");

        // …and the channel's own tree beats both.
        make_tree(ch, "20260805.1");
        std::fs::write(
            ch.base().join("versions/20260805.1").join(ch.probe),
            "channel",
        )
        .unwrap();
        flip_current(ch, "20260805.1").unwrap();
        let r = ch.resolve().unwrap();
        assert_eq!(r.source, Source::Channel("20260805.1".into()));
        assert_eq!(read(&r), "channel");

        unsafe {
            std::env::remove_var("LISA_APPS_STATE");
            std::env::remove_var("LISA_APPS_ROOT");
        }
    }

    /// An empty directory behind `current` is not a payload. Reporting one
    /// as installed is how a state record comes to disagree with the
    /// running system (#239 defect 3).
    #[test]
    fn an_empty_tree_is_not_a_tree() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("LISA_APPS_STATE", dir.path());
            std::env::set_var("LISA_APPS_ROOT", dir.path().join("nothing"));
        }
        let ch = chan("zen");
        std::fs::create_dir_all(ch.base().join("versions/20260731.57")).unwrap();
        flip_current(ch, "20260731.57").unwrap();
        assert_eq!(current_version(ch).as_deref(), Some("20260731.57"));
        assert!(ch.resolve().is_none(), "an empty tree resolved");
        let text = report().unwrap();
        assert!(
            text.contains("does NOT launch"),
            "status called an empty tree current:\n{text}"
        );
        unsafe {
            std::env::remove_var("LISA_APPS_STATE");
            std::env::remove_var("LISA_APPS_ROOT");
        }
    }

    /// Every directory the channel machinery creates must stay writable by
    /// group `lisa`. lisa-apps-sync.service runs as root and usually gets
    /// there first; without this the desktop user's `lisa apps update` dies
    /// with `Permission denied (os error 13)` and the only way forward is
    /// sudo — which ADR-0034 §7b forbids and our own guard denies
    /// unconditionally (#239 defect 4).
    #[test]
    fn installing_leaves_the_channel_writable_by_the_lisa_group() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path().join("state")) };
        let ch = chan("zen");
        let tarball = tiny_payload(dir.path(), ch);
        if tarball.is_none() {
            eprintln!("skipping: no tar with --zstd on this host");
            unsafe { std::env::remove_var("LISA_APPS_STATE") };
            return;
        }
        place_tree(ch, "20260731.57", &|dest| {
            std::fs::copy(tarball.as_ref().unwrap(), dest)?;
            Ok(())
        })
        .unwrap();
        for d in [ch.base(), ch.base().join("versions")] {
            let mode = std::fs::metadata(&d).unwrap().permissions().mode() & 0o7777;
            assert_eq!(
                mode,
                DIR_MODE,
                "{} is {mode:o}, not {DIR_MODE:o} — the other uid cannot install here",
                d.display()
            );
        }
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    /// tmpfiles is what REPAIRS a device that already has the wrong modes:
    /// `d` adjusts an existing directory on every boot. A channel with no
    /// rule is a channel that stays broken for the user who owns the
    /// machine, so the table and the rules are pinned to each other.
    #[test]
    fn tmpfiles_covers_every_channel_directory() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe {
            std::env::remove_var("LISA_APPS_STATE");
            std::env::remove_var("LISA_APPS_ROOT");
        }
        let conf = std::fs::read_to_string(repo(
            "os/mkosi/mkosi.extra/usr/lib/tmpfiles.d/lisa-apps.conf",
        ))
        .unwrap();
        let rules: Vec<Vec<&str>> = conf
            .lines()
            .filter(|l| l.starts_with('d'))
            .map(|l| l.split_whitespace().collect())
            .collect();
        let has = |path: &Path| {
            rules.iter().any(|r| {
                r.get(1) == Some(&path.to_str().unwrap())
                    && r.get(2) == Some(&"2775")
                    && r.get(4) == Some(&"lisa")
            })
        };
        for c in CHANNELS {
            let base = c.base();
            assert!(has(&base), "no 2775 root:lisa tmpfiles rule for {base:?}");
            let versions = base.join("versions");
            assert!(
                has(&versions),
                "no 2775 root:lisa tmpfiles rule for {versions:?} — \
                 root creates it first and the user then cannot stage into it"
            );
        }
        // Track L gets the same rules, or /var/lib/lisa-apps does not
        // exist there at all (ADR-0034: the layer is a real install path,
        // not a demo).
        let pkgbuild = std::fs::read_to_string(repo("os/packages/lisa/PKGBUILD")).unwrap();
        assert!(
            pkgbuild.contains("tmpfiles.d/lisa-apps.conf"),
            "the lisa package does not install the lisa-apps tmpfiles rules"
        );
    }

    /// The launchers that CANNOT ask (`/usr/bin/lisa` is how you get the
    /// CLI in the first place; zen-browser ships in its own package) still
    /// have to agree with the table. They are pinned here instead — the
    /// one thing #239 proves is that an unchecked second spelling drifts.
    #[test]
    fn the_scripts_that_cannot_ask_are_pinned_to_the_table() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe {
            std::env::remove_var("LISA_APPS_STATE");
            std::env::remove_var("LISA_APPS_ROOT");
        }
        let cases = [
            ("runtime", "os/packages/lisa/lisa-resolver"),
            ("zen", "os/packages/zen-browser/zen-browser.sh"),
        ];
        for (name, script) in cases {
            let ch = chan(name);
            let text = std::fs::read_to_string(repo(script)).unwrap();
            let want = ch.current_link();
            assert!(
                text.contains(want.to_str().unwrap()),
                "{script} does not resolve {} — the channel moved and it did not",
                want.display()
            );
        }
        // lisa-app asks, so it spells no payload path; the one literal it
        // does carry is the image's own floor, and that must match too.
        let launcher = std::fs::read_to_string(repo("os/packages/lisa/lisa-app")).unwrap();
        let baked = chan("shell").baked.unwrap();
        assert!(
            launcher.contains(baked),
            "lisa-app's last-resort tree is not {baked}"
        );
    }

    /// Pruning is housekeeping. An old tree another uid unpacked cannot be
    /// removed by this one — the state every device that ran
    /// lisa-apps-sync.service as root is in — and failing the update over
    /// it would leave the machine unable to take a payload it has already
    /// downloaded, verified and activated (#239 defect 4).
    #[test]
    fn an_unremovable_old_tree_does_not_fail_the_install() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path().join("state")) };
        let ch = chan("zen"); // keep = 2
        let done = (|| {
            let Some(tarball) = tiny_payload(dir.path(), ch) else {
                eprintln!("skipping: no tar with --zstd on this host");
                return false;
            };
            for v in ["20260701.1", "20260702.2"] {
                make_tree(ch, v);
            }
            // Someone else's tree: read-only, so its contents cannot be
            // unlinked. Root would sail through this, and root is not who
            // the test is about.
            let locked = ch.base().join("versions/20260701.1");
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
            if std::fs::write(locked.join("probe"), "x").is_ok() {
                eprintln!("skipping: running as root, where nothing is unremovable");
                let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
                return false;
            }
            place_tree(ch, "20260703.3", &|dest| {
                std::fs::copy(&tarball, dest)?;
                Ok(())
            })
            .expect("an unprunable old tree failed the install");
            // The install happened and is current…
            assert_eq!(current_version(ch).as_deref(), Some("20260703.3"));
            assert!(ch.resolve().is_some());
            // …and the tree that could not be removed is still there,
            // reported rather than silently lost.
            assert!(locked.is_dir());
            let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
            true
        })();
        let _ = done;
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    /// A tarball with just the probe file in it, or None on a host whose
    /// tar has no zstd support.
    fn tiny_payload(dir: &Path, ch: &Channel) -> Option<PathBuf> {
        let src = dir.join("payload-src");
        let probe = src.join(ch.probe);
        std::fs::create_dir_all(probe.parent().unwrap()).unwrap();
        std::fs::write(&probe, "#!/bin/sh\n").unwrap();
        let tarball = dir.join("payload.tar.zst");
        let made = std::process::Command::new("tar")
            .args(["--zstd", "-cf"])
            .arg(&tarball)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .status();
        match made {
            Ok(s) if s.success() => Some(tarball),
            _ => None,
        }
    }
}
