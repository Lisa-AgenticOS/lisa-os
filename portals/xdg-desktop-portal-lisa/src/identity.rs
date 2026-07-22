//! Per-app identity (`docs/PLAN.md` §5.5): who is on the other end of
//! the bus connection?
//!
//! - **Flatpak (strong):** the sandbox mounts `/.flatpak-info` read-only
//!   inside the app's mount namespace; the portal reads it through
//!   `/proc/<pid>/root/.flatpak-info`. This is the identity mechanism
//!   upstream xdg-desktop-portal uses — the app cannot forge it.
//! - **Host (best effort, documented as weaker):** peer-cred pid →
//!   `/proc/<pid>/comm` → `.desktop` mapping (a desktop file whose Exec
//!   basename matches); falls back to `host:<comm>`.
//!
//! Parsing is pure and unit-tested everywhere; only the thin `/proc`
//! reads are Linux-at-runtime.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityKind {
    /// Verified via the sandbox's `.flatpak-info` — trustworthy.
    Flatpak,
    /// Peer-cred + `.desktop` mapping — best effort, spoofable by a
    /// malicious host process (which already owns the session anyway).
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    pub app_id: String,
    pub kind: IdentityKind,
}

impl AppIdentity {
    pub fn flatpak(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            kind: IdentityKind::Flatpak,
        }
    }

    pub fn host(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            kind: IdentityKind::Host,
        }
    }

    /// The identity used when nothing about the peer can be resolved
    /// (e.g. no pid over a p2p connection). Grants still key off it, so
    /// unknown callers share one bucket rather than bypassing consent.
    pub fn unknown() -> Self {
        Self::host("host:unknown")
    }
}

impl fmt::Display for AppIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.app_id)
    }
}

/// Parse the `[Application] name=` key out of a `.flatpak-info` file
/// (INI; see flatpak-metadata(5)).
pub fn parse_flatpak_info(contents: &str) -> Option<String> {
    let mut in_application = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_application = line == "[Application]";
            continue;
        }
        if in_application && let Some(value) = line.strip_prefix("name=") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// One installed `.desktop` file, reduced to what host mapping needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Desktop file id without the `.desktop` suffix (reverse-DNS style
    /// for well-behaved apps: `org.gnome.TextEditor`).
    pub id: String,
    /// Basename of the first word of `Exec=`.
    pub exec_basename: String,
}

/// Parse a `.desktop` file's `[Desktop Entry] Exec=` into a mapping row.
pub fn parse_desktop_entry(id: &str, contents: &str) -> Option<DesktopEntry> {
    let mut in_entry = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry && let Some(exec) = line.strip_prefix("Exec=") {
            let first = exec.split_whitespace().next()?;
            let basename = Path::new(first).file_name()?.to_str()?;
            return Some(DesktopEntry {
                id: id.trim_end_matches(".desktop").to_string(),
                exec_basename: basename.to_string(),
            });
        }
    }
    None
}

/// Best-effort host identity: a desktop file whose Exec basename matches
/// the process comm wins; otherwise `host:<comm>`.
pub fn host_app_id(comm: &str, entries: &[DesktopEntry]) -> String {
    entries
        .iter()
        .find(|e| e.exec_basename == comm)
        .map(|e| e.id.clone())
        .unwrap_or_else(|| format!("host:{comm}"))
}

/// Resolves a caller (by peer pid, when the transport provides one) to
/// an [`AppIdentity`]. Trait so the D-Bus layer stays testable off-Linux.
pub trait IdentityResolver: Send + Sync {
    fn identify(&self, pid: Option<u32>) -> AppIdentity;
}

/// Fixed identity — tests and single-tenant dev setups.
pub struct StaticIdentity(pub AppIdentity);

impl IdentityResolver for StaticIdentity {
    fn identify(&self, _pid: Option<u32>) -> AppIdentity {
        self.0.clone()
    }
}

/// The real resolver: reads `/proc` (Linux at runtime; compiles
/// everywhere, degrades to [`AppIdentity::unknown`] where `/proc` is
/// absent).
pub struct ProcResolver {
    proc_root: PathBuf,
    desktop_entries: Vec<DesktopEntry>,
}

impl ProcResolver {
    pub fn new() -> Self {
        Self::with_proc_root("/proc")
    }

    pub fn with_proc_root(proc_root: impl Into<PathBuf>) -> Self {
        Self {
            proc_root: proc_root.into(),
            desktop_entries: scan_desktop_entries(),
        }
    }
}

