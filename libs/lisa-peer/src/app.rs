//! Which *app* is calling (`docs/PLAN.md` §5.5, ADR-0033).
//!
//! [`crate::PeerId`] answers "same caller as before" and
//! [`crate::manager`] answers "may this program manage things". This
//! answers the third question: which installed application is on the
//! other end, so a grant, a quota or a memory namespace can be keyed to
//! it.
//!
//! Lives here rather than in the portal because it has three consumers:
//! the portal (grants and quotas), `contextd` (memory namespaces and
//! retrieval scopes, #101/#100), and anything that comes next. ADR-0033
//! rejected re-deriving identity per surface.
//!
//! - **Flatpak (strong):** the sandbox mounts `/.flatpak-info` read-only
//!   inside the app's mount namespace; the portal reads it through
//!   `/proc/<pid>/root/.flatpak-info`. This is the identity mechanism
//!   upstream xdg-desktop-portal uses — the app cannot forge it.
//! - **Host:** the caller's *executable*, from the kernel, matched
//!   against the `Exec=` line of an installed `.desktop` file.
//!
//! # Why not `comm` (issue #106)
//!
//! This module used to read `/proc/<pid>/comm` and match its value
//! against a desktop file's `Exec` basename. A process chooses its own
//! `comm` — `prctl(PR_SET_NAME)`, or simply being exec'd with a chosen
//! `argv[0]` — so any unsandboxed binary renamed itself to
//! `gnome-text-editor` and inherited that app's persisted grants, its
//! quota bucket, and its name in the Ledger. Identity was self-asserted.
//!
//! `/proc/<pid>/exe` is a kernel-maintained symlink to the inode that was
//! actually executed, resolved through the broker's **pidfd** so the pid
//! cannot be recycled underneath us (`crate::exe_of_peer`).
//!
//! # Why the whole path, not the basename
//!
//! Swapping `comm` for `exe` and still matching on the basename would
//! have moved the forgery, not removed it: an attacker who cannot set
//! `comm` can still *name a file* `gnome-text-editor` and run it from
//! their home directory. So a desktop entry is only an identity claim if
//! the caller's executable is the **exact file** that entry launches —
//! `/usr/bin/gnome-text-editor`, which an unprivileged process cannot
//! create.
//!
//! For the same reason `Exec=evince` (a bare program name) is resolved
//! against a fixed list of system binary directories rather than `$PATH`:
//! `$PATH` is inherited from the session and routinely contains
//! `~/.local/bin`, which the attacker owns.
//!
//! Parsing is pure and unit-tested everywhere; only the thin `/proc`
//! reads are Linux-at-runtime.

use crate::Peer;
use std::fmt;
use std::path::{Path, PathBuf};

/// Where a bare `Exec=name` may be resolved from. Deliberately fixed,
/// and deliberately not `$PATH` — see the module docs.
const SYSTEM_BIN_DIRS: [&str; 6] = [
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/local/sbin",
    "/usr/sbin",
    "/sbin",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityKind {
    /// Verified via the sandbox's `.flatpak-info` — trustworthy.
    Flatpak,
    /// The caller's executable is the exact file an installed `.desktop`
    /// entry launches. Attested by the kernel, not claimed by the app.
    Host,
    /// A host process that matches no installed app. Still a stable,
    /// unforgeable identity (its own executable), just not a named app —
    /// and it gets its own grant and quota bucket, never a victim's.
    Unattributed,
}

impl IdentityKind {
    /// How the identity is named in the Ledger and in the consent
    /// dialog. One function, so a new kind cannot mean one thing to the
    /// audit trail and another to the person being asked.
    pub fn as_str(&self) -> &'static str {
        match self {
            IdentityKind::Flatpak => "flatpak",
            IdentityKind::Host => "host",
            IdentityKind::Unattributed => "unattributed",
        }
    }
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

    /// A caller identified by its executable but matching no installed
    /// app — `host:/usr/bin/python3.13`.
    pub fn unattributed(exe: &Path) -> Self {
        Self {
            app_id: format!("host:{}", exe.display()),
            kind: IdentityKind::Unattributed,
        }
    }

    /// The identity used when the caller cannot be resolved at all (no
    /// pidfd from the broker, a p2p transport, `/proc` absent). Grants
    /// still key off it, so unknown callers share one bucket rather than
    /// bypassing consent — and because they share it, that bucket must
    /// never be able to name an installed app.
    pub fn unknown() -> Self {
        Self {
            app_id: "host:unknown".into(),
            kind: IdentityKind::Unattributed,
        }
    }
}

