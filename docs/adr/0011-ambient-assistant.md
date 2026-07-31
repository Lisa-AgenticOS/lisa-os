# ADR-0011: Lisa Ambient — the always-on, wake-word-free assistant

- **Status:** accepted (design; implementation staged)
- **Date:** 2026-07-23

## Context

The product vision is an assistant that is simply *present* — you speak,
it answers, no "hey Lisa" handshake — and that can see, hear, and read.
This is the single most trust-sensitive feature Lisa will ship:
"always listening" is exactly what makes people distrust Alexa, and
"sees your screen" is what made Recall a scandal. Lisa's whole thesis
(PLAN §1, §4: radical legibility, egress blocked by mechanism) has to be
what makes always-on *acceptable* rather than creepy.

So the design question is not "can we transcribe continuously" — llama
+ whisper make that easy — but "how do we make always-on provably
private, and how do we respond without a wake word without uploading a
live transcript of your life."

## Decision

Ship **Lisa Ambient**: a local, always-on listening loop that responds
only when it decides you addressed it, with privacy enforced by
architecture, not policy.

### The loop (all on-device)

```
mic ─▶ VAD (voice activity) ─▶ [speech segment] ─▶ local STT (whisper)
        │ silence: nothing happens, nothing stored
        ▼
   transcript ─▶ addressed-intent classifier (system model, guided:
                 {addressed: bool, confidence, intent})
        │ not addressed: transcript discarded, ring buffer overwritten
        ▼ addressed
   assistant loop (context: [selection] [screen] [my stuff]) ─▶ answer
        ▼
   local TTS (piper/kokoro) ─▶ speaker
```

### No wake word, done honestly (the novel piece)

Instead of a wake-word model gating the mic, the mic is always
VAD-gated locally, and **the system model classifies whether a
completed utterance was addressed to Lisa** — grammar-constrained to
`{addressed, confidence, intent}` (guided generation, §5.6). Speaking
*near* Lisa is not speaking *to* Lisa; the classifier is what tells them
apart. Wake-word mode (openWakeWord, **"Hey Lisa"**) is the shipping default
(owner-confirmed 2026-07-23) — reliable, low-power, and privacy-obvious;
the wake-word-*free* addressed-intent classifier is **Phase 2**, enabled
once its false-accept rate is measured acceptable on real hardware. The
classifier is built now (`liblisa::tasks::addressed_intent`) and also
serves a useful role *inside* a wake-word turn: disambiguating follow-up
speech without re-triggering the wake word.

### Privacy as mechanism (non-negotiable invariants)

1. **Nothing leaves the device.** The whole loop runs on
   `lisa-inferenced` (STT, classify, generate) and local TTS. Egress
   stays blocked (rule 5); a remote provider is used only if a request's
   scopes are explicitly consented (§5.11) — never for ambient audio.
2. **Nothing is persisted by default.** Audio lives in a fixed-size
   in-process **ring buffer**; segments are transcribed and the audio is
   overwritten. Transcripts of *un-addressed* speech are discarded
   immediately, never indexed. Only an *addressed* exchange is ledgered
   (and only its text envelope, per §5.7.6) — and only pinned to the
   context fabric if the user pins it.
3. **Every activation is in the Ledger.** "Lisa woke up at 15:04, heard
   N seconds, decided addressed=true, answered" — the Ledger app is the
   answer to "what did it hear?", which no competitor can give.
4. **A hard mute that is real.** A global mute cuts the capture thread
   (not just the UI), reflected by a persistent, always-visible
   indicator whenever the mic is live (the §5.7.5 "hardware-LED-style"
   dot). Mute state survives reboot.
5. **Not Recall.** Ambient is audio-on-request-of-speech, never ambient
   *screen* capture. Screen/selection context is pulled only for an
   addressed turn, per-invocation, provenance-tagged `screen`
   (untrusted, §5.7.4). No continuous visual capture, ever.

### Multimodal ("see, hear, read")

- **Hear:** the loop above.
- **Read:** `[selection]` (app-published `selection://current` or
  AT-SPI) and `[my stuff]` (context-fabric scopes) — already primitives
  (§5.6, §5.3).
- **See:** `[this window]` → ScreenCast portal frame → local VLM
  (§5.7.4), pulled only for an addressed turn, with the sharing
  indicator lit.

## Consequences

- Ambient is a strict superset of the existing Super+Space overlay
  (§5.7.1): the overlay is Ambient with the "addressed" decision made by
  a keypress instead of the classifier. They share one backend.
- New failure mode — **false activation** (responding when not
  addressed). Mitigated by: classifier confidence threshold, a visible
  "Lisa is listening/answering" state the user can cancel, and a
  measured false-accept CI/eval gate before Ambient is default.
- Compute: a small always-warm VAD + whisper-small + the resident
  system model. On Tier 0/1 hardware Ambient may fall back to wake-word
  mode (§5.9 power/thermal caps apply; background QoS).
- The addressed-intent classifier and the false-accept eval are new
  eval-harness targets (§11).

## Staging

1. **Substrate — done 2026-07-31, later than this list implied.** For a
   week "now" meant the *code* existed. It could not run: neither
   whisper.cpp nor piper is in Arch, so no device had an engine, and
   `lisa say` pointed at a voice path nothing creates, passed a flag
   piper does not have, and returned success on every failure — it could
   never have produced a sound. Both engines are packaged
   (`os/packages/`), a redistributable voice is pinned (LibriTTS-R,
   CC BY 4.0 — the obvious choices were a licence form or
   non-commercial), and the chain is verified as a round trip: piper
   says a sentence, whisper transcribes it back word for word — and
   both halves were then proven on the reference iMac itself
   (2026-07-31): its own microphone through the packaged whisper, and
   the packaged piper out of its own speakers. That mattered because
   this machine is exactly where "enumerated" and "works" came apart
   before (issue #44, ADR-0024): the speakers were mute while every
   session-level indicator was green, so neither device was assumed.
2. **Addressed-intent classifier (done):** guided-generation module +
   eval fixtures.
2a. **Push-to-talk — done 2026-07-31, and not in the original plan.**
   `lisa listen`, plus `dev.lisaos.Voice1` on the overlay backend and a
   held key in the shell. It was added because stage 3 needs hardware
   and stage 1 needed *something* a person could actually use in the
   meantime — and because it is the honest floor: an explicit key is the
   version of this feature that needs no trust argument at all. It
   satisfies invariants 3 (every activation is ledgered, as
   `voice.transcribe`) and 4's spirit (an indicator is on screen for
   exactly as long as the microphone is open), and it makes invariants
   1, 2 and 5 trivially true — there is no always-on path in the code.
3. **Ambient loop (needs audio hardware / the field iMac):** VAD +
   ring-buffer capture, the *hard* mute of invariant 4, the wake word,
   the overlay backend consuming audio turns. Not started. Nothing in
   this repo records unprompted, and nothing should until this stage is
   built deliberately rather than arrived at.
4. **Multimodal turn:** screen/selection/context attached to an
   addressed turn.
