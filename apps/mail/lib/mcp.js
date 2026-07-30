// The Agent Bus socket (issue #146 Phase 3) — I/O half; the protocol
// (and the provenance tag) live in mcp-protocol.js, pure and tested.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {APP_ID, handleRequest} from './mcp-protocol.js';

function socketDir() {
    const rt = GLib.getenv('XDG_RUNTIME_DIR');
    return rt ? `${rt}/lisa/mcp` : '/run/lisa/mcp';
}

/// The socket server. Owns <runtime>/lisa/mcp/app.lisaos.Mail.sock
/// for exactly as long as the app runs — mcp-bus defers socket
/// activation, so socket presence IS tool availability, and stop()
/// removes it rather than leaving a dead socket that times out callers.
export class McpServer {
    constructor(handlers) {
        // Wire tool names → app handlers, normalising each result.
        this._tools = {
            search_mail: async (args) => await handlers.searchMail(args ?? {}),
            read_message: async (args) => await handlers.readMessage(args ?? {}),
        };
        this._service = null;
        this._path = null;
    }

    start() {
        const dir = socketDir();
        GLib.mkdir_with_parents(dir, 0o700);
        this._path = `${dir}/${APP_ID}.sock`;
        GLib.unlink(this._path); // a stale socket from a crash blocks bind
        this._service = new Gio.SocketService();
        this._service.add_address(
            Gio.UnixSocketAddress.new(this._path),
            Gio.SocketType.STREAM, Gio.SocketProtocol.DEFAULT, null);
        this._service.connect('incoming', (_s, conn) => {
            this._serve(conn).catch(e => logError(e, 'lisa-mail mcp'));
            return true;
        });
        this._service.start();
        log(`lisa-mail: agent tools on ${this._path}`);
    }

    stop() {
        this._service?.stop();
        if (this._path) GLib.unlink(this._path);
    }

    async _serve(conn) {
        const input = new Gio.DataInputStream({base_stream: conn.get_input_stream()});
        const output = conn.get_output_stream();
        for (;;) {
            const [line] = await new Promise((res, rej) =>
                input.read_line_async(GLib.PRIORITY_DEFAULT, null, (s, r) => {
                    try { res(s.read_line_finish_utf8(r)); } catch (e) { rej(e); }
                }));
            if (line === null) break; // peer closed
            let req = null;
            try { req = JSON.parse(line); } catch { /* handled below */ }
            const reply = await handleRequest(req, this._tools);
            if (reply !== null)
                output.write_all(`${JSON.stringify(reply)}\n`, null);
        }
        conn.close(null);
    }
}
