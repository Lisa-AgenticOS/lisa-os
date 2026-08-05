#!/usr/bin/env bash
# check-desktop.sh — assert the desktop the image ACTUALLY got.
#
#   usage: check-desktop.sh <packages.manifest> [<desktop.lock>]
#
# Two questions, both answered from the BUILT ROOT rather than from a
# variable somebody could forget to update:
#
#   #273  Is the installed lisa-desktop-shell the one desktop.lock pins?
#         The shell was the image's only unpinned remote input:
#         `Packages=lisa-desktop-shell` (mkosi.conf.d/x86_64.conf)
#         resolved against the ROLLING `current` tag of the [lisa]
#         index, so two builds of the same commit could ship different
#         desktops and nothing in git recorded which. Everything else
#         is pinned — the ports by sha256 (os/packages/ports.lock), the
#         models by hash, mkosi by archive version, the in-tree
#         packages by construction.
#
#   #277  Were the shell and mutter built for the same GNOME series?
#         GNOME Shell links libmutter-<N>.so and loads Mutter's
#         typelib, so the pair is ABI-coupled, and mutter comes
#         UNPINNED from Arch while the shell is pinned here. 50.3
#         against 50.4 happens to work — libmutter keeps its soname
#         inside a major series, verified live on the reference iMac
#         (v20260805.81) — but at GNOME 51 the soname bumps and the
#         same silent drift produces a shell that cannot start. The
#         discovery point for that is a device's login screen, which is
#         the worst place a build-time fact can be discovered.
#
# The manifest is `pacman -Q` output — "<name> <version>" per line —
# which mkosi.postinst.chroot writes to /usr/lib/lisa/packages.manifest
# because /var/lib/pacman does not survive onto the shipped root (/var
# is its own partition and repart excludes it from the root copy).
#
# Callers:
#   * os/mkosi/mkosi.finalize — every lane (nightly, release, aarch64,
#     local `just image`), against $BUILDROOT before the image is
#     assembled. The nightly matters most: it builds from the rolling
#     index, so a shell publish or an Arch mutter bump shows up there
#     first, a day before a release could carry it.
#   * .github/workflows/release.yml — again, on the mounted artifact
#     that is about to be published.
#
# Nothing but bash builtins is used: mkosi's default tools tree is not
# a full distro (see mkosi.finalize's own note about findutils), and a
# gate that dies on a missing `awk` inside the sandbox is worse than no
# gate, because it fails for a reason nobody will read.

set -uo pipefail

manifest=${1-}
lock=${2-}

if [ -z "$manifest" ]; then
    echo "usage: check-desktop.sh <packages.manifest> [<desktop.lock>]" >&2
    exit 2
fi
# -f before -s, deliberately: a DIRECTORY has non-zero size, so `-s`
# alone lets one through, and then `read` fails with "Is a directory",
# the loop body never runs, and the empty-shell branch below reports
# "nothing to check" and exits 0. A caller that passed $BUILDROOT
# instead of $BUILDROOT/usr/lib/lisa/packages.manifest would have got a
# green gate that inspected nothing — the precise failure this file
# exists to prevent, one level up.
if [ ! -f "$manifest" ] || [ ! -r "$manifest" ] || [ ! -s "$manifest" ]; then
    echo "FAIL: $manifest is not a readable non-empty file — there is nothing" >&2
    echo "      to check, and a gate with nothing to check must not pass." >&2
    echo "      mkosi.postinst.chroot writes it with \`pacman -Q\`; a build" >&2
    echo "      that reaches here without one has lost that step. If you meant" >&2
    echo "      the built root, pass <root>/usr/lib/lisa/packages.manifest." >&2
    exit 1
fi

# The major GNOME series of a pacman version string: strip an epoch
# ("1:50.4-1"), strip the pkgrel ("50.4-1"), take what is left of the
# first dot. A version with no minor ("51-1") therefore answers 51 and
# not "51-1", which would compare unequal to a perfectly matching 51.2.
series() {
    local v=${1#*:}
    v=${v%%-*}
    printf '%s' "${v%%.*}"
}

shell_pkg=
shell_ver=
mutter_ver=
while read -r name ver _rest; do
    case "$name" in
    lisa-desktop-shell)
        # Always wins: on the x86_64 lanes this IS the shell, and it
        # provides/conflicts gnome-shell so both names can appear.
        shell_pkg=$name
        shell_ver=$ver
        ;;
    gnome-shell)
        # The aarch64 lane ships stock GNOME Shell (ADR-0021: the fork
        # has never been built for arm64) — an honest gap, and its ABI
        # coupling to mutter is exactly the same one.
        if [ -z "$shell_pkg" ]; then
            shell_pkg=$name
            shell_ver=$ver
        fi
        ;;
    mutter)
        mutter_ver=$ver
        ;;
    esac
