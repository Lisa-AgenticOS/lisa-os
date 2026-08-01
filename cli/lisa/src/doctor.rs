//! `lisa doctor` — the state of this machine, in one command.
//!
//! # Why this exists
//!
//! Every bug on the reference hardware this month was diagnosed the same
//! way: ssh in, run `journalctl`, run a probe, read a version. The
//! browser would not play video (a user-agent string), the machine
//! seemed to sleep (Wi-Fi powersave, not suspend), an update reported
//! success while nothing was staged. In each case the fix was minutes
//! and the *finding out* was the work — and it needed someone with a
//! shell on the machine.
//!
//! So this collects, in one pass, the things that turned out to matter,
//! and writes them somewhere shareable. It is not a log viewer: a viewer
//! helps the person sitting at the machine, and the expensive gap is
//! handing the state to someone who is not.
//!
//! # Redaction is the design, not a filter on the end
//!
//! A diagnostic bundle is pasted into issues and chat windows. It is
//! assembled from the Ledger (which holds prompt previews), from
//! provider configuration (which can hold a URL with a password in it,
//! #109), and from journals (which hold whatever any daemon logged). All
//! of that is content this OS spends considerable effort *not* leaking.
//!
//! So the rule here is inverted from a normal log tool: **nothing is
//! included unless it is known to be safe**, previews are dropped by
//! default rather than scrubbed, and every line that does go in passes
//! [`redact`] on the way. The alternative — collect everything, scrub
//! afterwards — fails the first time somebody logs a token in a shape
//! the scrubber does not know.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One titled block of the report.
pub struct Section {
    pub title: String,
    pub body: String,
}

/// Patterns that mean "this is a credential", regardless of context.
///
/// Deliberately generous: a false positive costs one unreadable line in
/// a diagnostic, and a false negative is a key in a GitHub issue.
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "pk-",
    "pk_",
    "ghp_",
    "gho_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "AIza",
    "ya29.",
    "AKIA",
    "ASIA",
    "hf_",
    "r8_",
    "gsk_",
    "tvly-",
    "pplx-",
    "dop_v1_",
    "glpat-",
];

/// Keys whose *value* is a secret whatever it looks like.
const SECRET_KEYS: &[&str] = &[
    "authorization",
    "x-api-key",
    "api_key",
    "apikey",
    "token",
    "access_token",
    "refresh_token",
    "id_token",
    "password",
    "passwd",
    "secret",
    "client_secret",
    "code_verifier",
    "cookie",
    "set-cookie",
];

/// Make one line safe to paste in public.
///
/// Four things, in order:
///
/// 1. **`key=value` and `key: value` where the key names a secret** —
///    the value goes, whatever its shape. This is the one that catches
///    a credential nobody anticipated the format of.
/// 2. **Known credential prefixes** anywhere in the line. `sk-…` is a
///    key even when it appears in prose.
/// 3. **URL userinfo** — `https://alice:hunter2@host/` is exactly the
///    shape issue #109 found being written to a 0644 file and an
///    append-only Ledger, so it will be in old rows.
/// 4. **The user's home path** becomes `~`. Not a secret, but a real
///    name and a real directory layout, and nothing in a diagnostic
///    needs either.
pub fn redact(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    // A header is written `authorization: Bearer eyJ…` — three
    // whitespace-separated tokens, where only the FIRST says the rest
    // is a credential. Judging tokens independently caught
    // `client_secret=…` and missed `client_secret: …`, which is the
    // spelling every HTTP log and JSON dump actually uses.
    let mut expecting_secret = false;
    for (i, token) in line.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if expecting_secret && !token.is_empty() {
            // `Bearer`/`Basic` is the scheme, not the credential: hold
            // the flag one more token.
            let word = token.trim_matches(|c: char| !c.is_alphanumeric());
            if word.eq_ignore_ascii_case("bearer") || word.eq_ignore_ascii_case("basic") {
                out.push_str(token);
                continue;
            }
            out.push_str("«redacted»");
            expecting_secret = false;
            continue;
        }
        expecting_secret = names_a_secret(token);
        out.push_str(&redact_token(token));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().into_owned();
        if home.len() > 1 {
            out = out.replace(&home, "~");
        }
    }
    out
}

