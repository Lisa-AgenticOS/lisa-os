#!/usr/bin/env -S gjs -m
// lisa-overlayd — headless assistant-overlay backend (PLAN §5.7.1).
//
// Owns org.lisa.Overlay1 on the session bus: state and token streams
// live here so frontends stay thin — the GNOME Shell extension and the
// wlr-layer-shell client (Track L) both just render this service.
//
// Per Ask(), the prompt first tries the Agent Bus lane (M5, ADR-0013):
//   0. [action] → org.lisa.Agent1.Discover(prompt) scored by
//      lib/agent.js (deterministic token overlap, no model in this
//      lane); a confident, arg-fillable hit becomes RequestCall with
//      actor "overlay", provenance ["user"]. "executed"/"failed"/
//      "denied" stream back as Token + Finished; a parked call raises
//      ConfirmationNeeded and waits for Respond(). Confirmations
//      parked by other clients surface too, via Agent1's
//      ConfirmationRequested signal.
// Otherwise the inference lane runs unchanged:
//   1. [my stuff] → `lisa context search` (Context Fabric, PLAN §5.3;
//      retrieval is ledgered by the CLI) → provenance-fenced envelope
//      (Appendix C via lib/envelope.js).
//   2. org.lisa.Inference1.OpenSession → (session path, token fd);
//      Session.Generate; tokens are read off the fd and re-emitted as
//      Token signals until EOF ⇒ Finished. Every generation is
//      ledgered by lisa-inferenced (dataflow rule 4).
// [this window] (§5.7.4, M6) and [selection] (§5.7.3 layer 3) are
// reported unavailable in Started meta until their layers land.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {decideAction, formatExecuted, reasonText, safeParse}
    from '../lib/agent.js';
import {buildEnvelope, parseContextHits, classifyAffordances}
    from '../lib/envelope.js';
import {OVERLAY_IFACE_XML, OVERLAY_BUS_NAME, OVERLAY_OBJECT_PATH}
    from '../lib/iface.js';

const INFERENCE_BUS = 'org.lisa.Inference1';
const INFERENCE_PATH = '/org/lisa/Inference1';
const INFERENCE_IFACE = 'org.lisa.Inference1';
const SESSION_IFACE = 'org.lisa.Inference1.Session';
const AGENT_BUS = 'org.lisa.Agent1';
const AGENT_PATH = '/org/lisa/Agent1';
const AGENT_IFACE = 'org.lisa.Agent1';
const AGENT_ACTOR = 'overlay';
const AGENT_PROVENANCE = ['user']; // typed prompts: a trusted chain (rule 6)
const CONTEXT_HITS = 3;

Gio._promisify(Gio.Subprocess.prototype, 'communicate_utf8_async');
Gio._promisify(Gio.InputStream.prototype, 'read_bytes_async');
Gio._promisify(Gio.DBusConnection.prototype, 'call');

class OverlayService {
    constructor(connection) {
        this._connection = connection;
        this._impl = Gio.DBusExportedObject.wrapJSObject(OVERLAY_IFACE_XML, this);
        this._impl.export(connection, OVERLAY_OBJECT_PATH);
        this._nextId = 1;
        this._active = null; // {id, cancellable, sessionPath, pendingCallId}
        this._external = new Map(); // query id → Agent1 call id (other actors' calls)
        // Consent for calls parked by other clients (`lisa do` without
        // a TTY answer, scripts): surface them as first-class overlay
        // queries. Own calls are filtered out by actor — agentd emits
        // this signal before the RequestCall reply arrives, so
        // id-tracking alone would race.
        this._connection.signal_subscribe(
            AGENT_BUS, AGENT_IFACE, 'ConfirmationRequested', AGENT_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (conn, sender, path, iface, signal, params) =>
                this._onConfirmationRequested(params));
    }

    // ---- D-Bus methods -------------------------------------------------

    Ask(prompt, options) {
        const id = this._nextId++;
        const opts = this._unpackOptions(options);
        // Fire and stream; errors surface as Finished("error", detail).
        this._run(id, prompt, opts).catch(e => {
            this._finish(id, 'error', String(e?.message ?? e));
        });
        return id;
    }

    Cancel(queryId) {
        const active = this._active;
        if (!active || active.id !== Number(queryId))
            return;
        active.cancelled = true;
        if (active.pendingCallId != null) {
            // Awaiting consent: cancel answers "deny" and closes the turn.
            const callId = active.pendingCallId;
            active.pendingCallId = null;
            this._agentCall('Confirm',
                new GLib.Variant('(tb)', [callId, false]), '(ss)')
                .catch(() => {});
            this._finish(active.id, 'cancelled', '');
            return;
        }
        active.cancellable.cancel();
        if (active.sessionPath)
            this._sessionCall(active.sessionPath, 'Cancel', null).catch(() => {});
    }

    Respond(queryId, approve) {
        this._respond(Number(queryId), Boolean(approve)).catch(e => {
            // Confirm itself failed (unknown call — answered elsewhere,
            // or expired after Agent1's TTL). Close the turn honestly.
            this._finish(Number(queryId), 'error', String(e?.message ?? e));
        });
    }

