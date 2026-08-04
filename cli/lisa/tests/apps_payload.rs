//! What the apps channel CARRIES (issue #239 defect 2).
//!
//! `release.yml` packed `shell/` and nothing else, so Mail, Surfer and
//! Preview — the three surfaces users spend the most time in — were not in
//! the update channel at all. They shipped only in the OS image, which
//! made ADR-0020's promise ("app fixes reach devices without an image
//! update") false for exactly the apps it mattered most for.
//!
//! The fix is one staging script (`os/repo-tools/build-apps-payload.sh`)
//! used by BOTH the package and the release tarball, and the test that
//! matters is not "does it contain apps/" — it is: *every launcher entry
//! point the system installs resolves inside the tree*. An app in the
//! payload that no `.desktop` looks for, or a `.desktop` pointing at
//! something the payload lacks, is defect 1 again in a different costume.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn stage() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("tree");
    let out = Command::new("bash")
        .arg(repo().join("os/repo-tools/build-apps-payload.sh"))
        .arg(&dest)
        .output()
        .expect("running build-apps-payload.sh");
    assert!(
        out.status.success(),
        "staging the payload failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (dir, dest)
}

/// Every `.desktop` / D-Bus service file the packaging installs, with the
/// relative path it hands to `lisa-app`.
fn launcher_entries() -> Vec<(String, String)> {
    let pkgbuild = std::fs::read_to_string(repo().join("os/packages/lisa/PKGBUILD")).unwrap();
    let mut out = Vec::new();
    for top in ["apps", "shell"] {
        for entry in walk(&repo().join(top)) {
            let ext = entry.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "desktop" && ext != "service" {
                continue;
            }
            let rel_file = entry
                .strip_prefix(repo())
                .unwrap()
                .to_string_lossy()
                .into_owned();
            // Only what the system actually installs. shell/settings is
            // deliberately not shipped (ADR-0012 v2 merged it into the
            // GNOME Intelligence panel), and its .desktop stays in the
            // tree as the panel's reference.
            if !pkgbuild.contains(&rel_file) {
                continue;
            }
            let text = std::fs::read_to_string(&entry).unwrap();
            for line in text.lines() {
                let Some(rest) = line.strip_prefix("Exec=") else {
                    continue;
                };
                let mut words = rest.split_whitespace();
                let prog = words.next().unwrap_or("");
                if !prog.ends_with("lisa-app") {
                    continue;
                }
                if let Some(rel) = words.next() {
                    out.push((rel_file.clone(), rel.to_string()));
                }
            }
        }
    }
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// The payload must satisfy every launcher the image installs — including
/// the ones from `apps/`, which it did not carry at all.
#[test]
fn the_payload_carries_every_surface_a_launcher_can_ask_for() {
    let (_d, tree) = stage();
    let entries = launcher_entries();
    assert!(
        entries.len() >= 6,
        "found only {} lisa-app entry points — the scan is broken, not the payload",
        entries.len()
    );
    for (from, rel) in &entries {
        assert!(
            tree.join(rel).is_file(),
            "{from} launches {rel}, which the apps payload does not contain"
        );
    }
    // Named explicitly so a future refactor that stops finding the apps/
    // surfaces fails here rather than passing on an empty set. These are
    // the three #239 was filed about.
    for app in ["mail/", "surfer/", "preview/"] {
        assert!(
            entries.iter().any(|(_, rel)| rel.starts_with(app)),
            "no launcher entry under {app} was checked"
        );
    }
}

/// Tests and scratch trees are not payload. They were excluded from the
/// tarball before; the staging script is now the only place that says so.
#[test]
fn the_payload_carries_no_test_trees() {
    let (_d, tree) = stage();
    for p in walk(&tree) {
        let s = p.to_string_lossy();
        assert!(
            !s.contains("/tests/") && !s.contains("/testing/") && !s.contains("/spike/"),
            "test material in the payload: {s}"
        );
    }
}

/// One list, two consumers. If the package composes the baked tree by
/// hand again, the channel and the image drift — which is how the payload
/// came to hold `settings` (not installed) and not `mail` (installed).
#[test]
fn the_package_and_the_release_tarball_stage_the_same_tree() {
    let script = "os/repo-tools/build-apps-payload.sh";
    let pkgbuild = std::fs::read_to_string(repo().join("os/packages/lisa/PKGBUILD")).unwrap();
    assert!(
        pkgbuild.contains(script),
        "PKGBUILD composes /usr/share/lisa/shell without {script}"
    );
    let release = std::fs::read_to_string(repo().join(".github/workflows/release.yml")).unwrap();
    assert!(
        release.contains(script),
        "release.yml builds lisa-apps_<ver>.tar.zst without {script}"
    );
    // …and nothing else may pack that asset behind the script's back.
    for line in release.lines() {
        let invokes_tar = line.split_whitespace().any(|w| w == "tar");
        assert!(
            !(invokes_tar && line.contains("lisa-apps_")),
            "release.yml packs the apps asset directly: {line}"
        );
    }
}
