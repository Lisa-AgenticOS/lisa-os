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

# Which stage to run: generate | augment | train | all (default).
#
# Split because the stages want different machines. Generating 10,000
# samples is VITS inference and takes a GPU well — piper-sample-generator
# uses MPS when it is there. Training is a small classifier over
# precomputed features and is happy on any CPU, but it needs the 16 GiB
# corpus and 10 GiB of noise nearby. So: generate on the fast machine,
# rsync the clips, train where the disk is.
STEP="${1:-all}"
case "$STEP" in
    generate|augment|train|all) ;;
    *) echo "usage: $(basename "$0") [generate|augment|train|all]" >&2; exit 2 ;;
esac
# Only the stages that consume the big corpora should insist on them.
wants_corpora=0
[ "$STEP" = "augment" ] || [ "$STEP" = "train" ] || [ "$STEP" = "all" ] && wants_corpora=1

# Pinned so a rerun trains the same thing. openWakeWord's training code
# moves; an unpinned clone means the model you ship and the model you
# reproduce are different models.
OWW_COMMIT="${LISA_OWW_COMMIT:-main}"
PSG_RELEASE="v1.0.0"

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
# `df -Pk`, not `df -g`. -g is a BSD/macOS spelling that GNU coreutils
# rejects outright ("df: invalid option -- 'g'"), so the first version of
# this line worked on the laptop it was written on and died on the first
# line on Linux — which is where this is actually meant to run.
avail_gib=$(df -Pk "$WORK" | tail -1 | awk '{printf "%d", $4/1024/1024}') || avail_gib=""
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
    # torch from the CPU index. The default Linux wheel depends on a
    # stack of nvidia-cuda-* packages — roughly 2.5 GiB — and there is no
    # NVIDIA GPU in any machine this targets: the reference iMac has a
    # Radeon Pro 560, which torch does not use either. Training here is
    # CPU regardless, so the CUDA payload is pure download and disk.
    uv pip install --python "$PY" \
        --index-url https://download.pytorch.org/whl/cpu \
        torch torchaudio
    uv pip install --python "$PY" \
        torchinfo torchmetrics \
        onnx onnxruntime tqdm 'scipy<1.15' scikit-learn pyyaml huggingface_hub \
        speechbrain audiomentations torch-audiomentations acoustics \
        pronouncing mutagen

    # webrtcvad is needed ONLY by the `generate` stage, and this is the
    # sharpest reason the stages are split across machines.
    #
    # openWakeWord itself never imports it — its vad.py runs Silero
    # through onnxruntime. But dscripka's generate_samples.py does, and
    # webrtcvad is a C extension with no wheel: it must be compiled. Lisa
    # OS has an immutable root with no cc on it, so the iMac can train
    # but cannot generate, whatever its disk. That is not a limitation to
    # work around — it is the reason `generate` belongs on a development
    # machine and `augment`/`train` belong where the corpora are.
    #
    # setuptools<81 because webrtcvad imports pkg_resources, which
    # setuptools removed in 81. Unpinned, the import fails with
    # "No module named 'pkg_resources'" and nothing points at setuptools.
    if [ "$STEP" = generate ] || [ "$STEP" = all ]; then
        # espeak-phonemizer turns the target phrase into phonemes for
        # the VITS model. It binds libespeak-ng through ctypes, so the
        # library has to exist on the machine (brew install espeak-ng /
        # pacman -S espeak-ng) — the pip package alone imports and then
        # fails at first use.
        uv pip install --python "$PY" 'setuptools<81' webrtcvad espeak-phonemizer || {
            echo "!! could not build webrtcvad — this machine has no C compiler." >&2
            echo "   Run '$(basename "$0") generate' on a dev machine, rsync" >&2
            echo "   the clips over, and continue with 'augment' here." >&2
            exit 1
        }
    fi
    touch .deps-installed
fi