    GetStatus() {
        return {
            state: GLib.Variant.new_string(this._active ? 'streaming' : 'idle'),
            active_query: GLib.Variant.new_uint64(this._active?.id ?? 0),
        };
    }

    // ---- internals -----------------------------------------------------

    _unpackOptions(options) {
        const out = {};
        for (const key of ['my_stuff', 'window', 'selection', 'model_hint']) {
            const v = options[key];
            if (v !== undefined)
                out[key] = v instanceof GLib.Variant ? v.recursiveUnpack() : v;
        }
        return out;
    }

    async _run(id, prompt, opts) {
        const cancellable = new Gio.Cancellable();
        this._active = {
            id, cancellable, sessionPath: null, cancelled: false,
            pendingCallId: null,
        };

        const {wanted, unavailable} = classifyAffordances(opts);

        // Agent Bus lane: an actionable prompt becomes an Agent1 tool
        // call; everything else keeps streaming inference below.
        const action = await this._discoverAction(prompt, cancellable);
        if (action) {
            await this._runAgent(id, action, unavailable);
            return;
        }

        let hits = [];
        if (wanted.includes('my_stuff'))
            hits = await this._searchContext(prompt, cancellable);

        this._emit('Started', new GLib.Variant('(ts)', [id, JSON.stringify({
            sources: hits.map(h => ({provenance: h.provenance, source: h.source})),
            unavailable,
        })]));

        const envelope = buildEnvelope(prompt, hits);
        const {sessionPath, stream} = await this._openSession(opts.model_hint);
        this._active.sessionPath = sessionPath;
        this._active.stream = stream;

        try {
            const params = {priority: GLib.Variant.new_string('interactive')};
            await this._sessionCall(sessionPath, 'Generate',
                new GLib.Variant('(sa{sv})', [envelope, params]));

            const decoder = new TextDecoder('utf-8');
            for (;;) {
                const bytes = await stream.read_bytes_async(
                    4096, GLib.PRIORITY_DEFAULT, cancellable);
                if (bytes.get_size() === 0)
                    break; // EOF = end-of-message (§5.1).
                this._emit('Token', new GLib.Variant('(ts)',
                    [id, decoder.decode(bytes.toArray(), {stream: true})]));
            }
            this._finish(id, this._active?.cancelled ? 'cancelled' : 'ok', '');
        } catch (e) {
            if (this._active?.cancelled)
                this._finish(id, 'cancelled', '');
            else
                throw e;
        } finally {
            stream.close(null);
            this._sessionCall(sessionPath, 'Close', null).catch(() => {});
        }
    }

    // ---- Agent Bus lane (org.lisa.Agent1) -------------------------------

    // Prompt → {tool, args, score} or null. Any failure (agentd not on
    // the bus, no confident hit, unfillable args) means the inference
    // lane takes the prompt — inference is the safe default.
    async _discoverAction(prompt, cancellable) {
        try {
            const reply = await this._agentCall('Discover',
                new GLib.Variant('(s)', [prompt]), '(s)', cancellable);
            const [toolsJson] = reply.deepUnpack();
            return decideAction(prompt, safeParse(toolsJson));
        } catch (e) {
            if (!this._active?.cancelled)
                log(`lisa-overlayd: Agent1 unavailable (${e?.message ?? e}); inference lane`);
            return null;
        }
    }

    async _runAgent(id, action, unavailable) {
        const {tool, args} = action;
        this._emit('Started', new GLib.Variant('(ts)', [id, JSON.stringify({
            sources: [], // the tool call is the retrieval; no context envelope
            unavailable,
            mode: 'agent',
            tool: `${tool.app_id}::${tool.name}`,
        })]));
        const options = {
            actor: GLib.Variant.new_string(AGENT_ACTOR),
            provenance: new GLib.Variant('as', AGENT_PROVENANCE),
        };
        const reply = await this._agentCall('RequestCall',
            new GLib.Variant('(sssa{sv})',
                [tool.app_id, tool.name, JSON.stringify(args), options]),
            '(tss)');
        const [callId, disposition, detailJson] = reply.deepUnpack();
        switch (disposition) {
        case 'confirm-chip':
        case 'confirm-modal':
            if (this._active?.id === id) {
                // Parked: Respond()/Cancel() drive it from here.
                this._active.pendingCallId = Number(callId);
                this._emit('ConfirmationNeeded',
                    new GLib.Variant('(ts)', [id, detailJson]));
            } else {
                // Replaced by a newer query mid-flight — deny rather
                // than leak a confirmation that expires unanswered.
                this._agentCall('Confirm',
                    new GLib.Variant('(tb)', [Number(callId), false]), '(ss)')
                    .catch(() => {});
            }
            break;
        case 'executed':
        case 'failed':
        case 'denied':
            this._emitOutcome(id, disposition, detailJson);
            break;
        default:
            this._finish(id, 'error', `unknown Agent1 disposition "${disposition}"`);
        }
    }