impl Default for ProcResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentityResolver for ProcResolver {
    fn identify(&self, pid: Option<u32>) -> AppIdentity {
        let Some(pid) = pid else {
            return AppIdentity::unknown();
        };
        let proc_pid = self.proc_root.join(pid.to_string());
        // Flatpak first: the sandbox cannot remove its own marker.
        if let Ok(info) = std::fs::read_to_string(proc_pid.join("root/.flatpak-info"))
            && let Some(app_id) = parse_flatpak_info(&info)
        {
            return AppIdentity::flatpak(app_id);
        }
        match std::fs::read_to_string(proc_pid.join("comm")) {
            Ok(comm) => AppIdentity::host(host_app_id(comm.trim(), &self.desktop_entries)),
            Err(_) => AppIdentity::unknown(),
        }
    }
}

/// Index `applications/*.desktop` across the XDG data dirs.
fn scan_desktop_entries() -> Vec<DesktopEntry> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share"));
    }
    match std::env::var("XDG_DATA_DIRS") {
        Ok(paths) => dirs.extend(paths.split(':').map(PathBuf::from)),
        Err(_) => dirs.extend(["/usr/local/share".into(), "/usr/share".into()]),
    }
    let mut entries = Vec::new();
    for dir in dirs {
        let apps = dir.join("applications");
        let Ok(read) = std::fs::read_dir(&apps) else {
            continue;
        };
        for file in read.flatten() {
            let name = file.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.ends_with(".desktop") {
                continue;
            }
            if let Ok(contents) = std::fs::read_to_string(file.path())
                && let Some(entry) = parse_desktop_entry(name, &contents)
            {
                entries.push(entry);
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_info_yields_the_application_name() {
        let info = "[Application]\nname=org.example.Demo\nruntime=org.gnome.Platform/x86_64/47\n\n[Context]\nshared=network;\n";
        assert_eq!(
            parse_flatpak_info(info).as_deref(),
            Some("org.example.Demo")
        );
    }

    #[test]
    fn flatpak_info_ignores_name_keys_outside_the_application_section() {
        let info = "[Context]\nname=liar\n[Instance]\nname=also-liar\n";
        assert_eq!(parse_flatpak_info(info), None);
        assert_eq!(parse_flatpak_info(""), None);
    }

    #[test]
    fn desktop_entry_maps_exec_basename() {
        let contents = "[Desktop Entry]\nName=Text Editor\nExec=/usr/bin/gnome-text-editor %U\nType=Application\n";
        let entry = parse_desktop_entry("org.gnome.TextEditor.desktop", contents).unwrap();
        assert_eq!(entry.id, "org.gnome.TextEditor");
        assert_eq!(entry.exec_basename, "gnome-text-editor");
    }

    #[test]
    fn desktop_entry_ignores_exec_in_action_sections() {
        let contents = "[Desktop Action new-window]\nExec=evil\n";
        assert_eq!(parse_desktop_entry("a.desktop", contents), None);
    }

    #[test]
    fn host_identity_prefers_desktop_mapping_then_falls_back() {
        let entries = vec![DesktopEntry {
            id: "org.gnome.TextEditor".into(),
            exec_basename: "gnome-text-editor".into(),
        }];
        assert_eq!(
            host_app_id("gnome-text-editor", &entries),
            "org.gnome.TextEditor"
        );
        assert_eq!(host_app_id("vim", &entries), "host:vim");
    }

    #[test]
    fn proc_resolver_reads_flatpak_info_then_comm() {
        let root = tempfile::tempdir().unwrap();
        let flatpak = root.path().join("101/root");
        std::fs::create_dir_all(&flatpak).unwrap();
        std::fs::write(
            flatpak.join(".flatpak-info"),
            "[Application]\nname=org.example.Sandboxed\n",
        )
        .unwrap();
        let host = root.path().join("202");
        std::fs::create_dir_all(&host).unwrap();
        std::fs::write(host.join("comm"), "somebin\n").unwrap();

        let resolver = ProcResolver::with_proc_root(root.path());
        assert_eq!(
            resolver.identify(Some(101)),
            AppIdentity::flatpak("org.example.Sandboxed")
        );
        assert_eq!(resolver.identify(Some(202)).kind, IdentityKind::Host);
        assert_eq!(resolver.identify(Some(202)).app_id, "host:somebin");
        assert_eq!(resolver.identify(Some(999)), AppIdentity::unknown());
        assert_eq!(resolver.identify(None), AppIdentity::unknown());
    }
}
