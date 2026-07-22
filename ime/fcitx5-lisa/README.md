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
- `src/http.{h,cpp}` — the protocol half: loopback-only OpenAI-compat
  client (plain POSIX sockets, zero dependencies — ADR-0007). All
  model behavior stays daemon-side; every generation is ledgered by
  lisa-inferenced.
- `tests/http_test.cpp` — unit tests for the protocol half; pure
  standard C++, runs on any dev host (`just ime-test`) and as a CTest.
- `CMakeLists.txt` + `lisa-addon.conf.in` — build + addon
  registration (`cmake -B build && cmake --build build && cmake
  --install build`; needs fcitx5 headers, so Arch/CI).

## Status

Working first pass of layer 2's proofread action. Growing on this
skeleton: the floating compose panel (rewrite/tone menu, "continue
writing"), dictation as an input mode (§5.7.5), and the §5.7.3
acceptance run (gedit / VS Code / Discord / xterm round-trips < 2 s on
reference-16GB) — which needs the Linux desktop rig (the iMac). The
addon compiles only against fcitx5 on Linux; CI owns the compile gate,
dev hosts run the protocol tests.
