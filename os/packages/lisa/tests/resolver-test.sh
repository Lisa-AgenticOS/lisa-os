#!/bin/sh
# Exercise /usr/bin/lisa's resolver (issue #52) against fake binaries.
#
# This script is the recovery path for a broken device — `lisa update` runs
# through it — so its failure modes are pinned here rather than trusted:
# a bad channel payload must never make the CLI unreachable.
#
# Run from the repo root: os/packages/lisa/tests/resolver-test.sh
set -e
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir -p "$T/usr/lib/lisa/bin" "$T/chan/bin"
printf '#!/bin/sh\necho BAKED "$@"\n' > "$T/usr/lib/lisa/bin/lisa"; chmod +x "$T/usr/lib/lisa/bin/lisa"
printf '#!/bin/sh\necho CHANNEL "$@"\n' > "$T/chan/bin/lisa"; chmod +x "$T/chan/bin/lisa"
R="$T/resolver"
sed -e "s#^BAKED=.*#BAKED=$T/usr/lib/lisa/bin/lisa#" \
    -e "s#^CHANNEL=.*#CHANNEL=$T/chan/bin/lisa#" \
    "$(dirname "$0")/../lisa-resolver" > "$R"; chmod +x "$R"

fail=0
check() { [ "$2" = "$3" ] || { echo "FAIL: $1 — got '$2' want '$3'"; fail=1; }; }

check "channel preferred"        "$(LISA_RESOLVED= "$R" x)"                 "CHANNEL x"
check "escape hatch"             "$(LISA_NO_CHANNEL=1 "$R" x)"              "BAKED x"
check "LISA_CLI override"        "$(LISA_CLI=$T/usr/lib/lisa/bin/lisa "$R" x)" "BAKED x"
chmod -x "$T/chan/bin/lisa"
check "non-executable skipped"   "$("$R" x)"                                "BAKED x"
rm -f "$T/chan/bin/lisa"
check "missing channel falls back" "$("$R" x)"                              "BAKED x"
# A channel payload that re-enters the resolver must not loop forever.
mkdir -p "$T/chan/bin"; cp "$R" "$T/chan/bin/lisa"
out=$("$R" x || echo TIMEOUT)
check "no infinite recursion"    "$out"                                     "BAKED x"
[ $fail -eq 0 ] && echo "resolver: all checks passed"
exit $fail
