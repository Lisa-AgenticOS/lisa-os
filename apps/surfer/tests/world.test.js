// Which JS world agent scripts run in (#212).
//
// A unit test cannot prove world isolation — that is WebKit's behaviour
// and it was proved on the device with a hostile page (see README).
// What this file pins is the one line that decides it: the `world_name`
// argument of evaluate_javascript. `null` there means the PAGE's own
// world, where the page owns JSON.stringify and document.querySelector,
// and a page that owns those can forge a read_page result and retarget
// a confirmed fill. So: never null, and always the same named world.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {AGENT_WORLD, evaluateInAgentWorld} from '../lib/world.js';

/// The three arguments WebKit cares about, recorded. Signature per
/// WebKit-6.0.gir on the device:
///   evaluate_javascript(script, length, world_name, source_uri,
///                       cancellable, callback)
class FakeView {
    constructor({value = '"ok"', fail = null} = {}) {
        this.calls = [];
        this._value = value;
        this._fail = fail;
    }

    evaluate_javascript(script, length, worldName, sourceUri, cancellable, cb) {
        this.calls.push({script, length, worldName, sourceUri, cancellable});
        cb(this, {});
    }

    evaluate_javascript_finish(_res) {
        if (this._fail) throw this._fail;
        return {to_string: () => this._value};
    }
}

const view = new FakeView({value: '{"title":"T"}'});
const result = await evaluateInAgentWorld(view, 'document.title');

const failing = new FakeView({fail: new Error('script threw')});
let rejected = null;
try { await evaluateInAgentWorld(failing, 'boom'); } catch (e) { rejected = e; }

test('agent scripts never run in the page world', () => {
    // The literal defect: the third argument was null.
    assert(view.calls.length === 1, 'script was not evaluated');
    assert(view.calls[0].worldName !== null,
        'world_name null runs the script in the page\'s own world (#212)');
    assert(typeof view.calls[0].worldName === 'string' &&
           view.calls[0].worldName.length > 0,
        'world_name must be a real name, not an empty string');
});

test('every agent script shares ONE named world', () => {
    // Same world for read and write: they must see the same isolated
    // globals, and a second world name would be a second surface to
    // remember to isolate.
    assertEq(view.calls[0].worldName, AGENT_WORLD);
    const other = new FakeView();
    evaluateInAgentWorld(other, 'x');
    assertEq(other.calls[0].worldName, AGENT_WORLD);
});

test('the script and length are passed through unchanged', () => {
    assertEq(view.calls[0].script, 'document.title');
    assertEq(view.calls[0].length, -1);
});

test('the value comes back as a string, and a throw rejects', () => {
    assertEq(result, '{"title":"T"}');
    assert(rejected instanceof Error, 'a failing script must reject');
    assertEq(rejected.message, 'script threw');
});

await finish('surfer/world');
