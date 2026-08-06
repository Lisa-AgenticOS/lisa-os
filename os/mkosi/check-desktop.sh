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
#   #297  ...and did either question actually get ASKED? Both used to
#         be skippable in silence. A manifest with no shell line at all
#         — a truncated `pacman -Q` write, or the last line lost to a
#         missing trailing newline — printed "nothing to check" and
#         exited 0; and the whole #273 pin block was guarded on the
#         shell being the FORK, so an image that shipped STOCK
#         gnome-shell never consulted desktop.lock and still printed
#         OK. That is the case where the pin matters most: CLAUDE.md
#         records 2026-08-04, when a stock-named package outranked the
#         fork by pkgrel. Both are now failures, and the one legitimate
#         stock lane (aarch64, ADR-0021) is recognised from the
#         manifest rather than assumed — and says out loud that the pin
#         went unchecked.
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
# gate, because it fails for a reason nobody will read. (`--selftest`
# is the one exception — it uses `mktemp` and runs only when a human or
# CI asks for it, never inside mkosi.)
#
#   check-desktop.sh --selftest
#
# ...exists because #297 was three vacuous passes in a row: no shell
# line, a stock shell, and a manifest whose last line had no trailing
# newline. Every one of them exited 0. The cases below are that
# mutation matrix, made permanent, and they include the DELETION shapes
# — the pin removed from the lock, mutter removed from the manifest,
# the shell removed from the manifest — because deletion is what
# actually happens and it was the one shape nothing here saw.

set -uo pipefail

manifest=${1-}
lock=${2-}

