import {showFileAction, BUS_NAME, OBJECT_PATH} from '../lib/previewer-protocol.js';

let pass = 0, fail = 0;
function ok(c, w) { if (c) { pass++; console.log(`  ok    ${w}`); } else { fail++; console.log(`  FAIL  ${w}`); } }
const eq = (a, b, w) => ok(JSON.stringify(a) === JSON.stringify(b), `${w} (got ${JSON.stringify(a)})`);

console.log('preview/previewer');

// Quick Look's rule: Space on what you are already looking at closes it.
eq(showFileAction('file:///a.png', {uri: 'file:///a.png', visible: true}, true),
   {action: 'close'}, 'space on the file already shown closes it');

// The one that is annoying to get wrong: Space on a DIFFERENT file must
// swap, not close. Closing there fights the user mid-browse.
eq(showFileAction('file:///b.png', {uri: 'file:///a.png', visible: true}, true),
   {action: 'show', uri: 'file:///b.png'}, 'space on a different file swaps to it');

eq(showFileAction('file:///a.png', {uri: 'file:///a.png', visible: false}, true),
   {action: 'show', uri: 'file:///a.png'}, 'the same file, not currently visible, opens');

eq(showFileAction('file:///a.png', {uri: 'file:///a.png', visible: true}, false),
   {action: 'show', uri: 'file:///a.png'}, 'the caller can ask for show-without-toggle');

// The names Nautilus actually calls. Wrong here = the key silently does
// nothing, which is indistinguishable from the feature not existing.
ok(BUS_NAME === 'org.gnome.NautilusPreviewer2', 'the bus name is the v2 one Nautilus 50 calls');
ok(OBJECT_PATH === '/org/gnome/NautilusPreviewer', 'the object path has no 2 in it — deliberately');

console.log(`preview/previewer: ${pass} passed, ${fail} failed`);
if (fail) throw new Error(`${fail} failed`);
