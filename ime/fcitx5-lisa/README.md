# fcitx5-lisa — writing tools & dictation everywhere

Spec: docs/PLAN.md §5.7.3 layer 2, §5.7.5. Milestone: M4. ADR-0007
(why C++, and why it stays thin).

The input-method trick that reaches everything that accepts text —
GTK, Qt, Electron, terminals, XWayland: IM protocols are the coverage
Apple gets via private toolkit hooks.

## Layout

- `src/lisa.cpp` — the fcitx5 addon (C++, Fcitx5Core; Linux-only).
  v1: select text in any app, hit the trigger key (default
  Control+Alt+Space) → the selection is proofread by lisa-inferenced
  and committed back via the IM commit string (a commit replaces the
  active selection in standard toolkits). Trigger key, endpoint, and
  timeout are fcitx-configurable. The HTTP round-trip runs off the
  fcitx loop; the commit hops back via EventDispatcher with a watched
  InputContext reference.
- `src/doubleshift.{h,cpp}` — double-tap-Shift → summon the assistant
  overlay. A pure state machine (no fcitx5 types, unit-tested
  anywhere, like the http half): press+release of a bare Shift key
  (either side, mixing sides counts), twice, second press within
  400 ms of the first release → fire on the second release. Any other
  key in between, any non-Shift modifier held (Ctrl/Alt/Super), a
  chorded second Shift, or a repeat resets; a too-slow second tap
  restarts the sequence as a fresh first tap. After a fire, a 1 s
  debounce swallows extra taps (triple-tap fires once). Shift events
  are never consumed — taps pass through to the app.
- `src/http.{h,cpp}` — the protocol half: loopback-only OpenAI-compat
  client (plain POSIX sockets, zero dependencies — ADR-0007). All
  model behavior stays daemon-side; every generation is ledgered by
  lisa-inferenced.
- `tests/http_test.cpp`, `tests/doubleshift_test.cpp` — unit tests
  for the fcitx5-free halves; pure standard C++, run on any dev host
  (`just ime-test`) and as CTests.
- `CMakeLists.txt` + `lisa-addon.conf.in` — build + addon
  registration (`cmake -B build && cmake --build build && cmake
  --install build`; needs fcitx5 headers, so Arch/CI).

## Overlay summon (double-tap Shift)

On detection the addon calls `Summon("", {})` on the session bus —
`dev.lisaos.Overlay1.UI` at `/dev/lisaos/Overlay1/UI` (ADR-0016
names; the authoritative interface XML lives in
`shell/overlay-extension/lib/iface.js`). An empty prompt just shows
the layer, exactly like Super+Shift+Space. The call rides fcitx5's
own `dbus` addon (declared as an optional dependency in
`lisa-addon.conf.in`) via `callAsync` with a 1 s reply timeout:
fire-and-forget, never blocking key processing; if no overlay
frontend owns the name, nothing happens.

Config: `DoubleShiftSummon` (bool, default **True**) in the addon's
fcitx configuration (`conf/lisa.conf`, or fcitx5-configtool →
Addons → Lisa Writing Tools) turns the gesture off.

## Packaging

Built as **`lisa-ime`**, an output of the `lisa` split PKGBUILD
(`os/packages/lisa/`) — same tarball and version as the rest of Lisa, so
the addon cannot drift from the daemon it talks to. It ships
`/usr/lib/fcitx5/lisa.so` and the addon registration, plus two things
without which the addon is present and inert:

- `/etc/profile.d/lisa-ime.sh` — `GTK_IM_MODULE`/`QT_IM_MODULE`/
  `XMODIFIERS`. GNOME defaults to IBus, so without these fcitx5 runs,
  the addon loads, and not one key ever reaches it. (GTK4 Wayland apps
  negotiate text-input-v3 with mutter and ignore these; the variables
  are for GTK3, Qt and XWayland, which is most of what people run.)
- `/etc/xdg/autostart/fcitx5-lisa.desktop` — starts fcitx5 with the
  session, ordered after GNOME's own ibus-daemon and taking over with
  `--replace`.

## Status

Working first pass of layer 2's proofread action, plus the
double-tap-Shift overlay summon. **Neither has ever run on a device**:
until 2026-07-31 nothing built the addon, so it existed only as source
and unit tests. It is now packaged and in the image lane, and still
unverified on hardware.

It also did not compile. Arch's fcitx5 exports only the innermost
include directory from `Fcitx5::Module::DBus`, so the canonical
`<fcitx-module/dbus/dbus_public.h>` could not resolve, and the
`FCITX_ADDON_DEPENDENCY_LOADER` macro sat below the method that calls
it, which C++ rejects for a deduced return type. Both are fixed. The CI
job that compiles this was path-filtered to `ime/**`, so an upstream
move underneath us produced no signal at all — the filter now watches
the PKGBUILD too.

Growing on this skeleton: the floating compose panel (rewrite/tone menu, "continue
writing"), dictation as an input mode (§5.7.5), and the §5.7.3
acceptance run (gedit / VS Code / Discord / xterm round-trips < 2 s on
reference-16GB) — which needs the Linux desktop rig (the iMac). The
addon compiles only against fcitx5 on Linux; CI owns the compile gate,
dev hosts run the protocol tests.