if [ "$manifest" = "--selftest" ]; then
    tmp=$(mktemp -d) || exit 1
    trap 'rm -rf "$tmp"' EXIT
    rc=0

    good_lock=$tmp/desktop.lock
    printf '%s\n' \
        '# a comment line' \
        'lisa-desktop-shell-50.4-1-x86_64.pkg.tar.zst  aaaa  https://example.invalid/x' \
        > "$good_lock"

    printf 'gnome-session 50.1-1\nlisa-desktop-shell 50.4-1\nmutter 50.4-1\n' \
        > "$tmp/fork.mft"
    # The trailing-newline case: the shell is the LAST line and there is
    # no final newline. `read` returns non-zero on it, so the old loop
    # dropped it and the script reported "no desktop in this image".
    printf 'mutter 50.4-1\nlisa-desktop-shell 50.4-1' > "$tmp/notrail.mft"
    printf 'foo 1.0\nbar 2.0\n'                       > "$tmp/noshell.mft"
    printf 'gnome-shell 1:50.4-1\nmutter 50.4-1\n'    > "$tmp/stock.mft"
    printf 'gnome-shell 1:50.4-1\nlinux-aarch64 6.16-1\nmutter 50.4-1\n' \
        > "$tmp/arm.mft"
    printf 'lisa-desktop-shell 50.3-1\nmutter 50.4-1\n' > "$tmp/stalepin.mft"
    printf 'lisa-desktop-shell 50.4-1\n'                > "$tmp/nomutter.mft"
    printf 'lisa-desktop-shell 50.4-1\nmutter 51.0-1\n' > "$tmp/abi.mft"

    printf '%s\n' '# nothing pinned here' > "$tmp/empty.lock"
    printf '%s\n' \
        'lisa-desktop-shell-50.4-1-x86_64.pkg.tar.zst  aaaa  https://example.invalid/x' \
        'lisa-desktop-shell-50.5-1-x86_64.pkg.tar.zst  bbbb  https://example.invalid/y' \
        > "$tmp/two.lock"
    # A `.sig` and a `-debug` split package published beside the real
    # artifact both match a bare `lisa-desktop-shell-*` glob.
    printf '%s\n' \
        'lisa-desktop-shell-debug-50.4-1-x86_64.pkg.tar.zst  cccc  https://example.invalid/d' \
        'lisa-desktop-shell-50.4-1-x86_64.pkg.tar.zst  aaaa  https://example.invalid/x' \
        'lisa-desktop-shell-50.4-1-x86_64.pkg.tar.zst.sig  dddd  https://example.invalid/s' \
        > "$tmp/noisy.lock"

    mkdir -p "$tmp/adir"
    : > "$tmp/empty.mft"

    expect() {                    # expect <rc> <must-contain|-> <label> <args...>
        local want=$1 needle=$2 label=$3; shift 3
        local out got
        out=$("$0" "$@" 2>&1); got=$?
        if [ "$got" -ne "$want" ]; then
            printf 'selftest FAIL %s: wanted exit %d, got %d\n' "$label" "$want" "$got"
            printf '%s\n' "$out" | while IFS= read -r l; do printf '              | %s\n' "$l"; done
            rc=1
            return
        fi
        if [ "$needle" != "-" ]; then
            case $out in
                *"$needle"*) ;;
                *)  printf 'selftest FAIL %s: exit %d as wanted, but the output never said "%s"\n' \
                        "$label" "$got" "$needle"
                    printf '%s\n' "$out" | while IFS= read -r l; do printf '              | %s\n' "$l"; done
                    rc=1
                    return ;;
            esac
        fi
        printf 'selftest ok   %s (exit %d)\n' "$label" "$got"
    }

    # -- the positive controls. Without these every "fails" below could
    #    be the script failing for some unrelated reason.
    expect 0 "is the version"  "the pinned fork on a matching mutter passes" \
        "$tmp/fork.mft" "$good_lock"
    expect 0 "is the version"  "a lock with a .sig and a -debug line still resolves the pin" \
        "$tmp/fork.mft" "$tmp/noisy.lock"
    expect 0 "PIN NOT CHECKED" "the aarch64 lane passes and SAYS the pin went unchecked" \
        "$tmp/arm.mft" "$good_lock"

    # -- deletion: the thing to be checked is GONE (#297)
    expect 1 "names neither" "a manifest with NO shell line fails" \
        "$tmp/noshell.mft" "$good_lock"
    expect 1 "STOCK gnome-shell" "stock gnome-shell outside the aarch64 lane fails" \
        "$tmp/stock.mft" "$good_lock"
    expect 1 "pins no lisa-desktop-shell" "a lock with the pin line DELETED fails" \
        "$tmp/fork.mft" "$tmp/empty.lock"
    expect 1 "but mutter is not in the" "a manifest with mutter DELETED fails" \
        "$tmp/nomutter.mft" "$good_lock"
    expect 1 "no readable" "a lock argument that is not passed at all fails" \
        "$tmp/fork.mft"
    expect 1 "is not a readable non-empty file" "a manifest that does not exist fails" \
        "$tmp/nope.mft" "$good_lock"
    expect 1 "is not a readable non-empty file" "an empty manifest fails" \
        "$tmp/empty.mft" "$good_lock"
    expect 1 "is not a readable non-empty file" "a DIRECTORY passed as the manifest fails" \
        "$tmp/adir" "$good_lock"

    # -- alteration: the thing to be checked is WRONG
    expect 1 "different shell than" "a shell newer than the pin fails" \
        "$tmp/stalepin.mft" "$good_lock"
    expect 1 "never built against" "a shell/mutter series mismatch fails" \
        "$tmp/abi.mft" "$good_lock"
    expect 1 "package lines" "a lock pinning the shell twice fails" \
        "$tmp/fork.mft" "$tmp/two.lock"

    # -- parsing: the manifest is read whole
    expect 0 "is the version" "the shell on a final line with NO trailing newline is seen" \
        "$tmp/notrail.mft" "$good_lock"

    [ "$rc" -eq 0 ] && { echo "selftest: all cases behaved"; exit 0; }
    echo "selftest: THIS SCRIPT IS BROKEN — it did not react as stated above" >&2
    exit 1
fi

if [ -z "$manifest" ]; then
    echo "usage: check-desktop.sh <packages.manifest> [<desktop.lock>] | --selftest" >&2
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
lines=0
arm_kernel=
# `|| [ -n "$name" ]`: `read` returns non-zero on a final line with no
# trailing newline, and the loop body is then skipped — so a manifest
# whose last line is the shell silently LOSES the shell and the old
# code reported "no desktop in this image", exit 0 (#297). The whole
# defect in one missing clause.
while read -r name ver _rest || [ -n "$name" ]; do
    [ -n "$name" ] || continue
    lines=$((lines + 1))
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
    linux-aarch64)
        # The ONE fact in the manifest that says which lane built this
        # image: mkosi.conf.d/aarch64.conf installs `linux-aarch64`
        # (ALARM has no `linux` package) and `gnome-shell`, while the
        # x86_64 lane installs `linux` and `lisa-desktop-shell`. Read
        # from the built root, like everything else here — the
        # alternative is a flag every caller has to remember, which is
        # the class of thing this file exists to stop relying on.
        arm_kernel=$name
        ;;
    esac
done <"$manifest"

fail=0

