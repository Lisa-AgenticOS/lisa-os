# ADR-0048 — Lisa Desktop is a desktop, not a patched GNOME

- **Status:** accepted, partially executed — the core-versus-store test is
  recorded and PLAN §5.8 is rewritten around "we write the apps";
  `gnome-control-center-lisa` is on a retirement path with nothing removed;
  GTK4/libadwaita and Mutter stay upstream, indefinitely. The desktop half
  is ADR-0038 step 2 (see there): it builds and boots headless, and **nobody
  has logged into a session running it**. Of the named core apps, Files and
  Photos are a README each.
- **Date:** 2026-08-04
- **Extends:** ADR-0038 (fork the Shell, not Mutter). This is the same
  decision widened from *the Shell* to *the desktop experience*, which is
  what a user means by "GNOME".
- **Supersedes:** PLAN §5.8's "thin, opinionated forks/extensions of GNOME
  apps" as the plan for Files and Photos; `docs/PACKAGES.md`'s three
  patch-set rows.
- **Related:** ADR-0012 (the gnome-control-center panel this retires),
  ADR-0020 (the app update channel), ADR-0035 (the desktop is a prompt),
  ADR-0037 (the browser is a Lisa app), ADR-0039 (`lisa-desktop` as a
  repo), ADR-0046 + Amendment 1 (capability before storefront; build our
  own store; the elementary precedent), ADR-0047 (GJS + GTK4 is the one
  toolkit), issues #170, #208, #239, lisa-desktop#1.
- **Claims:**
  - `path:apps/files/README.md` — Files is a README
  - `path:apps/photos/README.md` — and so is Photos
  - `path:os/packages/lisa-desktop-control-center/PKGBUILD` — on a retirement path with nothing removed

## Context

ADR-0038 decided to fork GNOME Shell. It was written about *the Shell* —
one Mutter plugin, 1412 lines of extension JavaScript, a dock and a
wordmark. Read narrowly, it leaves everything else in place: patched
GNOME apps, a patched Settings, a session that is GNOME wearing a
different wallpaper.

That reading is not the intent, and leaving it unwritten is how it drifts
back. **"GNOME" to a person using the machine is not `gnome-shell`. It is
Files, Photos, Settings, the fonts, the dialogs, the file picker, the way
an app asks a question.** A fork of the Shell that ships GNOME Files, GNOME
Photos and GNOME Settings has changed the top 40 pixels of a GNOME desktop.

The end state we want looks more like macOS than like GNOME: a coherent
set of first-party apps that share one design language, one agent surface,
and one release. Patching upstream forever does not converge on that. It
converges on a maintained delta that has to be re-justified at every
upstream release, and whose ceiling is whatever the upstream maintainer
was already willing to allow.

### The fact that makes this cheap today

Checked on 2026-08-04, before writing a line of this ADR:

| directory | contents |
|---|---|
| `apps/files-patches/` | `README.md` — "Status: **not started** — scaffold placeholder" |
| `apps/photos-patches/` | `README.md` — same |
| `apps/mail-patches/` | `README.md` — same |

**Three planned patch sets. Zero lines of patch written.** Not one file
beyond the placeholder in any of them, since they were created on
2026-07-20.

Meanwhile `apps/mail` exists — a real GJS/GTK4 mail client, MCP-native,
shipped and reviewed. The patch-set plan for Mail was never executed
because somebody wrote the app instead, and the app is better than the
plan was.

So this is a fork in the road not yet taken. Choosing now costs two `git
mv`s and three rewritten READMEs. Choosing in a year, after a Nautilus
patch set exists and is pinned to a GNOME version and has its own anchor
guards, would cost the work plus the sunk-cost argument for keeping it.

## Decision

**Lisa Desktop is a desktop environment of our own.** Four parts, in the
order they matter.

### 1. Stop planning patches. Build the apps.