/// Whether this token is a bare secret NAME, so the next token is its
/// value: `password:`, `client_secret =`, `token`.
fn names_a_secret(token: &str) -> bool {
    let name = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
    // Only when the token carries no value of its own — `password=x`
    // is handled inside `redact_token`, and treating it as a name too
    // would eat the following, innocent word.
    let has_value = token
        .find(['=', ':'])
        .is_some_and(|i| !token[i + 1..].trim().is_empty());
    if has_value {
        return false;
    }
    let k = name.to_ascii_lowercase();
    SECRET_KEYS
        .iter()
        .any(|s| k == *s || k.ends_with(&format!("_{s}")) || k.ends_with(&format!("-{s}")))
}

fn redact_token(token: &str) -> String {
    // Split once into a name and a value, so both halves can be judged:
    // `OPENAI_API_KEY=sk-…` is a secret KEY *and* a secret VALUE, and
    // `endpoint=https://u:p@host` is neither until you look inside the
    // value. Reading only the whole token missed all three.
    let split = token
        .find(['=', ':'])
        .map(|i| (&token[..i], token.as_bytes()[i] as char, &token[i + 1..]));

    if let Some((key, sep, value)) = split {
        let k = key
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
            .to_ascii_lowercase();
        // `ends_with`, not equality: the name in the wild is
        // `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `--client-secret`.
        // Anchored at the end so `token_count` is not a credential.
        let named_secret = SECRET_KEYS
            .iter()
            .any(|s| k == *s || k.ends_with(&format!("_{s}")) || k.ends_with(&format!("-{s}")));
        if named_secret && !value.trim().is_empty() {
            return format!("{key}{sep}«redacted»");
        }
        if !value.is_empty() {
            let cleaned = redact_value(value);
            if cleaned != value {
                return format!("{key}{sep}{cleaned}");
            }
        }
    }
    redact_value(token)
}

/// Judge one value: a known credential shape, or a URL with userinfo.
fn redact_value(value: &str) -> String {
    let trimmed = value.trim_matches(|c: char| c == '"' || c == '\'' || c == ',');
    if SECRET_PREFIXES
        .iter()
        .any(|p| trimmed.starts_with(p) && trimmed.len() > p.len() + 4)
    {
        return value.replace(trimmed, "«redacted»");
    }
    // A bearer token is a value whose *previous* word named it, which
    // whitespace splitting already handles — but `Bearer eyJ…` arrives
    // as its own token, and a JWT is recognisable on its own.
    if trimmed.starts_with("eyJ") && trimmed.matches('.').count() >= 2 {
        return value.replace(trimmed, "«redacted»");
    }
    if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        && let Some((userinfo, host)) = rest.split_once('@')
        // An '@' after the first '/' is part of a path, not userinfo.
        && !userinfo.contains('/')
    {
        let scheme = if trimmed.starts_with("https") {
            "https"
        } else {
            "http"
        };
        return value.replace(trimmed, &format!("{scheme}://«redacted»@{host}"));
    }
    value.to_string()
}

/// Redact a whole block, line by line.
pub fn redact_block(text: &str) -> String {
    text.lines().map(redact).collect::<Vec<_>>().join("\n")
}

/// Run a command and return its stdout, or a one-line note about why
/// there is none.
///
/// A missing tool is information, not a failure: `systemctl` absent
/// means this is not a Lisa OS machine, which is exactly what a reader
/// needs to know before wondering why the units section is empty.
fn capture(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
            if text.is_empty() {
                let err = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
                if err.is_empty() {
                    "(nothing)".into()
                } else {
                    format!("(no output; stderr: {})", redact(&err))
                }
            } else {
                redact_block(&text)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            format!("({program} is not installed here)")
        }
        Err(e) => format!("({program} failed: {e})"),
    }
}

