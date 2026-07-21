# Lisa OS monorepo task runner (PLAN §9, Appendix D).

default: build

build:
    cargo build --workspace

test:
    cargo test --workspace

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

# What CI runs on every PR.
ci: lint test

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
