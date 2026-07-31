// dev.lisaos.Voice1 — the push-to-talk capture service (PLAN §5.7.5).
//
// WHY THIS IS IN THE BACKEND AND NOT THE EXTENSION
//
// A GNOME Shell extension runs *inside the compositor*. Spawning a
// recorder is cheap, but waiting for whisper is not: a base-model
// transcription of a few seconds of speech takes long enough that doing
// it in-process would freeze every window on the machine while Lisa
// thought. So the extension holds the key and paints the indicator, and
// everything with a subprocess in it happens out here.
//
// WHAT IT DOES NOT DO
//
// It does not listen on its own. There is no wake word, no always-on
// capture, and no timer: the microphone opens when StartListening is
// called and closes when StopListening is, both driven by a key a
// person is physically holding. That is a deliberate property, not a
// missing feature — an ambient loop is a different design with a
// different consent story (ADR-0011), and it needs its own ADR before
// anything in this repo starts recording unprompted.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {Capture, cleanTranscript, pickRecorder, recorderArgv, transcribeArgv}
    from '../lib/voice.js';
import {VOICE_IFACE_XML, VOICE_BUS_NAME, VOICE_OBJECT_PATH} from '../lib/iface.js';

/// A recording longer than this is almost certainly a stuck key rather
/// than somebody with a lot to say. Without it, a key-release event lost
/// to a compositor restart leaves a recorder running on the microphone
/// until the session ends, which is both a privacy problem and a disk
/// one, and nothing would report it.
const MAX_SECONDS = 120;

export class VoiceService {
    constructor(connection) {
        this._connection = connection;
        this._impl = Gio.DBusExportedObject.wrapJSObject(VOICE_IFACE_XML, this);
        this._impl.export(connection, VOICE_OBJECT_PATH);
        this._capture = new Capture();
        this._proc = null;
        this._wav = null;
        this._guard = 0;
    }

    // ---- D-Bus surface ----

    StartListening() {
        const started = this._capture.start();
        if (!started.ok)
            throw new Gio.DBusError({message: `cannot listen: ${started.reason}`});

        const recorder = pickRecorder(prog => !!GLib.find_program_in_path(prog));
        if (!recorder) {
            this._capture.cancel();
            throw new Gio.DBusError({
                message: 'no recorder installed (pw-record, parecord or arecord)',
            });
        }

        const id = started.id;
        this._wav = GLib.build_filenamev(
            [GLib.get_user_runtime_dir(), `lisa-ptt-${id}.wav`]);
        try {
            this._proc = Gio.Subprocess.new(
                recorderArgv(recorder, this._wav), Gio.SubprocessFlags.STDERR_PIPE);
        } catch (e) {
            this._capture.cancel();
            this._wav = null;
            throw new Gio.DBusError({message: `could not start ${recorder}: ${e.message}`});
        }

        // See MAX_SECONDS: a lost key-release must not leave the
        // microphone open for the rest of the session.
        this._guard = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, MAX_SECONDS, () => {
            this._guard = 0;
            if (this._capture.activeId === id && this._capture.state === 'recording') {
                log(`lisa-voiced: recording ${id} hit the ${MAX_SECONDS}s ceiling`);
                this._abort(id, `stopped after ${MAX_SECONDS}s — was the key stuck?`);
            }
            return GLib.SOURCE_REMOVE;
        });

        this._emit('ListeningStarted', new GLib.Variant('(t)', [id]));
        return id;
    }

    StopListening(id) {
        const stopped = this._capture.stop(id);
        if (!stopped.ok)
            throw new Gio.DBusError({message: stopped.reason});
        this._clearGuard();
        this._emit('Transcribing', new GLib.Variant('(t)', [id]));
        // SIGINT, not SIGKILL: the recorders finalise the WAV header on
        // interrupt. Killed outright they leave a file whose header
        // claims zero samples, which whisper reads as silence — the
        // failure would look like "Lisa never hears me" rather than
        // "the recording was truncated".
        this._proc?.send_signal(2);
        this._transcribe(id).catch(e => this._abort(id, e.message));
    }

    Cancel(id) {
        if (this._capture.activeId !== id)
            return;
        this._abort(id, 'cancelled');
    }

    GetState() {
        return {
            state: new GLib.Variant('s', this._capture.state),
            session_id: new GLib.Variant('t', this._capture.activeId),
            available: new GLib.Variant(
                'b', !!pickRecorder(p => !!GLib.find_program_in_path(p))),
        };
    }

    // ---- internals ----

    async _transcribe(id) {
        const wav = this._wav;
        try {
            // Wait for the recorder to exit so the WAV is complete.
            // Transcribing while it is still being written is how you
            // get a transcript that stops mid-word and no clue why.
            await this._proc.wait_async(null);
            this._proc = null;

            if (!wav || !GLib.file_test(wav, GLib.FileTest.EXISTS))
                throw new Error('the recorder produced no audio');

            const argv = transcribeArgv(wav);
            argv[0] = GLib.getenv('LISA_CLI') ?? 'lisa';
            const proc = Gio.Subprocess.new(argv,
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
            const [stdout, stderr] = await proc.communicate_utf8_async(null, null);
            if (!proc.get_successful())
                throw new Error((stderr ?? '').trim() || 'transcription failed');

            const text = cleanTranscript(stdout);
            // settle() before emitting: the transcript must not be
            // delivered under a session that a later press has replaced.
            if (!this._capture.settle(id).ok)
                return;
            this._emit('Transcribed', new GLib.Variant('(ts)', [id, text]));
        } finally {
            this._discard(wav);
        }
    }

    /// Fail a session and say why. Every path out of a recording goes
    /// through here or through _transcribe, so the audio is deleted
    /// exactly once and always.
    _abort(id, reason) {
        this._clearGuard();
        const wav = this._wav;
        if (this._proc) {
            this._proc.force_exit();
            this._proc = null;
        }
        this._discard(wav);
        if (this._capture.activeId === id)
            this._capture.cancel();
        this._emit('Failed', new GLib.Variant('(ts)', [id, reason]));
    }

    /// The recording is deleted as soon as it has been used. Nothing
    /// keeps audio of somebody speaking, and no surface is ever handed a
    /// path it could keep.
    _discard(wav) {
        if (this._wav === wav)
            this._wav = null;
        if (!wav)
            return;
        try {
            Gio.File.new_for_path(wav).delete(null);
        } catch {
            // Already gone is the expected case on the second call.
        }
    }

    _clearGuard() {
        if (this._guard) {
            GLib.source_remove(this._guard);
            this._guard = 0;
        }
    }

    _emit(signal, variant) {
        this._connection.emit_signal(
            null, VOICE_OBJECT_PATH, VOICE_BUS_NAME, signal, variant);
    }
}
