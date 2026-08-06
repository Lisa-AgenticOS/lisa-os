//! lisa-notes — the Notes app's MCP server, the first real tool on the
//! Agent Bus (ADR-0013). agentd's `McpDispatcher` connects to
//! `<socket_dir>/app.lisaos.notes.sock` and speaks newline-delimited
//! JSON-RPC 2.0; notes live in SQLite under the user's XDG data dir.

mod server;
mod storage;

use std::ffi::CString;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicPtr, Ordering};

const APP_ID: &str = "app.lisaos.notes";
/// Must match `mcp_bus::DEFAULT_SOCKET_DIR`.
const DEFAULT_SOCKET_DIR: &str = "/run/lisa/mcp";

/// The bound socket's path, for the signal handler to unlink (#219).
///
/// A raw pointer in an atomic rather than a `OnceLock<CString>`,
/// because a signal handler may only call async-signal-safe things: an
/// atomic load is one, and taking a lock or allocating is not. The
/// `CString` is leaked on purpose — it must outlive every path out of
/// this process, including the ones that do not unwind.
static SOCKET_PATH: AtomicPtr<libc::c_char> = AtomicPtr::new(std::ptr::null_mut());

/// Unlink the socket and leave, from inside a signal handler.
///
/// `unlink` and `_exit` are both on POSIX's async-signal-safe list;
/// nothing else here allocates, formats or locks. `_exit` rather than a
/// flag the accept loop polls, because the loop is parked in `accept()`
/// and would not look at a flag until the next connection — which, for
/// a socket we are trying to give back, may be never.
extern "C" fn release_and_exit(sig: libc::c_int) {
    let path = SOCKET_PATH.load(Ordering::SeqCst);
    if !path.is_null() {
        // SAFETY: `path` is a leaked, NUL-terminated C string that is
        // never freed and never rewritten after it is first stored.
        unsafe { libc::unlink(path) };
    }
    // SAFETY: `_exit` is always safe to call; it does not unwind.
    unsafe { libc::_exit(128 + sig) }
}

/// Arrange for `socket` to be removed however this process ends.
///
/// It used not to be removed at all on the way OUT — only cleared on
/// the way in, by the next start. So a killed server (systemd stopping
/// the unit, a logout, `pkill`) left the socket file sitting in the
/// runtime directory, and `mcp_bus` defers socket activation: it reads
/// socket PRESENCE as tool availability. agentd therefore went on
/// offering `search_notes` and `create_note` to the model and got
/// ECONNREFUSED on every dispatch, instead of "Notes is not running"
/// (#219). Found in exactly that state on the reference machine, for
/// Surfer and Preview.
fn release_socket_on_exit(socket: &Path) {
    let Ok(c_path) = CString::new(socket.as_os_str().as_encoded_bytes()) else {
        return; // a path with an interior NUL cannot be a real socket
    };
    SOCKET_PATH.store(c_path.into_raw(), Ordering::SeqCst);
    // SAFETY: installing a handler that only unlinks and `_exit`s.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = release_and_exit as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = 0;
        // The three that end a process without giving it a chance to
        // return from `main`. SIGKILL is deliberately absent: it cannot
        // be caught, which is why the next start still clears a stale
        // socket as well.
        for sig in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
            libc::sigaction(sig, &action, std::ptr::null_mut());
        }
    }
}

fn main() -> ExitCode {
    let socket = match socket_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("lisa-notes: {e}");
            return ExitCode::from(2);
        }
    };
    match run(&socket) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lisa-notes: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(socket: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let db = db_path().ok_or("no XDG_DATA_HOME or HOME set; cannot locate notes.db")?;
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = storage::Store::open(&db)?;

    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket.exists() {
        std::fs::remove_file(socket)?; // stale socket from a previous run
    }
    let listener = UnixListener::bind(socket)?;
    // Registered after the bind, so a failed bind never arms a handler
    // that would unlink somebody else's live socket.
    release_socket_on_exit(socket);
    eprintln!(
        "lisa-notes: listening on {}, db {}",
        socket.display(),
        db.display()
    );
    server::serve(listener, &store);
    Ok(())
}

