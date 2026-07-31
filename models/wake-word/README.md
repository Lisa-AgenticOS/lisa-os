# models/wake-word — training "Hey Lisa"

Spec: `docs/PLAN.md` §5.7.5, ADR-0011. Read those before changing this.

## What it does

Trains the wake word ADR-0011 names as the shipping default. **That model
does not exist**: openWakeWord publishes `alexa`, `hey_jarvis`,
`hey_mycroft`, `hey_rhasspy`, `timer` and `weather`, and nothing on
HuggingFace supplies ours (checked 2026-07-31). It has to be made, so
the recipe is a script rather than a notebook run from memory — a wake
word trained once by hand is one nobody can retrain.

## How it works

`train-hey-lisa.sh [generate|augment|train|all]`. openWakeWord trains a
small classifier over a frozen speech embedding; positives are
synthesised with piper, which is a multi-speaker VITS model — exactly
what upstream's own docs call for, and the same family this OS pins for
speech out.

```
bash train-hey-lisa.sh generate    # 10k samples — needs a compiler
bash train-hey-lisa.sh augment     # RIRs + MUSAN noise
bash train-hey-lisa.sh train       # 50k steps
```

`LISA_WW_WORK` sets the work directory. It wants ~30 GiB free.

## Where it can run, and why that is not a preference

Neither development machine can do all of it, for reasons that are
structural rather than fixable:

- **macOS cannot generate.** `espeak-phonemizer` dlopens `libc.so.6`;
  it is Linux-only at the C level. No pin or symlink changes that.
- **Lisa OS cannot generate.** `generate_samples.py` imports
  `webrtcvad`, a C extension with no wheel, and the immutable root has
  no compiler — whatever disk the machine has.

So the whole job runs in one **arm64 Linux container** on an Apple
Silicon host: native speed, glibc, and a compiler. Give it real memory
(`--memory 16g`); batch 50 through a VITS model is OOM-killed under the
default.

## Extending it

The target phrase, the negative phrases that must *not* trigger it, and
the sample counts are all in `hey-lisa.yml`. Retraining for a different
phrase is an edit there and a re-run.

## Limits

**Nothing here decides whether the model is good enough.** That is
`target_false_positives_per_hour: 0.2`, measured after training, and it
is written down before the result is known on purpose — a threshold
chosen once you can see the number is not a threshold. Until it is
measured, `models/catalog/catalog.toml` keeps the `openwakeword` entry
unpinned and push-to-talk stays the only activation, which is what PLAN
§5.7.5 calls the default anyway.

**The upstream pipeline has rotted, and the pins are load-bearing.**
Every one below was found by a run dying on it, usually hours in and
always after the previous step had succeeded:

| Pin / choice | Without it |
|---|---|
| `scipy<1.15` | `acoustics` imports `sph_harm`, removed from scipy |
| `pronouncing`, `torch-audiomentations`, `acoustics` | absent from openWakeWord's `install_requires` |
| `torch==2.5.1` + matching `torchaudio` | 2.6 flipped `torch.load` to `weights_only=True` |
| `setuptools<81` | `webrtcvad` imports `pkg_resources`, removed in 81 |
| **dscripka's** piper-sample-generator | rhasspy restructured at v2.0.0; `generate_samples` is gone from the root |
| checkpoint `en-us-libritts-high.pt` from **v1.0.0** | the fork's compiled-in default; the notebook's v2.0.0 file is a different name |
| plain `huggingface_hub` for RIRs | `datasets` breaks above *and* below — torchcodec at 5, removed pyarrow API before 3 |
| MUSAN, not AudioSet | AudioSet is 943 parquet shards needing an audio decoder; MUSAN is plain 16 kHz wavs, CC BY 4.0 |

Background noise is **not** optional and the script refuses without it:
augmentation against a silent room produces a wake word that works on
the developer's desk and nowhere else.
