#!/usr/bin/env bash
# Capture xentop-ng (normally in --demo mode) into a PNG for the docs.
#
#   docs/tools/screenshot.sh OUT.png COLSxROWS "keys to send" -- xentop-ng args...
#
# e.g. docs/tools/screenshot.sh docs/hero.png 200x56 "" -- --demo-cpus 128 --demo-mem 1T
# Keys are tmux send-keys arguments, space separated (e.g. "j j Enter").
set -euo pipefail

out="$1" size="$2" keys="$3"
shift 3
[ "${1:-}" = "--" ] && shift
cols="${size%x*}" rows="${size#*x}"
here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/../../target/release/xentop-ng"
tmp="$(mktemp -d)"
trap 'tmux kill-session -t xng-shot 2>/dev/null || true; rm -rf "$tmp"' EXIT

tmux kill-session -t xng-shot 2>/dev/null || true
tmux new-session -d -s xng-shot -x "$cols" -y "$rows" "$bin $*"
sleep 3
if [ -n "$keys" ]; then
    # shellcheck disable=SC2086
    tmux send-keys -t xng-shot $keys
fi
sleep 2.5
tmux capture-pane -e -p -t xng-shot > "$tmp/cap.ansi"

# Terminal background = the theme's background (first cell of row 2).
bg="$(python3 - "$tmp/cap.ansi" <<'PY'
import re, sys
line = open(sys.argv[1], encoding="utf-8").read().split("\n")[1]
m = re.findall(r"\x1b\[[0-9;]*48;2;(\d+);(\d+);(\d+)", line)
print("#%02x%02x%02x" % tuple(map(int, m[0])) if m else "#101014")
PY
)"
python3 "$here/ansi2svg.py" "$tmp/cap.ansi" "$tmp/shot.svg" --bg "$bg" --title "root@xcp-ng-demo: ~ — xentop-ng"
rsvg-convert -z 1.5 "$tmp/shot.svg" -o "$out"
echo "wrote $out"
