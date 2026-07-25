#!/bin/sh
# lisa-boot-repair — ESP self-repair on every successful boot (ADR-0022
# phase 1, issue #23). The #20 incident left the field machine with a
# dangling UKI for an erased slot and a healthy root with NO entry at
# all; recovery took an expert at the keyboard. This makes the common
# cases heal themselves:
#
#   stash    — keep a copy of the booted version's UKI on the durable
#              /var partition (~90 MB; survives everything the ESP can
#              suffer short of disk death).
#   restore  — if the ESP lost the booted version's UKI (interrupted
#              update, FAT corruption), put it back from the stash.
#   cleanup  — delete UKIs whose root_<ver> partition no longer exists,
#              EXCEPT the booted version's and never the last UKI left.
#
# UKI naming: release-channel installs are `lisa_<ver>.efi`, image-baked
# ones are `lisa-<ver>.efi` — both are handled, and the stash preserves
# the original basename.
#
# Every action logs to the journal with the `lisa-boot-repair:` prefix
# (boot-report ships the journal to the ESP, so repairs are visible
# offline; Ledger integration needs a CLI append verb — follow-up).

set -u

log() { echo "lisa-boot-repair: $*"; }

# --- locate the ESP (same fallback dance as boot-report) ---------------
esp=""
for d in /efi /boot /boot/efi; do
    if mountpoint -q "$d" 2>/dev/null; then
        esp="$d"
        break
    fi
done
[ -n "$esp" ] || exit 0
ukidir="$esp/EFI/Linux"
[ -d "$ukidir" ] || exit 0

booted_ver=$(. /etc/os-release 2>/dev/null && echo "${IMAGE_VERSION:-}")
[ -n "$booted_ver" ] || exit 0
stash_dir=/var/lib/lisa/uki

# The booted version's UKI under either naming convention.
booted_uki=""
for candidate in "$ukidir/lisa_${booted_ver}.efi" "$ukidir/lisa-${booted_ver}.efi"; do
    if [ -f "$candidate" ]; then
        booted_uki="$candidate"
        break
    fi
done

if [ -n "$booted_uki" ]; then
    # --- stash: booted UKI -> /var (booted version only) ---------------
    stash="$stash_dir/${booted_uki##*/}"
    if [ ! -f "$stash" ] || ! cmp -s "$booted_uki" "$stash"; then
        mkdir -p "$stash_dir"
        cp "$booted_uki" "$stash.tmp" && mv "$stash.tmp" "$stash" &&
            log "stashed ${booted_uki##*/} -> $stash_dir"
        for old in "$stash_dir"/lisa_*.efi "$stash_dir"/lisa-*.efi; do
            [ -e "$old" ] || continue
            [ "$old" = "$stash" ] || { rm -f "$old" && log "pruned stale stash $old"; }
        done
    fi
else
    # --- restore: /var stash -> ESP (the incident's missing case) ------
    for stash in "$stash_dir/lisa_${booted_ver}.efi" "$stash_dir/lisa-${booted_ver}.efi"; do
        if [ -f "$stash" ]; then
            target="$ukidir/${stash##*/}"
            cp "$stash" "$target.tmp" && mv "$target.tmp" "$target" && sync &&
                log "RESTORED missing ${target##*/} from stash (boot.repair)"
            break
        fi
    done
fi

# --- cleanup: dangling UKIs whose root partition is gone ---------------
total=0
for uki in "$ukidir"/lisa_*.efi "$ukidir"/lisa-*.efi; do
    [ -e "$uki" ] && total=$((total + 1))
done
for uki in "$ukidir"/lisa_*.efi "$ukidir"/lisa-*.efi; do
    [ -e "$uki" ] || continue
    ver=${uki##*/}
    ver=${ver#lisa_}
    ver=${ver#lisa-}
    ver=${ver%.efi}
    # Boot-counting suffixes (`lisa_2+3-0.efi`) are not part of the
    # version — strip them or every counted entry looks dangling.
    ver=${ver%%+*}
    [ "$ver" = "$booted_ver" ] && continue
    [ "$total" -le 1 ] && break
    if [ ! -e "/dev/disk/by-partlabel/root_${ver}" ]; then
        rm -f "$uki" && sync &&
            log "REMOVED dangling ${uki##*/} — no root_${ver} partition exists (boot.repair)"
        total=$((total - 1))
    fi
done
exit 0
