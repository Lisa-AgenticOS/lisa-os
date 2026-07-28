#!/usr/bin/env -S gjs -m
// lisa-consentd — the desktop consent surface, and nothing else
// (issue #145, ADR-0035 §4, PLAN §5.10).
//
// WHY THIS PROCESS EXISTS
//
// Until now the confirmation dialog lived in lisa-overlayd, which also
// hosts the model. So when the overlay routed a prompt to a privileged
// tool, the process that ASKED for the call was also the process that
// ANSWERED for the human: `_respond()` called Agent1.Confirm on the same
// connection that had made the RequestCall. agentd saw one peer, one
// unique name, and could not tell the difference — the model was
// approving itself, with a dialog drawn in between as decoration.
//
// This is the pattern xdg-desktop-portal uses and the reason it uses it:
// the thing that grants a capability must not be the thing that wants it.
//
// WHAT THIS PROCESS MUST NEVER GROW
//
// No model. No prompt entry. No tool calls of its own. Its only inputs
// are agentd's ConfirmationRequested signal and a human's click, and its
// only output is Agent1.Confirm. The moment it can be driven by
// generated text it stops being a second pair of eyes (ADR-0030:
// anything reachable from inside is not a guardrail).
//
// It deliberately does NOT expose a D-Bus method to approve a call. A
// peer that could ask this daemon to approve something would be able to
// launder its own request through it, which is exactly the hole being
// closed. The only approver is the pointer.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';
import Adw from 'gi://Adw?version=1';

const CONSENT_BUS_NAME = 'dev.lisaos.Consent1';
const CONSENT_OBJECT_PATH = '/dev/lisaos/Consent1';

const AGENT_BUS = 'dev.lisaos.Agent1';
const AGENT_PATH = '/dev/lisaos/Agent1';
const AGENT_IFACE = 'dev.lisaos.Agent1';

/// Read-only. There is no Approve() here on purpose — see the header.
const CONSENT_IFACE_XML = `
<node>
  <interface name="dev.lisaos.Consent1">
    <method name="Ping">
      <arg type="s" direction="out" name="version"/>
    </method>
    <!-- How many confirmations are on screen. For tests and for a
         status line; it reveals a count, never a call's contents. -->
    <method name="PendingCount">
      <arg type="u" direction="out" name="count"/>
    </method>
  </interface>
</node>`;

/// agentd's confirmation spec is JSON. Render only fields we recognise:
/// an unknown key must not reach the dialog as trusted text, because the
/// spec is assembled from a tool's own manifest and arguments, and those
/// come from an app.
function describe(specJson) {
    let spec;
    try {
        spec = JSON.parse(specJson);
    } catch {
        return {title: 'Confirm action', body: 'A privileged action is waiting.'};
    }
    const app = typeof spec.app_id === 'string' ? spec.app_id : 'an app';
    const tool = typeof spec.tool === 'string' ? spec.tool : 'an action';
    const tier = typeof spec.tier === 'string' ? spec.tier : '';
    // Arguments are shown as compact JSON rather than prose: a sentence
    // built from attacker-influenced values reads as if Lisa is
    // recommending it. A code block reads as data, which is what it is.
    let args = '';
    if (spec.args !== undefined) {
        try {
            args = JSON.stringify(spec.args);
        } catch {
            args = '';
        }
        if (args.length > 400)
            args = `${args.slice(0, 400)}…`;
    }
    const title = tier === 'destructive'
        ? 'Allow this destructive action?'
        : 'Allow this action?';
    return {
        title,
        body: `${app} wants to run ${tool}.`,
        args,
        escalated: spec.escalated === true,
    };
}

