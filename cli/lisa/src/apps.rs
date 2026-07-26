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
//! `<base>` is `/var/lib/lisa/apps` for `shell` — unchanged from ADR-0020, so
//! trees installed by older releases keep working — and
//! `/var/lib/lisa/apps/payloads/<name>` for everything since. Integrity:
//! sha256 against the release's SHA256SUMS manifest (same trust level as the
//! sysupdate transfer set; GPG signing lands with the M1 signed repo).

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

const APPS_DIR: &str = "/var/lib/lisa/apps";

fn apps_dir() -> PathBuf {
    std::env::var_os("LISA_APPS_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(APPS_DIR))
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
    /// State dir under `/var/lib/lisa/apps` ("" = the dir itself).
    subdir: &'static str,
    /// Installed version trees to keep (including the current one). Payload
    /// trees are hundreds of MiB; /var is finite.
    keep: usize,
    /// Fetch automatically when no tree is installed yet. True only for
    /// payloads with NO baked fallback in the image — otherwise a fresh
    /// install would pull a copy of something it already has, and skew the
    /// tree ahead of the image for no reason (ADR-0020 consequences).
    auto_sync: bool,
    /// A pre-migration path the image may still carry, reported by `status`
    /// so an operator can see WHY the browser works on a machine with no
    /// payload installed. Not a resolution path — that lives in the
    /// launcher (os/packages/zen-browser/zen-browser.sh).
    legacy_baked: Option<&'static str>,
}

const CHANNELS: &[Channel] = &[
    Channel {
        name: "shell",
        what: "GJS shell surfaces — assistant, launcher, Ledger, settings",
        prefix: "lisa-apps_",
        per_arch: false,
        subdir: "",
        keep: 3,
        auto_sync: false,
        // The image always bakes /usr/share/lisa/shell; lisa-app falls back
        // to it, so this is a permanent floor rather than a migration
        // leftover — nothing to warn about.
        legacy_baked: None,
    },
    Channel {
        name: "zen",
        what: "Zen browser — the tree that used to be baked as /opt/zen",
        prefix: "lisa-zen_",
        per_arch: true,
        subdir: "payloads/zen",
        keep: 2,
        auto_sync: true,
        legacy_baked: Some("/opt/zen/zen"),
    },
];

impl Channel {
    fn base(&self) -> PathBuf {
        let d = apps_dir();
        if self.subdir.is_empty() {
            d
        } else {
            d.join(self.subdir)
        }
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
    let base = ch.base();
    // `has_tree` as well as the version, deliberately: if `current` points
    // at a version whose tree is gone (hand-deleted, a truncated /var, a
    // failed rename), the symlink alone would say "already current" and
    // this would refuse to repair the very thing sync() was asked to
    // repair — a browser that stays broken forever.
    if current_version(&base).as_deref() == Some(ver.as_str()) && has_tree(&base) {
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
        .with_context(|| format!("creating {} (run as root)", versions.display()))?;
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

    flip_current(&base, ver)?;
    prune(ch, &base)
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
        .filter(|c| c.auto_sync && !has_tree(&c.base()))
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

/// `lisa apps status`: what is current, what is installed, what is baked.
pub fn status() -> anyhow::Result<()> {
    println!("payload arch: {}", payload_arch());
    for ch in CHANNELS {
        let base = ch.base();
        println!("\n{} — {}", ch.name, ch.what);
        match current_version(&base) {
            Some(v) if has_tree(&base) => println!("  current: {v}"),
            Some(v) => println!("  current: {v} (BROKEN — the tree it points at is gone)"),
            None => println!("  current: (none installed on /var)"),
        }
        for v in installed_versions(&base)? {
            println!("  installed: {v}");
        }
        if let Some(baked) = ch.legacy_baked.filter(|p| Path::new(p).exists()) {
            println!("  fallback: {baked} (baked by the image — pre-migration release)");
        }
    }
    Ok(())
}

/// `lisa apps rollback [channel]`: flip to the newest installed version
/// strictly OLDER than the current one; with none older, drop back to the
/// baked tree. Already on the baked tree → nothing to roll back to.
pub fn rollback(only: Option<&str>) -> anyhow::Result<()> {
    for ch in channels_for(only)? {
        let base = ch.base();
        let Some(current) = current_version(&base) else {
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
                flip_current(&base, prev)?;
                println!("{}: rolled back to {prev}", ch.name);
            }
            None => {
                let _ = std::fs::remove_file(base.join("current"));
                println!(
                    "{}: no older installed version — {}",
                    ch.name,
                    if ch.auto_sync {
                        "nothing on /var now; run `lisa apps update` to reinstall"
                    } else {
                        "the baked image tree is active again"
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
/// still exist — see `has_tree` for that.
fn current_version(base: &Path) -> Option<String> {
    base.join("current")
        .read_link()
        .ok()?
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
}

/// Whether `current` resolves to a real tree (metadata follows the symlink,
/// so a dangling link is false).
fn has_tree(base: &Path) -> bool {
    std::fs::metadata(base.join("current")).is_ok()
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
fn prune(ch: &Channel, base: &Path) -> anyhow::Result<()> {
    let current = current_version(base);
    let versions = installed_versions(base)?;
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
fn flip_current(base: &Path, ver: &str) -> anyhow::Result<()> {
    let staging = base.join("current.new");
    let _ = std::fs::remove_file(&staging);
    std::os::unix::fs::symlink(format!("versions/{ver}"), &staging)
        .with_context(|| format!("creating {}", staging.display()))?;
    std::fs::rename(&staging, base.join("current")).context("flipping the current symlink")?;
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

    #[test]
    fn flip_and_rollback_are_atomic_symlink_swaps() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path()) };
        let base = chan("shell").base();
        std::fs::create_dir_all(base.join("versions/1")).unwrap();
        std::fs::create_dir_all(base.join("versions/2")).unwrap();

        flip_current(&base, "2").unwrap();
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

    /// The zen channel lives beside the ADR-0020 shell tree, not inside it:
    /// an existing `/var/lib/lisa/apps/versions/<ver>` from an older release
    /// must keep resolving after this change.
    #[test]
    fn channels_do_not_share_state_dirs() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env, serialized by ENV_LOCK.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path()) };
        let shell = chan("shell").base();
        let zen = chan("zen").base();
        assert_eq!(shell, dir.path());
        assert_eq!(zen, dir.path().join("payloads/zen"));

        std::fs::create_dir_all(shell.join("versions/20260101.1")).unwrap();
        std::fs::create_dir_all(zen.join("versions/20260102.2")).unwrap();
        flip_current(&shell, "20260101.1").unwrap();
        flip_current(&zen, "20260102.2").unwrap();
        // The zen state dir is a sibling under payloads/, so it is not
        // mistaken for a shell version tree.
        assert_eq!(
            installed_versions(&shell).unwrap(),
            vec!["20260101.1".to_string()]
        );
        assert_eq!(
            installed_versions(&zen).unwrap(),
            vec!["20260102.2".to_string()]
        );
        assert!(has_tree(&shell) && has_tree(&zen));
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
            std::fs::create_dir_all(base.join("versions").join(v)).unwrap();
        }
        flip_current(&base, "20260104.4").unwrap();
        prune(ch, &base).unwrap();
        assert_eq!(
            installed_versions(&base).unwrap(),
            vec!["20260103.3".to_string(), "20260104.4".to_string()]
        );

        // Current is the OLDEST: it survives the prune even though it is
        // over the keep line, because deleting the running tree would take
        // the browser out from under the user.
        for v in ["20260105.5", "20260106.6"] {
            std::fs::create_dir_all(base.join("versions").join(v)).unwrap();
        }
        flip_current(&base, "20260103.3").unwrap();
        prune(ch, &base).unwrap();
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
        assert_eq!(current_version(&base).as_deref(), Some("20260726.1"));
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
        let base = chan("zen").base();
        std::fs::create_dir_all(base.join("versions/20260726.1")).unwrap();
        flip_current(&base, "20260726.1").unwrap();
        assert!(has_tree(&base));

        std::fs::remove_dir_all(base.join("versions/20260726.1")).unwrap();
        assert_eq!(current_version(&base).as_deref(), Some("20260726.1"));
        assert!(!has_tree(&base), "dangling current read as a live tree");
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }

    #[test]
    fn unknown_channel_is_an_error_not_a_silent_no_op() {
        assert!(channels_for(Some("nope")).is_err());
        assert_eq!(channels_for(Some("zen")).unwrap().len(), 1);
        assert_eq!(channels_for(None).unwrap().len(), CHANNELS.len());
    }
}
