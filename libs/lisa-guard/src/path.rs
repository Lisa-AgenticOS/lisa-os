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