    async _respond(queryId, approve) {
        let callId = null;
        const active = this._active;
        if (active?.id === queryId && active.pendingCallId != null) {
            callId = active.pendingCallId;
            active.pendingCallId = null;
        } else if (this._external.has(queryId)) {
            callId = this._external.get(queryId);
            this._external.delete(queryId);
        }
        if (callId === null)
            return;
        const reply = await this._agentCall('Confirm',
            new GLib.Variant('(tb)', [callId, approve]), '(ss)');
        const [status, detailJson] = reply.deepUnpack();
        this._emitOutcome(queryId, status, detailJson);
    }

    // Agent1's ConfirmationRequested: surface calls parked by other
    // actors. Own calls (actor "overlay") arrive through RequestCall's
    // reply instead — the signal fires before that reply, so matching
    // on actor sidesteps the race.
    _onConfirmationRequested(params) {
        const [callId, specJson] = params.deepUnpack();
        if (safeParse(specJson)?.actor === AGENT_ACTOR)
            return;
        const id = this._nextId++;
        this._external.set(id, Number(callId));
        this._emit('Started', new GLib.Variant('(ts)', [id, JSON.stringify({
            sources: [], unavailable: [], mode: 'agent', external: true,
        })]));
        this._emit('ConfirmationNeeded', new GLib.Variant('(ts)', [id, specJson]));
    }

    // Terminal outcome → response text (Token) + Finished. Status is
    // Agent1's vocabulary: "executed" (detail = ledger ref), "failed"
    // and "denied" (detail = reason).
    _emitOutcome(id, status, detailJson) {
        if (status === 'executed') {
            const {text, ledgerRef} = formatExecuted(detailJson);
            this._emit('Token',
                new GLib.Variant('(ts)', [id, text === '' ? 'done' : text]));
            this._finish(id, 'executed', ledgerRef === null ? '' : String(ledgerRef));
        } else {
            const reason = reasonText(detailJson);
            this._emit('Token', new GLib.Variant('(ts)', [id, reason]));
            this._finish(id, status, reason);
        }
    }

    _agentCall(method, params, replyType, cancellable = null) {
        return this._connection.call(
            AGENT_BUS, AGENT_PATH, AGENT_IFACE, method, params,
            replyType === null ? null : new GLib.VariantType(replyType),
            Gio.DBusCallFlags.NONE, -1, cancellable);
    }

    async _searchContext(query, cancellable) {
        try {
            const argv = [this._lisaCli(), 'context', 'search', query,
                '--limit', String(CONTEXT_HITS)];
            const proc = Gio.Subprocess.new(argv,
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
            const [stdout] = await proc.communicate_utf8_async(null, cancellable);
            if (!proc.get_successful())
                return [];
            return parseContextHits(stdout);
        } catch (e) {
            logError(e, 'context search failed; answering without [my stuff]');
            return [];
        }
    }

    _lisaCli() {
        return GLib.getenv('LISA_CLI') ?? 'lisa';
    }

    async _openSession(modelHint) {
        const options = {};
        if (modelHint)
            options.model_hint = GLib.Variant.new_string(modelHint);
        const [ret, fdList] = await new Promise((resolve, reject) => {
            this._connection.call_with_unix_fd_list(
                INFERENCE_BUS, INFERENCE_PATH, INFERENCE_IFACE, 'OpenSession',
                new GLib.Variant('(a{sv})', [options]),
                new GLib.VariantType('(oh)'),
                Gio.DBusCallFlags.NONE, -1, null, null,
                (conn, res) => {
                    try {
                        resolve(conn.call_with_unix_fd_list_finish(res));
                    } catch (e) {
                        reject(e);
                    }
                });
        });
        const [sessionPath, fdIndex] = ret.deepUnpack();
        const fd = fdList.get(fdIndex); // dup'd; the stream owns it now.
        const stream = new Gio.UnixInputStream({fd, close_fd: true});
        return {sessionPath, stream};
    }

    _sessionCall(sessionPath, method, args) {
        return this._connection.call(
            INFERENCE_BUS, sessionPath, SESSION_IFACE, method, args,
            null, Gio.DBusCallFlags.NONE, -1, null);
    }

    _finish(id, status, detail) {
        if (this._active?.id === id)
            this._active = null;
        this._emit('Finished', new GLib.Variant('(tss)', [id, status, detail]));
    }

    _emit(signal, variant) {
        this._connection.emit_signal(null, OVERLAY_OBJECT_PATH,
            OVERLAY_BUS_NAME, signal, variant);
    }
}

const loop = new GLib.MainLoop(null, false);
Gio.bus_own_name(
    Gio.BusType.SESSION,
    OVERLAY_BUS_NAME,
    Gio.BusNameOwnerFlags.NONE,
    connection => new OverlayService(connection),
    () => log(`lisa-overlayd: owning ${OVERLAY_BUS_NAME}`),
    () => {
        logError(new Error(`lost ${OVERLAY_BUS_NAME} (another instance running?)`));
        loop.quit();
    });
loop.run();
