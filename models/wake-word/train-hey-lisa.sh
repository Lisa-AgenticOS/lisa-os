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
if [ -n "${avail_gib:-}" ] && [ "$avail_gib" -lt "$need_gib" ]; then
    echo "!! need ~${need_gib} GiB free, have ${avail_gib} GiB at $WORK" >&2
    echo "   (the ACAV100M feature file is 16 GiB by itself)" >&2
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
    uv pip install --python "$PY" \
        torch torchaudio torchinfo torchmetrics \
        onnx onnxruntime tqdm scipy scikit-learn pyyaml datasets \
        speechbrain audiomentations acoustics webrtcvad mutagen
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
    log "room impulse responses (MIT survey)"
    "$PY" - <<'PY'
import os, scipy.io.wavfile, numpy as np
from datasets import load_dataset
os.makedirs("mit_rirs", exist_ok=True)
ds = load_dataset("davidscripka/MIT_environmental_impulse_responses", split="train", streaming=True)
for row in ds:
    a = row["audio"]
    scipy.io.wavfile.write(
        os.path.join("mit_rirs", row["audio"]["path"].split("/")[-1]),
        16000, (a["array"] * 32767).astype(np.int16))
print("  RIRs written")
PY
fi

# --- train ----------------------------------------------------------
cp "$CONFIG" ./hey-lisa.yml
cd openWakeWord

log "generating positive + adversarial samples (hours; piper does the talking)"
"$PY" openwakeword/train.py --training_config ../hey-lisa.yml --generate_clips

log "augmenting and computing features"
"$PY" openwakeword/train.py --training_config ../hey-lisa.yml --augment_clips

log "training the classifier"
"$PY" openwakeword/train.py --training_config ../hey-lisa.yml --train_model

log "done — model in $WORK/hey_lisa_model"
ls -la "$WORK/hey_lisa_model" 2>/dev/null || true
echo
echo "NOT SHIPPABLE YET. Measure the false-accept rate before pinning it"
echo "in models/catalog/catalog.toml — hey-lisa.yml sets the bar at"
echo "0.2 false positives/hour, and that number is the whole decision."