`apps/files-patches` becomes `apps/files` (**Lisa Files**);
`apps/photos-patches` becomes `apps/photos` (**Lisa Photos**). Both are
Lisa apps in the shape ADR-0047 settled: GJS, GTK4/Adwaita, MCP-native
from the first commit, `app.lisaos.*` ids, the same test harness as Mail,
Preview and Surfer.

Neither exists. The directories hold a README describing an app that is
**not started**, and they must keep saying so until someone writes one
(CLAUDE.md rule 10).

`apps/mail-patches` is moot — Mail exists. It is not deleted here; it is
raised as a question for the owner, because nothing gets removed from this
tree without an explicit say-so.

Why write rather than patch, stated as an argument rather than a
preference:

- **MCP-native is not a patch.** Every Lisa app exposes an agent surface
  over `libs/mcp-bus`, with provenance on every result and a manifest that
  ADR-0046 §2 intends to enforce. Bolting that onto Nautilus means a patch
  set that touches the parts of Nautilus most likely to move, in a
  codebase whose maintainers have not agreed to any of it.
- **The delta only grows.** ADR-0012's panel is 3 lines of registration
  plus two subtractive edits and it already needs four `grep` tripwires.
  A file manager we actually want — semantic search, ask-this-folder, an
  agent that can propose a batch move behind a confirm tier — is not a
  three-line delta. Past some size a patch set is a fork with worse
  ergonomics, and we would arrive there having never chosen it.
- **The interpreted-source property is load-bearing.** ADR-0047 and
  ADR-0046 Amendment 1 both rest on it: our apps ship as source, so a fix
  reaches the reference iMac by `scp`, and the artifact a reviewer reads
  is the artifact that runs. A patched C app gives up both.

### 2. Fork the Shell: byte-identical first, then diverge.

Unchanged from ADR-0038 step 2, restated because it is the gate on
everything visual. lisa-desktop#1 — *"ADR-0038 step 2: vendor + build —
Lisa Desktop boots as a session, byte-identical to GNOME Shell 50.3"* — is
the next piece of work, and its deliverable is deliberately boring.

**Byte-identical first is the whole discipline.** It separates two
questions that most forks fail by attempting at once:

1. Can we vendor, build, package, sign, install and boot our own session?
2. What do we change?

A fork that answers both simultaneously cannot tell a build problem from
a design problem, and debugs a session that neither boots nor looks right
with no known-good state to bisect against. Answering (1) alone produces a
session that is *provably ours* and *provably works*, after which every
subsequent defect has exactly one cause: the change we just made.

### 3. Retire `os/packages/gnome-control-center-lisa` in favour of `shell/settings`.

**This is a direction, not an action taken today.** Nothing is removed in
this ADR.

The reason is written in the package's own header:

> Re-pinning to a new GNOME release: bump pkgver, rebuild, and **fix the
> two anchors if upstream moved them (the guards below fail loudly if
> so)**.

That is the patch treadmill described in its own PKGBUILD, with tripwires
already installed because we know from experience that it breaks. The
`prepare()` function carries four `grep` guards, an `awk` block that
deletes a brace-balanced blueprint group by matching the string "Support
GNOME", and a regression guard for a GNOME 50 subpage trap that already
bit us once. Every one of those is correct engineering, and every one of
them is rent.

`shell/settings` is the replacement, and it already exists and works:
`lisa-settings.js` (GJS/GTK4/libadwaita), `lib/model.js` (pure
view-model, unit-tested via `shell/testing/harness.js`),
`app.lisaos.Settings.desktop`, and two live sections — Local models and
Providers.

**What must be true before the package can be dropped:**

