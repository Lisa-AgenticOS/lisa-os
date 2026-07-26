# ADR-0023: Slim core, /var grows — apps and heavy payloads leave the image

- Status: accepted (design; phase 1 = Zen migration **implemented**
  2026-07-26, issue #51; phase 2 = installer pre-pull, phase 3 = slot
  shrink — **premise corrected, see "Phase 1, measured"**)
- Date: 2026-07-25 (measurements added 2026-07-26)
- Relates: ADR-0020 (app update channel), ADR-0001/0003 (Track I image),
  issue #37 (Flutter SDK to /var), issue #46 (the 23 GiB flash pain that
  prompted this), M7 (installer/OOBE)

## Context

The Track I image has grown into a 23 GiB flash: 1 GiB ESP + 2 × 10 GiB
root slots + 2 GiB var seed. The root slots were bumped 8 → 10 GiB when
the bundled desktop stack (GNOME, Zen browser, llama.cpp) pushed the
populated root past 8.3 GiB — and every byte of root payload is paid
TWICE, because the A/B scheme carries two full slots. Meanwhile three
subsystems already pull their payloads to the durable /var partition
after boot instead of riding the image:

- models (`lisa models get`, blake3-pinned, /var/lib/lisa-models)
- shell apps (ADR-0020: `lisa apps update`, versioned trees under
  /var/lib/lisa/apps with an atomic `current` symlink and rollback)
- the Flutter SDK (issue #37: `lisa forge --setup`, sha256-pinned,
  /var/lib/lisa/flutter)

The question: what else should leave the image, and what must never?

## Decision

Split by role. **The image carries the OS contract; /var carries what
the user grows.**

**Stays in the image (substrate — atomicity and rollback are the point):**

- systemd/boot chain, kernel, initrd
- the Lisa daemons (inferenced, modeld, contextd, agentd, remoted) and
  llama.cpp — inference is the operating system here, not an app
- GNOME session, portals, the Shell extensions, the CLI
- anything a failed update of which must be undone by boot-counting
  rollback

**Leaves the image (payloads — pulled to /var, verified, own rollback):**

- GUI applications, starting with **Zen browser** (measured 363 MiB of
  root payload → 726 MiB across both slots; this ADR's original "~1.5 GiB"
  was an estimate and was wrong — see "Phase 1, measured"): moves to the
  ADR-0020 apps channel as phase 1
- models, Flutter SDK (already out)
- future app-suite members (§5.8) and forge-built apps — these were
  always going to live on the apps channel

**Delivery rules for out-of-image payloads:**

1. Every payload is hash-pinned (blake3 or sha256) and staged-then-
   renamed — a partial download is never visible (the ADR-0020 and #37
   mechanics, unchanged).
2. Every payload has its own rollback: the apps channel keeps prior
   version trees; models/SDKs are re-fetchable by pin. The boot-rollback
   guarantee is explicitly NOT extended to /var payloads — a broken app
   is fixed by the channel, not by rebooting into the other slot.
3. First-boot experience: the M7 installer pre-pulls the default app set
   into the target's /var during installation (the installer is online
   anyway) — "batteries included" without image bytes. A pure
   dd-and-boot install without network simply starts with the core and
   pulls apps when connected.

**Phase 3 — slot shrink:** once Zen is out, re-measure the populated
root; target 7 GiB slots. Image drops to ~1+7+7+2 = 17 GiB, and further
as the app suite stays off-image. Slot size changes only apply to fresh
installs (an existing A/B disk cannot shrink its slots in place — same
constraint as the 8→10 GiB bump, which forced re-flashes).

## Phase 1, measured (2026-07-26, issue #51)

Both upstream Zen 1.21.8b tarballs were downloaded, verified against the
digests pinned in `os/packages/zen-browser/PKGBUILD`, and unpacked:

| | x86_64 | aarch64 |
|---|---|---|
| `/opt/zen` tree (apparent bytes) | 380,572,815 (363 MiB) | 343,547,566 (328 MiB) |
| across both A/B slots | 726 MiB | 655 MiB |
| channel payload, `.tar.zst -19` | 95.4 MiB | 83.1 MiB |

