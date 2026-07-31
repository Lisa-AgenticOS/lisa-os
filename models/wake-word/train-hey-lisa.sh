#!/usr/bin/env bash
# Train the "Hey Lisa" wake word (ADR-0011, PLAN §5.7.5).
#
# WHY THIS EXISTS
# ADR-0011 names openWakeWord with "Hey Lisa" as the shipping default.
# No such model exists: upstream publishes alexa, hey_jarvis,
# hey_mycroft, hey_rhasspy, timer and weather, and nothing on
# HuggingFace supplies ours. It has to be trained, and a wake word
# trained once by hand on somebody's laptop is a wake word nobody can
# ever retrain — so the recipe is a script in the repo rather than a
# notebook run from memory.
#
# HOW IT WORKS
# openWakeWord trains a small classifier on top of a frozen speech
# embedding. Positives are synthesised: upstream's own docs call for a
# multi-speaker VITS model, and piper IS a VITS model — the generator
# checkpoint below is en_US-libritts_r-medium, the SAME voice this OS
# pins for speech out (models/catalog). 904 speakers is what makes ten
# thousand samples sound like ten thousand people rather than one.
#
# RESUMABLE ON PURPOSE. It downloads ~17 GiB and runs for hours; every
# step checks for its own output first, so an interrupted run continues
# instead of starting again.
#
# LIMITS
# It does not decide whether the model is good enough. That is
# target_false_positives_per_hour in hey-lisa.yml, measured after
# training, and the reason nothing is pinned in the catalog yet: a wake
# word that fires unprompted is the fastest way to lose trust in an
# always-on assistant.
set -euo pipefail

WORK="${LISA_WW_WORK:-$HOME/.cache/lisa/wake-word}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="$HERE/hey-lisa.yml"

# Pinned so a rerun trains the same thing. openWakeWord's training code
# moves; an unpinned clone means the model you ship and the model you
# reproduce are different models.
OWW_COMMIT="${LISA_OWW_COMMIT:-main}"
PSG_RELEASE="v2.0.0"

log() { printf '\n=== %s\n' "$*"; }

# --- preflight ------------------------------------------------------
# The feature corpus alone is 16 GiB. Finding that out after a
# four-hour sample-generation run is the failure this prevents.
need_gib=30
# mkdir FIRST. `df` on a path that does not exist yet fails, and under
# `set -euo pipefail` that killed this script before it printed a single
# line — with df's stderr sent to /dev/null, the failure was completely
# silent. Exactly the shape of bug this repo keeps finding.
mkdir -p "$WORK"
avail_gib=$(df -g "$WORK" | tail -1 | awk '{print $4}') || avail_gib=""
# Subtract what is already cached. The 30 GiB figure is the FRESH-run
# cost; on a resume most of it is on disk, and comparing the full figure
# against remaining free space refuses to continue a run that is three
# quarters done. That is not hypothetical — it happened here, right
# after the 16 GiB download finally succeeded.
cached_gib=$(du -sk "$WORK" 2>/dev/null | awk '{printf "%d", $1/1024/1024}') || cached_gib=0
still_gib=$(( need_gib - ${cached_gib:-0} ))
[ "$still_gib" -lt 5 ] && still_gib=5   # always keep working headroom
if [ -n "${avail_gib:-}" ] && [ "$avail_gib" -lt "$still_gib" ]; then
    echo "!! need ~${still_gib} GiB more free, have ${avail_gib} GiB at $WORK" >&2
    echo "   (${cached_gib:-0} GiB already cached; a fresh run needs ~${need_gib})" >&2
    exit 1
fi
command -v uv >/dev/null || { echo "!! uv is required (https://docs.astral.sh/uv/)" >&2; exit 1; }

echo "workdir: $WORK  (${avail_gib:-?} GiB free)"
cd "$WORK"

# --- environment ----------------------------------------------------
# Its own interpreter, not the host's: this machine runs Python 3.14,
# which PyTorch does not support. Pinning 3.11 here is what stops that
# being discovered as an import error two steps in.
if [ ! -d .venv ]; then
    log "creating the training environment (python 3.11)"
    uv venv --python 3.11 .venv
fi
PY="$WORK/.venv/bin/python"

if [ ! -f .deps-installed ]; then
    log "installing torch and the openWakeWord training stack"
    # Every pin here was found by a run dying on it, hours in. They are
    # listed with the reason so the next person does not rediscover them
    # one failure at a time:
    #
    #   scipy<1.15   — `acoustics` imports scipy.special.sph_harm, which
    #                  scipy removed. Without the pin, `import acoustics`
    #                  fails and openwakeword.data cannot load at all.
    #   pronouncing, torch-audiomentations, acoustics
    #                — imported by openwakeword/data.py but absent from
    #                  its install_requires, so `pip install -e` leaves
    #                  them out and training dies at the first import.
    uv pip install --python "$PY" \
        torch torchaudio torchinfo torchmetrics \
        onnx onnxruntime tqdm 'scipy<1.15' scikit-learn pyyaml huggingface_hub \
        speechbrain audiomentations torch-audiomentations acoustics \
        pronouncing webrtcvad mutagen
    touch .deps-installed
fi