/// The Lisa units worth knowing the state of, by their INSTALLED name.
///
/// Not their repo filename: `os/packages/lisa/lisa-remoted-user.service`
/// installs as `usr/lib/systemd/user/lisa-remoted.service`. Listing the
/// source name too put a line in every report that read
///
///     lisa-remoted-user.service            not running
///
/// for a unit no system can ever have. A status list with a permanent
/// false alarm in it is worse than a shorter one — it teaches the reader
/// that "not running" here is normal.
const UNITS: &[&str] = &[
    "lisa-inferenced.service",
    "lisa-modeld.service",
    "lisa-contextd.service",
    "lisa-agentd.service",
    "lisa-harnessd.service",
    "lisa-remoted.service",
    "lisa-inferenced-dbus.service",
    "xdg-desktop-portal-lisa.service",
];

/// Unit names some D-Bus activation file starts, read from the files
/// themselves (`SystemdService=`) rather than guessed from a naming
/// convention — the mapping is arbitrary on purpose
/// (`dev.lisaos.Portal.service` → `xdg-desktop-portal-lisa.service`).
fn dbus_activatable_units() -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for dir in [
        "/usr/share/dbus-1/services",
        "/usr/share/dbus-1/system-services",
        "/usr/local/share/dbus-1/services",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            for line in text.lines() {
                if let Some(v) = line.trim().strip_prefix("SystemdService=") {
                    out.insert(v.trim().to_string());
                }
            }
        }
    }
    out
}