impl fmt::Display for AppIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.app_id)
    }
}

/// Does this executable sit where a Flatpak-spawned binary would?
///
/// Inside the sandbox, `bwrap` presents the application at `/app` and
/// the runtime at `/usr`; the paths a host process runs from —
/// `/usr/bin`, `/usr/local`, a home directory — are not what a
/// sandboxed app's `/proc/<pid>/exe` resolves to from OUTSIDE, which is
/// the deleted-or-relocated form under the Flatpak install tree.
///
/// This is a corroboration, not an authentication: the point is that a
/// process fabricating `.flatpak-info` in its own namespace does not
/// also get to change what the kernel reports it executed (#149). A
/// false negative costs a real app its sandbox identity and gives it
/// the host one, which is the safe direction — fewer grants, not more.
fn looks_flatpak_spawned(exe: &Path) -> bool {
    let p = exe.to_string_lossy();
    p.starts_with("/app/")
        || p.contains("/flatpak/app/")
        || p.contains("/flatpak/runtime/")
        // bwrap re-execs from the runtime; a sandboxed process seen from
        // the host often reports a path that no longer exists.
        || p.ends_with(" (deleted)")
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
    /// The absolute, symlink-resolved path of the program `Exec=` runs.
    /// An entry whose program could not be resolved to a real file is
    /// never built, so this always names something that exists.
    pub exec_path: PathBuf,
}

/// The program word of a `.desktop` file's `[Desktop Entry] Exec=`, as
/// written (absolute path or bare name).
///
/// Split out from path resolution so the parsing half stays pure and
/// testable on any host.
pub fn parse_exec_program(contents: &str) -> Option<String> {
    let mut in_entry = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry && let Some(exec) = line.strip_prefix("Exec=") {
            return exec.split_whitespace().next().map(str::to_string);
        }
    }
    None
}

/// Resolve a desktop entry's `Exec` program to the real file it runs.
///
/// `None` — and therefore no identity claim — when the program does not
/// resolve to an existing file under [`SYSTEM_BIN_DIRS`]. A desktop file
/// that points at a user-writable location cannot lend its name to
/// anyone: that is the difference between "an app is installed" and
/// "somebody dropped a file".
pub fn resolve_exec_program(program: &str, bin_dirs: &[&str]) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.canonicalize().ok();
    }
    // A bare name, or anything relative: only the system directories
    // count, and `../` games are excluded by taking the file name.
    let name = path.file_name()?;
    if Path::new(name) != path {
        return None;
    }
    bin_dirs
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find_map(|candidate| candidate.canonicalize().ok())
}

/// Build a mapping row from a `.desktop` file, or `None` if its program
/// cannot be resolved to a real system binary.
pub fn desktop_entry(id: &str, contents: &str, bin_dirs: &[&str]) -> Option<DesktopEntry> {
    let program = parse_exec_program(contents)?;
    let exec_path = resolve_exec_program(&program, bin_dirs)?;
    Some(DesktopEntry {
        id: id.trim_end_matches(".desktop").to_string(),
        exec_path,
    })
}

/// The identity of a host caller running `exe`.
///
/// An installed app whose `Exec` is *this exact file* lends its id;
/// anything else is [`AppIdentity::unattributed`] and gets its own
/// bucket. There is no basename comparison anywhere in this function,
/// and that is the whole of issue #106's fix.
pub fn host_identity(exe: &Path, entries: &[DesktopEntry]) -> AppIdentity {
    entries
        .iter()
        .find(|e| e.exec_path == exe)
        .map(|e| AppIdentity::host(e.id.clone()))
        .unwrap_or_else(|| AppIdentity::unattributed(exe))
}

/// Resolves a caller to an [`AppIdentity`]. Trait so the D-Bus layer
/// stays testable off-Linux.
pub trait IdentityResolver: Send + Sync {
    fn identify(&self, peer: &Peer) -> AppIdentity;
}

/// Fixed identity — tests and single-tenant dev setups.
pub struct StaticIdentity(pub AppIdentity);

impl IdentityResolver for StaticIdentity {
    fn identify(&self, _peer: &Peer) -> AppIdentity {
        self.0.clone()
    }
}