done <"$manifest"

if [ -z "$shell_pkg" ]; then
    # Minimal boot-check images carry no desktop at all. Say so out
    # loud: a gate that quietly checked nothing reads exactly like a
    # gate that passed.
    echo "check-desktop: no shell in $manifest — nothing to check (no desktop in this image)."
    exit 0
fi

fail=0

# --- #273: the shell is the pinned one -----------------------------
if [ "$shell_pkg" = lisa-desktop-shell ]; then
    pinned=
    pinned_file=
    if [ -n "$lock" ] && [ -r "$lock" ]; then
        while read -r fname _sha _url; do
            case "$fname" in
            "" | \#*) continue ;;
            lisa-desktop-shell-*)
                pinned_file=$fname
                v=${fname#lisa-desktop-shell-}
                v=${v%%.pkg.tar.*}
                pinned=${v%-*} # drop the trailing -<arch> field
                ;;
            esac
        done <"$lock"
    fi

    if [ -z "$lock" ] || [ ! -r "$lock" ]; then
        echo "FAIL: lisa-desktop-shell $shell_ver is installed, but no readable"
        echo "      desktop.lock was given (\"${lock:-<none>}\") — so nothing here can"
        echo "      say whether this is the shell the commit intended or whatever"
        echo "      the rolling [lisa] index happened to hold (#273)."
        echo "      Pass os/mkosi/desktop.lock as the second argument."
        fail=1
    elif [ -z "$pinned" ]; then
        echo "FAIL: $lock pins no lisa-desktop-shell-*.pkg.tar.* file, but this"
        echo "      image installed lisa-desktop-shell $shell_ver."
        echo "      Add the line — <filename>  <sha256>  <url> — naming the file"
        echo "      on the lisa-desktop release that built it."
        echo "      See os/mkosi/README.md \"The desktop is pinned\"."
        fail=1
    elif [ "$pinned" != "$shell_ver" ]; then
        echo "FAIL: the image installed a different shell than $lock pins."
        echo "        pinned:    lisa-desktop-shell $pinned  ($pinned_file)"
        echo "        installed: lisa-desktop-shell $shell_ver"
        echo "      Either something resolved lisa-desktop-shell from the [lisa]"
        echo "      index instead of the pinned file in PackageDirectories=, or"
        echo "      the lock is stale."
        echo "      To take the newer shell deliberately: bump os/mkosi/desktop.lock"
        echo "      (filename, sha256 AND url) to the lisa-desktop release that"
        echo "      built it — one commit, recorded in git, which is the whole"
        echo "      point of the pin."
        fail=1
    else
        echo "desktop: lisa-desktop-shell $shell_ver is the version $lock pins: OK"
    fi
fi

# --- #277: the shell and mutter share a GNOME series ---------------
if [ -z "$mutter_ver" ]; then
    echo "FAIL: $shell_pkg $shell_ver is installed but mutter is not in the"
    echo "      manifest at all. A GNOME Shell without libmutter cannot start;"
    echo "      either the manifest is wrong or the image is."
    fail=1
else
    s_series=$(series "$shell_ver")
    m_series=$(series "$mutter_ver")
    if [ "$s_series" != "$m_series" ]; then
        echo "FAIL: the image pairs a mutter the shell was never built against."
        echo "        $shell_pkg $shell_ver  -> GNOME series $s_series"
        echo "        mutter $mutter_ver  -> GNOME series $m_series"
        echo "      GNOME Shell links libmutter-<N>.so and loads Mutter's typelib."
        echo "      Across a major series that soname changes, so this does not"
        echo "      degrade: the session dies at login, on a device, after the"
        echo "      image shipped."
        echo "      What to do, whichever moved:"
        echo "        * mutter moved (Arch is rolling): rebase the fork in the"
        echo "          lisa-desktop repo onto GNOME $m_series, publish the shell,"
        echo "          then bump os/mkosi/desktop.lock here; or hold the image on"
        echo "          the Arch snapshot that still carries mutter $s_series."
        echo "        * the shell moved: take the matching mutter."
        echo "      Do not silence this check — it is the backstop that makes the"
        echo "      drift loud while it is still a build (#277)."
        fail=1
    else
        echo "desktop: $shell_pkg $shell_ver and mutter $mutter_ver are both GNOME series $s_series: OK"
    fi
fi

exit "$fail"
