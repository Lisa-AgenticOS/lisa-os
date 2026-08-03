import {activeIconName, candidatePaths, shouldUseActive, isTransientPeek} from '../lib/stateicon.js';

let passed = 0, failed = 0;
function ok(cond, name, got) {
    if (cond) { passed++; console.log(`  ok    ${name}`); }
    else { failed++; console.log(`  FAIL  ${name}${got !== undefined ? ` (got ${JSON.stringify(got)})` : ''}`); }
}

ok(activeIconName('app.lisaos.Surfer.desktop') === 'app.lisaos.Surfer-active',
    'the .desktop suffix is stripped before -active is appended');
ok(activeIconName('app.lisaos.Surfer') === 'app.lisaos.Surfer-active',
    'a bare id works too');
ok(activeIconName('') === null && activeIconName(undefined) === null,
    'garbage in, null out — not "-active"');

const paths = candidatePaths('app.lisaos.Surfer-active', ['/home/u/.local/share', '/usr/share']);
ok(paths[0] === '/home/u/.local/share/icons/hicolor/512x512/apps/app.lisaos.Surfer-active.png',
    'user data dir is searched first', paths[0]);
ok(paths.includes('/usr/share/icons/hicolor/scalable/apps/app.lisaos.Surfer-active.svg'),
    'scalable svg is a candidate');
ok(paths.length === 8, 'four sizes per data dir', paths.length);

ok(!shouldUseActive(0, true), 'stopped never swaps');
ok(!shouldUseActive(1, true),
    'STARTING keeps the idle icon — a launch that dies must not leave a lying active icon');
ok(shouldUseActive(2, true), 'running with a variant swaps');
ok(!shouldUseActive(2, false), 'running without a variant keeps the stock icon');

ok(isTransientPeek('app.lisaos.PreviewPeek.desktop'), 'the Preview peek is a transient — no dock presence');
ok(!isTransientPeek('app.lisaos.Preview.desktop'), 'the real Preview app IS a dock citizen');
ok(!isTransientPeek('app.lisaos.Surfer.desktop'), 'other apps are untouched');

console.log(`desktop/stateicon: ${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
