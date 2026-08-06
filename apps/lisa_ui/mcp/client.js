// An MCP client over a unix socket, for a GJS app talking to its own
// backend (ADR-0056, ADR-0047 §6).
//
// # Why an app talks to itself through MCP
//
// Notes has been an MCP server with no window since ADR-0013. Its
// SQLite store, its tools and its tests all exist; only the GUI was
// missing. The GUI could have opened that database directly — and then
// there would be two writers, two schemas to keep in step, and two
// answers to "what notes are there", one of which the agent cannot see.
//
// Instead the window is another CLIENT of the same tool surface. What
// the person sees and what the model sees are the same list by
// construction, because they came down the same socket through the same
// five tools. That is "apps are agent surfaces" as a fact about the
// code rather than a slogan: the agent surface is not a bolt-on beside
// the app, it IS the app's API, and the window is its first consumer.
//
// # What this is not
//
// Not a bus, not a dispatcher, not a replacement for `libs/mcp-bus` —
// that is agentd's Rust client for talking to apps. This is the small
// counterpart for a GJS window talking to its own daemon.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

const decoder = new TextDecoder();
const encoder = new TextEncoder();

/// Where an app's MCP socket lives.
///
/// `XDG_RUNTIME_DIR/lisa/mcp` first, matching what lisa-notes uses on a
/// desktop; `/run/lisa/mcp` is the system default from its README.
export function socketPath(appId) {
    const run = GLib.getenv('XDG_RUNTIME_DIR');
    const dir = run ? `${run}/lisa/mcp` : '/run/lisa/mcp';
    return `${dir}/${appId}.sock`;
}

/// A client for one app's MCP socket.
///
/// Deliberately connect-per-call rather than one long-lived socket. A
/// window that holds a socket open has to handle the daemon restarting
/// under it, and a note-taking app is not a hot path — the simpler
/// thing that cannot get into a wedged state is worth more here than
/// the round trip it saves.
export class McpClient {
    constructor(appId) {
        this.appId = appId;
        this.path = socketPath(appId);
        this._id = 0;
    }

    /// Is the backend there at all? A window that opens onto a silent
    /// error is worse than one that says the daemon is not running.
    isAvailable() {
        return GLib.file_test(this.path, GLib.FileTest.EXISTS);
    }

    /// Call a tool. Returns the parsed payload.
    ///
    /// Throws with the tool's own message when the server answers
    /// `isError`, so a caller sees "note 7 does not exist" rather than
    /// a generic failure — the reason the error path is a RESULT and
    /// not a protocol error (see ../mcp/protocol.js).
    async call(name, args = {}) {
        const client = new Gio.SocketClient();
        const addr = new Gio.UnixSocketAddress({path: this.path});
        const conn = await new Promise((resolve, reject) => {
            client.connect_async(addr, null, (src, res) => {
                try { resolve(src.connect_finish(res)); } catch (e) { reject(e); }
            });
        });

        const req = {
            jsonrpc: '2.0',
            id: ++this._id,
            method: 'tools/call',
            params: {name, arguments: args},
        };
        const out = conn.get_output_stream();
        await new Promise((resolve, reject) => {
            out.write_all_async(encoder.encode(`${JSON.stringify(req)}\n`),
                GLib.PRIORITY_DEFAULT, null, (src, res) => {
                    try { src.write_all_finish(res); resolve(); } catch (e) { reject(e); }
                });
        });

        // Newline-delimited JSON-RPC (libs/mcp-bus): read to the first
        // newline, not to EOF — the server keeps the connection open.
        const input = new Gio.DataInputStream({base_stream: conn.get_input_stream()});
        const line = await new Promise((resolve, reject) => {
            input.read_line_async(GLib.PRIORITY_DEFAULT, null, (src, res) => {
                try {
                    const [bytes] = src.read_line_finish(res);
                    resolve(bytes ? decoder.decode(bytes) : '');
                } catch (e) { reject(e); }
            });
        });
        conn.close(null);

        if (!line)
            throw new Error(`${this.appId}: no reply from ${this.path}`);
        const reply = JSON.parse(line);
        if (reply.error)
            throw new Error(reply.error.message ?? 'tool call failed');

        const result = reply.result ?? {};
        const text = result.content?.[0]?.text;
        if (result.isError)
            throw new Error(text ?? 'tool call failed');
        if (text === undefined)
            return result;
        try {
            return JSON.parse(text);
        } catch {
            // A tool that answers with prose rather than JSON is
            // answering; hand it back rather than failing on it.
            return text;
        }
    }
}
