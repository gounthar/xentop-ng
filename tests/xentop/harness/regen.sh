#!/bin/sh
# Regenerate the golden outputs in tests/xentop/ from the real xentop.
#
#   tests/xentop/harness/regen.sh XEN_SOURCE_TREE
#
# Compiles XEN_SOURCE_TREE/tools/xentop/xentop.c, unmodified, against
# stub.c (a fake libxenstat replaying the *.snap fixtures, with a fake
# clock), then runs every line of cases.txt:
#   <case name> <fixture> <xentop arguments...>
# and stores xentop's stdout in <case name>.out. Needs gcc and ncurses.
set -eu

xen="${1:?usage: $0 XEN_SOURCE_TREE}"
here="$(cd "$(dirname "$0")" && pwd)"
dir="$(dirname "$here")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

gcc -O1 -w -DINCLUDE_CURSES_H='<ncurses.h>' -DHOST_Linux \
    -I"$xen/tools/include" -I"$xen/tools/xentop" \
    -o "$tmp/xentop" "$xen/tools/xentop/xentop.c" "$here/stub.c" -lncurses -lm

grep -v '^#' "$dir/cases.txt" | while read -r name fixture args; do
    [ -n "$name" ] || continue
    # shellcheck disable=SC2086 # word splitting of the argument list is wanted
    XT_FIXTURE="$dir/$fixture" "$tmp/xentop" $args > "$dir/$name.out"
    echo "$name: xentop $args"
done
