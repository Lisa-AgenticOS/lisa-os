#!/usr/bin/env bash
# check-plymouth.sh — the boot splash is configured, and configured once.
#
#   usage: check-plymouth.sh <tree> [<tree>...]
#          check-plymouth.sh --selftest
#
# A tree is anything laid out like a root: `os/mkosi/mkosi.extra`,
# `os/mkosi/initrd-overlay`, a built $BUILDROOT, or a mounted image.
#
# WHY THIS EXISTS (#283). `mkosi.extra/usr/share/plymouth/themes/
# default.plymouth` and its initrd twin were symlinks to
# `lisa/lisa.plymouth`, a theme deleted on 2026-07-28 in 1fec591. They
# shipped DANGLING to every device for eight days and nothing noticed,
# because plymouth reads `plymouthd.conf` first and never fell through
# to them. A splash is the one subsystem whose failure mode is a black
# screen — indistinguishable from a hang, and on the reference iMac
# indistinguishable from the splash itself, whose background is
# 0x000000. Config that is wrong but unreached is not "fine"; it is the
# next person's two-hour diagnosis.
#
# Three questions, all answerable from the trees:
#
#   1. Does every symlink under usr/share/plymouth resolve INSIDE its
#      own tree? This is #283 exactly.
#   2. Does each tree that pins `[Daemon] Theme=` pin a theme that tree
#      (or the package that fills it) can actually provide? A tree that
#      carries themes at all must carry the one it names.
#   3. Do the trees AGREE? The initrd and the root each get their own
#      copy — plymouthd resolves the theme once, in the initrd, and
#      keeps it across the switch-root, so a disagreement means the
#      root's answer is a decoy that is never read.
#   4. Is the Lisa watermark the same file in every tree that has one?
#      Same reason, one level down: the mark rides in both trees and the
#      README has asked in prose since #45 that they move together.
#
# ...and question 0, which is the one this file got wrong (#298):
#
#   0. Is the pin THERE AT ALL, and the watermark? The first version
#      enforced AGREEMENT and nothing else, so `rm
#      initrd-overlay/etc/plymouth/plymouthd.conf` printed
#      "-- no etc/plymouth/plymouthd.conf in this tree" and exited 0 —
#      restoring, in silence, the exact "the initrd relies on the
#      package's plymouthd.defaults, stated by nobody" condition #283
#      was filed to close. Deleting the watermark passed the same way.
#      "Updated in one tree" failed; "removed from one tree" did not,
#      and #283 was itself a deletion artifact, so deletion is the
#      regression that actually happens.
#
#      Every tree handed to this script is a root that boots: both
#      os/mkosi/mkosi.extra and os/mkosi/initrd-overlay carry the pin
#      and the mark today, and a $BUILDROOT or a mounted image must too.
#      So absence is a FAILURE, not a note. A gate with nothing to check
#      must not read like a gate that checked.
#
# Bash builtins only: no find, no awk, no sed. The same reasoning
# check-desktop.sh gives — a gate that dies on a missing tool fails for
# a reason nobody reads.
#
# And no bash-4 builtins either. The first draft walked the tree with
# `shopt -s globstar` and collected the pins with `mapfile`; its own
# --selftest reported "a dangling symlink fails: wanted exit 1, got 0"
# on the macOS dev host, because /bin/bash there is 3.2 and has
# neither — the glob matched nothing, so the check that exists to catch
# #283 passed a tree containing #283. CLAUDE.md requires this repo to
# work on macOS *and* Linux, and a gate that silently checks nothing on
# half the dev hosts is worse than no gate. Hence walk_tree() below.

set -uo pipefail
shopt -s dotglob nullglob

# The two files every tree must carry. Relative to the tree root, and
# named once so the failure text and the presence assertion cannot drift
# apart. WATERMARK's directory is `spinner/` because the pinned theme is
# the plymouth package's spinner with our mark overlaid onto it — see
# the themes/ discussion in check_tree().
REQUIRED_PIN=etc/plymouth/plymouthd.conf
REQUIRED_WATERMARK=usr/share/plymouth/themes/spinner/watermark.png

fail=0
note() { printf '%s\n' "$*"; }
bad()  { printf 'FAIL: %s\n' "$*" >&2; fail=1; }

