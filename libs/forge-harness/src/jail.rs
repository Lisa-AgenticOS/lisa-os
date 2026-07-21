//! The tool jail (`docs/PLAN.md` §5.12.1): every file operation the
//! harness performs on behalf of a model is confined to the project
//! directory — path traversal and absolute paths are rejected before
//! any I/O. The same jail confines BYO agent backends.

use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JailError {
    #[error("path escapes the project jail: {0}")]
    Escape(String),
    #[error("jail io: {0}")]
    Io(#[from] std::io::Error),
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

    /// Validate a project-relative path: no absolute paths, no `..`.
    fn resolve(&self, rel: &str) -> Result<PathBuf, JailError> {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err(JailError::Escape(rel.into()));
        }
        for component in rel_path.components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                _ => return Err(JailError::Escape(rel.into())),
            }
        }
        Ok(self.root.join(rel_path))
    }

    pub fn write(&self, rel: &str, content: &str) -> Result<(), JailError> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn read(&self, rel: &str) -> Result<String, JailError> {
        Ok(std::fs::read_to_string(self.resolve(rel)?)?)
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

    #[test]
    fn nested_writes_and_reads_stay_inside() {
        let dir = tempfile::tempdir().unwrap();
        let jail = Jail::new(dir.path()).unwrap();
        jail.write("lib/src/main.dart", "void main() {}").unwrap();
        assert_eq!(jail.read("lib/src/main.dart").unwrap(), "void main() {}");
        assert!(dir.path().join("lib/src/main.dart").exists());
    }
}