if [ -z "$shell_pkg" ]; then
    # #297: this used to print "nothing to check (no desktop in this
    # image)" and exit 0 — vacuous-by-absence, the exact shape of
    # failure this script was written to catch one level up.
    #
    # There is no desktopless lane. Both mkosi profiles install a shell
    # (mkosi.conf.d/x86_64.conf -> lisa-desktop-shell,
    # mkosi.conf.d/aarch64.conf -> gnome-shell), so a manifest with
    # neither is a broken build or a broken manifest, never "no desktop
    # here". If a desktopless profile is ever added, this branch is
    # where it gets an explicit, named opt-out — not a silent pass.
    echo "FAIL: $manifest names neither lisa-desktop-shell nor gnome-shell"
    echo "      in its $lines line(s). Every mkosi profile installs one of the"
    echo "      two (x86_64.conf -> lisa-desktop-shell, aarch64.conf ->"
    echo "      gnome-shell), so this is a build that lost its desktop or a"
    echo "      \`pacman -Q\` write that was truncated — not an image without"
    echo "      one. Nothing below could run, and a gate with nothing to check"
    echo "      must not exit 0."
    exit 1
fi

# --- #273: the shell is the pinned one -----------------------------
if [ "$shell_pkg" = lisa-desktop-shell ]; then
    pinned=
    pinned_file=
    pin_lines=0
    if [ -n "$lock" ] && [ -r "$lock" ]; then
        while read -r fname _sha _url || [ -n "$fname" ]; do
            case "$fname" in
            "" | \#*) continue ;;
            # `.sig` published beside the package, and the `-debug`
            # split package, both match a bare `lisa-desktop-shell-*`
            # glob. The version field of a pacman filename always
            # starts with a digit, so requiring one after the prefix
            # separates the artifact from its neighbours without a
            # regex engine — and `release.yml`'s fetch loop shares this
            # parse, so a `.sig` line here became a download there.
            lisa-desktop-shell-[0-9]*.pkg.tar.*.sig) continue ;;
            lisa-desktop-shell-[0-9]*.pkg.tar.*)
                pin_lines=$((pin_lines + 1))
                pinned_file=$fname
                v=${fname#lisa-desktop-shell-}
                v=${v%%.pkg.tar.*}
                pinned=${v%-*} # drop the trailing -<arch> field
                ;;
            esac
        done <"$lock"
    fi

    if [ "$pin_lines" -gt 1 ]; then
        # Taking the LAST match silently made a second line win. Two
        # pins for one package is not a pin.
        echo "FAIL: $lock carries $pin_lines lisa-desktop-shell package lines."
        echo "      The last one silently won, so which desktop this image was"
        echo "      supposed to contain is not a fact the lock states. Leave one."
        fail=1
    elif [ -z "$lock" ] || [ ! -r "$lock" ]; then
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
elif [ -n "$arm_kernel" ]; then
    # The one lane where a stock shell is the recorded, intended state
    # (ADR-0021), and desktop.lock's own footer says so: the fork is
    # arch=(x86_64), so there is nothing to pin here. Still NOT silent —
    # the pin is the image's only unpinned remote input, and "the check
    # did not run" has to read differently from "the check passed".
    echo "desktop: PIN NOT CHECKED — this image ships STOCK gnome-shell"
    echo "         $shell_ver on the aarch64 lane ($arm_kernel), where the fork"
    echo "         does not exist and $lock pins nothing (ADR-0021)."
    echo "         The #277 ABI half below still runs."
else
    # #297: the whole block above used to be guarded on the shell being
    # the fork, so THIS case — stock GNOME Shell on a lane that was
    # supposed to get Lisa Desktop — skipped the pin entirely and the
    # script printed OK. It is not hypothetical: CLAUDE.md records
    # 2026-08-04, when a stock-named package outranked the fork by
    # pkgrel and the fork lost the resolution. The pin matters most in
    # exactly the case that was skipping it.
    echo "FAIL: this image installed STOCK gnome-shell $shell_ver, not"
    echo "      lisa-desktop-shell, and it is not the aarch64 lane (no"
    echo "      linux-aarch64 in $manifest)."
    echo "      So $lock was never consulted: the desktop this image contains"
    echo "      is whatever pacman resolved, with nothing in git recording it."
    echo "      This is the 2026-08-04 shape — a stock-named package"
    echo "      outranking the fork — which CLAUDE.md's 'fork packages replace"
    echo "      stock by contract, never by name' rule exists to prevent."
    echo "      Check that lisa-desktop-shell still carries"
    echo "      provides=/conflicts= on gnome-shell and that it resolved from"
    echo "      PackageDirectories= rather than losing to the index."
    fail=1
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
