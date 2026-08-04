//! Paths the OWNER put out of bounds (#253).
//!
//! The mirror of [`crate::overrides`], and the asymmetry between them is
//! the whole design:
//!
//! | | direction | who may reach it |
//! |---|---|---|
//! | [`Overrides`](crate::overrides) | **loosens** a `Deny` to `Confirm` | the owner, out-of-band ONLY |
//! | `Protections` | **tightens** — adds a `HardNo` | the owner, from anywhere |
//!
//! Tightening is the owner exercising ownership of their own machine,
//! which is exactly what ADR-0029's second test protects: a guardrail
//! sits between the model and the machine, never between a person and
//! their own machine. Adding `~/Documents/Legal` to the refusal set is
//! not a security decision anybody needs protecting from — and **the
//! failure mode of being talked into *more* protection is not a failure
//! mode**, which is why this one may be offered from a dialog while
//! `Overrides` may not.
//!
//! # The structural guarantee
//!
//! This type cannot weaken anything, and that is enforced by shape
//! rather than by discipline:
//!
//! * It holds **only paths the owner added**. There is no representation
//!   of a built-in rule in here, so there is no `remove` that could
//!   reach one — `remove` can only take back something `add` put in.
//! * [`judge`](crate::action::judge) consults it as an **additional**
//!   refusal, never as a lookup that could answer "allowed". A path
//!   absent from this set means "this set has no opinion", never "this
//!   set permits it".
//!
//! So the worst a corrupted or hostile protections file can do is refuse
//! too much. That is a usability failure, recoverable by editing the
//! file; the opposite would be a security failure, recoverable by
//! nothing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Paths this machine's owner has put out of bounds for agent actions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Protections {
    paths: BTreeSet<PathBuf>,
}

impl Protections {
    /// A protections set from an iterator of paths.
    pub fn from_paths<I, P>(paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        Protections {
            paths: paths.into_iter().map(Into::into).collect(),
        }
    }

    /// Put a path out of bounds. Idempotent.
    ///
    /// Relative paths are refused rather than stored: a protection that
    /// depends on a working directory protects a different thing
    /// depending on where the agent happens to be, which is worse than
    /// no protection because it reads as one.
    pub fn add(&mut self, path: impl Into<PathBuf>) -> bool {
        let path = path.into();
        if !path.is_absolute() {
            return false;
        }
        self.paths.insert(path)
    }

    /// Take back a protection **this set added**.
    ///
    /// There is deliberately no way to reach a built-in rule from here.
    /// Removing `/etc` from this set does not make `/etc` writable: the
    /// built-in `rm.system_path` never consulted this set in the first
    /// place. The test `removing_a_builtin_path_does_not_permit_it`
    /// asserts that rather than trusting the comment.
    pub fn remove(&mut self, path: impl AsRef<Path>) -> bool {
        self.paths.remove(path.as_ref())
    }

    /// Is this target inside something the owner protected?
    ///
    /// Component-wise, so `/home/me/Legalese` is not caught by a
    /// protection on `/home/me/Legal`.
    pub fn covers(&self, target: impl AsRef<Path>) -> bool {
        let target = target.as_ref();
        self.paths.iter().any(|p| target.starts_with(p))
    }

    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.paths.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Parse the on-disk form: one absolute path per line, `#` comments.
    ///
    /// The same shape as `guard-allow`, deliberately — two files a
    /// person edits should not have two grammars. A line that is not an
    /// absolute path is DROPPED rather than rejecting the whole file: a
    /// typo in one entry must not silently disarm the other nine, which
    /// is the failure mode of parse-all-or-nothing on a security file.
    pub fn parse(text: &str) -> Self {
        let mut out = Protections::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if !line.is_empty() {
                out.add(line);
            }
        }
        out
    }

    /// The owner's protections file, or an empty set.
    ///
    /// Unreadable, absent or empty all mean "no owner protections",
    /// which is the state every machine starts in. Failing closed here
    /// would mean refusing everything when the file is missing, which is
    /// not a safer default — it is a broken one.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Write the set back, one path per line, with a header saying what
    /// the file is and which direction it may be edited in.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = String::from(
            "# Folders agents may not change on this machine (#253).\n\
             #\n\
             # One absolute path per line. Adding a line TIGHTENS what agents\n\
             # may do and is always safe. Nothing here can widen anything:\n\
             # removing a line takes back only what you added, never a rule\n\
             # Lisa ships. Loosening lives in `guard-allow`, next to this file.\n\
             #\n\
             # Written by Settings; safe to edit by hand.\n",
        );
        for p in &self.paths {
            text.push_str(&p.display().to_string());
            text.push('\n');
        }
        std::fs::write(path, text)
    }
}

/// Where the owner's protections live.
///
/// Beside `guard-allow`, because they are the two halves of one idea and
/// a person looking for either should find both.
pub fn protections_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("lisa").join("guard-protect"))
}

