// Settings Providers page — view-model tests (PLAN §5.11, ADR-0008).
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    EGRESS_COLOR, SCOPES, parseState, describeProvider, providerRows,
    claudeSignInState, consentRows, anythingLeaves, offloadSummary,
    validateCustomProvider, remoteReadiness, providersDisabledReason,
    parseCatalog, fitBadge, localModelSubtitle, localModelRows,
    profileSummary, providerModelHelp, PROVIDER_LOGOS, providerLogoFile,
    modelHintFor, parseModelList,
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

test('consentRows marks prompt as the primary scope with the explainer', () => {
    const rows = consentRows(parseState(null).mayOffload);
    assertEq(rows[0].id, 'prompt', 'prompt stays first');
    assert(rows[0].primary, 'prompt is primary');
    assert(rows[0].description.includes('required'), rows[0].description);
    assert(!rows.slice(1).some(r => r.primary), 'only prompt is primary');
});

test('remoteReadiness: a key with prompt off is the consent trap', () => {
    const r = remoteReadiness(parseState({
        providers: [{id: 'openai', has_credential: true}],
        may_offload: {prompt: false},
    }));
    assert(!r.usable, 'prompt off refuses every remote request');
    assertEq(r.reason, 'prompt-off');
    assert(r.hasKeyedProvider && !r.promptAllowed);
});

test('remoteReadiness: a key with prompt on is ready', () => {
    const r = remoteReadiness(parseState({
        providers: [{id: 'openai', has_credential: true}],
        may_offload: {prompt: true},
    }));
    assert(r.usable);
    assertEq(r.reason, 'ready');
});

test('remoteReadiness: without any stored key it is not usable', () => {
    const r = remoteReadiness(parseState({
        providers: [{id: 'openai', has_credential: false}],
        may_offload: {prompt: true},
    }));
    assert(!r.usable);
    assertEq(r.reason, 'no-key');
    const empty = remoteReadiness(parseState(null));
    assert(!empty.usable);
    assertEq(empty.reason, 'no-key', 'defaults carry no credential');
});