fn os_section() -> Section {
    let mut body = String::new();
    match std::fs::read_to_string("/etc/os-release") {
        Ok(text) => {
            for key in ["PRETTY_NAME", "IMAGE_VERSION", "VERSION_ID", "BUILD_ID"] {
                if let Some(v) = crate::os_release_value(&text, key) {
                    let _ = writeln!(body, "{key:<14} {v}");
                }
            }
        }
        Err(e) => {
            let _ = writeln!(body, "(/etc/os-release unreadable: {e})");
        }
    }
    let _ = writeln!(body, "{:<14} {}", "kernel", capture("uname", &["-r"]));
    // Which A/B slot is running, from three angles, because they can
    // disagree and the disagreement is the bug: the booted entry, the
    // root the kernel was told to mount, and the device actually
    // mounted. Issue #141 was a pinned default silently defeating every
    // future update — visible here as an entry that never changes while
    // the staged version does.
    let entry = capture("bootctl", &["status", "--no-pager"])
        .lines()
        .find(|l| l.trim_start().starts_with("Current Entry:"))
        .map(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
        .unwrap_or_default()
        .unwrap_or_else(|| "(unknown)".into());
    let _ = writeln!(body, "{:<14} {}", "boot entry", entry);
    // …and the CAUSE, not just the symptom. The lines above show which
    // entry booted, which only reveals a pin by comparing two runs
    // weeks apart. A pinned default is one file and one EFI variable,
    // and reading them says outright whether this machine can ever boot
    // an update.
    match pinned_default(
        &read_first(&["/efi/loader/loader.conf", "/boot/loader/loader.conf"]),
        read_efi_default().as_deref(),
    ) {
        Some(pin) => {
            let _ = writeln!(body, "{:<14} {} — PINNED", "boot default", pin);
            let _ = writeln!(
                body,
                "{:<14} a pinned default means staged updates install and never boot.",
                ""
            );
            let _ = writeln!(
                body,
                "{:<14} clear it with `bootctl set-default @saved` (issue #141).",
                ""
            );
        }
        None => {
            let _ = writeln!(body, "{:<14} not pinned", "boot default");
        }
    }
    let cmdline_root = std::fs::read_to_string("/proc/cmdline")
        .unwrap_or_default()
        .split_whitespace()
        .find_map(|w| w.strip_prefix("root=").map(str::to_string))
        .unwrap_or_else(|| "(unknown)".into());
    let _ = writeln!(body, "{:<14} {}", "root=", cmdline_root);
    let _ = writeln!(
        body,
        "{:<14} {}",
        "root device",
        capture("findmnt", &["-no", "SOURCE", "/"])
    );
    Section {
        title: "System".into(),
        body,
    }
}

fn read_first(paths: &[&str]) -> String {
    paths
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// systemd-boot writes the sticky default here.
const EFI_DEFAULT: &str =
    "/sys/firmware/efi/efivars/LoaderEntryDefault-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f";

fn read_efi_default() -> Option<String> {
    let raw = std::fs::read(EFI_DEFAULT).ok()?;
    // EFI variables carry a 4-byte attribute prefix, then UTF-16LE.
    let body = raw.get(4..)?;
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .take_while(|&c| c != 0)
        .collect();
    let text = String::from_utf16_lossy(&units);
    Some(text).filter(|t| !t.is_empty())
}

/// Is the boot default pinned to one specific entry?
///
/// Issue #141: the reference iMac staged 20260727.43, rebooted, and came
/// up on 20260726.34. Nothing failed and nothing logged — the only
/// evidence was /etc/os-release, and a reboot was spent debugging audio
/// that had never been installed. A machine that silently stops updating
/// is indistinguishable from one that works, which is what makes this
/// worth a dedicated check rather than an eyeball on the entry name.
///
/// `@saved` is NOT a pin: it follows whatever booted last, which is how
/// sd-boot is meant to track an A/B swap. A pattern is not a pin either
/// — `lisa_*.efi` still resolves to the newest match. Only a literal
/// entry freezes the machine.
pub fn pinned_default(loader_conf: &str, efi_var: Option<&str>) -> Option<String> {
    let is_pin = |value: &str| -> Option<String> {
        let v = value.trim();
        if v.is_empty() || v.eq_ignore_ascii_case("@saved") || v.contains('*') || v.contains('?') {
            return None;
        }
        Some(v.to_string())
    };
    // The EFI variable wins: bootctl set-default writes there, and it
    // overrides loader.conf.
    if let Some(pin) = efi_var.and_then(is_pin) {
        return Some(pin);
    }
    // loader.conf is `key value`, whitespace-separated — not `key=value`.
    loader_conf
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let rest = l.strip_prefix("default")?;
            // A key is only `default` if a space follows: `defaults` and
            // `default-something` are different keys entirely.
            rest.starts_with(char::is_whitespace).then_some(rest)
        })
        .and_then(is_pin)
}

fn units_section() -> Section {
    let mut body = String::new();
    let _ = writeln!(body, "-- failed units (system) --");
    let _ = writeln!(
        body,
        "{}",
        capture("systemctl", &["--failed", "--no-pager", "--no-legend"])
    );
    let _ = writeln!(body, "\n-- failed units (user) --");
    let _ = writeln!(
        body,
        "{}",
        capture(
            "systemctl",
            &["--user", "--failed", "--no-pager", "--no-legend"]
        )
    );
    let _ = writeln!(body, "\n-- Lisa units --");
    let activatable = dbus_activatable_units();
    for unit in UNITS {
        // `is-active` alone hides "not installed", which is the answer
        // half the time and the one that explains the rest.
        let sys = capture("systemctl", &["is-active", unit]);
        let usr = capture("systemctl", &["--user", "is-active", unit]);
        let state = if sys != "inactive" && !sys.starts_with('(') {
            format!("system:{sys}")
        } else if usr != "inactive" && !usr.starts_with('(') {
            format!("user:{usr}")
        } else if activatable.contains(*unit) {
            // Idle is the CORRECT state for these — the bus starts them
            // on the first call and they exit when done.
            // xdg-desktop-portal-lisa read "not running" in every report
            // while being perfectly healthy, which is indistinguishable
            // from the daemon that genuinely died.
            "idle (D-Bus activated)".into()
        } else {
            "not running".into()
        };
        let _ = writeln!(body, "{unit:<36} {state}");
    }
    Section {
        title: "Services".into(),
        body,
    }
}

