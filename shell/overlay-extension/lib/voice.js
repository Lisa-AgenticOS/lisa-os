// Push-to-talk capture logic (PLAN §5.7.5), kept pure so it can be
// tested without a compositor, a microphone or a model.
//
// The parts that need hardware — spawning the recorder, waiting for
// whisper — live in the backend service. What is here is everything
// that can be decided from arguments alone, which is most of the ways
// this feature can be wrong: the wrong sample rate, the wrong recorder
// flags, a transcript that is really an error message, and a state
// machine that lets two recordings run at once.

/// Recorders in preference order.
///
/// pw-record is native to the image's PipeWire stack. arecord comes
/// from alsa-utils, which is in the image so sound can be proved BELOW
/// PipeWire — issue #44 was a case where every session-level indicator
/// was green and the speakers were mute (ADR-0024), and the same
/// applies to a microphone.
export const RECORDERS = ['pw-record', 'parecord', 'arecord'];

/// whisper.cpp wants 16 kHz mono 16-bit. Handing it 44.1 kHz stereo
/// makes it resample internally and transcribe slightly worse for no
/// gain — nothing here is recording music.
export const RATE = 16000;

/// First recorder that exists on this machine. `have` answers "is this
/// program installed?" so the caller decides how to look.
export function pickRecorder(have) {
    return RECORDERS.find((r) => have(r)) ?? null;
}

/// The argv for a recorder that runs until it is signalled.
///
/// Note what is NOT here: a duration. Push-to-talk is defined by the
/// key — you speak for as long as you hold it — so the recorder must
/// run open-ended and be stopped. A duration flag would silently cut
/// people off mid-sentence, which reads as a bad transcript rather than
/// as a truncated recording.
export function recorderArgv(recorder, wavPath) {
    switch (recorder) {
    case 'pw-record':
        return [recorder, `--rate=${RATE}`, '--channels=1', '--format=s16', wavPath];
    case 'parecord':
        return [recorder, `--rate=${RATE}`, '--channels=1', '--format=s16le',
            '--file-format=wav', wavPath];
    case 'arecord':
        // No -d: same reason as above. -q keeps its progress chatter out
        // of the journal.
        return [recorder, '-q', '-f', 'S16_LE', '-r', String(RATE), '-c', '1', wavPath];
    default:
        throw new Error(`unknown recorder: ${recorder}`);
    }
}

/// Transcription goes through `lisa transcribe`, so the model lives in
/// exactly one place (the CLI resolves it from the store) rather than
/// being resolved a second time, differently, here.
export function transcribeArgv(wavPath) {
    return ['lisa', 'transcribe', wavPath];
}

/// whisper.cpp annotates non-speech, and those annotations are not
/// something anybody meant to say. A recording of silence comes back as
/// "[BLANK_AUDIO]" or "(silence)", which handed to the assistant
/// becomes a question about a literal bracket.
const NON_SPEECH =
    /^\s*[[(]\s*(blank_audio|blank audio|silence|music|inaudible|no speech)\s*[\])]\s*$/i;

/// Clean a whisper transcript into something worth sending, or '' when
/// there is nothing. Returning '' rather than the annotation is what
/// makes "I pressed the key and said nothing" a no-op instead of a
/// confusing answer.
export function cleanTranscript(stdout) {
    const text = (stdout ?? '').trim();
    if (!text || NON_SPEECH.test(text))
        return '';
    // whisper emits these inline too, e.g. "[BLANK_AUDIO] hello".
    const stripped = text
        .replace(/[[(]\s*(?:BLANK_AUDIO|blank audio|silence|music|inaudible)\s*[\])]/gi, '')
        .replace(/\s+/g, ' ')
        .trim();
    return stripped;
}

/// The push-to-talk state machine.
///
/// One recording at a time, and every transition names the session it
/// applies to. Without the id check a slow whisper run from a previous
/// press could deliver its transcript into a press that came after it,
/// and the user would see the wrong words appear — the kind of bug that
/// looks like the model hallucinating.
export class Capture {
    constructor() {
        this._id = 0;
        this._state = 'idle';
        this._active = 0;
    }

    get state() {
        return this._state;
    }

    get activeId() {
        return this._active;
    }

    /// Begin a session. Refuses while one is running rather than
    /// silently replacing it: two recorders on one microphone produce
    /// two useless files, and the second failure is the confusing one.
    start() {
        if (this._state !== 'idle')
            return {ok: false, reason: `already ${this._state}`};
        this._id += 1;
        this._active = this._id;
        this._state = 'recording';
        return {ok: true, id: this._id};
    }

    /// The key came up. Moves to transcribing; the transcript arrives
    /// later and asynchronously.
    stop(id) {
        if (this._state !== 'recording' || id !== this._active)
            return {ok: false, reason: 'no such recording'};
        this._state = 'transcribing';
        return {ok: true, id};
    }

    /// A transcript (or failure) came back. Ignored unless it belongs to
    /// the session still in flight.
    settle(id) {
        if (id !== this._active)
            return {ok: false, reason: 'stale session'};
        this._state = 'idle';
        this._active = 0;
        return {ok: true};
    }

    /// Abandon whatever is happening — the key was released after the
    /// session already failed, the user hit Escape, the shell is
    /// shutting down.
    cancel() {
        const was = this._active;
        this._state = 'idle';
        this._active = 0;
        return was;
    }
}
