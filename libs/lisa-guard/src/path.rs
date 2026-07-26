//! Filesystem containment: the agent reaches the directory it was given
//! and nothing above it (ADR-0029 §1).
//!
//! The component check alone — reject absolute paths, reject `..` — is
//! what the forge jail used to do, and it is not enough, because
//! `std::fs` follows symlinks. A link inside the project pointing at
//! `$HOME/.ssh` turned a legal-looking relative path into a write outside
//! the root. So containment is re-asserted *per component*, against the
//! canonical path, as the walk descends.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ContainError {
    #[error("path escapes the agent's directory: {0}")]
    Escape(String),
    #[error("containment io: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve `rel` inside `root`, or refuse.
///
/// `root` must already be canonical — [`std::fs::canonicalize`] it once
/// when the jail is built, not on every call.
///
/// Refuses, in order: absolute paths; any component that is not a plain
/// name or `.` (so `..`, drive prefixes and root components are out); any
/// existing component whose canonical form leaves `root` (the symlink
/// case); and any component that is a *dangling* symlink, which
/// `canonicalize` reports as merely absent while `fs::write` would
/// happily follow it and create the file at the far end.
pub fn contain(root: &Path, rel: &str) -> Result<PathBuf, ContainError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(ContainError::Escape(rel.into()));
    }

    let mut cur = root.to_path_buf();
    for component in rel_path.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => {
                cur.push(name);
                match cur.canonicalize() {
                    Ok(real) => {
                        if !real.starts_with(root) {
                            return Err(ContainError::Escape(rel.into()));
                        }
                        // Descend from the resolved path, so the next
                        // component is judged against where we actually
                        // are rather than where the name suggested.
                        cur = real;
                    }
                    Err(e) => {
                        // A dangling symlink canonicalizes to NotFound but
                        // is still followed on write — treat it as the
                        // escape it is rather than as an absent file.
                        if cur
                            .symlink_metadata()
                            .is_ok_and(|m| m.file_type().is_symlink())
                        {
                            return Err(ContainError::Escape(rel.into()));
                        }
                        if e.kind() != std::io::ErrorKind::NotFound {
                            return Err(ContainError::Io(e));
                        }
                        // Genuinely absent. Its parent was verified
                        // contained on the previous pass, so a new entry
                        // beneath it is contained too.
                    }
                }
            }
            _ => return Err(ContainError::Escape(rel.into())),
        }
    }
    Ok(cur)
}