fn journal_section(lines: usize) -> Section {
    let mut args: Vec<String> = vec![
        "--no-pager".into(),
        "-b".into(),
        "-n".into(),
        lines.to_string(),
        "-p".into(),
        // warning and worse. An info-level dump is where prompts and
        // URLs live, and it is not what a fault looks like.
        "warning".into(),
    ];
    for unit in UNITS {
        args.push("-u".into());
        args.push((*unit).into());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut body = capture("journalctl", &refs);
    if body.trim().is_empty() || body == "(nothing)" {
        body = "(no warnings or errors from the Lisa units this boot)".into();
    }
    Section {
        title: format!("Journal — Lisa units, warning and worse, last {lines}"),
        body,
    }
}

fn ledger_section(include_previews: bool) -> Section {
    let mut body = String::new();
    match lisa_ledger::Ledger::open(lisa_ledger::Ledger::default_path()) {
        Ok(ledger) => {
            let _ = writeln!(
                body,
                "entries: {}",
                ledger.count().map(|c| c.to_string()).unwrap_or("?".into())
            );
            match ledger.tail(25) {
                Ok(entries) => {
                    for e in entries {
                        // The preview field is prompt text. It is the
                        // single most useful thing in the Ledger for
                        // debugging and the single most private, so it
                        // is opt-in and says so when withheld.
                        let preview = if include_previews {
                            redact(&e.preview)
                        } else if e.preview.is_empty() {
                            String::new()
                        } else {
                            format!("«{} chars withheld»", e.preview.len())
                        };
                        let _ = writeln!(
                            body,
                            "{:<24} {:<10} {:<28} {}",
                            e.kind, e.status, e.app_id, preview
                        );
                    }
                }
                Err(e) => {
                    let _ = writeln!(body, "(tail failed: {e})");
                }
            }
        }
        Err(e) => {
            let _ = writeln!(body, "(ledger unavailable: {e})");
        }
    }
    if !include_previews {
        let _ = writeln!(
            body,
            "\nPrompt previews withheld. `--include-previews` adds them — \
             read what you are about to share first."
        );
    }
    Section {
        title: "Ledger".into(),
        body,
    }
}

fn storage_section() -> Section {
    Section {
        title: "Storage".into(),
        body: capture("df", &["-h", "/", "/var", "/home"]),
    }
}

fn desktop_section() -> Section {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "{:<16} {}",
        "gnome-shell",
        capture("gnome-shell", &["--version"])
    );
    // NOT $XDG_SESSION_TYPE. Over SSH that is `tty`, which describes
    // the shell running this command and says nothing about the desktop
    // — and running `lisa doctor` over SSH is the whole point. Ask
    // logind what sessions exist instead.
    let sessions = capture("loginctl", &["list-sessions", "--no-legend"]);
    let graphical = sessions.lines().filter(|l| l.contains("seat")).count();
    let _ = writeln!(
        body,
        "{:<16} {} seated session(s); this shell is {}",
        "session",
        graphical,
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "(unset)".into())
    );
    let _ = writeln!(body, "\n-- enabled extensions --");
    let _ = writeln!(
        body,
        "{}",
        capture("gnome-extensions", &["list", "--enabled"])
    );
    Section {
        title: "Desktop".into(),
        body,
    }
}

/// Assemble the whole report.
pub fn report(include_previews: bool, journal_lines: usize) -> Vec<Section> {
    vec![
        os_section(),
        units_section(),
        storage_section(),
        desktop_section(),
        journal_section(journal_lines),
        ledger_section(include_previews),
    ]
}

pub fn render(sections: &[Section]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "lisa doctor — {}", crate::version_line());
    for s in sections {
        let _ = writeln!(out, "\n=== {} ===", s.title);
        let _ = writeln!(out, "{}", s.body.trim_end());
    }
    out
}

