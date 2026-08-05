// Does the dock's prompt actually take a keystroke, and does a badge
// survive the Dash rebuilding itself? (ADR-0035 §2, #190)
//
// Neither question a unit test can answer, and both are exactly the
// shape of defect this repo keeps shipping: a correct decision behind
// a connection that never fires (#241, #244, #255, #262). `submission()`
// and `BadgeState` are pure and tested next door. What is asserted HERE
// is that a real pointer press puts the caret in a real entry, that a
// real key reaches it, that pressing Return really does call
// `dev.lisaos.Overlay1.UI.Summon` with what was typed, that a program
// name really launches a program — and that a badge drawn on a dash
// icon is still there after the dash throws that icon away.
//
// Nothing here is mocked except the assistant itself, which is a stub
// only because the overlay extension is a separate tree: the stub owns
// the real bus name on the real bus, so a typo in the name the dock
// calls fails this the same way it would fail a person.
//
// Driven by `run-dock-prompt-smoke.sh`, which owns the isolation. Read
// that file's header before changing how it starts.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const REPORT = GLib.getenv('LISA_SMOKE_REPORT');
/// Where the fake app writes when it is launched for real.
const LAUNCH_MARK = GLib.getenv('LISA_SMOKE_LAUNCH_MARK');

/// The interface the dock calls. Spelled out here rather than imported
/// from the overlay tree, because that is precisely the copy under
/// test: if the dock's name and this one disagree, the call goes
/// nowhere and this run fails.
const SUMMON_XML = `
<node>
  <interface name="dev.lisaos.Overlay1.UI">
    <method name="Summon">
      <arg type="s" name="prompt" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
    </method>
    <method name="Hide"/>
    <method name="GetVisible">
      <arg type="b" name="visible" direction="out"/>
    </method>
  </interface>
</node>`;

let transcript = '';
let failures = 0;

function say(line) {
    transcript += `${line}\n`;
    if (REPORT) {
        try {
            GLib.file_set_contents(REPORT, transcript);
        } catch (e) {
            // The runner treats a missing transcript as failure, so a
            // write that cannot land is already covered.
        }
    }
}

function check(name, actual, expected) {
    const ok = String(actual) === String(expected);
    if (!ok)
        failures += 1;
    say(`${ok ? 'ok   ' : 'FAIL '} ${name}: expected ${expected}, got ${actual}`);
}

const sleep = ms => new Promise(resolve =>
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => {
        resolve();
        return GLib.SOURCE_REMOVE;
    }));

/// The dock is chrome, not a child of any exported object, so the only
/// honest way to find it is the way a person does: look at the screen.
function findByClass(actor, styleClass) {
    if (actor.style_class === styleClass)
        return actor;
    for (const child of actor.get_children()) {
        const found = findByClass(child, styleClass);
        if (found)
            return found;
    }
    return null;
}

/// Every badge pill currently on screen, by the text it shows.
function badgeLabels() {
    const found = [];
    const walk = actor => {
        if (actor.style_class === 'lisa-dock-badge')
            found.push(actor.text);
        for (const child of actor.get_children())
            walk(child);
    };
    walk(Main.layoutManager.uiGroup);
    return found;
}

export default class DockPromptSmoke extends Extension {
    enable() {
        this._summons = [];
        this._impl = Gio.DBusExportedObject.wrapJSObject(SUMMON_XML, {
            Summon: (prompt, options) => {
                this._summons.push(prompt);
                say(`   (stub assistant heard: ${JSON.stringify(prompt)})`);
            },
            Hide: () => {},
            GetVisible: () => false,
        });
        this._impl.export(Gio.DBus.session, '/dev/lisaos/Overlay1/UI');
        this._nameId = Gio.bus_own_name(
            Gio.BusType.SESSION, 'dev.lisaos.Overlay1.UI',
            Gio.BusNameOwnerFlags.NONE, null, null, null);

        this._run().catch(e => {
            say(`FAIL  threw: ${e}\n${e.stack}`);
            this._quit(1);
        });
    }

    disable() {}

    _quit(code) {
        say(failures > 0 || code !== 0 ? 'RESULT: FAIL' : 'RESULT: PASS');
        global.context.terminate();
    }

