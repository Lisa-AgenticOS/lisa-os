//! `lisa apps` — the app-update channel (ADR-0020): update the interpreted
//! shell-app tree from the release channel without touching a boot slot.
//!
//! Layout: `/var/lib/lisa/apps/versions/<ver>/` per version;
//! `current` is a symlink flipped atomically (symlink + rename). Apps launch
//! through `/usr/bin/lisa-app`, which prefers `current` over the baked
//! `/usr/share/lisa/shell`, so an update takes effect on the next app
//! launch — no reboot. Integrity: sha256 against the release's SHA256SUMS
//! manifest (same trust level as the sysupdate transfer set; GPG signing
//! lands with the M1 signed repo).

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const APPS_DIR: &str = "/var/lib/lisa/apps";

fn apps_dir() -> PathBuf {
    std::env::var_os("LISA_APPS_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(APPS_DIR))
}

/// `lisa apps update`: fetch the newest apps tarball, verify, install, flip.
pub fn update() -> anyhow::Result<()> {
    let mut resp = ureq::get(crate::RELEASES_API)
        .call()
        .context("fetching the latest release")?;
    let release: serde_json::Value = resp.body_mut().read_json().context("parsing the release")?;
    let tag = release["tag_name"].as_str().unwrap_or("?");

    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| -> Option<(String, String)> {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            if pred(name) {
                Some((
                    name.to_string(),
                    a["browser_download_url"].as_str()?.to_string(),
                ))
            } else {
                None
            }
        })
    };
    let Some((tar_name, tar_url)) =
        find(&|n| n.starts_with("lisa-apps_") && n.ends_with(".tar.zst"))
    else {
        bail!("release {tag} carries no lisa-apps tarball — apps still ride the image there");
    };
    let Some((_, sums_url)) = find(&|n| n == "SHA256SUMS") else {
        bail!("release {tag} has no SHA256SUMS manifest");
    };
    let ver = tar_name
        .trim_start_matches("lisa-apps_")
        .trim_end_matches(".tar.zst")
        .to_string();

    let base = apps_dir();
    let versions = base.join("versions");
    let dest = versions.join(&ver);
    if base.join("current").read_link().ok().as_deref()
        == Some(Path::new(&format!("versions/{ver}")))
    {
        println!("apps tree {ver} is already current");
        return Ok(());
    }
    std::fs::create_dir_all(&versions).with_context(|| {
        format!(
            "creating {} (needs the lisa group write access)",
            versions.display()
        )
    })?;

    // Expected hash from the manifest.
    let sums = ureq::get(&sums_url)
        .call()
        .context("fetching SHA256SUMS")?
        .body_mut()
        .read_to_string()?;
    let expected = sums
        .lines()
        .find_map(|l| l.strip_suffix(&tar_name).map(|h| h.trim().to_string()))
        .with_context(|| format!("{tar_name} is not in SHA256SUMS"))?;

    // Download + hash.
    println!("downloading {tar_name} ({tag})…");
    let mut body = Vec::new();
    ureq::get(&tar_url)
        .call()
        .context("downloading the apps tarball")?
        .body_mut()
        .as_reader()
        .read_to_end(&mut body)?;
    let got = hex::encode(Sha256::digest(&body));
    if got != expected {
        bail!("checksum mismatch for {tar_name}: expected {expected}, got {got}");
    }

    // Unpack into a fresh version dir (tar handles zstd via --zstd).
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::create_dir_all(&dest)?;
    let tmp_tar = dest.with_extension("tar.zst.partial");
    std::fs::write(&tmp_tar, &body)?;
    let status = std::process::Command::new("tar")
        .arg("--zstd")
        .arg("-xf")
        .arg(&tmp_tar)
        .arg("-C")
        .arg(&dest)
        .status()
        .context("running tar (needs zstd support)")?;
    let _ = std::fs::remove_file(&tmp_tar);
    if !status.success() {
        let _ = std::fs::remove_dir_all(&dest);
        bail!("unpacking {tar_name} failed ({status})");
    }

    flip_current(&base, &ver)?;
    println!("apps tree {ver} is current — running apps pick it up on their next launch");
    Ok(())
}

/// `lisa apps status`: what is current, what is installed, what is baked.
pub fn status() -> anyhow::Result<()> {
    let base = apps_dir();
    match base.join("current").read_link() {
        Ok(target) => println!(
            "current: {}",
            target
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| target.display().to_string())
        ),
        Err(_) => println!("current: (baked image tree — no apps update installed)"),
    }
    for v in installed_versions(&base)? {
        println!("installed: {v}");
    }
    Ok(())
}

/// `lisa apps rollback`: flip to the newest installed version strictly
/// OLDER than the current one; with none older, drop back to the baked
/// tree. Already on the baked tree → nothing to roll back to.
pub fn rollback() -> anyhow::Result<()> {
    let base = apps_dir();
    let Some(current) = base
        .join("current")
        .read_link()
        .ok()
        .and_then(|t| t.file_name().map(|s| s.to_string_lossy().into_owned()))
    else {
        println!("already on the baked image tree — nothing to roll back");
        return Ok(());
    };
    let mut versions = installed_versions(&base)?;
    versions.retain(|v| ver_key(v) < ver_key(&current));
    match versions.last() {
        Some(prev) => {
            flip_current(&base, prev)?;
            println!("rolled back to apps tree {prev}");
        }
        None => {
            let _ = std::fs::remove_file(base.join("current"));
            println!("no older installed version — the baked image tree is active again");
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

fn installed_versions(base: &Path) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base.join("versions")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out.sort_by_key(|v| ver_key(v));
    Ok(out)
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

    #[test]
    fn flip_and_rollback_are_atomic_symlink_swaps() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-scoped env; no parallel test reads it.
        unsafe { std::env::set_var("LISA_APPS_STATE", dir.path()) };
        let base = apps_dir();
        std::fs::create_dir_all(base.join("versions/1")).unwrap();
        std::fs::create_dir_all(base.join("versions/2")).unwrap();

        flip_current(&base, "2").unwrap();
        assert_eq!(
            base.join("current").read_link().unwrap(),
            Path::new("versions/2")
        );
        // Rolling back with 1 installed flips to 1.
        rollback().unwrap();
        assert_eq!(
            base.join("current").read_link().unwrap(),
            Path::new("versions/1")
        );
        // Rolling back again drops to the baked tree, and stays there.
        rollback().unwrap();
        assert!(base.join("current").read_link().is_err());
        rollback().unwrap();
        assert!(base.join("current").read_link().is_err());
        unsafe { std::env::remove_var("LISA_APPS_STATE") };
    }
}