pub fn run(
    bundle: Option<PathBuf>,
    include_previews: bool,
    journal_lines: usize,
) -> anyhow::Result<()> {
    let text = render(&report(include_previews, journal_lines));
    match bundle {
        None => print!("{text}"),
        Some(path) => {
            let path = if path.as_os_str().is_empty() {
                default_bundle_path()
            } else {
                path
            };
            write_bundle(&path, &text)?;
            println!("wrote {}", path.display());
            println!(
                "Read it before you share it — it is redacted, not anonymous, \
                 and it describes your machine."
            );
        }
    }
    Ok(())
}

fn default_bundle_path() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("lisa-doctor-{stamp}.txt"))
}

/// 0600, because of what is in it.
fn write_bundle(path: &Path, text: &str) -> anyhow::Result<()> {
    use std::io::Write as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundle is pasted into issues. A key in one is a key on the
    /// internet, so this is the test that matters most in the module.
    #[test]
    fn known_credential_shapes_never_survive() {
        let cases = [
            "OPENAI_API_KEY=sk-proj-abcdef1234567890",
            "authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig",
            "x-api-key: sk-ant-api03-secretvalue",
            "using token ghp_16CharactersLongToken00",
            "password=hunter2",
            "client_secret: 9d1c250a-e61b-44d9-88ed-5944d1962f5e",
            "code_verifier=dBjftJeZ4CVP-mB92K27uhbUJU1p1r",
            "cookie: session=abc123def456",
            "AIzaSyD-ExampleGoogleApiKeyValue",
            "hf_ExampleHuggingFaceTokenValue",
        ];
        for line in cases {
            let out = redact(line);
            for leaked in [
                "sk-proj-abcdef1234567890",
                "eyJhbGciOiJIUzI1NiJ9.payload.sig",
                "sk-ant-api03-secretvalue",
                "ghp_16CharactersLongToken00",
                "hunter2",
                "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
                "dBjftJeZ4CVP-mB92K27uhbUJU1p1r",
                "abc123def456",
                "AIzaSyD-ExampleGoogleApiKeyValue",
                "hf_ExampleHuggingFaceTokenValue",
            ] {
                assert!(
                    !out.contains(leaked),
                    "redact({line:?}) leaked {leaked:?} — got {out:?}"
                );
            }
            assert!(
                out.contains("«redacted»"),
                "nothing was redacted in {out:?}"
            );
        }
    }

    /// Issue #109's exact shape: a password inside a provider URL. Those
    /// rows are already in Ledgers written before that fix, so a
    /// diagnostic will meet them.
    #[test]
    fn a_credential_inside_a_url_is_removed_and_the_host_survives() {
        let out = redact("endpoint=https://alice:hunter2@llm.corp.example/v1 status=ok");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("alice"), "{out}");
        // The host is the diagnostic value — losing it would make the
        // redaction useless rather than safe.
        assert!(out.contains("llm.corp.example/v1"), "{out}");
        assert!(out.contains("status=ok"), "{out}");
    }

    /// Issue #141: the iMac staged 20260727.43, rebooted, and came up on
    /// 20260726.34 with nothing failing and nothing logged. A machine
    /// that silently stops updating looks exactly like one that works,
    /// which is why the pin is worth reading directly.
    #[test]
    fn a_literal_default_is_a_pin() {
        assert_eq!(
            pinned_default("timeout 3\ndefault lisa_20260726.34.efi\n", None).as_deref(),
            Some("lisa_20260726.34.efi")
        );
        // bootctl set-default writes the EFI variable, and it wins over
        // loader.conf — so a machine can be pinned with a clean conf.
        assert_eq!(
            pinned_default("timeout 3\n", Some("lisa_20260726.34.efi")).as_deref(),
            Some("lisa_20260726.34.efi")
        );
    }

    /// The check is only useful if it stays quiet on healthy machines.
    /// A warning every user sees on every run is a warning nobody reads.
    #[test]
    fn the_shapes_that_are_not_pins_stay_quiet() {
        // @saved follows whatever booted last — exactly how sd-boot is
        // meant to track an A/B swap.
        assert_eq!(pinned_default("default @saved\n", None), None);
        assert_eq!(pinned_default("", Some("@saved")), None);
        // A pattern still resolves to the newest match.
        assert_eq!(pinned_default("default lisa_*.efi\n", None), None);
        assert_eq!(pinned_default("default lisa_?.efi\n", None), None);
        // Nothing configured at all.
        assert_eq!(pinned_default("timeout 3\nconsole-mode keep\n", None), None);
        assert_eq!(pinned_default("", None), None);
        // …which is the shape the reference iMac actually had once the
        // pin was cleared: comments and no default line.
        assert_eq!(
            pinned_default("#timeout 3\n#console-mode keep\n", None),
            None
        );
    }

    #[test]
    fn a_commented_or_differently_named_key_is_not_a_default() {
        // The pin is load-bearing; misreading a comment as one would
        // tell a healthy machine it can never update again.
        assert_eq!(
            pinned_default("#default lisa_20260726.34.efi\n", None),
            None
        );
        // `defaults` and `default-x` are different keys entirely.
        assert_eq!(pinned_default("defaults lisa_1.efi\n", None), None);
        assert_eq!(pinned_default("default-entry lisa_1.efi\n", None), None);
        // An empty value is not a pin either.
        assert_eq!(pinned_default("default   \n", None), None);
    }

    /// A redactor that eats everything is not safe, it is useless: the
    /// point is a readable diagnostic.
    #[test]
    fn ordinary_diagnostic_lines_come_through_intact() {
        for line in [
            "lisa-inferenced.service            system:active",
            "IMAGE_VERSION  20260729.54",
            "Failed to allocate puller: Operation not supported",
            "https://api.openai.com/v1 reachable",
            "GET /v1/models 200 in 412ms",
            "kernel 7.1.5-arch1-2",
        ] {
            assert_eq!(redact(line), line, "an ordinary line was mangled");
        }
    }

    /// A `key: value` split must not fire on a URL scheme, or every
    /// endpoint in the report becomes unreadable.
    #[test]
    fn a_url_scheme_is_not_mistaken_for_a_secret_key() {
        assert_eq!(
            redact("endpoint https://api.anthropic.com"),
            "endpoint https://api.anthropic.com"
        );
        // …and an '@' in a path is not userinfo.
        assert_eq!(
            redact("https://example.com/users/@alice"),
            "https://example.com/users/@alice"
        );
    }

    /// The home directory is a real name and a real layout. Neither
    /// belongs in something pasted into a public issue.
    #[test]
    fn the_home_path_becomes_a_tilde() {
        // SAFETY: single-threaded test process; restored below.
        let prev = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", "/home/flakerimi") };
        let out = redact("staged /home/flakerimi/.local/share/lisa/ledger.db");
        assert_eq!(out, "staged ~/.local/share/lisa/ledger.db");
        match prev {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    /// Every line of a block, not just the first.
    #[test]
    fn redaction_applies_to_every_line_of_a_block() {
        let block = "fine line one\nOPENAI_API_KEY=sk-proj-11112222333344445555\nfine line two";
        let out = redact_block(block);
        assert!(!out.contains("sk-proj-1111"), "{out}");
        assert_eq!(out.lines().count(), 3);
        assert_eq!(out.lines().next().unwrap(), "fine line one");
    }

    /// Prompt text is withheld by default. A diagnostic that quietly
    /// includes what the user typed is the failure this tool must not
    /// have.
    #[test]
    fn previews_are_withheld_unless_asked_for() {
        let withheld = ledger_section(false);
        assert!(withheld.body.contains("withheld"), "{}", withheld.body);
    }

    /// A missing tool is a fact worth reporting, not a crash.
    #[test]
    fn an_absent_program_is_reported_rather_than_fatal() {
        let out = capture("definitely-not-a-real-program-xyz", &["--help"]);
        assert!(out.contains("not installed here"), "{out}");
    }
}
