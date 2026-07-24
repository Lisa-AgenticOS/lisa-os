//! MCP transport for the Agent Bus (`docs/PLAN.md` §5.4, ADR-0009,
//! ADR-0013): a per-app unix-socket MCP client and the [`McpDispatcher`]
//! that agentd swaps in for its `NullDispatcher` placeholder.
//!
//! Wire protocol: newline-delimited JSON-RPC 2.0 over a per-app unix
//! domain socket at `<base_dir>/<app_id>.sock`. One short-lived
//! connection per dispatch — `initialize`, `notifications/initialized`,
//! then `tools/call`. Socket activation (`mcp.activatable`, spawn on
//! demand) is deliberately deferred: the app's socket must already be
//! live for a dispatch to succeed.

mod client;
mod dispatcher;

pub use client::{McpClient, McpError, PROTOCOL_VERSION};
pub use dispatcher::{DEFAULT_TIMEOUT, Dispatcher, McpDispatcher};

/// Default directory holding per-app MCP sockets.
pub const DEFAULT_SOCKET_DIR: &str = "/run/lisa/mcp";
