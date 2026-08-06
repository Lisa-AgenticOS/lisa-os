# ADR-0034: `lisa dev` — developer tooling in the user's home, rootless

- **Status:** accepted, partially executed — phases 0, 1 and 2 are implemented and
  proven by execution on a real rootless podman: `lisa dev
  install|remove|list|shell|reset|doctor|check`, a /home disk guard that
  measures the container store's own filesystem, shims that refuse to
  shadow anything on `PATH`, and an isolation test with positive
  controls. Not yet run on the reference iMac, which is the machine
  phase 0 shipped to. The two rules it establishes are CLAUDE.md 7a and
  7b.
- Date: 2026-07-26
- Relates: ADR-0019 (dedicated /home), ADR-0020 (app channel), ADR-0023
  (slim core, /var grows), ADR-0029/0030 (guardrails), PLAN §3, §6
- Supersedes nothing; establishes two rules other ADRs will lean on
- **Claims:**
  - `symbol:fn guard_disk@cli/lisa/src/devbox.rs` — the /home disk guard that measures the store's own filesystem
  - `symbol:NEVER_SHIM@cli/lisa/src/devbox.rs` — shims that refuse to shadow anything on PATH
  - `symbol:fn the_box_cannot_reach_lisas_own_data_or_sockets@cli/lisa/src/devbox.rs` — the isolation test with its positive control

## Context

Lisa's root is immutable and replaced wholesale by the next A/B update.
So the ordinary Linux answer to "I need `mysql` to develop against" —
`pacman -S mysql` — does not work: the write lands in a root that
disappears on the next `lisa update`. Lisa has no answer to this today.

It is worth being precise about what is *not* the problem. Lisa already
has package delivery, and more of it than it looked:

- the OS image itself (mkosi + pacman at build time)
- the app channel (ADR-0020) — versioned trees under `/var`, atomic
  `current` symlink, hash-pinned, rollback
