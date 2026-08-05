# ADR-0051 — Third-party packages are built on change and consumed by pin, not rebuilt per release

- **Status:** accepted, partially executed
- **Date:** 2026-08-05

## Context

The release job compiled five pinned third-party packages inline on
every dispatch: llama.cpp, whisper.cpp, piper, zen-browser and the
gnome-control-center fork. Each is pinned to a fixed version and
changes perhaps monthly; llama.cpp alone measured ~5 minutes per build.
A weekly-rotating cache papered over this, but the combined cache
action saves only when the *job* succeeds — and on 2026-08-05 four
release attempts in a row built everything, failed at a verification
step near the end, saved nothing, and paid the full build again.

Meanwhile #273 named the same problem from the other side for
`lisa-desktop-shell`: a component the image takes from a rolling source
is an unpinned input — two builds of the same commit can differ.

The owner asked for both halves at once: stop rebuilding these in the
release, and consume them as packages from a repo.

## Decision

A **ports lane**, three parts:

1. **`ports.yml`** builds a port when its PKGBUILD (or the shared
   recipe script) changes — never on a release cadence — and uploads
   the artifacts to the rolling `ports` release on this repo. The
   per-port makepkg flags live in one place,
   `os/repo-tools/build-port.sh`, so no second lane can drift from
   them.
2. **`os/packages/ports.lock`** pins each artifact by filename and
   sha256, in git. Bumping a port is two deliberate commits: the
   PKGBUILD (ports.yml builds and prints the new lock lines in its
   summary), then the lock. What an image contains is decided by the
   commit that built it, never by what a rolling tag happens to hold.
3. **release.yml fetches and verifies** the locked artifacts into the
   local package directory instead of building them, and still uploads
   them to the versioned release — so every release remains a complete
   package set and the [lisa] publish flow is unchanged.

The ports are: llama.cpp, whisper.cpp, piper, zen-browser (both split
packages), gnome-control-center-lisa (both split packages), and
gnome-online-accounts-lisa — the last discovered *by this review* as a
finished package (Lisa's own verified Google OAuth client, so the
consent screen names the OS that is asking) that nothing had ever
built or installed.

## What deliberately stays in the release build

- **The workspace `lisa-*` packages** — they change every release and
  are the thing being released.
- **`lisa-audio-cs8409`** — rebuilt per release *as a gate*: it is
  coupled to the kernel the image takes, and its build failing is how
  kernel drift becomes a red release instead of silently dead
  speakers. Prebuilding it would defeat its purpose.
- **`lisa-keyring`, `cyrus-sasl-xoauth2`** — seconds to build, and
  both have check() functions whose claims are worth re-asserting.

The **aarch64 lane is not a consumer**: it builds every port from
source natively. No x86_64 binary is usable there, and upstream
llama.cpp ships no Linux arm64 binaries to substitute.

## Why not upstream's own release binaries

Considered for llama.cpp and rejected: upstream ships shared-library
bundles built with curl enabled, where ours is one static binary built
with `LLAMA_CURL=OFF` — the inference engine is *incapable* of egress
rather than merely confined (rule 5 as a compile-time property). And
with no Linux arm64 upstream binaries, adopting theirs would mean two
different supply chains for the most security-central binary in the
OS. Building unmodified pinned source everywhere is one story.

## Status of execution

Done 2026-08-05: the `ports` release exists, seeded from
v20260804.77's assets after verifying every PKGBUILD still matches its
shipped artifact; ports.lock, build-port.sh and ports.yml are in the
tree. **Not yet done:** the release.yml switch from building to
fetching (deliberately held until the in-flight release lands — the
build step of a release being actively stabilized is the wrong thing
to rewrite), and gnome-online-accounts-lisa's first build + image
wiring, which needs a seated device check of Google sign-in and mail
sync afterwards.

## Consequences

- A failed release retry costs image-assembly time, not
  compile-everything time. The weekly package cache and its ldd
  staleness check retire with the switch.
- The image build gains two pinned inputs it did not have (the ports
  by sha256), and loses its heaviest unpinned behavior.
- The `ports` release tag is rolling and its assets are clobberable —
  the lock's sha256 is what makes that safe: a replaced artifact fails
  the fetch loudly instead of shipping silently.
- One more thing to know when bumping a port: the two-commit dance.
  The ports.yml summary prints the exact lock lines to paste, and the
  release build refuses a lock/artifact mismatch, so forgetting the
  second commit is loud, not silent.