**This ADR's premise was off by roughly 4×.** Zen is 363 MiB of root
payload, not ~1.5 GiB, and it is not "the single biggest reason the image
is 23 GiB" — the 23 GiB is fixed geometry (1 GiB ESP + 2 × 10 GiB root +
2 GiB var seed), independent of how full the root actually is. What Zen
actually costs is 726 MiB of the populated root and ~90 MiB of every
release download.

The consequence for **phase 3 is real and unwelcome**: if the populated
root was ~8.3 GiB when the slots went 8 → 10 GiB, removing Zen leaves
~7.9 GiB. **7 GiB slots do not fit.** Phase 3 as written is not reachable
by removing Zen alone; either the target becomes 8.5 GiB slots (image
~1+8.5+8.5+2 = 20 GiB, still a 3 GiB win and still a 32 GB stick), or
more payload has to leave the image first — GNOME, llama.cpp and
`linux-firmware` are the next candidates by size and each needs the same
"measure it, do not estimate it" treatment this correction came from.
release.yml now records the populated-root total and the `/opt/zen` share
in every release's job summary, so the phase-3 decision reads a
measurement rather than a memory.

**Never lose the browser (the migration's actual hard part).** The image
and the channel are decoupled, but a device that already has Zen baked in
does not get to choose when its root is replaced. The path implemented:

1. `zen-browser` splits into `zen-browser-launcher` (~10 KiB: the
   `.desktop`, the hicolor icons, and `/usr/bin/zen-browser` as a
   resolver) and `zen-browser` (the `/opt/zen` payload). **The launcher
   stays in the image permanently**, so the app-grid entry and the
   command exist on every Lisa system regardless of where — or whether —
   the payload is present. The `.desktop` gained `Icon=zen-browser` from
   hicolor in place of an absolute path into `/opt`, which would have
   broken the moment the payload moved.
2. The resolver tries `$LISA_ZEN_DIR` →
   `/var/lib/lisa/apps/payloads/zen/current` → `/opt/zen`, and if nothing
   resolves, says what to run — on stderr and as a desktop notification,
   because a `.desktop` launch has no terminal.
3. `lisa update` pre-fetches missing payloads onto the persistent `/var`
   **before** it stages a slot, and `lisa-apps-sync.timer` retries until
   they are there. Both use `lisa apps sync`, which only *acquires* what
   is absent — it never moves an installed payload to another version.
4. **One overlap release** still bakes `/opt/zen` while also publishing
   the channel payload. This is not caution, it is necessity: the
   pre-fetch in (3) has to already be on a device before the update that
   removes the baked copy, and a device running an older image has a
   `lisa update` that stages slots without pulling anything. The overlap
   release is what installs the resolver, the timer and the pre-fetch,
   with the baked copy underneath as the floor. The release after it
   drops `zen-browser` from `Packages=` and takes the 363 MiB.

## What was rejected

- **Everything-in-image** (status quo): pays every app twice in disk,
  couples app updates to OS releases (ADR-0020 already rejected that),
  and made tonight's 23 GiB stick flash the norm.
- **Everything-post-boot** (netboot-style minimal core): breaks the
  offline dd-and-boot story and moves the inference substrate itself
  outside the rollback guarantee — the one thing Lisa must never do.
- **Flatpak as the app vehicle**: a second packaging universe with its
  own runtimes (gigabytes) and update daemon; the ADR-0020 channel is
  already built, verified, and Ledger-visible. Revisit only if
  third-party app distribution becomes a goal.

## Consequences

- Image and stick flashes shrink in phase 3 — by ~3 GiB on the measured
  numbers, not the ~6 GiB this ADR first assumed (see "Phase 1,
  measured"); OS updates download ~90 MiB less for every device forever.
- App updates continue to ship same-day via the apps channel without an
  OS release.
- The installer gains a small app-selection step (M7), and the apps
  channel manifest grows a "default set" the installer reads.
- Zen's packaging moves from `os/packages/zen-browser` (image) to an
  apps-channel artifact; the image keeps only its .desktop indirection
  until the migration completes (users must not lose the browser during
  the transition).
- Payload provenance (pins, signatures once the M1 signing story lands)
  becomes load-bearing for everything on /var — it already is for
  models; the bar is uniform now.
