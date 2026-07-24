//! lisa-notes — the Notes app's MCP server, the first real tool on the
//! Agent Bus (ADR-0013). agentd's `McpDispatcher` connects to
//! `<socket_dir>/app.lisaos.notes.sock` and speaks newline-delimited
//! JSON-RPC 2.0; notes live in SQLite under the user's XDG data dir.

mod server;
mod storage;

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const APP_ID: &str = "app.lisaos.notes";
/// Must match `mcp_bus::DEFAULT_SOCKET_DIR`.
const DEFAULT_SOCKET_DIR: &str = "/run/lisa/mcp";

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
        assert_eq!(tools.len(), 4);
        for tool in tools {
            assert_eq!(
                tool["input_schema"]["type"], "object",
                "input_schema must be an object schema"
            );
        }

        let by_name = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
        assert_eq!(by_name("list_notes")["tier"], "read");
        assert_eq!(by_name("create_note")["tier"], "write");
        assert_eq!(by_name("create_note")["undo"]["tool"], "delete_note");
        assert_eq!(by_name("create_note")["undo"]["map"]["id"], "$result.id");
        assert_eq!(by_name("delete_note")["undo"]["tool"], "restore_note");
        assert_eq!(by_name("delete_note")["undo"]["map"]["id"], "$input.id");
    }
}