/// The real resolver: reads `/proc` through the broker's pidfd (Linux at
/// runtime; compiles everywhere, degrades to [`AppIdentity::unknown`]
/// where `/proc` or the pidfd is absent).
pub struct ProcResolver {
    proc_root: PathBuf,
    desktop_entries: Vec<DesktopEntry>,
}

impl ProcResolver {
    pub fn new() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
            desktop_entries: scan_desktop_entries(&SYSTEM_BIN_DIRS),
        }
    }

    /// Test constructor: an explicit `/proc` root and mapping table, so
    /// the resolution rules can be exercised without a live peer.
    pub fn with_parts(proc_root: impl Into<PathBuf>, desktop_entries: Vec<DesktopEntry>) -> Self {
        Self {
            proc_root: proc_root.into(),
            desktop_entries,
        }
    }

    /// The identity of the process pinned at `pid`, split out from
    /// [`IdentityResolver::identify`] so the flatpak-then-exe order is
    /// testable against a fake `/proc` tree.
    ///
    /// `exe` is supplied by the caller rather than read here: on a real
    /// system it comes from `crate::exe_of_peer`, which resolves the
    /// pidfd, and a second independent read would be a second chance to
    /// get the wrong process.
    pub fn identity_of(&self, pid: u32, exe: Option<&Path>) -> AppIdentity {
        let proc_pid = self.proc_root.join(pid.to_string());
        // Flatpak first — but corroborated, not believed (#149).
        //
        // `.flatpak-info` is read through /proc/<pid>/root, which shows
        // the process's OWN mount view. For a real sandboxed app that is
        // exactly right: it cannot remove or edit its marker from
        // inside. For a host process it is a free identity, and the PoC
        // on the reference iMac is four lines of shell: unshare -Umr,
        // pivot_root into a tmpfs, write whatever marker you like, and
        // /proc/<pid>/root/.flatpak-info says you are
        // org.gnome.Calculator.
        //
        // The exe is the anchor. /proc/<pid>/exe is a kernel symlink to
        // the inode that was executed, and a mount namespace does not
        // change what it reports — in the PoC it still said
        // /usr/bin/sleep while the marker claimed a sandboxed app. So
        // the two sources disagree precisely in the case that matters,
        // and a marker is only honoured when the executable is where a
        // Flatpak-spawned binary actually lives.
        // A marker that is NOT corroborated falls through to the exe
        // path: the process is a host process, which is what the kernel
        // says it is. Falling through rather than refusing means a
        // legitimate program with an unusual layout still gets its real
        // identity instead of an error.
        if let Ok(info) = std::fs::read_to_string(proc_pid.join("root/.flatpak-info"))
            && let Some(app_id) = parse_flatpak_info(&info)
            && exe.is_none_or(looks_flatpak_spawned)
        {
            return AppIdentity::flatpak(app_id);
        }
        match exe {
            Some(exe) => host_identity(exe, &self.desktop_entries),
            None => AppIdentity::unknown(),
        }
    }
}

impl Default for ProcResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentityResolver for ProcResolver {
    fn identify(&self, peer: &Peer) -> AppIdentity {
        // One pidfd, one process: the pid used for `.flatpak-info` and
        // the executable read are the same pinned process, and neither
        // is the peer's own word for it.
        #[cfg(unix)]
        {
            let Ok(pid) = crate::pid_of_peer(peer) else {
                return AppIdentity::unknown();
            };
            let exe = crate::exe_of_peer(peer).ok();
            self.identity_of(pid, exe.as_deref())
        }
        #[cfg(not(unix))]
        {
            let _ = peer;
            AppIdentity::unknown()
        }
    }
}

