# Developing Lisa OS on a Mac

macOS is a first-class dev host for everything in the Rust workspace and
the shell/IME unit suites (CLAUDE.md, "Repo mechanics"). This is the
path from a fresh clone to green tests. OS-image and systemd work stay
on Linux/CI — see [What stays on Linux/CI](#what-stays-on-linuxci).

## Prerequisites

- **Rust, stable** via [rustup](https://rustup.rs). There is no
  `rust-toolchain` pin; the workspace uses edition 2024, so any current
  stable works (verified with 1.97).
- **`just`** — `brew install just`.
- **A JS runtime for `just shell-test`** — the recipe auto-detects, in
  order: `gjs`, `node`, then the `jsc` binary Apple ships inside
  JavaScriptCore.framework. A stock Mac therefore needs nothing extra;
  installing node also works.
- **A C++ compiler for `just ime-test`** — the Xcode Command Line Tools
  `c++` (Apple clang) is enough; the recipe compiles the fcitx5-lisa
  protocol tests directly, no fcitx5 needed.

Optional, for real-model inference: `brew install llama.cpp` (provides
`llama-server`).

## Clone to green

```console
$ git clone https://github.com/Lisa-AgenticOS/lisa-os && cd lisa-os
$ git config core.hooksPath .githooks   # pre-push runs the lint gate
$ just build        # cargo build --workspace
$ just test         # cargo test --workspace
$ just shell-test   # shell-surface unit tests (PLAN §5.7)
$ just ime-test     # fcitx5-lisa protocol tests (PLAN §5.7.3)
```

`just lint` (fmt --check + clippy -D warnings) is the CI gate — run
`just lint && just test` before every commit (CLAUDE.md).

## Smoke tests

`just smoke` needs nothing installed: it builds `lisa-inferenced` +
`lisa`, starts the daemon, and round-trips `lisa ask` through the
deterministic **stub engine**, then checks `/health`. Expect a
`[lisa-inferenced stub] You said: …` reply — that is the pass state on
a model-less machine.

For real tokens, `just smoke-real` supervises `llama-server` over a
model from the local store:

```console
$ brew install llama.cpp                      # llama-server on PATH
$ cargo run -p lisa -- models get qwen3-0.6b-instruct-q8
$ just smoke-real                             # streams a real haiku
```

`lisa models get <id>` resolves the pinned source + blake3 hash from the
model catalog (`models/catalog/catalog.toml`) into
`~/.local/share/lisa/models`; `lisa models catalog --runnable` shows
which catalog entries fit this machine.

## What stays on Linux/CI

Per CLAUDE.md: "`just image`/`just vm` and systemd/portal work are
Linux-only and run in CI."

- `just image` / `just vm` — the mkosi Track-I image; the recipes refuse
  to run on a non-Linux host and point at `.github/workflows/nightly.yml`.
- systemd units and the xdg-desktop-portal backend — they build in the
  workspace but only *run* under Linux.
- `just egress-test` — needs a Linux systemd host; CI runs it on every
  push.
- `just layer-e2e` — the Track-L end-to-end runs in an Arch container
  via podman (on Apple silicon it substitutes the Arch Linux ARM image;
  see the justfile comment).
