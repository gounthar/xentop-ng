#!/usr/bin/env bash
# Record a scripted xentop-ng session (normally --demo) as an animated GIF.
#
#   docs/tools/record.sh OUT.gif COLSxROWS SCRIPT -- xentop-ng args...
#
# SCRIPT is a file with one step per line: "<frames> <keys...>", e.g.
#   6           # six frames, one per second, no keys
#   2 j j       # send "j j", then two frames
#   3 Enter
# Keys are tmux send-keys arguments. Lines starting with # are ignored.
set -euo pipefail

out="$1" size="$2" script="$3"
shift 3
[ "${1:-}" = "--" ] && shift
cols="${size%x*}" rows="${size#*x}"
here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/../../target/release/xentop-ng"
tmp="$(mktemp -d)"
trap 'tmux kill-session -t xng-rec 2>/dev/null || true; rm -rf "$tmp"' EXIT

tmux kill-session -t xng-rec 2>/dev/null || true
tmux new-session -d -s xng-rec -x "$cols" -y "$rows" "$bin $*"
sleep 3

n=0
while read -r frames keys; do
    case "$frames" in ''|'#'*) continue ;; esac
    if [ -n "$keys" ]; then
        # shellcheck disable=SC2086
        tmux send-keys -t xng-rec $keys
    fi
    for _ in $(seq "$frames"); do
        sleep 1
        n=$((n + 1))
        tmux capture-pane -e -p -t xng-rec > "$tmp/$(printf %03d $n).ansi"
    done
done < <(sed 's/#.*//' "$script")

for f in "$tmp"/*.ansi; do
    bg="$(python3 - "$f" <<'PY'
import re, sys
line = open(sys.argv[1], encoding="utf-8").read().split("\n")[1]
m = re.findall(r"\x1b\[[0-9;]*48;2;(\d+);(\d+);(\d+)", line)
print("#%02x%02x%02x" % tuple(map(int, m[0])) if m else "#101014")
PY
)"
    python3 "$here/ansi2svg.py" "$f" "${f%.ansi}.svg" --bg "$bg" --title "root@xcp-ng-demo: ~ — xentop-ng"
    rsvg-convert -z 0.9 "${f%.ansi}.svg" -o "${f%.ansi}.png"
done

# One frame per second; a shared, dithered palette keeps gradients smooth
# and the file small enough for a README.
magick -delay 100 -loop 0 "$tmp"/*.png -dither FloydSteinberg -colors 160 -fuzz 1.5% -layers OptimizeTransparency "$out"
echo "wrote $out ($n frames, $(du -h "$out" | cut -f1))"