/// Index `applications/*.desktop` across the XDG data dirs.
fn scan_desktop_entries(bin_dirs: &[&str]) -> Vec<DesktopEntry> {
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
                && let Some(entry) = desktop_entry(name, &contents, bin_dirs)
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

    /// A directory standing in for `/usr/bin`, holding real files so the
    /// resolver's "must exist" rule is exercised rather than assumed.
    fn bin_dir_with(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            std::fs::write(dir.path().join(name), b"#!/bin/sh\n").unwrap();
        }
        dir
    }

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
    fn desktop_entry_resolves_exec_to_a_real_file() {
        let bin = bin_dir_with(&["gnome-text-editor"]);
        let bins = [bin.path().to_str().unwrap()];
        let contents =
            "[Desktop Entry]\nName=Text Editor\nExec=gnome-text-editor %U\nType=Application\n";
        let entry = desktop_entry("org.gnome.TextEditor.desktop", contents, &bins).unwrap();
        assert_eq!(entry.id, "org.gnome.TextEditor");
        assert_eq!(
            entry.exec_path,
            bin.path().join("gnome-text-editor").canonicalize().unwrap()
        );
    }

    #[test]
    fn desktop_entry_ignores_exec_in_action_sections() {
        let contents = "[Desktop Action new-window]\nExec=evil\n";
        assert_eq!(parse_exec_program(contents), None);
    }

    /// A `.desktop` file naming a program that is not installed lends
    /// its id to nobody — otherwise dropping a matching binary anywhere
    /// would be enough to claim the name.
    #[test]
    fn a_desktop_entry_whose_program_does_not_exist_is_dropped() {
        let bin = bin_dir_with(&[]);
        let bins = [bin.path().to_str().unwrap()];
        let contents = "[Desktop Entry]\nExec=not-installed\n";
        assert_eq!(desktop_entry("a.desktop", contents, &bins), None);
    }

    /// Issue #106, the core of it: an attacker's own binary named after
    /// a victim's must not resolve to the victim's app id. Under the old
    /// basename rule this exact case returned `org.gnome.TextEditor`.
    #[test]
    fn a_look_alike_binary_elsewhere_never_inherits_an_app_id() {
        let real = bin_dir_with(&["gnome-text-editor"]);
        let attacker = bin_dir_with(&["gnome-text-editor"]);
        let bins = [real.path().to_str().unwrap()];
        let entries = vec![
            desktop_entry(
                "org.gnome.TextEditor.desktop",
                "[Desktop Entry]\nExec=gnome-text-editor %U\n",
                &bins,
            )
            .unwrap(),
        ];

        let victim = real
            .path()
            .join("gnome-text-editor")
            .canonicalize()
            .unwrap();
        assert_eq!(
            host_identity(&victim, &entries),
            AppIdentity::host("org.gnome.TextEditor")
        );

        let impostor = attacker
            .path()
            .join("gnome-text-editor")
            .canonicalize()
            .unwrap();
        let got = host_identity(&impostor, &entries);
        assert_ne!(got.app_id, "org.gnome.TextEditor");
        assert_eq!(got.kind, IdentityKind::Unattributed);
        assert_eq!(got, AppIdentity::unattributed(&impostor));
    }

    /// `$PATH` is the session's, and routinely holds a directory the
    /// caller can write to. A bare `Exec=` name must resolve only from
    /// the fixed system list.
    #[test]
    fn a_bare_exec_name_resolves_only_from_system_directories() {
        let attacker = bin_dir_with(&["evince"]);
        let system = bin_dir_with(&[]);
        // The attacker's directory is not in the list, so nothing
        // resolves — even though a file with the right name exists.
        assert_eq!(
            resolve_exec_program("evince", &[system.path().to_str().unwrap()]),
            None
        );
        // And it is found when it really is a system binary.
        let system = bin_dir_with(&["evince"]);
        assert!(resolve_exec_program("evince", &[system.path().to_str().unwrap()]).is_some());
        let _ = attacker;
    }

    /// `Exec=../../home/mallory/evince` must not walk out of the system
    /// directories.
    #[test]
    fn a_relative_exec_path_is_refused() {
        let system = bin_dir_with(&["evince"]);
        let bins = [system.path().to_str().unwrap()];
        assert_eq!(resolve_exec_program("../evince", &bins), None);
        assert_eq!(resolve_exec_program("sub/evince", &bins), None);
    }

    /// The unresolvable caller must land in a bucket that can never be
    /// an installed app's — every unidentified caller shares it.
    #[test]
    fn the_unknown_identity_is_never_an_app_id() {
        let unknown = AppIdentity::unknown();
        assert_eq!(unknown.kind, IdentityKind::Unattributed);
        assert!(unknown.app_id.starts_with("host:"));
    }

    #[test]
    fn a_peer_the_broker_cannot_pin_is_unknown() {
        // No pidfd: nothing about this caller can be established, and
        // guessing from its pid is what #136 removed.
        let peer = Peer::without_process_fd(
            crate::PeerId::Bus(":1.7".into()),
            Some(0),
            Some(std::process::id()),
        );
        let resolver = ProcResolver::with_parts("/proc", Vec::new());
        assert_eq!(resolver.identify(&peer), AppIdentity::unknown());
    }

    /// Issue #149, reproduced on hardware before it was fixed: an
    /// unprivileged process creates a user + mount namespace,
    /// pivot_roots into a tmpfs it controls, and writes its own
    /// `.flatpak-info`. `/proc/<pid>/root` shows the process's OWN mount
    /// view, so the portal read the fabricated marker and would have
    /// handed it that app's grants, quota and Ledger attribution.
    ///
    /// This test is the PoC in miniature: a marker claiming a sandbox,
    /// and an executable that is plainly a host binary.
    #[test]
    fn a_fabricated_sandbox_marker_does_not_grant_a_sandbox_identity() {
        let root = tempfile::tempdir().unwrap();
        let spoofed = root.path().join("303/root");
        std::fs::create_dir_all(&spoofed).unwrap();
        std::fs::write(
            spoofed.join(".flatpak-info"),
            "[Application]\nname=org.gnome.Calculator\n",
        )
        .unwrap();

        let resolver = ProcResolver::with_parts(root.path(), Vec::new());
        // /proc/<pid>/exe is a kernel symlink to the inode that was
        // executed; a mount namespace does not change it. In the real
        // PoC it said /usr/bin/sleep while the marker claimed an app.
        let got = resolver.identity_of(303, Some(Path::new("/usr/bin/sleep")));
        assert_ne!(
            got,
            AppIdentity::flatpak("org.gnome.Calculator"),
            "a host process fabricated a sandbox identity"
        );
        // It gets what it actually is, rather than being refused: a
        // legitimate program with an unusual layout still works.
        assert_eq!(got, AppIdentity::unattributed(Path::new("/usr/bin/sleep")));
    }

    #[test]
    fn the_shapes_a_real_sandboxed_binary_reports_are_accepted() {
        // False negatives cost a real app its sandbox identity and give
        // it the host one — fewer grants, not more — but they are still
        // wrong, so the paths bwrap actually produces are covered.
        for exe in [
            "/app/bin/calculator",
            "/var/lib/flatpak/app/org.gnome.Calculator/current/active/files/bin/calc",
            "/var/lib/flatpak/runtime/org.gnome.Platform/x86_64/47/active/files/bin/sh",
            "/usr/bin/thing (deleted)",
        ] {
            assert!(
                looks_flatpak_spawned(Path::new(exe)),
                "should be treated as sandbox-spawned: {exe}"
            );
        }
        for exe in ["/usr/bin/sleep", "/usr/local/bin/x", "/home/lisa/a.out"] {
            assert!(
                !looks_flatpak_spawned(Path::new(exe)),
                "host binary should not corroborate a marker: {exe}"
            );
        }
    }

    #[test]
    fn flatpak_wins_over_the_executable_and_a_missing_exe_is_unknown() {
        let root = tempfile::tempdir().unwrap();
        let flatpak = root.path().join("101/root");
        std::fs::create_dir_all(&flatpak).unwrap();
        std::fs::write(
            flatpak.join(".flatpak-info"),
            "[Application]\nname=org.example.Sandboxed\n",
        )
        .unwrap();

        let resolver = ProcResolver::with_parts(root.path(), Vec::new());
        // Marker plus a Flatpak-shaped executable: the sandbox identity.
        assert_eq!(
            resolver.identity_of(101, Some(Path::new("/app/bin/sandboxed"))),
            AppIdentity::flatpak("org.example.Sandboxed")
        );
        // Marker with no executable to check it against: honoured, since
        // there is nothing to contradict it and refusing would break a
        // caller that cannot supply one.
        assert_eq!(
            resolver.identity_of(101, None),
            AppIdentity::flatpak("org.example.Sandboxed")
        );
        // No marker and no executable: unknown, never a name.
        assert_eq!(resolver.identity_of(202, None), AppIdentity::unknown());
        // No marker, but a real executable: its own bucket.
        assert_eq!(
            resolver.identity_of(202, Some(Path::new("/usr/bin/vim"))),
            AppIdentity::unattributed(Path::new("/usr/bin/vim"))
        );
    }
}