/// Write `content` to `rel` inside `root`, refusing to follow a symlink
/// at the final component **at open time**.
///
/// [`contain`] alone is a check, and a check is not a guarantee: between
/// it returning and `fs::write` opening the file, anything may replace
/// the target with a symlink pointing outside. Review round 1 (#66)
/// measured that race against the previous check-then-write jail —
/// **18,599 of 20,001 writes landed outside the root**. `O_NOFOLLOW`
/// closes it by making the kernel refuse the open rather than trusting
/// what we saw a moment ago.
///
/// Residual, and honestly stated: `O_NOFOLLOW` guards only the final
/// component. Swapping a *parent* directory for a symlink between the
/// check and the open is still possible; closing that needs
/// `openat2(RESOLVE_BENEATH)` on Linux or Landlock (ADR-0029 phase 3).
pub fn write_contained(root: &Path, rel: &str, content: &str) -> Result<(), ContainError> {
    use std::io::Write;

    let path = contain(root, rel)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }

    let mut file = options.open(&path).map_err(|e| {
        // ELOOP here means the target became a symlink after we checked.
        if e.raw_os_error() == Some(libc::ELOOP) {
            ContainError::Escape(rel.into())
        } else {
            ContainError::Io(e)
        }
    })?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    }

    #[test]
    fn plain_relative_paths_resolve() {
        let (_d, root) = root();
        assert_eq!(
            contain(&root, "lib/main.dart").unwrap(),
            root.join("lib/main.dart")
        );
        assert_eq!(contain(&root, "./lib").unwrap(), root.join("lib"));
        assert_eq!(contain(&root, "").unwrap(), root);
    }

    #[test]
    fn absolute_and_traversal_are_refused() {
        let (_d, root) = root();
        for bad in ["/etc/passwd", "../outside", "ok/../../outside", ".."] {
            assert!(
                matches!(contain(&root, bad), Err(ContainError::Escape(_))),
                "{bad} should have been refused"
            );
        }
    }

    // The escape ADR-0029 was written for: the component check passes,
    // the write lands outside.
    #[cfg(unix)]
    #[test]
    fn symlink_out_of_the_root_is_refused() {
        let (_d, root) = root();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();

        assert!(matches!(
            contain(&root, "escape/owned.txt"),
            Err(ContainError::Escape(_))
        ));
        assert!(matches!(
            contain(&root, "escape"),
            Err(ContainError::Escape(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_out_of_the_root_is_refused() {
        let (_d, root) = root();
        // Never created, so `canonicalize` says NotFound — but a write
        // through it would still create /tmp/lisa-guard-dangling-target.
        std::os::unix::fs::symlink("/tmp/lisa-guard-dangling-target", root.join("dangling"))
            .unwrap();

        assert!(matches!(
            contain(&root, "dangling"),
            Err(ContainError::Escape(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_that_stays_inside_is_fine() {
        let (_d, root) = root();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();

        assert_eq!(contain(&root, "link").unwrap(), root.join("real"));
        assert_eq!(
            contain(&root, "link/file.txt").unwrap(),
            root.join("real/file.txt")
        );
    }

    /// Review round 1 (#66). The previous jail checked, then wrote; a
    /// thread flipping the target to an outside symlink in between landed
    /// 18,599 of 20,001 writes outside the root. `O_NOFOLLOW` makes the
    /// kernel refuse rather than trusting the earlier check.
    #[cfg(unix)]
    #[test]
    fn a_symlink_swapped_in_after_the_check_cannot_win_the_race() {
        const ROUNDS: usize = 4000;
        let (_d, root) = root();
        let outside = tempfile::tempdir().unwrap();
        let loot = outside.path().join("loot.txt");
        let target = root.join("out.txt");
        std::fs::create_dir_all(root.join("lib")).unwrap();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flipper = {
            let (stop, target, loot) = (stop.clone(), target.clone(), loot.clone());
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = std::fs::remove_file(&target);
                    let _ = std::os::unix::fs::symlink(&loot, &target);
                    let _ = std::fs::remove_file(&target);
                    let _ = std::fs::write(&target, "plain");
                }
            })
        };

        for _ in 0..ROUNDS {
            // Either outcome is fine — succeed inside, or refuse. What
            // must never happen is the write landing at `loot`.
            let _ = write_contained(&root, "out.txt", "agent output");
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        flipper.join().unwrap();

        let escaped = std::fs::read_to_string(&loot).unwrap_or_default();
        assert!(
            !escaped.contains("agent output"),
            "a write escaped the root through a swapped symlink"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_contained_refuses_a_symlink_target_and_writes_real_files() {
        let (_d, root) = root();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path().join("loot.txt"), root.join("link.txt")).unwrap();

        assert!(matches!(
            write_contained(&root, "link.txt", "pwned"),
            Err(ContainError::Escape(_))
        ));
        assert!(!outside.path().join("loot.txt").exists());

        write_contained(&root, "lib/src/main.dart", "void main() {}").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("lib/src/main.dart")).unwrap(),
            "void main() {}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_parent_cannot_smuggle_a_deeper_write() {
        let (_d, root) = root();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(outside.path().join("nested")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("hop")).unwrap();

        assert!(matches!(
            contain(&root, "hop/nested/deep/file.txt"),
            Err(ContainError::Escape(_))
        ));
    }
}
