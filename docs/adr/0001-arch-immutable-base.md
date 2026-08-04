# ADR-0001: Fork Arch Linux; ship an immutable, atomic, image-based OS via mkosi

- **Status:** accepted — the mkosi/UKI A/B image builds, boots, and
  demonstrates update *and* rollback in CI.
- **Date:** 2026-07-20

## Context

Lisa's product requirement is *current* kernels and Mesa: NPU drivers
(`amdxdna` since 6.14, Intel `ivpu`) and llama.cpp's fast-moving Vulkan
backend make driver velocity the whole game (PLAN §3). The base must also
be atomic and verifiable, because the security story (measured egress,
dm-verity, hash-pinned models — PLAN §5.10) is only honest on an
immutable image.

## Decision

Fork Arch Linux in governance (pinned snapshot mirror + ~100–200-package
custom repo), not in packaging. Build the product image with `mkosi`:
signed UKI, `systemd-repart` partitions, A/B roots via
`systemd-sysupdate`, dm-verity on the base, automatic boot-counting
rollback. `/usr` read-only, `/etc` overlay with factory reset, models
content-addressed under `/var/lib/lisa/models`. Flatpak-first for GUI
apps — the portal is the security boundary. GNOME base for phases 1–2.

## Consequences

- We inherit Arch's package velocity and ~12k packages; our delta stays
  small. SteamOS 3 and CachyOS are precedent that this shape ships.
- We own snapshot promotion cadence (soak → channel), like SteamOS holo.
- Rejected: Debian (perpetual kernel/Mesa backports), Fedora
  bootc/Universal Blue (fastest atomic path, but couples us to Fedora
  cadence and rpm-ostree opinions — noted as the fallback accelerant if
  mkosi-on-Arch stalls in M0), from-scratch (zero product value).
- Dev cost: image work requires Linux; macOS/other hosts develop the
  Rust workspace and defer image builds to CI.