test('providersDisabledReason: offline explains, online is silent', () => {
    const reason = providersDisabledReason({offline: true});
    assert(reason.includes('not running'), reason);
    assert(reason.includes('cannot be saved'), reason);
    assertEq(providersDisabledReason({offline: false}), null);
    assertEq(providersDisabledReason({}), null);
    assertEq(providersDisabledReason(null), null);
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

// --- Local models (§8 hardware-aware fit) -----------------------------

const sampleCatalog = {
    profile: {os: 'linux', arch: 'x86_64', total_ram_gb: 8, tier: 1,
        unified_memory: false, gpu_nodes: 1, npu_nodes: 0},
    models: [
        {id: 'gemma-3-1b-it-q8', task: 'system', license: 'Gemma-Terms',
            min_ram_gb: 2, fit: 'runs', installed: true, available: true, note: 'n'},
        {id: 'whisper-base-en', task: 'asr', license: 'MIT',
            min_ram_gb: 1, fit: 'runs', installed: false, available: true, note: 'stt'},
        {id: 'qwen3-8b-instruct-q4', task: 'system', license: 'Apache-2.0',
            min_ram_gb: 8, fit: 'tight', installed: false, available: false, note: ''},
        {id: 'big-70b', task: 'system', license: 'x',
            min_ram_gb: 48, fit: 'toobig', installed: false, available: true, note: ''},
    ],
};

test('parseCatalog is defensive: garbage → no profile, no models', () => {
    for (const raw of ['nope', '{}', null, undefined, '[]']) {
        const c = parseCatalog(raw);
        assertEq(c.models, [], `models for ${raw}`);
        assert(c.profile === null || c.profile === undefined, `no profile for ${raw}`);
    }
});

test('parseCatalog round-trips the profile and models', () => {
    const c = parseCatalog(JSON.stringify(sampleCatalog));
    assertEq(c.profile.total_ram_gb, 8);
    assertEq(c.models.length, 4);
});

test('localModelRows order: installed, then runs, then tight, then remote', () => {
    const rows = localModelRows(sampleCatalog.models);
    assertEq(rows.map(r => r.id),
        ['gemma-3-1b-it-q8', 'whisper-base-en', 'qwen3-8b-instruct-q4', 'big-70b']);
});

test('canGet only when pinned, not installed, and it fits locally', () => {
    const rows = localModelRows(sampleCatalog.models);
    const by = id => rows.find(r => r.id === id);
    assertEq(by('gemma-3-1b-it-q8').canGet, false, 'already installed');
    assertEq(by('whisper-base-en').canGet, true, 'pinned, fits, not installed');
    assertEq(by('qwen3-8b-instruct-q4').canGet, false, 'no pinned artifact');
    assertEq(by('big-70b').canGet, false, 'too big to run locally');
    assertEq(by('big-70b').remoteOnly, true);
});

test('fitBadge prefers installed, then names the local fit', () => {
    assertEq(fitBadge({installed: true, fit: 'runs'}).kind, 'installed');
    assertEq(fitBadge({fit: 'runs'}).kind, 'runs');
    assertEq(fitBadge({fit: 'tight'}).kind, 'tight');
    assertEq(fitBadge({fit: 'toobig'}).kind, 'toobig');
    assertEq(fitBadge({}).kind, 'unknown');
});

test('localModelSubtitle joins task, license, ram', () => {
    assertEq(localModelSubtitle(sampleCatalog.models[0]),
        'system · Gemma-Terms · needs ~2 GiB');
});

test('profileSummary states capacity, or a clear fallback', () => {
    assert(profileSummary(sampleCatalog.profile).includes('8 GiB RAM'));
    assert(profileSummary(sampleCatalog.profile).includes('tier 1'));
    assert(profileSummary(null).toLowerCase().includes('unavailable'));
});

test('providerModelHelp surfaces the route and real notes, never invents models', () => {
    const hf = providerModelHelp({id: 'huggingface', notes: 'openai/gpt-oss-120b:cheapest'});
    assertEq(hf.route, 'remote:huggingface:<model-id>');
    assert(hf.hint.includes('gpt-oss-120b'), 'uses the registry notes');
    const bare = providerModelHelp({id: 'openai'});
    assertEq(bare.route, 'remote:openai:<model-id>');
    assert(bare.hint.includes('remote:openai:'), 'falls back to the routing format');
});

// --- Provider logos, model hints, live model list ----------------------

test('providerLogoFile maps every branded built-in, else the generic mark', () => {
    // The 13 built-ins with a brand logo; tinker has none.
    for (const id of ['openai', 'anthropic', 'google', 'moonshot', 'deepseek',
        'groq', 'mistral', 'xai', 'openrouter', 'perplexity', 'together',
        'fireworks', 'huggingface'])
        assertEq(providerLogoFile(id), `${id}.svg`, id);
    assertEq(Object.keys(PROVIDER_LOGOS).length, 13, 'exactly the branded set');
    assertEq(providerLogoFile('tinker'), 'generic.svg');
    assertEq(providerLogoFile('my-lab'), 'generic.svg', 'custom endpoints');
    assertEq(providerLogoFile(''), 'generic.svg');
    assertEq(providerLogoFile(undefined), 'generic.svg');
});

test('modelHintFor builds the ready-to-use remote route', () => {
    assertEq(modelHintFor('openai', 'gpt-5.2'), 'remote:openai:gpt-5.2');
    assertEq(modelHintFor('together', 'org/model'),
        'remote:together:org/model', 'namespaced ids pass through');
});

test('parseModelList round-trips the broker reply and tolerates junk', () => {
    assertEq(parseModelList('["b-model","a-model"]'), ['b-model', 'a-model']);
    assertEq(parseModelList(['x', 'y']), ['x', 'y'], 'already-parsed array');
    for (const raw of ['not json', '{}', null, undefined, '{"data":[]}', '42'])
        assertEq(parseModelList(raw), [], `junk: ${raw}`);
    // Junk entries are dropped, real ids kept, order preserved.
    assertEq(parseModelList(['keep', '', '  ', 7, null, {}, ['x'], 'also-keep']),
        ['keep', 'also-keep']);
    assertEq(parseModelList('[]'), [], 'an empty list is not an error');
});

finish('settings/model');
