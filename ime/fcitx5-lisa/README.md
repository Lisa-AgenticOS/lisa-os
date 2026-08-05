# fcitx5-lisa — writing tools & dictation, where IM protocols reach

Spec: docs/PLAN.md §5.7.3 layer 2, §5.7.5. Milestone: M4. ADR-0007
(why C++, and why it stays thin).

The input-method trick that reaches text fields through the IM
protocols — GTK3, Qt, Electron, terminals, XWayland. That is the
coverage Apple gets via private toolkit hooks, and under GNOME Wayland
it is **not** everything (#208). Re-checked on the reference iMac,
build `20260805.81`:

- **GTK4/Wayland-native apps do not route through fcitx.** mutter does
  not grant `zwp_input_method_v2` to third-party input methods — GNOME
  routes text input to its own ibus path. fcitx5 says so itself, in
  its own self-diagnosis, on every start:

      waylandmodule.cpp:666] Using Wayland native input method protocol: 0

  Most of Lisa's own apps are in this set. (The earlier diagnosis said
  the symptom was `waylandim` loading and unloading on every start.
  That is **not** what this build does: `waylandim` loads and stays
  loaded, and the only `Unloading addon waylandim` line in the journal
  is part of the teardown of all nineteen addons when an old instance
  exits on `--replace`. The mechanism was right, the symptom was
  someone else's. The line above is the one to check.)
- **The shell itself routes keys to no IM at all**, so nothing here can
  work on the desktop or in the overview by construction.
- What *does* work is everything reached by `GTK_IM_MODULE=fcitx` /
  `QT_IM_MODULE=fcitx` / `XMODIFIERS=@im=fcitx`, exported from
  `/etc/environment.d/50-lisa-ime.conf` so the user manager applies it
  before the session starts. (`/etc/profile.d` was the earlier attempt
  and is sourced by login shells only, which the graphical session is
  not — that is why the environment was empty.) Verified on the
  reference device: `/proc/<gnome-shell>/environ` carries all three.