# --- sources --------------------------------------------------------
[ -d openWakeWord ] || { log "cloning openWakeWord"; git clone https://github.com/dscripka/openWakeWord.git; }
( cd openWakeWord && git checkout -q "$OWW_COMMIT" )
uv pip install --python "$PY" -e ./openWakeWord >/dev/null

# dscripka's FORK, not rhasspy's upstream. openWakeWord's train.py does
# `from generate_samples import generate_samples` after putting this
# directory on sys.path, and rhasspy restructured at v2.0.0 into a
# piper_sample_generator/ package with no such module at the root. The
# fork keeps the flat layout train.py was written against.
#
# The symptom is a bare "No module named 'generate_samples'", which
# reads like a missing dependency and is really a repository that moved.
[ -d piper-sample-generator ] || {
    log "cloning piper-sample-generator (dscripka fork)"
    git clone https://github.com/dscripka/piper-sample-generator.git
}
# en-us-libritts-high, from rhasspy's v1.0.0 release — the file name and
# the release BOTH matter. train.py calls generate_samples() without a
# model argument, so the fork's compiled-in default path is what gets
# opened, and that default is models/en-us-libritts-high.pt. Handing it
# v2.0.0's en_US-libritts_r-medium.pt (which is what openWakeWord's
# notebook wgets, for its own different call path) produces a
# FileNotFoundError naming a file nobody asked for. The fork's README is
# the authority here, not the notebook.
#
# Same family and same reasoning as the voice this OS pins for speech
# out — multi-speaker LibriTTS, so ten thousand samples sound like ten
# thousand people — but a different checkpoint, so do not "tidy" the two
# into one.
if [ ! -f piper-sample-generator/models/en-us-libritts-high.pt ]; then
    log "fetching the multi-speaker generator checkpoint (243 MiB)"
    mkdir -p piper-sample-generator/models
    curl -fL --retry 3 -o piper-sample-generator/models/en-us-libritts-high.pt \
        "https://github.com/rhasspy/piper-sample-generator/releases/download/${PSG_RELEASE}/en-us-libritts-high.pt"
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
if [ "$wants_corpora" = 1 ]; then
fetch "https://huggingface.co/datasets/davidscripka/openwakeword_features/resolve/main/validation_set_features.npy" \
      "validation_set_features.npy" "false-positive validation features (~180 MiB)"
fetch "https://huggingface.co/datasets/davidscripka/openwakeword_features/resolve/main/openwakeword_features_ACAV100M_2000_hrs_16bit.npy" \
      "openwakeword_features_ACAV100M_2000_hrs_16bit.npy" "ACAV100M negative features (16 GiB — the long one)"
fi

# Emptiness, not existence. The generate stage creates an empty mit_rirs
# as a placeholder for train.py's module-level scan, so a bare -d test
# would skip this download and augment against nothing.
if [ "$wants_corpora" = 1 ] && { [ ! -d mit_rirs ] || [ -z "$(ls -A mit_rirs 2>/dev/null)" ]; }; then
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
# that works on the developer's desk and nowhere else.
#
# MUSAN rather than the notebook's AudioSet/FMA: it exists for exactly
# this job (music, noise and speech, already 16 kHz plain wavs), it is
# CC BY 4.0 so it can be named here without a licence question, and it
# needs no parquet reader or audio decoder — the dependency that has
# already broken this script twice.
if [ "$wants_corpora" = 1 ] && { [ ! -d background_clips ] || [ -z "$(ls -A background_clips 2>/dev/null)" ]; }; then
    if [ ! -f musan.tar.gz ]; then
        log "background noise: MUSAN (10.3 GiB, CC BY 4.0 — the long one)"
        curl -fL --retry 3 -C - -o musan.tar.gz "https://openslr.org/resources/17/musan.tar.gz"
    fi
    [ -d musan ] || { log "extracting MUSAN"; tar xf musan.tar.gz; }
    log "collecting background clips"
    mkdir -p background_clips
    # Flattened with symlinks: MUSAN nests three levels deep, and a flat
    # directory is safe whatever globbing the augmentation does. Symlinks
    # rather than copies so this costs no second 10 GiB.
    find musan -name '*.wav' -type f -exec ln -sf "$PWD/{}" background_clips/ \; 2>/dev/null || true
    n=$(ls -A background_clips 2>/dev/null | wc -l | tr -d ' ')
    echo "  $n background clips"
    if [ "$n" -lt 100 ]; then
        echo "!! only $n background clips — augmentation would be nearly a no-op" >&2
        exit 1
    fi
fi

if [ "$STEP" = generate ] || [ "$STEP" = all ]; then
    # train.py resolves background_paths and rir_paths at module level,
    # before it looks at which stage was asked for — so --generate_clips
    # dies on a missing ./background_clips it will never read. Empty
    # placeholders satisfy the scan without pretending to be corpora;
    # the augment stage builds the real ones, and refuses if they are
    # still empty when it actually needs them.
    mkdir -p background_clips mit_rirs
    log "generating positive + adversarial samples (piper does the talking)"
    "$PY" "$TRAIN" --training_config ./hey-lisa.yml --generate_clips
fi

if [ "$STEP" = augment ] || [ "$STEP" = all ]; then
    log "augmenting and computing features"
    "$PY" "$TRAIN" --training_config ./hey-lisa.yml --augment_clips
fi

if [ "$STEP" = train ] || [ "$STEP" = all ]; then
    log "training the classifier"
    "$PY" "$TRAIN" --training_config ./hey-lisa.yml --train_model
fi

log "done — model in $WORK/hey_lisa_model"
ls -la "$WORK/hey_lisa_model" 2>/dev/null || true
echo
echo "NOT SHIPPABLE YET. Measure the false-accept rate before pinning it"
echo "in models/catalog/catalog.toml — hey-lisa.yml sets the bar at"
echo "0.2 false positives/hour, and that number is the whole decision."
