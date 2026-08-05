// The Agent Bus socket — I/O half. The protocol and the provenance tag
// live in mcp-protocol.js, pure and tested. Deliberately the same shape
// as apps/surfer/lib/mcp.js so the two read side by side; if this
// pattern needs changing it needs changing in both, and a reader should
// be able to see that at a glance.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {APP_ID, handleRequest} from './mcp-protocol.js';

function socketDir() {
    const rt = GLib.getenv('XDG_RUNTIME_DIR');
    return rt ? `${rt}/lisa/mcp` : '/run/lisa/mcp';
}

/// Owns <runtime>/lisa/mcp/app.lisaos.Preview.sock while the app runs.
/// mcp-bus defers socket activation, so socket presence IS tool
/// availability — which is why stop() unlinks rather than leaving a
/// dead socket for callers to time out against.
export class McpServer {
    constructor(handlers) {
        this._tools = {
            read_document: async () => await handlers.readDocument(),
            add_note: async (args) => await handlers.addNote(args),
            highlight: async (args) => await handlers.highlight(args),
            move_page: async (args) => await handlers.movePage(args),
            rotate_page: async (args) => await handlers.rotatePage(args),
            remove_page: async (args) => await handlers.removePage(args),
            export_page: async (args) => await handlers.exportPage(args),
        };
        this._service = null;
        this._path = null;
    }

    start() {
        const dir = socketDir();
        GLib.mkdir_with_parents(dir, 0o700);
        this._path = `${dir}/${APP_ID}.sock`;
        // A stale socket from a crash blocks bind. Verified on the
        // device that Surfer leaves one behind on SIGTERM and restarts
        // fine because of exactly this line — so it is load-bearing,
        // not defensive decoration.
        GLib.unlink(this._path);
        this._service = new Gio.SocketService();
        this._service.add_address(
            Gio.UnixSocketAddress.new(this._path),
            Gio.SocketType.STREAM, Gio.SocketProtocol.DEFAULT, null);
        this._service.connect('incoming', (_s, conn) => {
            this._serve(conn).catch(e => logError(e, 'lisa-preview mcp'));
            return true;
        });
        this._service.start();
        log(`lisa-preview: agent tools on ${this._path}`);
    }

    /// Stop listening and remove the socket file.
    ///
    /// Idempotent: the app calls this from more than one exit path
    /// (#219), and unlinking a path a NEW instance has since bound
    /// would take a live socket away from it.
    stop() {
        if (!this._service) return;
        this._service.stop();
        this._service.close();
        this._service = null;
        if (this._path) GLib.unlink(this._path);
        this._path = null;
    }

    async _serve(conn) {
        const input = new Gio.DataInputStream({base_stream: conn.get_input_stream()});
        const output = conn.get_output_stream();
        for (;;) {
            const [line] = await new Promise((res, rej) =>
                input.read_line_async(GLib.PRIORITY_DEFAULT, null, (s, r) => {
                    try { res(s.read_line_finish_utf8(r)); } catch (e) { rej(e); }
                }));
            if (line === null) break;
            let req = null;
            try { req = JSON.parse(line); } catch { /* handled by handleRequest */ }
            const reply = await handleRequest(req, this._tools);
            if (reply !== null)
                output.write_all(`${JSON.stringify(reply)}\n`, null);
        }
        conn.close(null);
    }
}