/// The protections in force for this machine's owner.
pub fn active() -> Protections {
    protections_path()
        .map(|p| Protections::load(&p))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_protected_path_covers_itself_and_what_is_under_it() {
        let p = Protections::from_paths(["/home/me/Documents/Legal"]);
        assert!(p.covers("/home/me/Documents/Legal"));
        assert!(p.covers("/home/me/Documents/Legal/contract.pdf"));
        assert!(!p.covers("/home/me/Documents"));
    }

    #[test]
    fn matching_is_component_wise_not_textual() {
        // `/home/me/Legalese` shares a prefix with `/home/me/Legal` as a
        // STRING and is a different directory. A guard that refused it
        // would be refusing the wrong thing, and one that used the same
        // sloppy match to ALLOW would be the mirror bug.
        let p = Protections::from_paths(["/home/me/Legal"]);
        assert!(!p.covers("/home/me/Legalese"));
        assert!(!p.covers("/home/me/Legalese/notes.txt"));
    }

    #[test]
    fn a_relative_path_is_refused_rather_than_stored() {
        // It would protect a different directory depending on where the
        // agent happened to be — worse than no protection, because it
        // reads as one in the settings list.
        let mut p = Protections::default();
        assert!(!p.add("Documents/Legal"));
        assert!(!p.add("../secrets"));
        assert!(p.is_empty());
        assert!(p.add("/home/me/Documents/Legal"));
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn remove_takes_back_only_what_add_put_in() {
        let mut p = Protections::from_paths(["/home/me/Legal"]);
        assert!(p.remove("/home/me/Legal"));
        assert!(!p.covers("/home/me/Legal"));
        // Removing something that was never here is a no-op, not an
        // error and not a grant.
        assert!(!p.remove("/etc"));
        assert!(!p.remove("/home/me/Legal"));
    }

    #[test]
    fn removing_a_builtin_path_does_not_permit_it() {
        // THE ACCEPTANCE CRITERION, asserted rather than argued. `/etc`
        // is refused by `rm.system_path`, a built-in that never consults
        // this set. Putting it in and taking it out again must leave the
        // built-in exactly where it was — this type has no
        // representation of a built-in rule and therefore no way to
        // reach one.
        let mut p = Protections::default();
        assert!(p.add("/etc"));
        assert!(p.remove("/etc"));
        assert!(!p.covers("/etc"), "the OWNER's entry is gone");
        // …and the built-in is untouched, which is the half that matters.
        assert!(crate::rules::is_system_target("/etc"));
        assert!(crate::rules::is_under_system_root("/etc/passwd"));
    }

    #[test]
    fn an_empty_set_has_no_opinion_rather_than_permitting() {
        // A path absent from this set means "no opinion here", never
        // "allowed". The only question this type answers is "did the
        // owner protect this?" — and `false` is not a grant.
        let p = Protections::default();
        assert!(!p.covers("/etc/passwd"));
        assert!(!p.covers("/home/me/anything"));
        assert!(crate::rules::is_system_target("/etc"));
    }

    #[test]
    fn a_typo_drops_one_line_rather_than_disarming_the_file() {
        // Parse-all-or-nothing on a SECURITY file is the wrong trade: one
        // bad line would silently leave the other nine unprotected, and
        // the person who typed it would have no way to see that.
        let p = Protections::parse(
            "# my folders\n             /home/me/Legal\n             Documents/typo          # relative — dropped\n             \n             /home/me/Tax   # trailing comment\n",
        );
        assert_eq!(p.len(), 2);
        assert!(p.covers("/home/me/Legal/contract.pdf"));
        assert!(p.covers("/home/me/Tax"));
        assert!(!p.covers("Documents/typo"));
    }

    #[test]
    fn a_missing_file_is_no_protections_rather_than_refuse_everything() {
        // Failing closed here would mean refusing every action on a
        // machine whose owner never opened the page — not a safer
        // default, a broken one.
        let p = Protections::load(Path::new("/nonexistent/guard-protect"));
        assert!(p.is_empty());
    }

    #[test]
    fn what_save_writes_is_what_load_reads() {
        let dir = std::env::temp_dir().join("lisa-guard-protect-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("lisa").join("guard-protect");
        let want = Protections::from_paths(["/home/me/Legal", "/home/me/Tax"]);
        want.save(&file).unwrap();
        assert_eq!(Protections::load(&file), want);
        // And the header survives a round trip as comments, not as paths.
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# Folders agents may not change"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adding_is_idempotent_and_ordered() {
        let mut p = Protections::default();
        assert!(p.add("/b"));
        assert!(!p.add("/b"), "the second add changes nothing");
        assert!(p.add("/a"));
        let got: Vec<_> = p.iter().map(|x| x.display().to_string()).collect();
        assert_eq!(
            got,
            vec!["/a", "/b"],
            "stable order, so the UI list is stable"
        );
    }
}
