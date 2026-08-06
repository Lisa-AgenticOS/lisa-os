#!/usr/bin/env -S gjs -m
// lisa-consentd.js — the consent dialog, and nothing else
// (issues #145, #251, #289; ADR-0035 §4, ADR-0030, PLAN §5.10).
//
// THIS FILE NO LONGER TOUCHES THE BUS.
//
// It used to be the whole consent surface: it owned `dev.lisaos.Consent1`
// and it called `dev.lisaos.Agent1.Confirm`. That worked, and it left
// #289 open, because `Exec=/usr/bin/lisa-app consent/lisa-consentd.js`
// ends in `exec gjs -m "$found"` — so the kernel's answer for
// `/proc/<pid>/exe` on the process owning the consent name was
// `/usr/bin/gjs-console`. agentd's program allowlist therefore had to
// contain an INTERPRETER, and an interpreter on an allowlist authorises
// every program it can run. A hostile GJS script that forks and execs
// gjs gets a fresh pid, satisfies the same-process check too, and is
// then indistinguishable from this dialog.
//
// So the peer moved into a binary of its own: `shell/consent/daemon`,
// installed as `/usr/bin/lisa-consentd`. It owns the name, it subscribes
// to agentd's signals, and it makes the `Confirm` call. This file is its
// CHILD — the window it draws with — and it is spawned with no
// `DBUS_SESSION_BUS_ADDRESS`, so it cannot open a session bus connection
// even if something replaced its contents.
//
// THE CHANNEL
//
// One JSON object per line, stdin in and stdout out
// (`shell/consent/daemon/src/protocol.rs`):
//
//     in   {"kind":"confirm","call_id":41,"spec":"<agentd's json>"}
//     in   {"kind":"refusal","call_id":41,"report":"<agentd's json>"}
//     out  {"call_id":41,"answer":"allow"|"deny"|"dismiss"}
//
// The call id travels out and comes back so the parent can match an
// answer to a dialog. It is not a capability: this process has no bus,
// and the parent drops an answer for a dialog it did not open.
//
// WHAT THIS FILE MUST NEVER GROW
//
// No model. No prompt entry. No tool calls of its own. Its only inputs
// are the parent's messages and a human's click, and its only output is
// which button was pressed. The moment it can be driven by generated
// text it stops being a second pair of eyes (ADR-0030: anything
// reachable from inside is not a guardrail).

import Gio from 'gi://Gio';
import GioUnix from 'gi://GioUnix';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';
import Adw from 'gi://Adw?version=1';

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

/// The pipe back to `/usr/bin/lisa-consentd`.
///
/// Every answer goes through here and nowhere else: this process has no
/// other way to affect anything, which is what makes "the dialog" a
/// window rather than an authority.
class Channel {
    constructor(onMessage, onClosed) {
        this._onMessage = onMessage;
        this._onClosed = onClosed;
        this._in = new Gio.DataInputStream({
            base_stream: new GioUnix.InputStream({fd: 0, close_fd: false}),
        });
        this._out = new GioUnix.OutputStream({fd: 1, close_fd: false});
        this._readLine();
    }

    /// Report a click. Written and flushed immediately: a person has
    /// answered, and a buffered answer is a call that stays parked.
    answer(callId, verdict) {
        const line = `${JSON.stringify({call_id: callId, answer: verdict})}\n`;
        try {
            this._out.write_all(line, null);
            this._out.flush(null);
        } catch (e) {
            logError(e, 'lisa-consentd.js: could not report the answer');
        }
    }

    _readLine() {
        this._in.read_line_async(GLib.PRIORITY_DEFAULT, null, (stream, res) => {
            let line;
            try {
                [line] = stream.read_line_finish_utf8(res);
            } catch (e) {
                logError(e, 'lisa-consentd.js: reading from the daemon');
                this._onClosed();
                return;
            }
            // EOF: the daemon is gone. Exit rather than linger with
            // windows nobody can answer to — the calls behind them are
            // still parked in agentd and expire, which is safe.
            if (line === null) {
                this._onClosed();
                return;
            }
            if (line.length > 0) {
                try {
                    this._onMessage(JSON.parse(line));
                } catch (e) {
                    logError(e, 'lisa-consentd.js: unreadable message from the daemon');
                }
            }
            this._readLine();
        });
    }
}

class ConsentDialogs {
    constructor(channel) {
        this._channel = channel;
        this._open = new Map(); // call_id -> Adw.Window
    }

    /// One message from the daemon. Two kinds, never one with a flag:
    /// a renderer that could confuse a refusal with a confirmation would
    /// draw an Allow button on something with no parked call behind it.
    dispatch(msg) {
        const id = Number(msg.call_id);
        if (!Number.isSafeInteger(id) || id < 0)
            return;
        // The daemon already de-duplicates, and so does this: agentd
        // re-emits on reconnect, and one dialog per call is the rule at
        // every layer that can enforce it.
        if (this._open.has(id))
            return;
        if (msg.kind === 'confirm')
            this._prompt(id, describe(String(msg.spec ?? '')));
        else if (msg.kind === 'refusal')
            this._report(id, describeRefusal(String(msg.report ?? '')));
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
        // widens anything, and the answer this sends is `dismiss`, which
        // the daemon can never turn into a `Confirm` — agentd parked
        // nothing, so there is nothing to answer.
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
            this._channel.answer(callId, 'dismiss');
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
            this._channel.answer(callId, approve ? 'allow' : 'deny');
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
}

// `Adw.init()` initialises GTK too. Deliberately NOT `Gtk.Application`:
// an application registers on the session bus, and this process is
// spawned without one on purpose (`renderer.rs::STRIPPED_ENV`). A plain
// main loop needs no bus and no application id.
Adw.init();
const loop = new GLib.MainLoop(null, false);
let dialogs = null;
const channel = new Channel(
    (msg) => dialogs.dispatch(msg),
    () => loop.quit());
dialogs = new ConsentDialogs(channel);
loop.run();