- `shell/settings` covers what the panel covers today: the Intelligence
  surface *and* the System-page OS-updates row (running / staged /
  available via `lisa update --check`, ADR-0012 and issue #144). Dropping
  the package without that regresses a machine to a Settings page that
  cannot say what OS it is running.
- Lisa Desktop's session launches `shell/settings` for the settings
  affordance, so nothing routes a user into `gnome-control-center` and
  finds a Lisa panel missing.
- The About page's provenance is handled somewhere. The current package
  also removes upstream's "Support GNOME" donate group; a replacement
  Settings that ships stock gnome-control-center alongside it would put
  that group back.
- Stock `gnome-control-center` either stays installed (for the panels we
  do not intend to write — printers, Bluetooth, Wacom) or its absence is
  a deliberate, documented loss. **It is not a goal to reimplement GNOME
  Settings.** The goal is to stop patching it.

Until all of that holds, the package stays and stays maintained.

### 4. Keep GTK4/Adwaita and Mutter. Both, indefinitely.

Toolkit and compositor are *foundation*. They are not the experience.
Owning them buys CVE duty and Wayland-protocol duty, and returns
approximately no identity — ADR-0038's table already priced this and
nothing here changes it.

**The precedent is elementary OS,** which ADR-0046 Amendment 1 already
cites for its AppCenter. Pantheon looks and behaves nothing like GNOME —
its own dock, its own panel, its own apps, its own design language — and
Gala, its window manager, is a Mutter plugin. They own the user
experience; they did not take on a compositor to get it. That is the exact
split this ADR is making, and it is a shipped, decade-old proof that the
split holds.

### 5. Core apps ship with the desktop. Everything else comes from the store.

Once "desktop environment" is a thing we own, "which apps are part of it"
stops being a taste question and becomes a boundary — and a boundary needs
a test, not a list. A list gets argued into a pile.

**The test: an app is core if removing it breaks a promise the OS makes.**
Two ways to qualify, and one of them is enough:

1. **The system's own thesis depends on it** — the model, the context
   fabric or the Agent Bus offers a capability that silently disappears
   without this app.
2. **It is the default handler for something the OS must handle
   regardless** — a person double-clicks a thing and the desktop has to
   have an answer.

Everything else is an app that happens to run on Lisa, and it is
distributed the way ADR-0046 distributes apps.

By that test, as of 2026-08-04:

| | apps | why |
|---|---|---|
| **Core** — ships with the desktop | Assistant, Files, Preview, Terminal, Surfer, Mail, Notes | see below |
| **Core** — desktop surfaces | launcher, desktop/dock, consent, settings, Ledger, overlay-extension | they *are* the desktop |
| **Store** — independently installable | Recorder; Photos when it exists; every future app | nothing breaks without them |

The test doing work at the two edges, which is the part worth keeping:

- **Recorder feels like a system utility and is not core.** It looks like
  something an OS ships — a recorder, a transcript, a meeting summary —
  and nothing on the system depends on it. No tool vanishes, no file type
  goes unhandled, no thesis weakens. Store. (It is also **not started**:
  `apps/recorder` holds one README.)
- **Notes feels like an ordinary app and is core.** A local note vault is
  the most replaceable thing imaginable — except that
  `apps/notes/app.lisaos.notes.json` advertises `search_notes` at read
  tier, and `libs/bus-tools` and `shell/assistant` both hand it to the
  model as a system capability. Removing Notes does not remove an app; it
  removes a tool the assistant offers, and the failure mode is the model
  calling something that is no longer there.

The others, briefly: **Surfer** because ADR-0037 made the web an agent
surface rather than a bundled binary; **Mail** because mail → contextd
(#170) is a large part of what makes "my stuff" answerable at all;
**Preview and Terminal** because they are default handlers — Preview was
written precisely because *nothing on the reference device claimed
`image/*`*; **Files** for the same reason, once it exists; **Assistant**
because it is the product.

**Consequence for packaging — stated, not implemented.** It depends on
#239 landing first, and this ADR does not touch it:

- Core apps and desktop surfaces **version together** in the desktop
  payload. They are one product and one CI gate; a launcher that ships
  independently of the shell it launches into is two products pretending.
- Store apps get **per-app payloads with their own versions, channels and
  rollback**. Channels reuse PLAN §6's existing vocabulary — `edge`,
  `beta`, `stable` — rather than inventing a second scheme for apps.
- Today the app channel is monolithic: one `shell` tarball, one version.
  **That is an updater, not a store,** and no amount of storefront UI
  fixes it. This is the same argument ADR-0046 Amendment 1 makes from the
  other end — an Install button sits directly on this mechanism — which is
  why the two decisions are one decision seen from two sides.

So ADR-0046's catalog is not an arbitrary list of things we felt like
shipping. **It is a consequence of the architecture in this ADR:** once
there is a desktop environment and there are apps running on it, the
store's contents are whatever falls on the far side of the test above.

**Caveat, written in deliberately.** The core list is a judgement call
made on 2026-08-04 against the system as it exists. **Files and Photos are
unbuilt** — placing one in core and the other in store is a prediction
about apps nobody has written, and it should be re-run against the real
things when they exist. Nothing here describes Files or Photos as
shipping, because they do not.

## Why patching has a ceiling, not just a cost

This is the load-bearing half of the argument, and it is not about effort.

**Issue #208 is the proof.** The system-wide double-tap-Shift gesture that
summons the assistant does not work on the device. The diagnosis, run on
2026-08-03 against the real hardware, is not "the code is wrong" — keys
injected directly into fcitx5 over its D-Bus frontend produced exactly one
`dev.lisaos.Overlay1.UI.Summon` call. The whole chain works. What fails is
delivery:

> `waylandim` loads then unloads on every start: **mutter does not grant
> `zwp_input_method_v2` to third-party input methods** — GNOME routes text
> input to its own ibus path.

The gesture is not hard. It is **unreachable**. There is no patch to
`fcitx5-lisa` that fixes it, because the decision is made in a component
that has chosen not to expose the capability to third parties, and we are
a third party for exactly as long as we run somebody else's session.

That will keep happening, and it will keep happening in the same place: an
AI-native desktop's interesting behaviours are the *system-wide* ones. A
gesture that works everywhere. A selection that any app can hand to the
assistant. A prompt that is the desktop rather than a window on it
(ADR-0035). Those are precisely the behaviours a desktop grants to itself
and withholds from extensions — not out of hostility, but because
granting them to arbitrary third parties would be a security defect in
*their* system.

The extension route's ceiling is therefore structural, and #208 is the
first time we have hit it with a measurement rather than a suspicion.

## What this costs, stated plainly

ADR-0038 already accepted the Shell's maintenance burden. Widening the
decision widens the bill, and hiding that would make this ADR useless to
whoever reads it next.

- **We own security updates for everything we vendor.** GNOME Shell gets
  CVEs. So does Mutter — and although we do not fork Mutter, an image that
  pins a Mutter version owns the duty of moving that pin promptly. Every
  app we write instead of patch is an app whose bugs are ours alone, with
  no upstream to fix them for us.
- **Rebasing is forever, and its cost scales with the width of the
  divergence.** This is the single most important operational sentence in
  this ADR: **narrow, deliberate divergence keeps rebasing tractable; wide
  divergence means owning the stack forever.**
- **The discipline that follows from it:** diverge where the vision
  requires it — input handling, the prompt-as-desktop surfaces, the dock,
  agent affordances — and stay stock everywhere else. Every additional
  file touched in the vendored tree is a merge conflict scheduled for a
  date we do not choose. A change we cannot justify against ADR-0035 or an
  agent affordance is a change we do not make.
- **Two apps we do not have.** Files and Photos are real work — a file
  manager especially, which is a deceptively deep app. Until they exist,
  Lisa either ships GNOME's or ships without. **Shipping stock GNOME Files
  in the interim is fine and expected**; what this ADR forbids is
  *patching* it.
- **Cinnamon is still the warning** (ADR-0038): a hard fork that produced
  a genuinely distinct desktop and has been behind on Wayland ever since.
  The risk is not failure. It is lag.
- **One contributor, one reference device.** Unchanged, and it is the
  reason the sequencing above refuses to run two hard problems at once.

## Mobile is parked, and is not an argument for any of this

Recorded because it will come up, and because the wrong version of it
would poison the reasoning.

A phone is a different operating system, not a smaller desktop. It needs
telephony and a modem stack, aggressive power management with real
suspend/wake discipline, a touch-first input model (not a mouse model with
bigger targets), a different application lifecycle where the system kills
apps at will, and an entirely different distribution and update story.
None of that is desktop work with a narrower screen.

So: **a Lisa phone is a possible future that nobody is planning for
today,** and it must not be used to justify the desktop decision. If the
desktop fork is right, it is right on desktop grounds alone — #208's
ceiling, the coherence of a first-party app set, the prompt-as-desktop
thesis. If someone later argues "we need to own the desktop because of
mobile", that is this paragraph telling them the argument was already
considered and declined.

## What would reverse this

- **Upstream grants the capability.** If Mutter exposes
  `zwp_input_method_v2` (or an equivalent) to third-party input methods,
  and the GNOME Shell extension API grows the seams ADR-0035 needs, the
  ceiling argument evaporates and the fork is paying for something we
  could have had. Watch #208 and the corresponding upstream work; this is
  the single most likely reverser.
- **ADR-0038 step 2 does not land.** ADR-0038 already says this: if
  vendoring and booting a byte-identical session takes more than a few
  weeks, the fork is more expensive than believed. This ADR adds a second
  consequence — if we cannot own the *Shell*, we certainly should not be
  writing a file manager, and the app decisions here should be reverted to
  "ship stock GNOME apps unpatched".
- **The divergence stays narrow after the visual work lands.** If, once
  the dock-prompt and the absorbed extensions ship, the diff against
  upstream is still small, then extensions were adequate and the honest
  move is to rebase onto upstream and go back.
- **Lisa Files or Lisa Photos turns out to be a multi-year project.** A
  file manager that keeps missing the bar users hold it to — trash,
  network mounts, archive handling, thumbnailing, the file *chooser* which
  is GTK's and not ours — is evidence that the app-set half of this ADR
  was scoped by ambition rather than measurement. Shipping stock GNOME
  Files unpatched is always available, and choosing it is not a failure of
  this decision, only of one branch of it.
- **The core/store test starts producing answers nobody accepts.** If the
  test says "core" for an app the desktop plainly should not carry, or
  "store" for one it obviously must, the test is wrong and should be
  rewritten — not quietly overridden case by case, which is how a test
  decays into the list it was meant to replace.
- **A second contributor never arrives.** Everything here is priced for a
  project that grows. If the headcount is still one when the vendored tree
  needs its third rebase, narrowing the divergence — not abandoning the
  fork — is the correction, and the discipline section above is where it
  starts.

## Consequences

- `docs/PLAN.md` §5.8 no longer describes Files and Photos as patch sets,
  and `docs/PACKAGES.md`'s "Forked / patched upstream" table loses its
  three speculative rows. A plan that names a strategy nobody will follow
  is the defect CLAUDE.md rule 10 exists to prevent.
- `apps/files` and `apps/photos` exist as directories with honest,
  not-started READMEs. Under ADR-0039 they belong to `lisa-apps` when
  they are real; today they are placeholders in the monorepo like every
  other unbuilt app.
- The `lisa-desktop` repo's scope is the whole desktop surface, not just
  the Shell — while remaining, per ADR-0046 Amendment 1 §2, **pre-fork
  today**: it currently holds extensions and the IME riding on stock
  GNOME Shell. No plan may assume fork-only capability until
  lisa-desktop#1 is done.
- ADR-0012 acquires an end state. It was correct when written — a sidebar
  panel needed a build, because gnome-control-center has no plugin API —
  and it stays in force until the retirement conditions above are met.
- The first-party catalog of ADR-0046 gets a defensible boundary rather
  than a curated list: it distributes what the core/store test puts on the
  store side. Its claim stays the same and gets easier to make — these are
  apps we wrote, review and sign, in one toolkit, with one design
  language.
- **The app channel needs per-app payloads before a store is a store.**
  One `shell` tarball at one version can update the desktop; it cannot
  install, version or roll back an individual app. That work follows
  #239, and until it exists the "store" half of §5 is a boundary we have
  drawn, not a mechanism we have built.
