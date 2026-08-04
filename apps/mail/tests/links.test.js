import {linkAction} from '../lib/links.js';
import {assert, finish, test} from '../../../shell/testing/harness.js';

const ok = (cond, what) => test(what, () => assert(cond));
const is = (uri, action, w) => ok(linkAction(uri).action === action, `${w} (got ${linkAction(uri).action})`);

is('', 'in-place', 'the message load itself proceeds');
is('about:blank', 'in-place', 'about:blank is the message load');
is('https://example.com/x', 'external', 'https opens in the browser');
is('http://example.com', 'external', 'http opens in the browser');
is('mailto:a@b.test', 'external', 'mailto opens the composer');

// The two the reading pane used to get wrong.
is('data:text/html,<h1>Your bank</h1>', 'refuse',
   'a data: link cannot replace the reading pane — that is a spoof surface');
is('file:///home/lisa/.ssh/id_ed25519', 'refuse',
   'a sender cannot open a local file');
is('smb://attacker.test/share', 'refuse', 'a sender cannot reach the network by scheme');
is('javascript:alert(1)', 'refuse', 'javascript: never');

// Case and whitespace are the same attack.
is('  DATA:text/html,x', 'refuse', 'case and leading space do not evade the check');
is('  HtTpS://example.com', 'external', 'case does not break the allowlist either');

finish('mail/links');
