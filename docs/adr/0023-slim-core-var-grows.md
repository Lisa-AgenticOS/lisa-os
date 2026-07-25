# ADR-0023: Slim core, /var grows — apps and heavy payloads leave the image

- Status: accepted (design; phase 1 = Zen migration, phase 2 = installer
  pre-pull, phase 3 = slot shrink)
- Date: 2026-07-25
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

- GUI applications, starting with **Zen browser** (~1.5 GiB of root
  payload → ~3 GiB across both slots): moves to the ADR-0020 apps
  channel as phase 1
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

- Image and stick flashes shrink by ~6 GiB in phase 3; OS updates
  download less for every device forever.
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