- the runtime channel (issue #52) — **already a channel of the same
  mechanism**, not a separate one
- two stragglers with bespoke fetchers: models (blake3-pinned) and the
  Flutter SDK (sha256-pinned, ~1 GB)

So there is no missing *package manager*. There is a missing answer to
"where does a developer's toolchain live on an immutable OS".

## Decision

### 1. The dependency rule

> **The install path, the update path, and the recovery path may not
> depend on infrastructure we do not control. Everything else may.**

This is why Lisa does not distribute itself through a third-party package
manager. Issue #45 is the precedent: `lisa update` was one upstream
dependency reshuffle away from being unable to download anything, because
libcurl arrived only as an accidental transitive dependency of
NetworkManager. The fix was to declare it and assert it in CI. Taking a
third party into the *distribution* path is the same mistake one layer
up — and `VISION.md` promises a system that "does not get discontinued by
a vendor".

Developer tooling is explicitly outside those three paths, so it *may*
use outside infrastructure. It should still not use a **second** one:

### 2. Arch packages, in a rootless container

`lisa dev install mysql` resolves to `pacman` inside a managed container,
not to a new ecosystem.

The decisive argument is not that Homebrew is bad. It is that **we
already trust Arch's repositories for the entire OS image** — the kernel,
systemd, every daemon. `mysql`, `postgres`, `node` are already packaged
there and signed by the keyring we already depend on. Adding Homebrew
would mean a second supply chain, a second signing story and a second
security-response process, for packages we can already get. It also makes
the aarch64 question disappear: ALARM packages the same names, whereas
Homebrew's Linux bottles are documented for x86_64 with ARM32 explicitly
bottle-less.

**A container, not root layering.** Layering packages onto the root
(rpm-ostree style) means the running system is no longer the artifact CI
tested — it is that artifact plus whatever accreted, reapplied after
every slot swap. That quietly voids the guarantee the immutable design
exists for. A container leaves the root byte-identical to what shipped,
and `lisa dev reset` becomes a real recovery instead of a reinstall.

### 3. The scope rule: `/var` is the system's, `$HOME` is the user's

> **System-scope payloads live on `/var`** — models (large, shared,
> group-readable by the daemons), the app channel, the runtime channel.
> **Per-user tooling lives in `$HOME`** — dev containers, shims, caches.

Rootless podman already stores under `~/.local/share/containers`, so
"container" and "user directory" were never in tension.

Three reasons this is right, beyond preference:

- **No sudo, anywhere.** `escalate.privilege` is an unoverridable `Deny`
  in our own guard (ADR-0029). A dev-install path needing root would have
  to be an exception to the rule we spent a day defending. User scope
  means no carve-out.
- **It already survives updates.** `/home` is a real partition created at
  first boot by systemd-repart (ADR-0019), same mechanism that keeps
  Wi-Fi profiles and SSH keys across A/B swaps. No new persistence work.
- **Per-user, and removable.** Toolchains do not collide between users,
  and uninstalling is deleting a directory rather than unpicking system
  state.

### 4. The controls, one of which already exists

- **`pkg.mutate` already applies.** `lisa-guard` classifies package
  installs as `Confirm`. So if *you* type `lisa dev install mysql` it
  runs; if the *agent* decides to, it asks. That is precisely ADR-0030's
  line — guardrails between the model and the machine, never between a
  person and their own machine — with no new policy.
- **Argv, not a shell string.** `lisa dev install <name>` is judged by
  `check_command`, the surface that produced zero findings in review
  round 4, rather than by the shell reader that has leaked in four
  consecutive rounds.
- **Ledgered** as `dev.install`. "You can read exactly what it did"
  should cover the toolchain too.
- **No daemon access.** The container gets no D-Bus to `contextd`,
  `agentd` or `remoted` and no context-fabric mount, so a dev tool cannot
  reach your data.

## Prerequisites that must land first

- **`subuid`/`subgid` are not in the image.** There is no `/etc/subuid`,
  no `/etc/subgid`, no `newuidmap`. Rootless containers cannot map user
  namespaces without them. This is a one-time image change and it must
  ship *before* the feature — it is exactly the kind of thing that
  silently does not work if assumed (see the `lisa-apps-sync.timer`
  regression, which shipped disabled on both arches).
- **A container runtime is not in `Packages=`.** Adding podman to the
  image has a size cost that ADR-0023 phase 3 has to account for.
- **Disk comes out of `/home`'s quarter.** The repart weights are
  `var 3 : home 1`, because models dominate. A dev container plus
  packages is easily several GB, and it now competes with documents
  rather than with the model store. `lisa dev` should refuse, loudly,
  when `/home` is tight rather than filling it.

## What was rejected

- **Publishing Lisa through Homebrew.** A tap needs nobody's permission,
  but buys almost nothing: no discovery (taps are not searchable), no
  upgrade benefit (the CLI self-updates via the runtime channel), no
  dependency resolution to do. Against a per-release formula to maintain
  and someone else's conventions becoming our breakage. Separately,
  `homebrew/core` explicitly rejects self-updating software — which Lisa
  is, on purpose.
- **Root package layering.** Voids the "your root is the tested artifact"
  guarantee.
- **Building a general-purpose package manager.** Dependency solving,
  mirrors, a trust and signing story, a security-response process — a
  decade of work where bugs are catastrophic, for something Arch already
  does. CLAUDE.md rule 4: boring tech for plumbing.

## Consequences

- Lisa gains a developer story without gaining a packaging ecosystem.
- The two rules above are reusable: ADR-0031's server mode and the
  artifact store both need "who owns this path" answered the same way.
- The app channel stays narrow — Lisa artifacts only, because those need
  Ledger entries, provenance and tiers that a generic package has no
  concept of.
- **Sequencing:** behind manifest signing (#56/#97, actively exploited)
  and the ADR-0033 rollout. This is the most user-visible of the queued
  work and the least urgent of it; shipping it before the trust boundary
  is closed would be building a front door onto an unlocked house.