# `[Daemon] Theme=` out of a plymouthd.conf, without sed. Section-aware:
# a `Theme=` under some other section is not the daemon's theme.
read_theme() {
    local file=$1 line section='' value=''
    [ -r "$file" ] || return 1
    while IFS= read -r line || [ -n "$line" ]; do
        line=${line%$'\r'}
        line=${line#"${line%%[![:space:]]*}"}      # ltrim
        line=${line%"${line##*[![:space:]]}"}      # rtrim
        case $line in
            '#'*|';'*|'') continue ;;
            '['*']')     section=${line#[}; section=${section%]}; continue ;;
        esac
        case $line in
            Theme=*) [ "$section" = "Daemon" ] && value=${line#Theme=} ;;
        esac
    done < "$file"
    [ -n "$value" ] || return 1
    printf '%s\n' "$value"
}

# Depth-first walk, bash-3.2 safe. Descends only into real directories:
# a symlinked directory is checked as a link and not followed, so a
# self-referential link cannot spin this forever.
FOUND_LINKS=0
walk_tree() {
    local root=$1 dir=$2 entry target resolved
    for entry in "$dir"/*; do
        if [ -L "$entry" ]; then
            FOUND_LINKS=$((FOUND_LINKS + 1))
            target=$(readlink "$entry")
            case $target in
                /*) resolved="$root$target" ;;        # absolute: inside the tree
                *)  resolved="${entry%/*}/$target" ;; # relative: beside the link
            esac
            if [ -e "$resolved" ]; then
                note "  ok    symlink ${entry#"$root"} -> $target"
            else
                bad "${entry#"$root"} is a DANGLING symlink to '$target'"
                bad "      (resolved: $resolved). A splash whose theme path"
                bad "      does not exist paints nothing; see #283."
            fi
        elif [ -d "$entry" ]; then
            walk_tree "$root" "$entry"
        fi
    done
}

check_tree() {
    local root=$1
    [ -d "$root" ] || { bad "$root is not a directory"; return; }

    # ---- 1. no dangling symlink under usr/share/plymouth -------------
    FOUND_LINKS=0
    [ -d "$root/usr/share/plymouth" ] && walk_tree "$root" "$root/usr/share/plymouth"
    [ "$FOUND_LINKS" -eq 0 ] && note "  ok    no symlinks under usr/share/plymouth"

    # ---- 2. the pinned theme is one this tree can provide ------------
    local conf="$root/$REQUIRED_PIN" theme=
    if [ -r "$conf" ]; then
        if ! theme=$(read_theme "$conf"); then
            bad "${conf#"$root"} sets no [Daemon] Theme= — the image would"
            bad "      inherit whatever the plymouth package's defaults say,"
            bad "      which that file itself warns is rewritten on upgrade."
            return
        fi
        note "  ok    pins [Daemon] Theme=$theme"
        printf '%s\n' "$theme" >> "$PINS"

        # A directory under themes/ is a THEME only if it carries the
        # <name>.plymouth that names it. The overlay trees carry
        # `themes/spinner/` holding nothing but our watermark.png — that
        # is a partial overlay of the plymouth package's spinner, not a
        # theme, and counting it as one made the first version of this
        # check fail the very trees it was written for.
        local d name complete=0
        for d in "$root"/usr/share/plymouth/themes/*/; do
            name=${d%/}; name=${name##*/}
            [ -r "$d$name.plymouth" ] && complete=$((complete + 1))
        done
        if [ "$complete" -gt 0 ]; then
            if [ -r "$root/usr/share/plymouth/themes/$theme/$theme.plymouth" ]; then
                note "  ok    theme '$theme' present in this tree"
            else
                bad "this tree carries $complete complete theme(s) but not"
                bad "      '$theme' — usr/share/plymouth/themes/$theme/$theme.plymouth"
                bad "      is missing, so plymouthd falls back to 'text'."
            fi
        else
            note "  ok    theme '$theme' comes from the plymouth package (this"
            note "        tree carries no complete theme — nothing to verify)"
        fi
    else
        # #298: this used to be `note "-- no ... in this tree"` and exit
        # 0. Deleting the pin from the initrd tree therefore PASSED,
        # which is #283 with the file removed instead of dangling.
        bad "$root carries no $REQUIRED_PIN."
        bad "      Every tree this gate is handed is a root that boots, and a"
        bad "      root with no pin inherits the plymouth package's"
        bad "      plymouthd.defaults — which that file's own header says is"
        bad "      rewritten on upgrade. That is the #283 condition restored by"
        bad "      deletion. Absence here is a failure, never a note."
    fi
}

selftest() {
    local tmp rc=0
    tmp=$(mktemp -d) || return 1
    trap 'rm -rf "$tmp"' RETURN

    # A tree that is right in every way — which since #298 means it
    # carries BOTH required files, because absence is now a failure.
    mkdir -p "$tmp/good/etc/plymouth" "$tmp/good/usr/share/plymouth/themes/bgrt" \
             "$tmp/good/usr/share/plymouth/themes/spinner"
    printf '[Daemon]\nTheme=bgrt\n' > "$tmp/good/etc/plymouth/plymouthd.conf"
    printf 'Name=BGRT\n' > "$tmp/good/usr/share/plymouth/themes/bgrt/bgrt.plymouth"
    printf 'PNG\n' > "$tmp/good/usr/share/plymouth/themes/spinner/watermark.png"

    # #283 itself: a symlink to a theme that was deleted.
    cp -R "$tmp/good" "$tmp/dangling"
    ln -s lisa/lisa.plymouth "$tmp/dangling/usr/share/plymouth/themes/default.plymouth"

    # A pin naming a theme the tree does not carry.
    cp -R "$tmp/good" "$tmp/wrongpin"
    printf '[Daemon]\nTheme=lisa\n' > "$tmp/wrongpin/etc/plymouth/plymouthd.conf"

    # A Theme= that is not the daemon's.
    cp -R "$tmp/good" "$tmp/nopin"
    printf '[Boot]\nTheme=bgrt\n' > "$tmp/nopin/etc/plymouth/plymouthd.conf"

    # #298: the same good tree with one of the two required files
    # DELETED. These are the cases the first version of this script
    # passed with exit 0 — the reason it existed at all was #283, which
    # was itself a deletion.
    cp -R "$tmp/good" "$tmp/nopinfile"
    rm -f "$tmp/nopinfile/etc/plymouth/plymouthd.conf"
    cp -R "$tmp/good" "$tmp/nomark"
    rm -f "$tmp/nomark/usr/share/plymouth/themes/spinner/watermark.png"

    # Two trees that disagree — the initrd's answer and the root's.
    cp -R "$tmp/good" "$tmp/other"
    printf '[Daemon]\nTheme=spinner\n' > "$tmp/other/etc/plymouth/plymouthd.conf"
    printf 'Name=Spinner\n' > "$tmp/other/usr/share/plymouth/themes/bgrt/bgrt.plymouth"
    mkdir -p "$tmp/other/usr/share/plymouth/themes/spinner"
    printf 'Name=Spinner\n' > "$tmp/other/usr/share/plymouth/themes/spinner/spinner.plymouth"

    expect() {                       # expect <want-rc> <label> <tree>...
        local want=$1 label=$2; shift 2
        local out got
        out=$("$0" "$@" 2>&1); got=$?
        if [ "$got" -eq "$want" ]; then
            printf 'selftest ok   %s (exit %d)\n' "$label" "$got"
        else
            printf 'selftest FAIL %s: wanted exit %d, got %d\n' "$label" "$want" "$got"
            printf '%s\n' "$out" | while IFS= read -r l; do printf '              | %s\n' "$l"; done
            rc=1
        fi
    }

    # An overlay tree shaped like ours: a pin, and a partial `spinner/`
    # holding only the watermark. The theme itself comes from the
    # package. This case is here because the first version of the check
    # FAILED it — see the themes/ discussion in check_tree().
    mkdir -p "$tmp/overlay/etc/plymouth" \
             "$tmp/overlay/usr/share/plymouth/themes/spinner"
    printf '[Daemon]\nTheme=bgrt\n' > "$tmp/overlay/etc/plymouth/plymouthd.conf"
    printf 'PNG\n' > "$tmp/overlay/usr/share/plymouth/themes/spinner/watermark.png"

    # The same overlay with a watermark somebody updated in one tree only.
    cp -R "$tmp/overlay" "$tmp/overlay2"
    printf 'PNG-NEWER\n' > "$tmp/overlay2/usr/share/plymouth/themes/spinner/watermark.png"

    expect 0 "a clean tree passes"                 "$tmp/good"
    expect 0 "an overlay tree passes"              "$tmp/overlay"
    expect 0 "matching watermarks pass"            "$tmp/overlay" "$tmp/overlay"
    expect 1 "a watermark updated in one tree fails" "$tmp/overlay" "$tmp/overlay2"

    # ---- the deletion half (#298). Every one of these exited 0 before.
    expect 1 "a tree with NO pin fails"            "$tmp/nopinfile"
    expect 1 "a tree with NO watermark fails"      "$tmp/nomark"
    expect 1 "the pin deleted from one of two trees fails" \
                                                   "$tmp/good" "$tmp/nopinfile"
    expect 1 "the watermark deleted from one of two trees fails" \
                                                   "$tmp/good" "$tmp/nomark"

    # No comparator on PATH must FAIL, not quietly pass. This case is
    # here because the first version degraded to "existence only" and
    # printed ok — green on macOS and on CI runners, a no-op on Lisa,
    # which ships no diffutils. The shim keeps only the two externals
    # the script legitimately needs.
    mkdir -p "$tmp/shim"
    for t in mktemp readlink; do
        p=$(command -v "$t") && ln -sf "$p" "$tmp/shim/$t"
    done
    # Invoked through an absolute bash rather than the shebang: with
    # PATH stripped, `#!/usr/bin/env bash` cannot find bash either, and
    # the case would "fail" for the wrong reason (exit 127).
    local out got
    out=$(PATH="$tmp/shim" "${BASH:-/bin/bash}" "$0" "$tmp/overlay" "$tmp/overlay" 2>&1)
    got=$?
    if [ "$got" -eq 1 ] && case $out in *"Refusing to report a pass"*) true ;; *) false ;; esac; then
        printf 'selftest ok   no comparator on PATH fails loudly (exit %d)\n' "$got"
    else
        printf 'selftest FAIL no comparator on PATH: wanted exit 1 with a refusal, got %d\n' "$got"
        printf '%s\n' "$out" | while IFS= read -r l; do printf '              | %s\n' "$l"; done
        rc=1
    fi
    expect 1 "a dangling symlink fails (#283)"     "$tmp/dangling"
    expect 1 "a pin with no such theme fails"      "$tmp/wrongpin"
    expect 1 "Theme= outside [Daemon] fails"       "$tmp/nopin"
    expect 1 "trees that disagree fail"            "$tmp/good" "$tmp/other"
    expect 0 "trees that agree pass"               "$tmp/good" "$tmp/good"

    return $rc
}

