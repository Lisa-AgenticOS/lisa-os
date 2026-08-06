# Lisa OS monorepo task runner (PLAN §9, Appendix D).

default: build

build:
    cargo build --workspace

test:
    cargo test --workspace

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    # A stray apostrophe in a workflow comment closes the container
    # script and fails the build somewhere unrelated. Cheap to check.
    python3 os/repo-tools/check-workflow-quoting.py
    # Mount-based sandboxing in a per-user unit silently breaks peer
    # identity for the whole session (#161). Cheap to check, brutal to
    # debug from the refusals it causes.
    python3 os/repo-tools/check-user-units.py
    # A no-egress daemon whose unit says so in a comment and enforces
    # nothing is how lisa-inferenced-dbus shipped (#275). The units are
    # discovered, so a new daemon cannot arrive unclassified.
    python3 os/repo-tools/check-egress-units.py
    # A/B root slots that differ in size build and boot fine, then
    # corrupt the first update — sysupdate writes A's byte image into B.
    # A comment saying "MUST match" is not a mechanism.
    python3 os/repo-tools/check-repart-slots.py
    # A stale EMBEDDING_MODEL does not error — the embedder just never
    # finds it and silently falls back to a chat model (#163).
    python3 os/repo-tools/check-embedding-model.py
    # The three-violets defect, mechanized (ADR-0038 step 1): any hex a
    # shell/app surface hardcodes must be a branding/tokens.json token,
    # and the generated token sheets must match their source.
    python3 os/repo-tools/check-tokens.py
    # A manifest installed where agentd does not look does not error,
    # warn or log — the app's tools simply never reach the model (#241).
    python3 os/repo-tools/check-app-manifests.py
    # A chord reserved in a comment and bound by nobody is a feature that
    # exists everywhere except on the keyboard (#255).
    python3 os/repo-tools/check-shell-keys.py
    # favorite-apps in the wrong override stanza compiles to nothing: the
    # dock shipped a default that had never been applied (#263).
    python3 os/repo-tools/check-dock.py
    # docs/KEYBOARD.md is derived from the files that bind the keys, so
    # nobody has to trust a table (#257). A shortcut added, removed or
    # rebound without the map following fails here.
    python3 os/repo-tools/build-keymap.py --check
    # A GTK4 set_parent() with no unparent() finalizes the parent with a
    # child attached — two Surfer sessions ended there, with no crash and
    # no window (#258).
    python3 os/repo-tools/check-widget-lifecycle.py
    # The knowledge pack (#175) is generated from component READMEs; a
    # stale committed copy would ship the model answers about last
    # month's OS.
    python3 os/repo-tools/build-knowledge.py --check
    # A boot splash pointed at a theme that no longer exists is a black
    # screen, and on hardware with no ACPI BGRT image a correct Lisa
    # splash is ALSO a black screen — so the two are indistinguishable
    # by eye. #283 shipped dangling for eight days on exactly that
    # margin. --selftest first: a checker whose own cases do not behave
    # must not be trusted to have checked anything.
    os/mkosi/check-plymouth.sh --selftest
    os/mkosi/check-plymouth.sh os/mkosi/mkosi.extra os/mkosi/initrd-overlay
    # A fork that replaces stock BY NAME wins only by pkgrel, and loses
    # the day Arch ships a higher pkgver — which it did on 2026-08-04,
    # and the device still carries `gnome-control-center 50.3-2` as a
    # result (#284). The repo half runs here, offline: it fails on a
    # PKGBUILD that takes a stock name, on half a provides/conflicts
    # contract, on a contract with no replaces= to act on it (R8 — the
    # rename that read as done everywhere and had reached no device),
    # and on anything forking the foundation. The manifest
    # half needs an image, so it runs where there is one:
    #   check-desktop-inventory.py --manifest <root>/usr/lib/lisa/packages.manifest --lane release
    python3 os/repo-tools/check-desktop-inventory.py
    # The ADR index used to be written by hand, and said "36 of the 37
    # records below" while there were 50. A page describing 50 files
    # must be derived from them: this rejects a status line outside the
    # vocabulary, a supersession pointing at nothing, and a stale index.
    python3 os/repo-tools/build-adr-index.py --check

