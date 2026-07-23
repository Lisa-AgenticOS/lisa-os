# Lisa OS — Release Plan

When to cut a release, what to batch before one, and the gate every
release must pass. Companion to `docs/ROADMAP.md` (scope) and the
"dev ⇄ field loop" section there (mechanics).
**Updated: 2026-07-23.**

## Philosophy

Lisa is an immutable A/B image: changes reach devices as **image
updates**, not package installs (`pacman` is inert on the device). Two
release rhythms:

- **Security/heartbeat (automatic):** a weekly boot-gated rebuild carries
  Arch security fixes into a fresh image. No human decision; boot-counting
  rolls back a bad one.
- **Feature (deliberate):** cut when a *coherent, green batch* of work is
  ready — not per-commit. A feature that half-works on the device is worse
  than not shipping it, because Settings/desktop regressions are painful
  to debug over SSH.

We hold for coherent batches instead of releasing every commit: the field
loop (build → update → reboot) is slow, so each release should be worth a
reboot.

## The gate (every release)

- [ ] `just lint && just test` green; CI green on `main`.
- [ ] Image **boot-check passes** in CI (no boot, no release).
- [ ] New/changed daemons start under their sandbox (no-egress intact).
- [ ] Every new architecture decision has an ADR.
- [ ] No surface ships half-wired: if it's in the image, it works or
      degrades gracefully (never a dead button / broken panel).
- [ ] Field-critical fixes from the last cycle are in.

## Unreleased on `main` (since v20260723.9)

| Change | In image via | Ready? |
|---|---|---|
| Timezone/locale/keymap bake (8272f02) | mkosi.conf | ✅ auto |
| `lisa` scoped-ACL + `models …--json` (2669206, 38cfb57) | `lisa-cli` (rebuilt from HEAD) | ✅ auto |
| Injection corpus 600 (2abb668) | tests only | ✅ n/a |
| GJS AI panel — Settings app (38cfb57) | **not packaged** — `lisa-shell` ships overlay/launcher/ledger only | 🟠 needs packaging |
| Native Intelligence panel — g-c-c fork (1089f19, d1dac50) | **not built/wired** | 🟠 needs build + repo + image switch |

Everything in `lisa-cli` and `mkosi.conf` ships automatically on the next
image build. The two AI surfaces do **not** yet.

## Next feature release — "Intelligence"

Goal: the AI settings work end-to-end on the device — you open GNOME
Settings, see **Intelligence**, and Local models populate.

**Batch (in dependency order):**

1. **Package the GJS Settings app** — add `shell/settings` to the
   `lisa-shell` package (install `lisa-settings.js` + `lib/` + the
   `org.lisa.Settings.desktop`). Without it, the native panel's
   "Manage providers" bridge has no target. *Small, low-risk.*
2. **Build + wire the g-c-c fork** — a CI job runs `makepkg` for
   `os/packages/gnome-control-center-lisa` (x86 Arch) and publishes it to
   the Lisa pacman repo layered above `[extra]`. Image then carries our
   Settings automatically. *Medium; the x86 makepkg is the real gate the
   arm64 compile-check can't be.*
3. **Package `lisa-remoted`** — add it to the `lisa` PKGBUILD `pkgname`
   set + a socket unit, so Providers actually work (not just the local
   models). Deferred (ADR-0008 socket bridge, Linux-verify). If it slips,
   the panel still ships useful: Local models are native, and the
   providers bridge opens the GJS app which shows the honest offline
   state. *Can land this release or the next.*

**Release when:**

- [ ] Items 1–2 green in CI; g-c-c package builds x86 clean (compile of
      the panel already verified on arm64: clean under `-Wall -Wextra`).
- [ ] Boot-check passes on the image that includes the forked Settings.
- [ ] Smoke on VM/device: Settings opens → **Intelligence** in the
      sidebar → Local models list populates (`lisa models catalog --json`)
      → "Get" works → providers bridge launches the app.
- [ ] Item 3 either working, or confirmed to degrade gracefully.

Not gated on: voice-on-device (whisper/piper build), the "Lisa" system
section (needs daemon-readable settings) — those are a later batch.

## Do we need a release *now*?

No. The one field-urgent fix (timezone) is already applied live on the
iMac, and it won't re-prompt on that install. A release only becomes
worthwhile when the Intelligence batch is ready — so build toward that,
then cut once. If a fresh USB install is needed before then, a quick
polish release (timezone + current `lisa`) is a one-command
`gh workflow run release`.

## Later batches (sketch)

- **Voice/Ambient on device:** whisper.cpp + piper built into the image;
  `lisa ambient` live-mic; the "Lisa" Settings section (default model,
  wake word) once those are daemon-readable.
- **Agent surfaces:** `lisa tools/call/undo` D-Bus verbs; first
  first-party app that exposes tools.
- **Hardening:** signed sysupdate manifests, dm-verity, `/etc` overlay,
  SSH key-auth (drop the provisional `lisa/lisa`).