/// `--socket <path>` (or `--socket=<path>`); default
/// `$LISA_MCP_DIR/app.lisaos.notes.sock` with `/run/lisa/mcp` as the dir.
fn socket_path() -> Result<PathBuf, String> {
    let mut socket = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            println!("usage: lisa-notes [--socket <path>]");
            std::process::exit(0);
        } else if arg == "--socket" {
            let value = args.next().ok_or("--socket needs a path")?;
            socket = Some(PathBuf::from(value));
        } else if let Some(value) = arg.strip_prefix("--socket=") {
            socket = Some(PathBuf::from(value));
        } else {
            return Err(format!("unknown argument {arg:?} (try --help)"));
        }
    }
    Ok(socket.unwrap_or_else(default_socket))
}

fn default_socket() -> PathBuf {
    // Same resolution order as agentd's dispatcher (LISA_MCP_DIR →
    // $XDG_RUNTIME_DIR/lisa/mcp → system default), so the server binds
    // exactly where the bus will look.
    let dir = std::env::var_os("LISA_MCP_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|r| PathBuf::from(r).join("lisa/mcp")))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_DIR));
    dir.join(format!("{APP_ID}.sock"))
}

/// `$XDG_DATA_HOME/lisa/notes.db`, falling back to
/// `~/.local/share/lisa/notes.db` (same rule as agentd's manifest dirs).
fn db_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("lisa/notes.db"))
}

#[cfg(test)]
mod tests {
    /// The shipped manifest must satisfy agentd's `Manifest::validate`
    /// rules (object schemas, declared undo tools, $input/$result refs).
    #[test]
    fn manifest_is_well_formed() {
        let m: serde_json::Value =
            serde_json::from_str(include_str!("../app.lisaos.notes.json")).unwrap();
        assert_eq!(m["lisa_manifest"], 1);
        assert_eq!(m["app_id"], "app.lisaos.notes");
        assert_eq!(m["mcp"]["transport"], "unix");
        assert_eq!(m["mcp"]["activatable"], false);

        let tools = m["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 7);
        for tool in tools {
            assert_eq!(
                tool["input_schema"]["type"], "object",
                "input_schema must be an object schema"
            );
        }

        let by_name = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(by_name("list_notes")["tier"], "read");
        // read_note is a READ that returns a body — the one thing the
        // surface could store and search but never hand back.
        assert_eq!(by_name("read_note")["tier"], "read");
        // An overwrite is the one write that had no way back: create is
        // undone by delete, delete by restore, and update by another
        // update carrying what the note held before.
        assert_eq!(by_name("update_note")["tier"], "write");
        assert_eq!(by_name("update_note")["undo"]["tool"], "update_note");
        assert_eq!(
            by_name("update_note")["undo"]["map"]["title"],
            "$result.previous_title"
        );
        assert_eq!(
            by_name("update_note")["undo"]["map"]["body"],
            "$result.previous_body"
        );
        assert!(
            by_name("read_note").get("undo").is_none(),
            "reading is a read: nothing to undo"
        );
        assert_eq!(
            by_name("read_note")["input_schema"]["required"],
            serde_json::json!(["id"])
        );
        assert_eq!(by_name("search_notes")["tier"], "read");
        assert!(
            by_name("search_notes").get("undo").is_none(),
            "search is a read: nothing to undo"
        );
        assert_eq!(
            by_name("search_notes")["input_schema"]["required"],
            serde_json::json!(["query"])
        );
        assert_eq!(by_name("create_note")["tier"], "write");
        assert_eq!(by_name("create_note")["undo"]["tool"], "delete_note");
        assert_eq!(by_name("create_note")["undo"]["map"]["id"], "$result.id");
        assert_eq!(by_name("delete_note")["undo"]["tool"], "restore_note");
        assert_eq!(by_name("delete_note")["undo"]["map"]["id"], "$input.id");
    }
}
