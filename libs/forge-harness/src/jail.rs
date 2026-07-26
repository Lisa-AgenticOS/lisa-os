//! The tool jail (`docs/PLAN.md` §5.12.1): every file operation the
//! harness performs on behalf of a model is confined to the project
//! directory — the agent reaches the directory it was given and nothing
//! above it.
//!
//! Containment itself lives in `lisa-guard` (ADR-0029), which is also
//! what the shell-facing surfaces call, so there is one implementation of
//! "inside the project" rather than one per caller. This module is the
//! I/O built on top of it.
//!
//! Scope, stated plainly: this jail confines the harness's own file
//! tools. It does **not** confine a subprocess — `run_tests` invokes
//! `cargo test`/`flutter test` over source the model just wrote, and that
//! code runs unrestricted. Closing that needs Landlock, which is ADR-0029
//! phase 3.

use lisa_guard::{ContainError, contain, write_contained};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JailError {
    #[error("path escapes the project jail: {0}")]
    Escape(String),
    #[error("jail io: {0}")]
    Io(#[from] std::io::Error),
}

impl From<ContainError> for JailError {
    fn from(e: ContainError) -> Self {
        match e {
            ContainError::Escape(p) => JailError::Escape(p),
            ContainError::Io(e) => JailError::Io(e),
        }
    }
}

pub struct Jail {
    root: PathBuf,
}

impl Jail {
    pub fn new(root: &Path) -> Result<Self, JailError> {
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    /// Validate a project-relative path: no absolute paths, no `..`, and
    /// no symlink that leaves the root at any depth.
    fn resolve(&self, rel: &str) -> Result<PathBuf, JailError> {
        Ok(contain(&self.root, rel)?)
    }

    /// Write a project file. Goes through `lisa-guard`'s `O_NOFOLLOW`
    /// write rather than `fs::write`, because check-then-write is a race
    /// a symlink swap wins nearly every time (ADR-0029, issue #66).
    pub fn write(&self, rel: &str, content: &str) -> Result<(), JailError> {
        Ok(write_contained(&self.root, rel, content)?)
    }

    pub fn read(&self, rel: &str) -> Result<String, JailError> {
        Ok(std::fs::read_to_string(self.resolve(rel)?)?)
    }

    /// The canonicalized project root. Read-only escapes (the agent
    /// already knows where the project lives) and for `current_dir` of
    /// jailed commands — never handed to model-supplied paths, which all
    /// go through `resolve`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// List one directory, sorted, directories with a trailing `/`.
    /// `""` and `"."` mean the project root.
    pub fn list(&self, rel: &str) -> Result<Vec<String>, JailError> {
        let dir = self.resolve(rel)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let mut name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type()?.is_dir() {
                name.push('/');
            }
            entries.push(name);
        }
        entries.sort();
        Ok(entries)
    }

    /// All files under `rel` (the whole project for `""`/`"."`), as
    /// clean project-relative paths. Hidden entries and build output
    /// directories (`.git`, `.dart_tool`, `target`, `build`,
    /// `node_modules`) are skipped; the result is capped so a huge tree
    /// cannot flood the agent context.
    pub fn walk(&self, rel: &str) -> Result<Vec<String>, JailError> {
        const MAX_FILES: usize = 5000;
        const SKIP_DIRS: &[&str] = &[".dart_tool", "target", "build", "node_modules"];
        let start = self.resolve(rel)?;
        let mut out = Vec::new();
        let mut stack = vec![start];
        while let Some(dir) = stack.pop() {
            if out.len() >= MAX_FILES {
                break;
            }
            if !dir.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    if !SKIP_DIRS.contains(&name.as_ref()) {
                        stack.push(path);
                    }
                } else if let Ok(rel) = path.strip_prefix(&self.root) {
                    let clean: PathBuf = rel
                        .components()
                        .filter_map(|c| match c {
                            Component::Normal(p) => Some(p),
                            _ => None,
                        })
                        .collect();
                    out.push(clean.to_string_lossy().into_owned());
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let jail = Jail::new(dir.path()).unwrap();
        assert!(matches!(
            jail.write("../outside.txt", "x"),
            Err(JailError::Escape(_))
        ));
        assert!(matches!(
            jail.write("/etc/passwd", "x"),
            Err(JailError::Escape(_))
        ));
        assert!(matches!(
            jail.write("ok/../../outside.txt", "x"),
            Err(JailError::Escape(_))
        ));
    }

    // ADR-0029: the component check passed and `fs::write` followed the
    // link, so a legal-looking relative path wrote outside the project.
    #[cfg(unix)]
    #[test]
    fn writes_do_not_follow_a_symlink_out_of_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let jail = Jail::new(dir.path()).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();

        assert!(matches!(
            jail.write("escape/owned.txt", "pwned"),
            Err(JailError::Escape(_))
        ));
        assert!(!outside.path().join("owned.txt").exists());
        assert!(matches!(
            jail.read("escape/owned.txt"),
            Err(JailError::Escape(_))
        ));
    }

    #[test]
    fn nested_writes_and_reads_stay_inside() {
        let dir = tempfile::tempdir().unwrap();
        let jail = Jail::new(dir.path()).unwrap();
        jail.write("lib/src/main.dart", "void main() {}").unwrap();
        assert_eq!(jail.read("lib/src/main.dart").unwrap(), "void main() {}");
        assert!(dir.path().join("lib/src/main.dart").exists());
    }

    #[test]
    fn list_marks_directories_and_stays_inside() {
        let dir = tempfile::tempdir().unwrap();
        let jail = Jail::new(dir.path()).unwrap();
        jail.write("lib/a.dart", "a").unwrap();
        jail.write("pubspec.yaml", "p").unwrap();
        assert_eq!(jail.list(".").unwrap(), ["lib/", "pubspec.yaml"]);
        assert_eq!(jail.list("").unwrap(), ["lib/", "pubspec.yaml"]);
        assert!(matches!(jail.list(".."), Err(JailError::Escape(_))));
    }

    #[test]
    fn walk_skips_hidden_and_build_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let jail = Jail::new(dir.path()).unwrap();
        jail.write("lib/a.dart", "a").unwrap();
        jail.write("lib/src/b.dart", "b").unwrap();
        jail.write(".git/config", "secret").unwrap();
        jail.write(".dart_tool/x", "x").unwrap();
        jail.write("target/y", "y").unwrap();
        assert_eq!(jail.walk(".").unwrap(), ["lib/a.dart", "lib/src/b.dart"]);
        assert_eq!(jail.walk("lib").unwrap(), ["lib/a.dart", "lib/src/b.dart"]);
    }
}
