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
    // Lead with the EFFECT when agentd computed one (#251): a person
    // asked to approve `delete_everything` with `{"target":"/"}` is being
    // asked to read a reverse-DNS id, a raw tool name and raw JSON — and
    // the JSON showing the real target is the part nobody reads. The
    // effect sentence is computed from the resolved target, so an
    // innocuous tool name cannot disguise where the call points.
    const effect = typeof spec.effect === 'string' && spec.effect
        ? spec.effect
        : '';
    return {
        title,
        body: effect
            ? `${app} wants to ${effect}.`
            : `${app} wants to run ${tool}.`,
        args,
        escalated: spec.escalated === true,
    };
}

/// A refusal, as agentd reports it (#251). Same discipline as
/// `describe`: only fields we recognise are rendered.
///
/// Note what is NOT read here, and could not be even if agentd sent it:
/// no arguments, no command, no URI. The refusal dialog must have no
/// path that performs, composes or copies the refused action — a
/// copy-to-clipboard or a "fix this" button would be the Allow button
/// rebuilt with extra steps, and the friction IS the safety.
function describeRefusal(reportJson) {
    let report;
    try {
        report = JSON.parse(reportJson);
    } catch {
        return {
            title: 'Refused — this is not something Lisa will do',
            body: 'An action was refused.',
        };
    }
    const app = typeof report.app_id === 'string' ? report.app_id : 'An app';
    const reason = typeof report.reason === 'string' ? report.reason : '';
    const needs = typeof report.needs === 'string' ? report.needs : '';
    const hard = report.kind === 'hard-no';
    return {
        title: hard
            ? 'Refused — this is not something Lisa will do'
            : 'Refused — outside what this run may touch',
        body: `${app} asked to do this, and it was not done.`,
        reason,
        // For an out-of-scope refusal, what WOULD permit it — as a
        // sentence, never as a control. Widening happens in Settings,
        // reached deliberately, because `~/.local/share/lisa/` holds the
        // Ledger and the grants themselves (#252, #253).
        needs: hard ? '' : needs,
        escalated: report.escalated === true,
        // The owner's own capability, stated rather than offered.
        footer: hard
            ? 'If you genuinely want this, do it yourself in a terminal.'
            : '',
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
        // A refusal arrives on its own signal, so this surface cannot
        // mistake one for something to draw an Allow button on. There is
        // no parked call behind it and no Confirm that could answer it.
        this._refusalSubId = this._bus.signal_subscribe(
            AGENT_BUS, AGENT_IFACE, 'RefusalReported', AGENT_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _sig, params) => this._onRefused(params));
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

    _onRefused(params) {
        const [callId, reportJson] = params.deepUnpack();
        const id = Number(callId);
        if (this._open.has(id))
            return;
        this._report(id, describeRefusal(reportJson));
    }

    /// The refusal dialog: it REPORTS. One button, no approving control.
    ///
    /// Modal-ish for the same reason #251 gives: the owner should learn
    /// immediately that outside content tried to destroy their system,
    /// rather than find it in a log. That justification collapses if
    /// these become common — at which point they train dismissal exactly
    /// as Allow dialogs do, and the category was drawn too wide. So the
    /// frequency of this window is a correctness signal for the
    /// catalogue, not just an annoyance.
    _report(callId, d) {
        const win = new Adw.Window({
            title: 'Lisa',
            modal: false,
            default_width: 460,
            resizable: false,
        });
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
        for (const line of [d.reason, d.needs, d.footer]) {
            if (!line)
                continue;
            // Not selectable, unlike the argument dump on the
            // confirmation dialog: there is nothing here to copy, and
            // making the refused target copyable would be the first step
            // back towards handing someone the loaded thing.
            const label = new Gtk.Label({label: line, wrap: true, xalign: 0});
            label.add_css_class('dim-label');
            box.append(label);
        }
        if (d.escalated) {
            const warn = new Gtk.Label({
                label: 'This was suggested by content from outside this machine.',
                wrap: true,
                xalign: 0,
            });
            warn.add_css_class('warning');
            box.append(warn);
        }

        const buttons = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL,
            spacing: 6,
            halign: Gtk.Align.END,
            margin_top: 6,
        });
        // The only control on this window. There is deliberately no
        // second button: nothing here approves, retries, copies or
        // widens anything, and no Confirm call is ever made from this
        // path — agentd parked nothing, so there is nothing to answer.
        const ok = new Gtk.Button({label: 'OK'});
        ok.add_css_class('suggested-action');
        buttons.append(ok);
        box.append(buttons);

        win.set_content(box);
        this._open.set(callId, win);
        const dismiss = () => {
            if (!this._open.has(callId))
                return;
            this._open.delete(callId);
            win.close();
            this._app.release();
        };
        ok.connect('clicked', dismiss);
        win.connect('close-request', () => {
            dismiss();
            return false;
        });
        ok.grab_focus();
        win.present();
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
        // Deny holds focus (#251). If Enter activates Allow, a
        // destructive action is one keystroke away from a person who was
        // still typing when the dialog appeared.
        win.present();
        deny.grab_focus();
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
    // Subscribe FIRST, own the name second (#244). agentd starts this
    // process by calling a method on dev.lisaos.Consent1 and emits
    // ConfirmationRequested as soon as the broker says the name is
    // owned — so owning the name has to mean "ready", or the very first
    // prompt is emitted into a session where nobody is listening yet and
    // the call sits parked until it expires.
    surface = new ConsentSurface(app);
    // No window at startup. The daemon idles until agentd asks.
    app.hold();
    Gio.bus_own_name(
        Gio.BusType.SESSION,
        CONSENT_BUS_NAME,
        Gio.BusNameOwnerFlags.NONE,
        null,
        () => log(`lisa-consentd: owning ${CONSENT_BUS_NAME}`),
        () => {
            // Losing the name means another consent surface took it.
            // Exiting is correct: two dialogs for one call is worse than
            // one, and agentd only trusts whoever owns the name.
            logError(new Error(`lost ${CONSENT_BUS_NAME} (another instance running?)`));
            app.quit();
        });
});
app.connect('activate', () => {});

app.run([]);
void surface;