if [ "${1-}" = "--selftest" ]; then
    selftest && { echo "selftest: all cases behaved"; exit 0; }
    echo "selftest: THIS SCRIPT IS BROKEN — it did not react as stated above" >&2
    exit 1
fi

if [ $# -eq 0 ]; then
    echo "usage: check-plymouth.sh <tree> [<tree>...] | --selftest" >&2
    exit 2
fi

PINS=$(mktemp) || exit 1
trap 'rm -f "$PINS"' EXIT
marks=()

trees=0
for tree in "$@"; do
    note "== $tree"
    trees=$((trees + 1))
    check_tree "$tree"
    wm="$tree/$REQUIRED_WATERMARK"
    if [ -r "$wm" ]; then
        marks+=("$wm")
        note "  ok    carries the Lisa watermark"
    else
        # #298 again, one level down: `note "-- no watermark in this
        # tree"` meant `rm` passed while `edit one copy` failed.
        bad "$tree carries no $REQUIRED_WATERMARK."
        bad "      The mark rides in BOTH trees by construction (#45): the"
        bad "      initrd paints the splash and the root is what survives the"
        bad "      switch-root. Removing one copy is not 'nothing to compare',"
        bad "      it is half a splash."
    fi
done

# ---- 0. every tree contributed a pin and a mark ----------------------
# check_tree() and the loop above already fail one tree at a time. This
# is the same invariant stated over the SET, which is what #298 asked
# for: assert the trees, not just agreement across whichever trees
# happen to carry something. It is the backstop for the shape neither
# per-tree check sees — a refactor that loses a tree entirely (a `break`,
# a glob that matched one path where two were meant) would otherwise
# leave the comparisons below holding one element and call that
# agreement.
pins=()
while IFS= read -r p || [ -n "$p" ]; do
    [ -n "$p" ] && pins+=("$p")
done < "$PINS"
if [ "${#pins[@]}" -ne "$trees" ]; then
    bad "$trees tree(s) were checked but only ${#pins[@]} produced a"
    bad "      [Daemon] Theme= pin. Every tree must pin the theme; agreement"
    bad "      among a subset is not coverage."
fi
if [ "${#marks[@]}" -ne "$trees" ]; then
    bad "$trees tree(s) were checked but only ${#marks[@]} carry the"
    bad "      watermark. Same reason: a comparison over a subset proves"
    bad "      nothing about the tree that dropped out."
fi

# ---- 4. the watermark moves in lockstep ------------------------------
# README "Both copies must move together": the mark lives in BOTH
# initrd-overlay/ and mkosi.extra/, plymouthd resolves its theme out of
# the initrd, and the root's copy is the one you can see afterwards.
# That rule has been prose since #45, and prose is how a "fixed" splash
# once shipped with only one of the two updated.
#
# Bash cannot compare binaries — NUL bytes do not survive a variable —
# so this needs one external tool, and WHICH one is not a detail. The
# first version said `command -v cmp || note "existence only"`, which is
# green on macOS and on GitHub's runners and a silent no-op on Lisa
# itself: the image ships no diffutils, so `cmp` and `diff` are both
# absent (verified on the reference iMac; sha256sum, md5sum, openssl and
# python3 are all present). A degraded check that still prints "ok" is
# the failure this file exists to prevent, so the chain is explicit and
# running out of it is a FAILURE, never a pass.
# Reading from stdin, so the digest is of the CONTENT and never carries
# the filename — the copies live at different paths by construction.
# `cut` is deliberately not used: one external tool per comparison, not
# two.
digest() {
    local out
    if   command -v sha256sum >/dev/null 2>&1; then out=$(sha256sum     < "$1")
    elif command -v shasum    >/dev/null 2>&1; then out=$(shasum -a 256 < "$1")
    elif command -v md5sum    >/dev/null 2>&1; then out=$(md5sum        < "$1")
    elif command -v openssl   >/dev/null 2>&1; then out=$(openssl dgst -sha256 < "$1")
    elif command -v cmp       >/dev/null 2>&1; then out=USE_CMP
    else return 1
    fi
    printf '%s\n' "${out%% *}"
}
if [ ${#marks[@]} -gt 1 ]; then
    if ! first_digest=$(digest "${marks[0]}"); then
        bad "no way to compare files here — none of sha256sum, shasum,"
        bad "      md5sum, openssl or cmp is on PATH, so the watermark"
        bad "      copies CANNOT be checked. Refusing to report a pass."
    elif [ "$first_digest" = "USE_CMP" ]; then
        for m in "${marks[@]}"; do
            cmp -s "${marks[0]}" "$m" || bad "watermark differs: ${marks[0]} vs $m"
        done
        [ "$fail" -eq 0 ] && note "== all ${#marks[@]} watermark copies are identical"
    else
        for m in "${marks[@]}"; do
            [ "$(digest "$m")" = "$first_digest" ] ||
                bad "watermark differs: ${marks[0]} vs $m"
        done
        [ "$fail" -eq 0 ] && note "== all ${#marks[@]} watermark copies are identical"
    fi
fi

# ---- 3. every tree that pins a theme must pin the SAME one -----------
# `pins` was read above, with section 0.
if [ ${#pins[@]} -gt 1 ]; then
    first=${pins[0]}
    for p in "${pins[@]}"; do
        if [ "$p" != "$first" ]; then
            bad "the trees disagree on the theme: ${pins[*]}"
            bad "      plymouthd resolves the theme ONCE, in the initrd, and"
            bad "      keeps it across the switch-root — so the root's copy"
            bad "      would be a decoy nobody reads."
            break
        fi
    done
    [ "$fail" -eq 0 ] && note "== all trees pin the same theme ($first)"
fi

if [ "$fail" -ne 0 ]; then
    echo "check-plymouth.sh: the boot splash configuration is not coherent" >&2
    exit 1
fi
echo "check-plymouth.sh: splash configuration is coherent"
