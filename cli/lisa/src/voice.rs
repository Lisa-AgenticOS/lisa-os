//! Voice + ambient loop for the CLI (`docs/PLAN.md` §5.7.5, ADR-0011).
//!
//! `transcribe` (whisper.cpp) → `ambient classify` (addressed-intent via
//! guided generation) → answer → `say` (piper / local TTS). This is the
//! Lisa Ambient loop minus live mic capture + the wake word, driven from
//! an audio file so it runs and tests without hardware. The daemon path
//! (inferenced supervising whisper-server, a `voiced` capture loop) is
//! the productionization — this proves the pieces.

use anyhow::{Context, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve a whisper ggml model: --model, $LISA_WHISPER_MODEL, or the
/// store's `whisper` ref.
pub fn whisper_model(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    if let Some(p) = std::env::var_os("LISA_WHISPER_MODEL") {
        return Ok(PathBuf::from(p));
    }
    let store_ref = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".local/share/lisa/models/refs/whisper"))
        .filter(|p| p.exists());
    store_ref.ok_or_else(|| {
        anyhow::anyhow!(
            "no whisper model — pass --model, set LISA_WHISPER_MODEL, or \
             `lisa models get whisper-base-en`"
        )
    })
}

/// Transcribe an audio file with whisper.cpp. Returns the plain text.
pub fn transcribe(audio: &Path, model: &Path) -> anyhow::Result<String> {
    if !audio.exists() {
        bail!("audio file {} not found", audio.display());
    }
    let out = Command::new("whisper-cli")
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .arg("-nt") // no timestamps
        .output()
        .context("running whisper-cli — is whisper.cpp installed?")?;
    if !out.status.success() {
        bail!(
            "whisper-cli failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    ledger_activation(audio, &text);
    Ok(text)
}

/// Record that the microphone was used (ADR-0011, invariant 3: *every
/// activation is in the Ledger*).
///
/// This sits in `transcribe` rather than in each caller because every
/// path that turns audio into words comes through here — `lisa listen`,
/// `lisa transcribe`, `lisa ambient`, and the push-to-talk service,
/// which shells out to this same verb. Ledgering per caller is how one
/// of them ends up not doing it.
///
/// What is written is the *shape* of the activation: how long the
/// recording was and a bounded preview. The full transcript is hashed,
/// not stored — the Ledger answers "what did it hear?" without becoming
/// a second copy of everything said near the machine.
///
/// A Ledger that cannot be opened must not stop somebody transcribing a
/// file. That is the opposite of the agent-loop rule (#129), where a
/// call that cannot be recorded must not run — the difference is that
/// this is a person acting on their own machine, and a guardrail never
/// sits between them and it (CLAUDE.md 6a).
fn ledger_activation(audio: &Path, text: &str) {
    let seconds = std::fs::metadata(audio)
        .map(|m| {
            // 16 kHz mono 16-bit == 32000 bytes/s, minus the header.
            // Approximate on purpose: the point is "about six seconds",
            // not a sample count.
            m.len().saturating_sub(44) as f64 / 32_000.0
        })
        .unwrap_or(0.0);
    let preview: String = text.chars().take(160).collect();
    let event = lisa_ledger::Event {
        kind: "voice.transcribe".into(),
        app_id: "host".into(),
        model: "whisper".into(),
        input_hash: blake3::hash(text.as_bytes()).to_hex().to_string(),
        preview,
        status: "ok".into(),
        detail: format!("{seconds:.1}s of audio, transcribed locally"),
        ref_id: None,
        output_tokens: 0,
        duration_ms: 0,
    };
    if let Ok(l) = lisa_ledger::Ledger::open(lisa_ledger::Ledger::default_path())
        && let Err(e) = l.append(&event)
    {
        eprintln!("warning: could not ledger this transcription: {e}");
    }
}

/// Record from the microphone, then transcribe it. This is push-to-talk
/// without a key to hold: the capture half that `transcribe` (file in)
/// and `say` (audio out) were both written against and neither had.
///
/// Recording stops at `seconds` or when the user interrupts, whichever
/// comes first. `keep` writes the audio somewhere permanent — useful
/// when a transcript is wrong and the question is whether the microphone
/// or the model is at fault.
pub fn listen(seconds: u32, model: &Path, keep: Option<&Path>) -> anyhow::Result<String> {
    let wav = match keep {
        Some(p) => p.to_path_buf(),
        None => std::env::temp_dir().join(format!("lisa-listen-{}.wav", std::process::id())),
    };

    // 16 kHz mono 16-bit: what whisper.cpp wants. Handing it 44.1 kHz
    // stereo makes it resample internally and transcribe slightly worse,
    // for no gain — nothing here is listening to music.
    let rec = recorder().context(
        "no recorder found (tried pw-record, parecord, arecord) — \
         on the image these come from pipewire and alsa-utils",
    )?;
    let args: Vec<&str> = match rec {
        "arecord" => vec!["-q", "-f", "S16_LE", "-r", "16000", "-c", "1", "-d"],
        // pw-record and parecord take the duration differently: they run
        // until signalled, so the ceiling is applied by timeout(1)-style
        // handling below rather than by a flag.
        _ => vec![],
    };

    eprintln!("listening for up to {seconds}s — speak now, Ctrl-C to stop");
    let status = if rec == "arecord" {
        let secs = seconds.to_string();
        Command::new(rec)
            .args(&args)
            .arg(&secs)
            .arg(&wav)
            .status()
            .with_context(|| format!("running {rec}"))?
    } else {
        // pw-record/parecord stop on signal. `timeout` is coreutils and
        // is present wherever these are.
        Command::new("timeout")
            .arg(seconds.to_string())
            .arg(rec)
            .arg("--rate=16000")
            .arg("--channels=1")
            .arg("--format=s16")
            .arg(&wav)
            .status()
            .with_context(|| format!("running {rec} under timeout"))?
    };
    // `timeout` exits 124 when it fires, which here is the NORMAL end of
    // a recording, not a failure. Treating it as an error is how this
    // would reject every recording that ran to the ceiling.
    let timed_out = status.code() == Some(124);
    if !status.success() && !timed_out {
        bail!("{rec} failed while recording (exit {:?})", status.code());
    }

    let bytes = std::fs::read(&wav)
        .with_context(|| format!("{rec} produced no recording at {}", wav.display()))?;
    if !is_wav(&bytes) {
        bail!(
            "{rec} wrote {} bytes and not a WAV — is a microphone connected?",
            bytes.len()
        );
    }
    // A header with no samples after it is a microphone that is muted or
    // permission-denied. whisper would transcribe it as silence and the
    // user would blame the model.
    if bytes.len() <= 44 {
        bail!("recorded a header and no audio — check the microphone is not muted");
    }

    let text = transcribe(&wav, model)?;
    if keep.is_none() {
        let _ = std::fs::remove_file(&wav);
    }
    Ok(text)
}

/// First available recorder. pw-record is native to the image's PipeWire
/// stack; arecord comes from alsa-utils, which is in the image so sound
/// can be proved below PipeWire (ADR-0024).
fn recorder() -> Option<&'static str> {
    ["pw-record", "parecord", "arecord"]
        .into_iter()
        .find(|b| which(b))
}

/// Speak text locally: piper on the image, `say` on a macOS dev host.
pub fn say(text: &str) -> anyhow::Result<()> {
    if which("piper") {
        match piper_voice() {
            Some(voice) => return say_with_piper(text, &voice),
            None => {
                // piper is installed but has nothing to speak with.
                // Falling silently through to the print branch would
                // read as "no TTS on this machine", which is a
                // different problem with a different fix.
                eprintln!(
                    "piper is installed but no voice is: run \
                     `lisa models get piper-libritts-r-medium-en-us`, \
                     or set LISA_PIPER_VOICE"
                );
            }
        }
    }
    if which("say") {
        Command::new("say").arg(text).status()?;
        return Ok(());
    }
    // No TTS available: print so the loop still completes.
    println!("[tts unavailable] {text}");
    Ok(())
}

/// Resolve a piper voice: `$LISA_PIPER_VOICE`, else the store ref the
/// catalog installs. Mirrors `whisper_model` — same store, same shape.
///
/// The `.onnx.json` beside it is not named here on purpose: piper's own
/// default for `--config` is the model path plus `.json`, and
/// `lisa models get` installs the config as `<id>.json` for exactly that
/// reason. One naming convention, agreed in two places.
pub fn piper_voice() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LISA_PIPER_VOICE") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
        return None;
    }
    let refs = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".local/share/lisa/models/refs");
    // The catalog id first, then a plain `voice` alias for anyone who
    // installed one by hand with `lisa models add`.
    for name in ["piper-libritts-r-medium-en-us", "voice"] {
        let p = refs.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Synthesize with piper and play the result.
///
/// Written this way after three defects in the previous version, all of
/// which produced a confident silence rather than an error:
///
///   - the voice path was `/usr/share/lisa/voices/en_US.onnx`, which
///     nothing in the image or the model store ever creates;
///   - `--output-raw` is not a piper flag (the CLI takes `-f -`), so the
///     31 bytes it emitted were a usage message, not audio;
///   - the child's stdout was captured and dropped, and every failure
///     path returned `Ok(())`, so a totally broken TTS reported success.
///
/// So: write a real WAV to a temp file, check it is a WAV, then play it,
/// and return an error whenever any of that does not happen.
fn say_with_piper(text: &str, voice: &Path) -> anyhow::Result<()> {
    use std::io::Write;

    let wav = std::env::temp_dir().join(format!("lisa-say-{}.wav", std::process::id()));
    let mut child = Command::new("piper")
        .arg("--model")
        .arg(voice)
        .arg("--output_file")
        .arg(&wav)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawning piper")?;
    child
        .stdin
        .take()
        .context("piper stdin")?
        .write_all(text.as_bytes())
        .context("writing text to piper")?;
    let out = child.wait_with_output().context("waiting for piper")?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&wav);
        bail!(
            "piper failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    // A zero-byte or non-RIFF file means piper "succeeded" without
    // producing audio. Playing it would be silence with no explanation.
    let header = std::fs::read(&wav)
        .with_context(|| format!("piper reported success but wrote no {}", wav.display()))?;
    if !is_wav(&header) {
        let _ = std::fs::remove_file(&wav);
        bail!(
            "piper produced {} bytes and not a WAV — the voice or its \
             .onnx.json is probably wrong",
            header.len()
        );
    }

    let played = play(&wav);
    let _ = std::fs::remove_file(&wav);
    played
}

/// Is this actually a RIFF/WAVE file with room for a header?
///
/// The guard that turns "piper exited 0 and the speakers stayed quiet"
/// into an error somebody can act on. 44 bytes is the canonical header
/// length, so anything shorter cannot carry one whatever its magic says.
fn is_wav(bytes: &[u8]) -> bool {
    bytes.len() >= 44 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

/// Play a WAV. pw-play is native on the image's PipeWire stack; aplay is
/// the ALSA fallback that `alsa-utils` guarantees (it is in the image so
/// sound can be proved below PipeWire — see ADR-0024/issue #44, where
/// every session-level indicator was green and the speakers were mute).
fn play(wav: &Path) -> anyhow::Result<()> {
    for player in ["pw-play", "paplay", "aplay", "afplay"] {
        if !which(player) {
            continue;
        }
        let st = Command::new(player)
            .arg(wav)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .with_context(|| format!("running {player}"))?;
        if st.success() {
            return Ok(());
        }
        // A player that exists and fails is worth reporting rather than
        // silently trying the next one, but not worth giving up over.
        eprintln!("{player} could not play the reply; trying another");
    }
    bail!("no working audio player (tried pw-play, paplay, aplay, afplay)")
}

fn which(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Wake-word gate (ADR-0011: "Hey Lisa" is the shipping default). True
/// when the utterance starts by addressing Lisa. Reliable on any model —
/// unlike the addressed-intent classifier, which over-triggers on small
/// models (a 1B model called a "let's order pizza" aside addressed).
/// Returns the request with the wake phrase stripped.
pub fn wake_word(transcript: &str) -> Option<String> {
    let lower = transcript.trim_start().to_ascii_lowercase();
    for phrase in ["hey lisa", "ok lisa", "okay lisa", "lisa,"] {
        if lower.starts_with(phrase) {
            let rest = transcript.trim_start()[phrase.len()..]
                .trim_start_matches([',', '.', '!', '?', ' '])
                .trim();
            return Some(rest.to_string());
        }
    }
    None
}

/// The addressed-intent decision (ADR-0011): {addressed, confidence,
/// intent}, produced by guided generation against the local model.
#[derive(Debug)]
pub struct Addressed {
    pub addressed: bool,
    pub confidence: f64,
    pub intent: String,
}

/// Classify whether `transcript` was addressed to Lisa, via the endpoint.
pub fn classify_addressed(transcript: &str, url: &str) -> anyhow::Result<Addressed> {
    let body = liblisa::tasks::addressed_intent().request(transcript);
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let mut resp = ureq::post(&endpoint)
        .send_json(&body)
        .with_context(|| format!("request to {endpoint} — is lisa-inferenced running?"))?;
    let json: serde_json::Value = resp.body_mut().read_json()?;
    if let Some(err) = json["error"]["message"].as_str() {
        bail!("inference error: {err}");
    }
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");
    let v: serde_json::Value =
        serde_json::from_str(content).with_context(|| format!("classifier output: {content}"))?;
    Ok(Addressed {
        addressed: v["addressed"].as_bool().unwrap_or(false),
        confidence: v["confidence"].as_f64().unwrap_or(0.0),
        intent: v["intent"].as_str().unwrap_or("").to_string(),
    })
}

/// One non-streaming answer from the local model (used by the ambient
/// loop to speak a reply).
pub fn answer(prompt: &str, url: &str) -> anyhow::Result<String> {
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let body = serde_json::json!({
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 200,
    });
    let mut resp = ureq::post(&endpoint)
        .send_json(&body)
        .with_context(|| format!("request to {endpoint} — is lisa-inferenced running?"))?;
    let json: serde_json::Value = resp.body_mut().read_json()?;
    if let Some(err) = json["error"]["message"].as_str() {
        bail!("inference error: {err}");
    }
    Ok(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_word_gates_and_strips() {
        assert_eq!(
            wake_word("Hey Lisa, what is the capital of France?").as_deref(),
            Some("what is the capital of France?")
        );
        assert_eq!(
            wake_word("OK Lisa turn off the lights").as_deref(),
            Some("turn off the lights")
        );
        assert_eq!(wake_word("I think we should order pizza tonight"), None);
        assert_eq!(wake_word("the mona lisa is a painting"), None);
    }

    /// The previous `say` reported success while producing no sound at
    /// all: it passed a flag piper does not have, captured the 31-byte
    /// usage message it got back, threw it away, and returned Ok. This
    /// is the check that makes that outcome an error instead.
    #[test]
    fn only_a_real_wav_counts_as_speech() {
        let mut wav = Vec::from(*b"RIFF\0\0\0\0WAVE");
        wav.resize(44, 0);
        assert!(is_wav(&wav), "a minimal RIFF/WAVE header is speech");

        // The exact failure that shipped: a short usage message.
        assert!(!is_wav(b"Usage: piper [options]\n"));
        assert!(!is_wav(b""));
        // Right magic, truncated before the header ends — piper dying
        // mid-write must not read as success.
        assert!(!is_wav(b"RIFF\0\0\0\0WAVE"));
        // A file of the right length that is not audio at all.
        assert!(!is_wav(&[0u8; 64]));
        // RIFF, but not WAVE (e.g. an AVI container).
        let mut avi = Vec::from(*b"RIFF\0\0\0\0AVI ");
        avi.resize(44, 0);
        assert!(!is_wav(&avi));
    }

    /// An explicitly-set voice that does not exist must resolve to
    /// nothing, so the caller says "no voice installed" rather than
    /// handing piper a path it will fail on in its own words.
    #[test]
    fn an_absent_explicit_voice_is_not_a_voice() {
        // SAFETY: single-threaded test; no other thread reads the env.
        unsafe { std::env::set_var("LISA_PIPER_VOICE", "/nonexistent/voice.onnx") };
        assert_eq!(piper_voice(), None);
        unsafe { std::env::remove_var("LISA_PIPER_VOICE") };
    }
}
