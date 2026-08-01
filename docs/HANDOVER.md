# Handover — dev moves to the second iMac (2026-07-22)

The original dev machine (iMac18,2, 2017) is now the **Lisa OS field
device**: it boots Lisa from its internal 28 GB blade SSD and stays on
Lisa. Development continues on the other iMac from a fresh clone. This
file is the context the next Claude Code session needs; read
`docs/STATUS.md` and `CLAUDE.md` first as usual.

## The dev loop (why the field machine exists)

1. Code + merge PRs here (the dev iMac).
2. Cut a release: `gh workflow run release.yml --ref main` (~15–25 min).
3. On the Lisa iMac: `lisa update --reboot` (or the daily
   `systemd-sysupdate.timer` stages it; reboots apply). Boot-counting
   auto-rolls-back a release that fails to boot twice.
4. Repeat. sysupdate pulls from `releases/latest/download/` — no
   per-device configuration.

## Field device facts (do not relearn these the hard way)

- Its internal blade SSD has **4096-byte sectors**: the `lisa-usb-*.raw`
  release images (512-byte GPT) can NOT be `dd`'d to it. Updates are
  fine (sysupdate writes filesystem images into existing partitions);
  only full re-provisioning needs the conversion: rewrite the GPT
  natively at 4K (same types/labels/attrs, tail partition grown),
  byte-copy root/var payloads, skip `_empty`, and rebuild the ESP as
  4K-sector FAT32 + file-copy. A ~150-line python tool (`gpt4k.py`) did
  this; it lived in a session scratchpad — recreate from this
  description if ever needed, or upstream a proper installer to
  `os/installer/` (backlog).
- Booting Lisa by default: macOS `bless --setBoot` is blocked by SIP
  (`0xe00002e2`). The firmware way: in the Option-key boot picker,
  select "EFI Boot" while holding **Control** → becomes the permanent
  default. To return to macOS: hold Option.
- The USB stick (62.5 GB, 512-byte sectors) is the portable tester —
  plain `dd` of release images works for it. Currently carries
  v20260722.5; re-flash to keep it current (32 GB+ sticks only since
  the 19 GiB desktop layout).
- Provisional login: `lisa`/`lisa`, GDM autologin (on the record in
  `os/mkosi/README.md`; replaced by M7 OOBE).
- ~~A Tinker API key is staged on the field device's ESP at
  `lisa-provision/tinker.key`~~ — **stale, and it mattered (#164).** The
  importing oneshot was never shipped by any installer, so nothing would
  have imported or scrubbed a staged key; it would simply have stayed in
  plaintext on a world-readable FAT partition. Checked on the reference
  iMac 2026-08-01: no `/efi/lisa-provision` and no `*.key` anywhere on
  the ESP, so nothing is exposed. The ESP route is now removed entirely;
  keys arrive through Settings › Intelligence (API key or OAuth).

## In-flight state at handover

- PR #5 (`m5-agentd`, Agent Bus core, ADR-0009): rebased onto main with
  conflicts resolved; a watcher on the old machine was merging it on
  green CI — **verify it merged**; if not, merge it (CI was the only
  gate left).
- PR #6 (`remote-providers`, §5.11 BYO providers + Settings app,
  ADR numbered 0008 on its branch): **needs the same rebase
  treatment** — renumber its ADR to the next free number (0010 after
  #5 lands) including in-file references and `docs/adr/README.md`,
  union the `Cargo.toml` members list, reconcile `docs/STATUS.md`, then
  merge on green. Its PKGBUILD packaging was deferred (PR #3 owned the
  file at the time) — wire `lisa-remoted` + `shell/settings` into
  `os/packages/lisa/PKGBUILD` as a follow-up.
- Merged today: #1 (iMac hardware + ESP boot diagnostics), #2 (M4 shell
  surfaces), #3 (GNOME desktop lane), #4 (portal/M2, ADR-0008).
- v20260722.6 = the desktop image (8G root slots — installs on old 5G
  layouts cannot sysupdate across; both field disks were re-provisioned).

## Near-term queue (from PLAN §10 + today's deferrals)

1. Field-test the desktop: GDM/GNOME on the Radeon Pro 560, Wi-Fi via
   the shell (NetworkManager+iwd), overlay on Super+Space, launcher,
   Ledger app. Failed boots leave journals in `lisa-debug/` on the ESP.
2. M4 acceptance: §5.7 latency budgets now measurable on real hardware;
   deferred pieces (writing-tools layer 1, voice v1, wlr frontend).
3. Wire inferenced → `lisa-remoted` forwarding (`remote:byo:*` hints
   parse but don't route yet); `lisa remote` CLI verbs; streaming.
4. agentd next slices (ADR-0009): MCP wire transport behind
   `Dispatcher`, `libs/mcp-bus`, CLI verbs, full 500-attempt injection
   corpus; overlay backend swaps to `RequestCall`.
5. M6 adapter lane: Tinker (credits available) as the LoRA training
   stack for catalog models; Inkling is a PCN-tier catalog candidate.
6. Backlogged hardening: signed sysupdate manifests (drop `Verify=no`),
   dm-verity, `/etc` overlay, 4Kn support in a real `os/installer`.

## Toolchain for a fresh dev machine

rustup (stable ≥ 1.97), `cargo install just`, `gh` CLI (auth as the
repo owner), git hooks: `git config core.hooksPath .githooks`. The Rust
workspace + JS/GJS tests all run on macOS; images build only in CI.