class ConsentSurface {
    constructor(app) {
        this._app = app;
        this._open = new Map(); // call_id -> Adw.Window
        this._bus = Gio.DBus.session;

        this._impl = Gio.DBusExportedObject.wrapJSObject(CONSENT_IFACE_XML, this);
        this._impl.export(this._bus, CONSENT_OBJECT_PATH);

        this._subId = this._bus.signal_subscribe(
            AGENT_BUS, AGENT_IFACE, 'ConfirmationRequested', AGENT_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _sig, params) => this._onRequested(params));
    }

    Ping() {
        return 'lisa-consentd 0.1';
    }

    PendingCount() {
        return this._open.size;
    }

    _onRequested(params) {
        const [callId, specJson] = params.deepUnpack();
        const id = Number(callId);
        // agentd re-emits on reconnect; one dialog per call.
        if (this._open.has(id))
            return;
        this._prompt(id, describe(specJson));
    }

    _prompt(callId, d) {
        const win = new Adw.Window({
            title: 'Lisa',
            modal: false,
            default_width: 460,
            resizable: false,
        });
        // Hold the app alive while a dialog is up: this daemon has no
        // main window, so without a hold the application would exit the
        // moment it goes idle and the call would expire unanswered.
        this._app.hold();

        const box = new Gtk.Box({
            orientation: Gtk.Orientation.VERTICAL,
            spacing: 12,
            margin_top: 18,
            margin_bottom: 18,
            margin_start: 18,
            margin_end: 18,
        });
        const title = new Gtk.Label({label: d.title, wrap: true, xalign: 0});
        title.add_css_class('title-2');
        box.append(title);
        box.append(new Gtk.Label({label: d.body, wrap: true, xalign: 0}));

        if (d.escalated) {
            // Rule 6: this call was steered by content whose provenance
            // is not trusted. That is the single most important thing on
            // this dialog, so it is not buried in the argument dump.
            const warn = new Gtk.Label({
                label: 'This was suggested by content from outside this machine.',
                wrap: true,
                xalign: 0,
            });
            warn.add_css_class('warning');
            box.append(warn);
        }
        if (d.args) {
            const args = new Gtk.Label({
                label: d.args,
                wrap: true,
                xalign: 0,
                selectable: true,
            });
            args.add_css_class('monospace');
            args.add_css_class('dim-label');
            box.append(args);
        }

        const buttons = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL,
            spacing: 6,
            halign: Gtk.Align.END,
            margin_top: 6,
        });
        const deny = new Gtk.Button({label: 'Deny'});
        const allow = new Gtk.Button({label: 'Allow'});
        allow.add_css_class('destructive-action');
        buttons.append(deny);
        buttons.append(allow);
        box.append(buttons);

        win.set_content(box);
        this._open.set(callId, win);

        const answer = (approve) => {
            if (!this._open.has(callId))
                return;
            this._open.delete(callId);
            win.close();
            this._app.release();
            this._confirm(callId, approve);
        };
        deny.connect('clicked', () => answer(false));
        allow.connect('clicked', () => answer(true));
        // Closing the window is a denial, never a silent nothing: a
        // dismissed dialog must not leave a privileged call parked until
        // its TTL, where it looks to the user like the action is still
        // going to happen.
        win.connect('close-request', () => {
            answer(false);
            return false;
        });
        win.present();
    }

    _confirm(callId, approve) {
        this._bus.call(
            AGENT_BUS, AGENT_PATH, AGENT_IFACE, 'Confirm',
            new GLib.Variant('(tb)', [callId, approve]),
            new GLib.VariantType('(ss)'),
            Gio.DBusCallFlags.NONE, -1, null,
            (conn, res) => {
                try {
                    conn.call_finish(res);
                } catch (e) {
                    // A refused Confirm is worth a log line and nothing
                    // more: the call stays parked and expires, which is
                    // the safe direction.
                    logError(e, `lisa-consentd: Confirm(${callId}) failed`);
                }
            });
    }
}

Adw.init();
const app = new Gtk.Application({
    application_id: 'dev.lisaos.Consent1.App',
    flags: Gio.ApplicationFlags.IS_SERVICE,
});
let surface = null;
app.connect('startup', () => {
    surface = new ConsentSurface(app);
    // No window at startup. The daemon idles until agentd asks.
    app.hold();
});
app.connect('activate', () => {});

Gio.bus_own_name(
    Gio.BusType.SESSION,
    CONSENT_BUS_NAME,
    Gio.BusNameOwnerFlags.NONE,
    null,
    () => log(`lisa-consentd: owning ${CONSENT_BUS_NAME}`),
    () => {
        // Losing the name means another consent surface took it. Exiting
        // is correct: two dialogs for one call is worse than one, and
        // agentd only trusts whoever owns the name.
        logError(new Error(`lost ${CONSENT_BUS_NAME} (another instance running?)`));
        app.quit();
    });

app.run([]);
void surface;
