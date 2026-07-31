// Unit tests for push-to-talk capture (PLAN §5.7.5): recorder
// selection, the argv each one needs, transcript cleaning, and the
// state machine that keeps two recordings off one microphone.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    Capture, RATE, RECORDERS, cleanTranscript, pickRecorder, recorderArgv,
    transcribeArgv,
} from '../lib/voice.js';

test('the first installed recorder wins, and absence is not a crash', () => {
    assertEq(pickRecorder(() => true), 'pw-record');
    // A machine with only alsa-utils — which is every machine where
    // PipeWire is broken, i.e. exactly when somebody is debugging audio.
    assertEq(pickRecorder((r) => r === 'arecord'), 'arecord');
    assertEq(pickRecorder((r) => r === 'parecord'), 'parecord');
    // Nothing installed must be answerable, so the caller can say so.
    assertEq(pickRecorder(() => false), null);
    assertEq(RECORDERS[0], 'pw-record');
});

test('every recorder is asked for 16 kHz mono, and none for a duration', () => {
    for (const r of RECORDERS) {
        const argv = recorderArgv(r, '/tmp/x.wav');
        assertEq(argv[0], r);
        assert(argv.includes('/tmp/x.wav'), `${r}: must write where told`);
        assert(argv.join(' ').includes(String(RATE)),
            `${r}: whisper wants ${RATE} Hz, got ${argv.join(' ')}`);
        // The bug this prevents is subtle: a duration flag cuts people
        // off mid-sentence, which reads as a bad transcript rather than
        // as a truncated recording. Push-to-talk ends when the key does.
        assert(!argv.includes('-d') && !argv.some((a) => a.startsWith('--duration')),
            `${r}: must run open-ended, got ${argv.join(' ')}`);
    }
    assertEq(RATE, 16000);
});

test('an unknown recorder is refused rather than half-configured', () => {
    let threw = false;
    try {
        recorderArgv('sox', '/tmp/x.wav');
    } catch {
        threw = true;
    }
    assert(threw, 'an unrecognised recorder must not silently produce argv');
});

test('transcription goes through the CLI, so the model is resolved once', () => {
    // Resolving the whisper model here as well as in `lisa` is how the
    // two would drift and disagree about which model is installed.
    assertEq(transcribeArgv('/tmp/x.wav'), ['lisa', 'transcribe', '/tmp/x.wav']);
});

test('silence is nothing said, not something to answer', () => {
    // Handed straight through, these become a question about a literal
    // bracket — the assistant answering a recording of a quiet room.
    assertEq(cleanTranscript('[BLANK_AUDIO]'), '');
    assertEq(cleanTranscript('  (silence) '), '');
    assertEq(cleanTranscript('[ Silence ]'), '');
    assertEq(cleanTranscript('[ BLANK_AUDIO ]'), '');
    assertEq(cleanTranscript('(no speech)'), '');
    assertEq(cleanTranscript(''), '');
    assertEq(cleanTranscript(null), '');
    assertEq(cleanTranscript(undefined), '');
    // Real speech survives untouched.
    assertEq(cleanTranscript('  what is the weather  '), 'what is the weather');
    // Mixed: the annotation goes, the words stay.
    assertEq(cleanTranscript('[BLANK_AUDIO] open my mail'), 'open my mail');
    assertEq(cleanTranscript('turn on [inaudible] the lights'), 'turn on the lights');
});

test('one recording at a time', () => {
    const c = new Capture();
    assertEq(c.state, 'idle');

    const a = c.start();
    assert(a.ok && a.id === 1, JSON.stringify(a));
    assertEq(c.state, 'recording');

    // Two recorders on one microphone produce two useless files, and it
    // is the second failure that is confusing to read.
    const b = c.start();
    assert(!b.ok, 'a second start while recording must be refused');
    assert(b.reason.includes('recording'), b.reason);

    assert(c.stop(1).ok);
    assertEq(c.state, 'transcribing');
    // Still busy: the key can be pressed again before whisper returns.
    assert(!c.start().ok, 'must not start while transcribing');

    assert(c.settle(1).ok);
    assertEq(c.state, 'idle');
    assert(c.start().ok, 'idle again once settled');
});

test('a late transcript cannot land in a later press', () => {
    // The failure this prevents looks like the model hallucinating: you
    // ask something, a slow whisper run from the PREVIOUS press returns,
    // and words you said a minute ago appear as the answer to this one.
    const c = new Capture();
    c.start();          // id 1
    c.stop(1);
    c.settle(1);
    c.start();          // id 2 — a new press
    assert(!c.settle(1).ok, 'session 1 must not settle session 2');
    assertEq(c.activeId, 2);
    assertEq(c.state, 'recording');
});

test('stop only applies to the recording that is actually running', () => {
    const c = new Capture();
    assert(!c.stop(1).ok, 'stop with nothing running is refused');
    c.start();
    assert(!c.stop(99).ok, 'stop with the wrong id is refused');
    assert(c.stop(1).ok);
});

test('cancel always returns to idle and reports what it dropped', () => {
    const c = new Capture();
    assertEq(c.cancel(), 0, 'cancelling nothing drops nothing');
    const {id} = c.start();
    assertEq(c.cancel(), id);
    assertEq(c.state, 'idle');
    // After a cancel the dropped session must not be able to settle and
    // deliver its transcript anyway.
    assert(!c.settle(id).ok, 'a cancelled session delivers nothing');
});

finish('overlay/voice');