**The Shell fork does not change this by itself.** Lisa Desktop ships a
forked GNOME Shell (ADR-0038/0048) and it is what runs on the device —
`/usr/share/licenses/lisa-desktop-shell/` exists and
`/usr/share/licenses/gnome-shell` does not. But **mutter is stock and
stays stock** (50.4 on the device; ADR-0048 rule: toolkit and
compositor are foundation, not experience), and bare-modifier gestures
are decided in libmutter — `overlay-key`, the tap-Super gesture, is
implemented there and not in the shell binary. So owning a *system-wide*
double-tap Shift is still an open design question, not a thing the fork
already grants. Until it is answered the gesture below is
**toolkit-scoped**, and `Super+Shift+Space` is the summon that works
everywhere.

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
- `notifications.conf` + `tests/notifications_test.cpp` — the shipped
  fcitx5 notification suppression (#191) and the test that keeps its
  marshalling form intact (#201). See "The IM panel" below.
- `CMakeLists.txt` + `lisa-addon.conf.in` — build + addon
  registration (`cmake -B build && cmake --build build && cmake
  --install build`; needs fcitx5 headers, so Arch/CI).

## Overlay summon (double-tap Shift)

**Scope, first, because it is the thing this gets wrong:** the detector
and the whole D-Bus chain work — injecting two clean Shift taps through
fcitx5's own frontend produces exactly one `Summon` call on the session
bus, measured. What does not happen is real keystrokes reaching fcitx5
in a GTK4/Wayland app or on the shell, for the reasons above. So this
fires in a terminal or an XWayland app and not on the desktop.

**As a system-wide gesture it is not bound at all** (#208). Nothing in
the compositor, the Shell fork, or any of the three Lisa extensions
binds a double-tap of Shift; the only implementation of the gesture in
this OS is the state machine in `src/doubleshift.cpp`, which by
definition can only see the keys fcitx5 is given. On the reference
device (`20260805.81`) grepping every shipped shell surface for a
Shift-tap handler finds three source *comments* and no handler:

    $ grep -rniE 'double.?tap|doubleShift|Shift_L|Shift_R|KEY_LEFTSHIFT' \
        /usr/share/lisa/shell/
    .../overlay-extension/extension.js:686:  // shape people know from Siri. Double-tap-Shift asks for this;
    .../overlay-extension/extension.js:700:  // double-tap.
    .../overlay-extension/extension.js:707:  // A second double-tap while listening means "stop", the same

What *is* live on that device: the addon is installed and loaded
(`/usr/lib/fcitx5/lisa.so`; `addonmanager.cpp:204] Loaded addon lisa`
on every fcitx5 start), no `~/.config/fcitx5/conf/lisa.conf` exists so
`DoubleShiftSummon` sits at its default of True, and the call's target
is up — `dev.lisaos.Overlay1.UI` is owned by `gnome-shell`. Every part
of the chain is in place except the one that cannot be: delivery of the
keystroke. **No one has yet pressed Shift twice on that device and
watched the overlay appear**, so "works in a GTK3/Qt/XWayland app" is
still a claim from the injected-event test, not from a keyboard.

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

## The IM panel: whose job it is (#191)

fcitx5 nags once per boot on GNOME Wayland — *"It is recommended to
install Input Method Panel GNOME Shell Extensions to provide the input
method popup… Otherwise you may not be able to see input method popup
when typing in GNOME Shell's activities search box."* Two ways out:
ship `kimpanel`, or let the Shell fork own the panel. **The fork owns
it**, and here is why on evidence rather than taste.

**A panel is a place for fcitx5 to draw candidates and preedit.** Under
Wayland an input method cannot place its own popup; a Shell extension
draws it over D-Bus (`org.kde.kimpanel.inputmethod`). Three facts
decide which half of the problem that solves for Lisa, all measured on
the reference iMac, `20260805.81`:

1. **fcitx5 is not in the path for the surfaces the nag names.** Its
   own self-diagnosis, every start:

       waylandmodule.cpp:666] Using Wayland native input method protocol: 0

   mutter grants no `zwp_input_method_v2`, so GTK4/Wayland apps *and
   the shell itself* — including the activities search box the message
   calls out — never reach fcitx5 at all. kimpanel would install a
   surface with nothing to display for precisely the case that
   motivates it.
2. **The client half is already live and ownerless.** fcitx5 ships its
   own `kimpanel` addon and it loads on every start; what is missing is
   an owner of the name, not a client:

       addonmanager.cpp:204] Loaded addon kimpanel
       kimpanel.cpp:138] Kimpanel new owner:

   (empty owner; `ListNames` on the session bus returns no kimpanel
   name, and no `*kimpanel*` extension exists in either
   `/usr/share/gnome-shell/extensions/` or the user's.) So the work is
   to *own a bus name from the Shell*, which is what a forked Shell is
   for — not to import a third-party extension to talk to a client we
   already have.
3. **There is nothing to draw yet.** `~/.config/fcitx5/profile` on the
   device is `DefaultIM=keyboard-us` — one plain keyboard layout,
   which produces no candidate list and no preedit. And the only Lisa
   addon in the picture emits a commit and nothing else: `lisa.cpp`
   calls `ic->commitString(result)` and never touches a candidate list
   or preedit. That answers the question the issue asked to check
   first: **the writing-tools popup needs no candidate window**, so
   option 1 does not become mandatory.

Against that, kimpanel costs an unpinned AUR port and the shell-version
compat break at every GNOME bump — the same trap our own extensions
have already been caught by — to render zero popups today.

**Where the panel lands when it is written: the `lisa-desktop` repo**,
not `shell/` here. That tree builds `lisa-desktop-shell`, which is the
package the image installs from the hosted index (pinned by
`os/mkosi/desktop.lock`), and it is the fork ADR-0038/0048 make ours.
`shell/` in this monorepo is the duplicate ADR-0039 step 6 has not
removed yet; a panel added here would not ship. The trigger to write it
is Lisa shipping a real input method (CJK, or the §5.7.3 compose
panel) — before that a panel has no content, and this note exists so
the next person does not rediscover the three facts above.

**Until then the nag is off, and the nag being off is the whole of what
shipped.** `notifications.conf` in this directory installs to
`/etc/xdg/fcitx5/conf/notifications.conf` (see the PKGBUILD's
`package_lisa-ime`) and hides fcitx5's own tip id:

    [HiddenNotifications]
    0=wayland-diagnose-gnome

The **section form is load-bearing** (#201). fcitx-config marshals this
option as `Option<std::vector<std::string>>`, i.e. numbered subkeys
under a section; `HiddenNotifications=wayland-diagnose-gnome` and
`HiddenNotifications/0=…` both parse cleanly, unmarshal to an empty
vector, and the nag comes back with no error, no warning and no log
line. `tests/notifications_test.cpp` (`just ime-test`) rejects all
three ways of getting that wrong — the flat key, the slash key, and a
list numbered from 1.

Verified by execution on the device 2026-08-03 with both controls: no
suppression → the diagnose runs *and* notifies (4 notification calls on
the bus); this form → the diagnose still runs, zero notifications. That
proof was not reproducible on 2026-08-05, and the reason is worth
recording rather than papering over: in the session then running,
`org.freedesktop.Notifications` had **no owner at all** —

    $ gdbus call --session --dest org.freedesktop.DBus \
        --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.GetNameOwner org.freedesktop.Notifications
    Error: GDBus.Error:org.freedesktop.DBus.Error.NameHasNoOwner: The name does not have an owner

so *no* fdo notification could be delivered by anything, and a "zero
notifications" result would have proved nothing about the suppression.
A re-verification needs a fresh boot into an autologin session.

## Packaging

Built as **`lisa-ime`**, an output of the `lisa` split PKGBUILD
(`os/packages/lisa/`) — same tarball and version as the rest of Lisa, so
the addon cannot drift from the daemon it talks to. It ships
`/usr/lib/fcitx5/lisa.so` and the addon registration, plus two things
without which the addon is present and inert:

- `/etc/environment.d/50-lisa-ime.conf` — `GTK_IM_MODULE`/
  `QT_IM_MODULE`/`XMODIFIERS`. GNOME defaults to IBus, so without these
  fcitx5 runs, the addon loads, and not one key ever reaches it. (GTK4
  Wayland apps negotiate text-input-v3 with mutter and ignore these;
  the variables are for GTK3, Qt and XWayland, which is most of what
  people run.) **This is the file that works** — the user manager
  imports `environment.d` before the graphical session starts, so the
  shell and everything it launches inherit the variables.
- `/etc/profile.d/lisa-ime.sh` — the earlier attempt at the same thing,
  still shipped for login shells. It does **nothing** for the graphical
  session, which is not a login shell; that is why the shell's environ
  was empty until `environment.d` landed (#208).
- `/etc/xdg/autostart/fcitx5-lisa.desktop` — starts fcitx5 with the
  session, ordered after GNOME's own ibus-daemon and taking over with
  `--replace`.

## Status

Working first pass of layer 2's proofread action, plus the
double-tap-Shift overlay summon. Until 2026-07-31 nothing built the
addon, so it existed only as source and unit tests.

Where it stands on the reference iMac (`20260805.81`, checked
2026-08-05, #208): **installed and loading, never exercised from a
keyboard.** `lisa.so` is on disk, `Loaded addon lisa` appears on every
fcitx5 start, the IM environment variables reach the session, and the
overlay owns its bus name. What has *not* been done on hardware is the
part that would justify the word "works": select text and hit
Control+Alt+Space, or tap Shift twice in a GTK3/XWayland app and watch
the overlay. Neither action has been performed on a device, so neither
is claimed here.

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
