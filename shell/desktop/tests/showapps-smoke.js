// Does pressing the dock's Show Apps button actually open the app grid?
// (#262)
//
// This is not a unit test and it is not a substitute for one. It exists
// because #262 was a bug that every unit test in this directory was
// blind to by construction: `showAppsAction` was pure, tested and
// correct, and the wiring that feeds it was untested and wrong. Four
// issues in one day had that shape (#241, #244, #255, #262), so the
// remedy is a check that asserts THE CONNECTION FIRES, not that a
// function returns the right string.
//
// It runs inside a real, throwaway gnome-shell, loads the real
// extension, synthesises a real pointer press on the real button, and
// reports where the overview ended up. Nothing here is a mock: if the
// button stops being a toggle, if the press lands on the wrong actor, if
// the handler is connected to a signal that never fires, or if the
// overview refuses to open, this notices and the run fails.
//
// Driven by `run-showapps-smoke.sh`, which is where the isolation
// lives. Read that file before changing anything about how it starts.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

/// Where the runner reads the transcript from. Passed in rather than
/// hardcoded so two runs cannot read each other's results.
const REPORT = GLib.getenv('LISA_SMOKE_REPORT');

/// The overview's state adjustment: 0 hidden, 1 window picker, 2 app
/// grid. Read straight from ControlsManager because it is the only
/// unambiguous answer to "which page is up" — `visible` cannot tell the
/// window picker from the grid, and that distinction is the whole bug.
const APP_GRID = 2;

let transcript = '';
let failures = 0;

function say(line) {
    transcript += `${line}\n`;
    if (REPORT) {
        try {
            GLib.file_set_contents(REPORT, transcript);
        } catch (e) {
            // The runner treats a missing/short transcript as failure,
            // so a write that cannot land is already covered.
        }
    }
}

function check(name, actual, expected) {
    const ok = actual === expected;
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
function findDock(actor) {
    if (actor.style_class === 'lisa-dock')
        return actor;
    for (const child of actor.get_children()) {
        const found = findDock(child);
        if (found)
            return found;
    }
    return null;
}

export default class ShowAppsSmoke extends Extension {
    enable() {
        this._run().catch(e => {
            say(`FAIL  threw: ${e}\n${e.stack}`);
            this._quit(1);
        });
    }

    disable() {}

    _quit(code) {
        say(failures > 0 || code !== 0 ? 'RESULT: FAIL' : 'RESULT: PASS');
        // `Meta.quit` needs an enum this shell may not export under the
        // name we expect, and a smoke check that dies on its own exit
        // path is worse than useless. `global.context.terminate()` is
        // the shell's own shutdown and takes no argument, so the runner
        // reads the verdict from the transcript, never from $?.
        global.context.terminate();
    }

    async _run() {
        // The dash sizes its icons over a few frames and the dock is
        // repositioned once that settles; pressing before then aims at
        // coordinates that are about to move.
        await sleep(4000);

        const dock = findDock(Main.layoutManager.uiGroup);
        if (!dock) {
            say('FAIL  no .lisa-dock in the chrome — the extension did not load');
            failures += 1;
            this._quit(1);
            return;
        }

        this._button = dock.dash.showAppsButton;
        say(`button: toggle_mode=${this._button.toggle_mode} ` +
            `reactive=${this._button.reactive} mapped=${this._button.mapped}`);

        const seat = Clutter.get_default_backend().get_default_seat();
        this._pointer = seat.create_virtual_device(
            Clutter.InputDeviceType.POINTER_DEVICE);
        const [x, y] = this._button.get_transformed_position();
        this._x = x + this._button.get_width() / 2;
        this._y = y + this._button.get_height() / 2;

        // The press must actually land on the button. If the dock has no
        // allocation, or something floats over it, the rest of this file
        // would be testing whatever IS under the pointer.
        const under = global.stage.get_actor_at_pos(
            Clutter.PickMode.REACTIVE, Math.round(this._x), Math.round(this._y));
        const hits = under === this._button || this._button.contains(under);
        check('the press lands on the show-apps button', hits, true);
        if (!hits) {
            this._quit(1);
            return;
        }

        // 1. The connection itself: from the desktop, one press opens
        //    the overview on the app grid. This is the assertion #262
        //    needed and did not have.
        await this._settle();
        await this._press();
        check('a press from the desktop opens the app grid', this._page(), APP_GRID);

        // 2. And a second press crosses back to the windows rather than
        //    latching over a page it cannot leave.
        await this._press();
        check('a second press returns to the window picker', this._page(), 1);

        // 3. The regression #262 is actually about. Our button's
        //    `checked` is display state that GNOME, our own `hidden`
        //    handler and a focus callback all write. Drift used to cost
        //    a press: latched over a closed overview, the next press was
        //    spent unlatching and nothing happened. Force exactly that
        //    state and press.
        await this._settle();
        this._button.checked = true;
        await sleep(300);
        check('the forced-latch state really is latched-and-closed',
            `${this._button.checked}/${Main.overview.visible}`, 'true/false');
        await this._press();
        check('a press is not swallowed by a stale latch', this._page(), APP_GRID);

        await this._settle();
        this._quit(failures > 0 ? 1 : 0);
    }

    /// Which page the overview is on: 0 hidden, 1 windows, 2 app grid.
    _page() {
        if (!Main.overview.visible)
            return 0;
        return Math.round(Main.overview._overview.controls._stateAdjustment.value);
    }

    async _settle() {
        if (Main.overview.visible)
            Main.overview.hide();
        await sleep(1500);
    }

    async _press() {
        const now = () => GLib.get_monotonic_time();
        this._pointer.notify_absolute_motion(now(), this._x, this._y);
        await sleep(250);
        this._pointer.notify_button(
            now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED);
        await sleep(120);
        this._pointer.notify_button(
            now(), Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED);
        // Long enough for the overview's ease to finish.
        await sleep(1500);
    }
}