fmt:
    cargo fmt --all

# Shell- and app-surface unit tests (PLAN §5.7, §5.8): pure-logic
# modules under shell/*/tests AND apps/*/tests — both trees, which is
# what the glob below has always covered and what CI now runs (#242).
# Runtime-agnostic — first JS runtime found wins: gjs (Linux/image),
# node (CI), jsc (macOS ships it).
shell-test:
    #!/usr/bin/env bash
    set -euo pipefail
    JSC=/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc
    if command -v gjs >/dev/null; then RUN=(gjs -m)
    elif command -v node >/dev/null; then RUN=(node)
    elif [ -x "$JSC" ]; then RUN=("$JSC" -m)
    else echo "no JS runtime found (gjs, node, or macOS jsc)" >&2; exit 1; fi
    for t in shell/*/tests/*.test.js apps/*/tests/*.test.js; do
        [ -e "$t" ] || continue
        echo "== $t"
        "${RUN[@]}" "$t"
    done

# fcitx5-lisa protocol tests (PLAN §5.7.3, ADR-0007). Pure C++/POSIX —
# runs anywhere; the addon itself compiles against fcitx5 in CI.
ime-test:
    #!/usr/bin/env bash
    set -euo pipefail
    out=$(mktemp -d)
    trap 'rm -rf "$out"' EXIT
    c++ -std=c++17 -Wall -Wextra -Iime/fcitx5-lisa/src -o "$out/http_test" \
        ime/fcitx5-lisa/tests/http_test.cpp ime/fcitx5-lisa/src/http.cpp
    "$out/http_test"
    c++ -std=c++17 -Wall -Wextra -Iime/fcitx5-lisa/src -o "$out/doubleshift_test" \
        ime/fcitx5-lisa/tests/doubleshift_test.cpp ime/fcitx5-lisa/src/doubleshift.cpp
    "$out/doubleshift_test"
    # The Wayland-diagnose nag stays suppressed only while
    # notifications.conf keeps the section form fcitx-config unmarshals
    # (#191/#201); a flat key parses fine and hides nothing.
    c++ -std=c++17 -Wall -Wextra -DSOURCE_DIR='"ime/fcitx5-lisa/tests"' \
        -o "$out/notifications_test" ime/fcitx5-lisa/tests/notifications_test.cpp
    "$out/notifications_test" ime/fcitx5-lisa/notifications.conf

# What CI runs on every PR.
ci: lint test shell-test ime-test

# Real-model smoke: needs llama-server on PATH and a model in the store
# (see `lisa models pull/add`; the catalog pins qwen3-0.6b-instruct-q8).
smoke-real name="qwen3-0.6b-instruct-q8":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p lisa-inferenced -p lisa >/dev/null
    ./target/debug/lisa-inferenced --model "$HOME/.local/share/lisa/models/refs/{{name}}" & D=$!
    trap 'kill $D 2>/dev/null || true' EXIT
    for _ in $(seq 1 120); do curl -sf 127.0.0.1:7777/health >/dev/null 2>&1 && break; sleep 0.5; done
    ./target/debug/lisa ask "write a haiku about entropy"

# End-to-end smoke: daemon up → streamed ask → health → daemon down.
smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p lisa-inferenced -p lisa >/dev/null
    ./target/debug/lisa-inferenced & DAEMON=$!
    trap 'kill $DAEMON 2>/dev/null || true' EXIT
    sleep 1
    ./target/debug/lisa ask "write a haiku about entropy"
    curl -sf 127.0.0.1:7777/health >/dev/null && echo "health: ok"

# Build the immutable OS image (Track I). Linux only; normally CI's job.
# --- Local x86_64 Linux, on a macOS dev host ---------------------------
#
# The image and every daemon are x86_64 Linux; the dev host is usually an
# arm64 Mac. Until now the only way to compile or package-build anything
# for the target was CI, which made a 30-minute round trip the shortest
# path to "does this even build" — the worst possible feedback loop for
# the components carrying the security-sensitive logic.
#
# These run a real x86_64 Linux userspace locally. Two notes learned the
# hard way:
#
#   - podman on Apple Silicon uses qemu-user, and `rustc -vV` dies there
#     with SIGSEGV. Apple's `container` (brew install container) uses
#     Rosetta and works. The runtime is detected below; `container` wins
#     when both are present.
#   - pacman's seccomp sandbox does not survive translation
#     ("error restricting syscalls via seccomp: 22"), so package recipes
#     pass --disable-sandbox. That weakens pacman's own sandboxing inside
#     a throwaway container, not on any real system.

_runtime := if `command -v container 2>/dev/null || true` != "" { "container" } else { "podman" }

# An x86_64 Linux shell with the repo mounted at /src.
linux-shell image="docker.io/library/archlinux:latest":
    {{_runtime}} run --rm -it --arch amd64 -v "$PWD:/src" {{image}} bash

# Build one workspace crate for x86_64 Linux. Output lands in
# target/x86_64-container/release/, deliberately not target/, so it never
# collides with the host build.
linux-build crate:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target/x86_64-container
    {{_runtime}} run --rm --arch amd64 \
      -v "$PWD:/src" -v "$PWD/target/x86_64-container:/out" \
      docker.io/library/rust:latest \
      bash -c "cd /src && cargo build --release -p {{crate}} --target-dir /out --locked"
    file "target/x86_64-container/release/{{crate}}"

# makepkg one of os/packages/* against live Arch, the way CI does.
# Catches a moved kernel pin or a patch anchor in minutes, not in a
# failed release.
linux-pkg pkg:
    #!/usr/bin/env bash
    set -euo pipefail
    {{_runtime}} run --rm --arch amd64 -v "$PWD:/src:ro" \
      docker.io/library/archlinux:latest bash -c '
        set -e
        pacman -Syu --disable-sandbox --noconfirm --needed base-devel git >/dev/null
        useradd -m builder
        cp -r /src/os/packages/{{pkg}} /home/builder/pkg
        chown -R builder /home/builder/pkg
        cd /home/builder/pkg
        su builder -c "makepkg -s --nocheck --skippgpcheck --noconfirm"
        ls -la *.pkg.tar.* '

image:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" != "Linux" ]; then
        echo "just image requires Linux (mkosi); CI builds it — see .github/workflows/nightly.yml" >&2
        exit 1
    fi
    mkosi --directory os/mkosi build

# Boot the built image in QEMU. Linux only.
vm:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" != "Linux" ]; then
        echo "just vm requires Linux (mkosi qemu)" >&2
        exit 1
    fi
    mkosi --directory os/mkosi qemu

# Track L: install/uninstall the Lisa Layer on stock Arch/Omarchy.
layer-install:
    bash os/layer/install.sh

layer-uninstall:
    bash os/layer/uninstall.sh

# Full layer e2e in an Arch container (podman). Uses Arch Linux ARM on
# Apple silicon — the official archlinux image is amd64-only and systemd
# segfaults under emulation.
layer-e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    IMG=docker.io/library/archlinux:latest
    case "$(uname -m)" in arm64|aarch64) IMG=docker.io/menci/archlinuxarm:latest ;; esac
    podman rm -f lisa-e2e 2>/dev/null || true
    podman run -d --name lisa-e2e --systemd=always -v "$PWD":/src:ro "$IMG" /usr/lib/systemd/systemd
    sleep 4
    podman exec lisa-e2e bash /src/tests/e2e/layer-test.sh
    podman rm -f lisa-e2e

# Egress sandbox verification — needs a Linux systemd host (CI does this;
# locally: bash tests/e2e/egress-test.sh inside the podman machine VM).
egress-test:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" != "Linux" ]; then
        echo "egress-test needs a Linux systemd host; CI runs it on every push." >&2
        exit 1
    fi
    cargo build -p lisa-inferenced
    bash tests/e2e/egress-test.sh target/debug/lisa-inferenced