# --- sources --------------------------------------------------------
[ -d openWakeWord ] || { log "cloning openWakeWord"; git clone https://github.com/dscripka/openWakeWord.git; }
( cd openWakeWord && git checkout -q "$OWW_COMMIT" )
uv pip install --python "$PY" -e ./openWakeWord >/dev/null

[ -d piper-sample-generator ] || {
    log "cloning piper-sample-generator"
    git clone https://github.com/rhasspy/piper-sample-generator.git
}
# The generator checkpoint: en_US-libritts_r-medium, 904 speakers — the
# same voice models/catalog pins for TTS, which is not a coincidence.
# openWakeWord and this OS independently chose it for the same reason.
if [ ! -f piper-sample-generator/models/en_US-libritts_r-medium.pt ]; then
    log "fetching the multi-speaker generator checkpoint (~200 MiB)"
    mkdir -p piper-sample-generator/models
    curl -fL --retry 3 -o piper-sample-generator/models/en_US-libritts_r-medium.pt \
        "https://github.com/rhasspy/piper-sample-generator/releases/download/${PSG_RELEASE}/en_US-libritts_r-medium.pt"
fi

# --- corpora --------------------------------------------------------
# Augmentation is what makes the model survive a real room. Without
# impulse responses and background noise it learns "Hey Lisa recorded in
# a silent studio", which is not where anybody uses a computer.
fetch() {  # url dest description
    [ -s "$2" ] && { echo "  have $(basename "$2")"; return; }
    log "downloading $3"
    curl -fL --retry 3 -C - -o "$2" "$1"
}
fetch "https://huggingface.co/datasets/davidscripka/openwakeword_features/resolve/main/validation_set_features.npy" \
      "validation_set_features.npy" "false-positive validation features (~180 MiB)"
fetch "https://huggingface.co/datasets/davidscripka/openwakeword_features/resolve/main/openwakeword_features_ACAV100M_2000_hrs_16bit.npy" \
      "openwakeword_features_ACAV100M_2000_hrs_16bit.npy" "ACAV100M negative features (16 GiB — the long one)"

if [ ! -d mit_rirs ]; then
    log "room impulse responses (MIT survey, 270 files)"
    # Fetched as plain files rather than through the `datasets` library.
    #
    # That repository is 270 ordinary 16 kHz wavs under 16khz/, so a
    # dataset loader buys nothing here and costs a great deal: datasets 5
    # decodes audio via torchcodec, which wants FFmpeg, while releases
    # before 3 use pa.PyExtensionType, which pyarrow removed in v15. Both
    # failure modes were hit in turn — each one AFTER the 16 GiB download
    # had already succeeded. A fragile dependency in the middle of a
    # multi-hour job is worth deleting rather than pinning around.
    "$PY" - <<'RIRPY'
import glob, os, shutil
from huggingface_hub import snapshot_download
d = snapshot_download(
    repo_id="davidscripka/MIT_environmental_impulse_responses",
    repo_type="dataset", allow_patterns="16khz/*.wav")
os.makedirs("mit_rirs", exist_ok=True)
n = 0
for w in glob.glob(os.path.join(d, "16khz", "*.wav")):
    shutil.copy(w, os.path.join("mit_rirs", os.path.basename(w)))
    n += 1
print(f"  {n} impulse responses")
if n == 0:
    # Silently having no RIRs makes augmentation a no-op and yields a
    # model that only works in a quiet room, with nothing to explain it.
    raise SystemExit("no impulse responses downloaded")
RIRPY
fi

# --- train ----------------------------------------------------------
cp "$CONFIG" ./hey-lisa.yml

# Run from $WORK, NOT from inside openWakeWord. train.py resolves every
# path in the config with os.path.abspath(), i.e. against the working
# directory — so `cd openWakeWord` silently repoints
# piper_sample_generator_path, mit_rirs, background_paths and the two
# .npy corpora at a directory none of them are in. The first symptom is
# a bare "No module named 'generate_samples'", which reads like a
# missing dependency rather than a wrong cwd.
TRAIN="$WORK/openWakeWord/openwakeword/train.py"

# Background audio is not optional. Without it the model is trained on
# speech in silence and learns "Hey Lisa in a quiet room", which is not
# where anybody uses a computer — and the way that fails is a wake word
# that works on the developer's desk and nowhere else. Refuse rather
# than quietly train something weaker than the config claims.
if [ ! -d background_clips ] || [ -z "$(ls -A background_clips 2>/dev/null)" ]; then
    cat >&2 <<'MSG'
!! background_clips/ is empty — refusing to train.

   The config asks for background noise to augment against. Populate it
   with a few thousand short 16 kHz clips (the upstream notebook uses
   AudioSet and FMA; any broad noise/speech corpus works), then re-run.
   Roughly 5-10 GiB, and this machine currently has less headroom than
   that once the 16 GiB feature file is accounted for.

   Training without it is possible by setting augmentation_rounds: 0 in
   hey-lisa.yml, but that model will not survive a real room, so it is a
   deliberate choice rather than a default.
MSG
    exit 1
fi

log "generating positive + adversarial samples (hours; piper does the talking)"
"$PY" "$TRAIN" --training_config ./hey-lisa.yml --generate_clips

log "augmenting and computing features"
"$PY" "$TRAIN" --training_config ./hey-lisa.yml --augment_clips

log "training the classifier"
"$PY" "$TRAIN" --training_config ./hey-lisa.yml --train_model

log "done — model in $WORK/hey_lisa_model"
ls -la "$WORK/hey_lisa_model" 2>/dev/null || true
echo
echo "NOT SHIPPABLE YET. Measure the false-accept rate before pinning it"
echo "in models/catalog/catalog.toml — hey-lisa.yml sets the bar at"
echo "0.2 false positives/hour, and that number is the whole decision."
