# ADR-0038: Lisa Desktop — a hard fork of GNOME Shell

- **Status:** accepted, partially executed — and widened by ADR-0048 from
  the Shell to the whole desktop experience. Step 1 (design tokens + the
  `check-tokens.py` gate) shipped 2026-08-03. Step 2 lives on
  `lisa-desktop`'s `vendor-gnome-shell-50.3` branch, not in this repo: the
  fork builds from a hash-pinned 50.3 tarball, `provides=`/`conflicts=`
  stock gnome-shell rather than depending on it, and boots headless owning
  `org.gnome.Shell` — with a deliberately EMPTY Lisa delta, because the
  milestone is "can we own this". Nobody has logged into it
  (lisa-desktop#1). `shell/desktop` here is still the extension of the
  extension era, which step 3 absorbs.
- Date: 2026-08-02
- Relates: ADR-0001 (Arch base), ADR-0012 (control-center panel),
  ADR-0035 (the desktop is a prompt), PLAN §3 (desktop environment
  strategy), issue #146
- Supersedes: PLAN §3's "we patch, we don't fork the Shell **yet**"
  (§3:80). This is the Phase 3 decision that line deferred.

## Context

PLAN §3 laid out a two-step plan. Phase 1–2: GNOME base, patched not
forked, because "Shell is extensible enough for our overlay/launcher
surfaces". Phase 3+: "evaluate a purpose-built shell (candidates: custom
GNOME Shell fork, or a wlroots/smithay compositor) once the daemons and
SDK are proven."

The daemons are proven. This is that evaluation, and its answer.

### What the extension route actually bought

A desktop review on 2026-08-02 counted the entire visual delta from
stock GNOME:

- three Shell extensions — 1412 lines of JS
- 178 lines of CSS (44 in `shell/desktop`, 134 in the overlay)
- a 3-line registration patch to gnome-control-center, plus two
  subtractive edits
- GOA rebuilt with two meson flags and no patch at all
- about a dozen gsettings keys

That is a wallpaper, a font name and one new surface. Not because the
work was lazy — `shell/desktop/extension.js` is careful code with good
reasoning in it — but because an extension can only decorate a shell
whose organising idea belongs to somebody else.

### Three things decided it

**1. The failure mode of an extension is silence.** `docs/STATUS.md`
records it happening: `metadata.json` was capped at shell-version 49
while the image shipped GNOME 50, so no extension loaded and the desktop
was stock GNOME with a violet wallpaper. Nobody noticed until a live
run. Every surface Lisa has drawn sits on private API —
`Main.panel._updatePanel()`, `layoutManager._updateHotCorners`,
`Main.sessionMode.panel`, the internal `Dash` — and each GNOME release
can rename any of them, with no error, no log line, and a desktop that
merely looks like somebody else's.

**2. ADR-0035 is not an extension-shaped change.** "The desktop is a
prompt" replaces the *overview* as the organising idea of the shell.
Fighting the overview forever from outside it is a permanent tax that
produces a worse result than owning the code. The one differentiator
that exists on no other desktop — a dock that is simultaneously the
launcher and the prompt — is precisely the part that cannot be bolted
on.

**3. Lisa is an immutable A/B image.** The usual argument against
forking a desktop is that users mix your shell with their distro's
packages and you own the support burden. That cannot happen here: the
root filesystem is the artifact CI tested, replaced wholesale
(ADR-0001). Lisa already ships a patched gnome-control-center on exactly
this reasoning (ADR-0012).

## Decision

**Fork GNOME Shell as Lisa Desktop. Do not fork Mutter.**

That line is the whole decision, and it is where the cost/benefit
inverts:

| Component | Language | Fork? | Why |
|---|---|---|---|
| Mutter | C | **No** | Compositor, window management, input, monitors, Wayland protocol work. Highest maintenance cost, lowest identity return. Upstream, unmodified, forever. |
| GNOME Shell | JS + CSS | **Yes** | Everything a person sees. Interpreted, so it iterates on the real device by copying files. This is where the identity lives. |
| GTK4 / libadwaita | C | **No** | The app toolkit. Themed, never forked. |
| Lisa's apps | GJS | n/a | Already ours. |

Lisa Desktop is a Mutter plugin, which is what GNOME Shell is. It keeps
GNOME's C libraries (`libshell`, `libst`, Clutter/St) and replaces the
JavaScript that drives them.

### The name

**Lisa Desktop.** User-facing strings, the session entry, the
`.desktop` file, the About page. Not "GNOME Shell (Lisa)" — a fork that
apologises for itself in its own title bar is not a product. The GNOME
lineage is credited in the About page and the licence headers, which is
both correct and required.

## What this costs, stated plainly

Nobody should read this ADR later and think the price was hidden.

- **We own the JavaScript.** Wayland protocol additions, fractional
  scaling, HDR, accessibility, input methods and security fixes that
  land in GNOME's JS become merges we perform, or features Lisa does not
  get.
- **Merging is forever.** The moment the fork stops tracking upstream it
  starts rotting, and the rot is invisible until a protocol Lisa does
  not implement becomes the one an app needs.
- **Cinnamon is the precedent and the warning.** A hard fork of GNOME
  Shell, done a decade ago, which produced a genuinely distinct desktop
  — and is still behind on Wayland. That is the shape of the risk: not
  failure, lag.
- **One reference device.** Everything is validated on an iMac18,2. No
  aarch64 machine has ever rendered this desktop (ADR-0021 is
  container-verified only).
- **~90% of the wanted identity did not require this.** The review was
  explicit. This decision buys the last 10% — the prompt-shaped shell —
  plus immunity from silent breakage, and pays full maintenance for
  both.

## Mechanism

The pattern already works in this repo: `gnome-control-center-lisa`
pins a signed upstream tag, applies its delta in `prepare()`, and
carries **nine `grep` anchors that fail the build loudly** when upstream
moves. Lisa Desktop uses the same discipline at a larger scale.

1. `os/packages/lisa-desktop/` vendors GNOME Shell's JS at a pinned
   signed tag, as a git subtree — not a patch set. Past a certain delta,
   patches are a worse tool than a merge.
2. Build to `/usr/share/lisa-desktop/`, ship a session file, and let
   `gnome-shell` remain installed and unused during the transition, so
   a broken build is one gsettings key away from a working desktop.
3. Upstream tracking is a scheduled rebase against the next signed tag,
   with the diff reviewed rather than auto-merged. A rebase that
   conflicts is information about where our delta is growing.
4. The three existing extensions collapse INTO the shell. They exist to
   reach private API from outside; inside, that reason is gone.

## Sequencing

Deliberately staged, so the fork is reversible until the point it is not:

1. **Tokens first, unchanged by this decision.** `branding/tokens.json`
   and its generator, the token sheet applied to the apps. It is
   identical work under fork or no fork, and it retires the
   three-violets defect. Do not let the fork delay it.
2. **Vendor and build.** Lisa Desktop building and booting as a session,
   byte-identical in behaviour to stock GNOME Shell 50. No design
   changes. The deliverable is "it builds, boots, and CI proves it".
3. **Absorb the extensions.** The dock, wordmark, panel reorder and hot
   corner move inside; the private-API monkeypatching is deleted rather
   than ported.
4. **The prompt in the dock** (ADR-0035 §2) — the first thing that could
   not have been built before, and the reason for all of the above.
5. **Replace the overview** with the prompt's expanded state, retiring
   `shell/launcher`'s separate search-provider path.

## Mechanism, corrected (2026-08-03, before any code)

Verified against the gnome-shell **50.3** source (the exact version the
device runs) at the start of step 2, because two of this ADR's
mechanism assumptions predate GNOME 45's ESM migration:

- **The UI JS is compiled into the binary.** `js-resources` is an
  embedded gresource; the installed `.gresource` files on the device
  are theme/icons/dbus-services only. The `GNOME_SHELL_JS` env var
  still exists (`shell-global.c`) but feeds the legacy `imports`
  search path, not the ESM entry (`resource:///org/gnome/shell/ui/init.js`,
  hardcoded in `main.c`).
- **Therefore "iterates on the real device by copying files" is wrong
  for the core shell.** The JS is still the seam — the fork's delta
  remains JavaScript, upstream C stays untouched — but every iteration
  is a package rebuild, not an scp. The good news is symmetrical: the
  tree's imports are relative (`./`, `../misc/`), so our JS drops into
  the build unchanged in structure.
- **Step 1 of Mechanism changes accordingly**: not a git subtree of
  the JS, but the `gnome-control-center-lisa` discipline scaled up —
  the PKGBUILD in `lisa-desktop` pins the upstream **50.3 tarball**
  (sha256
  `450458c44a26d25a9b84288e12b9005d4c5c44648cfc6b790be19a05de7f1735`),
  carries the JS delta, and rebuilds. A subtree becomes worth its
  weight when the delta outgrows patch review, same trigger as before.

The decision itself is unchanged — this is the "cost model changes →
amend, don't reinterpret" clause firing at the cheapest possible
moment: before the first line of fork code.

## What would change this

- **If step 2 takes more than a few weeks**, the fork is more expensive
  than believed and the wlroots/smithay option deserves the look PLAN §3
  offered it — a smaller surface, at the cost of every GNOME app
  integration Lisa currently gets free.
- **If the delta after step 5 is still small**, this was the wrong call
  and the honest move is to rebase onto upstream and go back to
  extensions.
- **If GNOME's JS/C boundary moves** such that the JS stops being a
  clean seam, the fork's cost model changes and this ADR needs rewriting
  rather than reinterpreting.

## Consequences

- Lisa owns its desktop, and can be held to it. No more "the extension
  did not load and the desktop silently became GNOME".
- CI must boot the image and assert Lisa Desktop is the running session.
  The extension-version incident is the argument: a desktop that fails
  by looking like something else needs a test that looks.
- PLAN §3 needs updating: phases 1–2 are complete and phase 3 is
  decided, not pending.
- ADR-0035's open questions become answerable rather than negotiable —
  the dock's struts, focus behaviour and overflow are ours to define.
- The maintenance burden is real, permanent, and lands on a project with
  one reference device. That is the trade, made knowingly.
