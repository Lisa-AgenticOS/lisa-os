// Settings Providers page — view-model tests (PLAN §5.11, ADR-0008).
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    EGRESS_COLOR, SCOPES, parseState, describeProvider, providerRows,
    claudeSignInState, consentRows, anythingLeaves, offloadSummary,
    validateCustomProvider,
} from '../lib/model.js';

const sampleState = {
    providers: [
        {id: 'openai', display_name: 'OpenAI', base_url: 'https://api.openai.com/v1',
            builtin: true, has_credential: false, oauth_available: false},
        {id: 'anthropic', display_name: 'Anthropic', base_url: 'https://api.anthropic.com',
            builtin: true, has_credential: true, oauth_available: false},
        {id: 'tinker', display_name: 'Tinker (Thinking Machines)',
            base_url: 'https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1',
            builtin: true, has_credential: true, oauth_available: false},
        {id: 'zzz-lab', display_name: 'Lab', base_url: 'http://10.0.0.2:8080/v1',
            builtin: false, has_credential: false, oauth_available: false},
        {id: 'together', display_name: 'Together.ai', base_url: 'https://api.together.ai/v1',
            builtin: true, has_credential: false, oauth_available: false},
    ],
    may_offload: {prompt: true, files: false, screen: true},
};

test('parseState defaults to nothing-leaves on garbage input', () => {
    for (const raw of ['not json', '{}', null, undefined, '[]']) {
        const s = parseState(raw);
        assertEq(s.providers, [], `providers for ${raw}`);
        assert(!anythingLeaves(s.mayOffload), `nothing leaves for ${raw}`);
    }
});

test('parseState keeps only known scopes and booleans', () => {
    const s = parseState(JSON.stringify({
        providers: [],
        may_offload: {prompt: true, telepathy: true, files: 'yes'},
    }));
    assertEq(s.mayOffload.prompt, true);
    assertEq(s.mayOffload.files, false, 'non-boolean is not consent');
    assertEq(Object.keys(s.mayOffload).length, SCOPES.length);
});

test('providerRows puts builtins first, custom rows sorted after', () => {
    const rows = providerRows(sampleState.providers);
    assertEq(rows.map(r => r.id),
        ['openai', 'anthropic', 'tinker', 'together', 'zzz-lab']);
    assert(rows[0].builtin && !rows[0].removable, 'builtins are not removable');
    const lab = rows.find(r => r.id === 'zzz-lab');
    assert(lab.removable, 'custom rows are removable');
});

test('describeProvider reports endpoint and credential presence', () => {
    const withKey = describeProvider(sampleState.providers[1]);
    assert(withKey.includes('key set'), withKey);
    const noKey = describeProvider(sampleState.providers[0]);
    assert(noKey.includes('no key'), noKey);
    const unset = describeProvider({id: 'x', builtin: true, has_credential: false});
    assert(unset.includes('endpoint not configured'), unset);
});

test('only the anthropic row offers Sign in with Claude', () => {
    const rows = providerRows(sampleState.providers);
    assertEq(rows.filter(r => r.showsSignIn).map(r => r.id), ['anthropic']);
});

test('Sign in with Claude stays disabled with the honest reason until endpoints exist', () => {
    const off = claudeSignInState({id: 'anthropic', oauth_available: false});
    assert(!off.enabled);
    assert(off.reason.includes('not published'), off.reason);
    const on = claudeSignInState({id: 'anthropic', oauth_available: true});
    assert(on.enabled);
    assertEq(on.reason, '');
    assert(!claudeSignInState({id: 'openai', oauth_available: true}).enabled);
});

test('consentRows cover every scope in stable order with active flags', () => {
    const s = parseState(sampleState);
    const rows = consentRows(s.mayOffload);
    assertEq(rows.map(r => r.id), SCOPES.map(x => x.id));
    assertEq(rows.find(r => r.id === 'prompt').active, true);
    assertEq(rows.find(r => r.id === 'files').active, false);
    assertEq(rows.find(r => r.id === 'screen').active, true);
});

test('offloadSummary states the measured egress condition', () => {
    assertEq(offloadSummary({}), 'Nothing leaves this machine.');
    const s = parseState(sampleState);
    const summary = offloadSummary(s.mayOffload);
    assert(summary.includes('leave your hardware'), summary);
    assert(summary.includes('Prompts'), summary);
    assert(summary.includes('Screen'), summary);
    assert(!summary.includes('Mail'), summary);
    assert(anythingLeaves(s.mayOffload));
});

test('custom provider validation matches the broker rules', () => {
    assertEq(validateCustomProvider(
        {id: 'homelab', displayName: 'Homelab', baseUrl: 'https://h.example/v1'}), []);
    assertEq(validateCustomProvider(
        {id: 'lab', displayName: 'Lab', baseUrl: 'http://10.0.0.2:1234/v1'}), [],
    'http allowed for local endpoints');
    const errs = validateCustomProvider(
        {id: 'Bad Id', displayName: ' ', baseUrl: 'ftp://x'});
    assertEq(errs.length, 3, JSON.stringify(errs));
    const dup = validateCustomProvider(
        {id: 'openai', displayName: 'X', baseUrl: 'https://x'}, ['openai']);
    assert(dup.some(e => e.includes('already taken')), JSON.stringify(dup));
});

test('the egress color is the ADR-0008 amber', () => {
    assertEq(EGRESS_COLOR, '#E66100');
});

finish('settings/model');
