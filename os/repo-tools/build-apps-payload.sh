#!/usr/bin/env bash
# Stage the Lisa apps tree — the ONE list of what a Lisa device runs as
# interpreted GJS (ADR-0020, ADR-0047).
#
# Two consumers, one list, deliberately:
#
#   * os/packages/lisa/PKGBUILD stages it into /usr/share/lisa/shell, the
#     copy the image bakes;
#   * .github/workflows/release.yml packs it into lisa-apps_<ver>.tar.zst,
#     the copy `lisa apps update` installs on /var.
#
# Issue #239 defect 2 is what happens when they are two lists: the release
# tarball was `tar -C shell .`, so it carried the shell surfaces and NONE
# of apps/ — Mail, Surfer and Preview could only be fixed by a full image
# update and a reboot, while ADR-0020 promised otherwise. The package, one
# file away, had been copying apps/ into the same tree since Surfer landed.
#
# `lisa-app <relpath>` resolves entries relative to the root of this tree,
# so "mail/lisa-mail.js" must mean the same thing in both copies. That is
# pinned by cli/lisa/tests/apps_payload.rs, which checks every .desktop and
# D-Bus service file the package installs against the staged tree.
#
# Usage: build-apps-payload.sh <stage-dir> [tarball]
set -euo pipefail

usage="usage: build-apps-payload.sh <stage-dir> [tarball]"
ap_dest=${1:?$usage}
ap_tarball=${2:-}
ap_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

# The GJS shell surfaces (PLAN §5.7). shell/settings is deliberately NOT
# here: the standalone Settings app was merged into GNOME Settings as the
# Intelligence panel (ADR-0012 v2), so nothing launches it, and shipping a
# second settings surface is how the wrong one gets edited. Its source
# stays in the tree as the panel's reference.
ap_surfaces=(overlay-extension launcher desktop ledger-app assistant consent)
# The apps (PLAN §5.8) that ride the same tree and the same launcher.
ap_apps=(surfer mail preview)

mkdir -p "$ap_dest"
for s in "${ap_surfaces[@]}"; do
    cp -a "$ap_root/shell/$s" "$ap_dest/"
done
for a in "${ap_apps[@]}"; do
    cp -a "$ap_root/apps/$a" "$ap_dest/"
done

# Test material and scratch trees are not payload. Scoped to the staging
# directory this script just created — it never touches the source tree.
find "$ap_dest" -depth -type d \( -name tests -o -name testing -o -name spike \) \
    -exec rm -rf {} +

# A tree with no assistant is not a Lisa apps tree: that entry point is
# `lisa apps`' probe for "is this a real payload", so a staging change
# that silently drops it must fail here rather than on a desktop.
test -f "$ap_dest/assistant/lisa-assistant.js" || {
    echo "build-apps-payload.sh: staged tree has no assistant entry point" >&2
    exit 1
}

if [ -n "$ap_tarball" ]; then
    # Contents at the tarball root, no wrapping directory — `lisa apps`
    # unpacks straight into versions/<ver>/.
    tar --zstd -cf "$ap_tarball" -C "$ap_dest" .
    echo ">> $ap_tarball ($(du -sh "$ap_dest" | cut -f1) staged)"
fi