    async _run() {
        // The dash sizes its icons over a few frames and the dock is
        // repositioned once that settles; pressing before then aims at
        // coordinates that are about to move.
        await sleep(5000);

        const dock = findByClass(Main.layoutManager.uiGroup, 'lisa-dock');
        if (!dock) {
            say('FAIL  no .lisa-dock in the chrome — the extension did not load');
            failures += 1;
            this._quit(1);
            return;
        }
        const entry = findByClass(dock, 'lisa-dock-prompt');
        if (!entry) {
            say('FAIL  the dock has no prompt — ADR-0035 §2 is not built here');
            failures += 1;
            this._quit(1);
            return;
        }
        say(`prompt: width=${entry.width} reactive=${entry.reactive} ` +
            `can_focus=${entry.can_focus} mapped=${entry.mapped}`);

        const seat = Clutter.get_default_backend().get_default_seat();
        this._pointer = seat.create_virtual_device(
            Clutter.InputDeviceType.POINTER_DEVICE);
        this._keyboard = seat.create_virtual_device(
            Clutter.InputDeviceType.KEYBOARD_DEVICE);
        this._entry = entry;

        // 0. The prompt must not have the keyboard before anybody asks.
        //    A permanent entry that grabs focus on appearing breaks
        //    every keyboard workflow on the machine (ADR-0035).
        check('the prompt does not hold the keyboard at startup',
            global.stage.key_focus === entry.clutter_text, false);

        // 1. A press in the field puts the caret in the field.
        await this._clickEntry();
        check('a press puts the caret in the prompt',
            global.stage.key_focus === entry.clutter_text, true);
        // And the dock holds a modal grab, which is the part that
        // matters off a headless shell: with no grab the caret still
        // moves and the field still takes text HERE — because nothing
        // has focus in a shell with no windows — and takes nothing at
        // all on a desktop where any window is focused.
        check('and the dock holds the keyboard grab',
            global.stage.get_grab_actor?.() === dock, true);

        // 2. And a typed key reaches it. This is the assertion the
        //    modal grab exists for: without one, an St.Entry in chrome
        //    receives keystrokes only while no window is focused.
        await this._type('hello lisa');
        check('typing reaches the prompt', entry.get_text(), 'hello lisa');

        // 3. Return hands it to the assistant, over the real bus, under
        //    the real name.
        await this._key(0xff0d);
        await sleep(800);
        check('Return summons the assistant with what was typed',
            this._summons[this._summons.length - 1], 'hello lisa');
        check('and the prompt is empty again', entry.get_text(), '');
        check('and the keyboard has been handed back',
            global.stage.key_focus === entry.clutter_text, false);

        // 4. Escape clears before it releases — two presses, two
        //    different jobs.
        await this._clickEntry();
        await this._type('half a thought');
        await this._key(0xff1b);
        await sleep(300);
        check('Escape clears the text', entry.get_text(), '');
        check('but keeps the keyboard',
            global.stage.key_focus === entry.clutter_text, true);
        await this._key(0xff1b);
        await sleep(300);
        check('a second Escape gives the keyboard back',
            global.stage.key_focus === entry.clutter_text, false);
        check('and the grab with it — the rest of the desktop can be clicked',
            global.stage.get_grab_actor?.() === dock, false);

        // 5. A program name launches the program instead of asking
        //    about it. The fake app writes a file when it runs, so this
        //    is an observation and not an inference.
        const before = this._summons.length;
        await this._clickEntry();
        await this._type('smokeapp');
        await this._key(0xff0d);
        await sleep(1500);
        check('a program name launched a program',
            GLib.file_test(LAUNCH_MARK ?? '/nonexistent', GLib.FileTest.EXISTS), true);
        check('and did NOT go to the assistant', this._summons.length, before);

        // 6. Badges (#190): a signal any peer may send draws a pill…
        await this._emitBadge('smokeapp.desktop', 4);
        await sleep(1200);
        check('a LauncherEntry update draws a badge', badgeLabels().join(','), '4');

        // …and it survives the Dash throwing that icon away.
        //
        // A rebuild alone is NOT the hazard, which is worth stating
        // because the first version of this check tested nothing: the
        // Dash reuses the icon actor of an app that stays in the list,
        // so a badge painted on it survives a redisplay whether or not
        // anything is tracking it. The hazard is an item DESTROYED and
        // later recreated — unpinned and repinned, or a non-favourite
        // whose last window closes and opens again. Then the pill is
        // gone with the actor, and only something holding what the app
        // SAID can put it back.
        const favourites = new Gio.Settings({schema_id: 'org.gnome.shell'});
        favourites.set_strv('favorite-apps', ['smokeapp2.desktop']);
        await sleep(2000);
        check('unpinning takes the icon and its badge away',
            badgeLabels().join(','), '');
        favourites.set_strv('favorite-apps', ['smokeapp.desktop', 'smokeapp2.desktop']);
        await sleep(2000);
        check('a recreated icon gets its badge back', badgeLabels().join(','), '4');

        // …and an emitter can clear it, which is the half that is
        // usually missing.
        await this._emitBadge('smokeapp.desktop', 0);
        await sleep(1200);
        check('count 0 takes the badge off', badgeLabels().length, 0);

        this._quit(failures > 0 ? 1 : 0);
    }

    _emitBadge(desktopId, count) {
        Gio.DBus.session.emit_signal(
            null, '/com/canonical/Unity/LauncherEntry',
            'com.canonical.Unity.LauncherEntry', 'Update',
            new GLib.Variant('(sa{sv})', [
                `application://${desktopId}`,
                {
                    'count': new GLib.Variant('x', count),
                    'count-visible': new GLib.Variant('b', count > 0),
                },
            ]));
        return sleep(200);
    }

    async _clickEntry() {
        const [x, y] = this._entry.get_transformed_position();
        const [w, h] = this._entry.get_transformed_size();
        // Left of centre, clear of the go-arrow at the right edge.
        const px = Math.round(x + w * 0.3);
        const py = Math.round(y + h / 2);
        const under = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, px, py);
        if (under !== this._entry && !this._entry.contains(under)) {
            say(`FAIL  the press would land on ${under} — not the prompt`);
            failures += 1;
        }
        const now = () => GLib.get_monotonic_time();
        this._pointer.notify_absolute_motion(now(), px, py);
        await sleep(250);
        this._pointer.notify_button(now(), Clutter.BUTTON_PRIMARY,
            Clutter.ButtonState.PRESSED);
        await sleep(120);
        this._pointer.notify_button(now(), Clutter.BUTTON_PRIMARY,
            Clutter.ButtonState.RELEASED);
        await sleep(600);
    }

    async _key(keyval) {
        const now = () => GLib.get_monotonic_time();
        this._keyboard.notify_keyval(now(), keyval, Clutter.KeyState.PRESSED);
        await sleep(40);
        this._keyboard.notify_keyval(now(), keyval, Clutter.KeyState.RELEASED);
        await sleep(60);
    }

    async _type(text) {
        for (const ch of text) {
            // Latin letters and space are their own keyvals; this probe
            // types nothing else.
            await this._key(ch === ' ' ? 0x020 : ch.charCodeAt(0));
        }
        await sleep(300);
    }
}
