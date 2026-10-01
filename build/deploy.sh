#!/usr/bin/env bash
# Deploy xentop-ng and its patched libxenstat to an XCP-ng 8.3 host.
#
#   usage: build/deploy.sh HOST        (HOST may be user@host; default user root)
#
# Installs, and touches nothing outside /opt/xentop-ng:
#   /opt/xentop-ng/lib/libxenstat.so.4.17.0 (+ libxenstat.so.4.17 symlink)
#   /opt/xentop-ng/bin/xentop-ng
#   /opt/xentop-ng/bin/xenstat-ext-test     (if built)
#   /opt/xentop-ng/bin/xtop                 wrapper: LD_LIBRARY_PATH + exec
#
# The system libxenstat (/usr/lib64) is left untouched; only processes started
# through the xtop wrapper pick up the patched library.

set -euo pipefail

if [ $# -ne 1 ] || [ -z "$1" ]; then
    echo "usage: $0 HOST" >&2
    exit 2
fi

HOST="$1"
case "$HOST" in
    -*) echo "error: HOST must not start with '-'" >&2; exit 2 ;;
esac
case "$HOST" in *@*) ;; *) HOST="root@$HOST" ;; esac

OUT="$(cd "$(dirname "$0")" && pwd)/out"
PREFIX=/opt/xentop-ng

for f in libxenstat.so.4.17.0 xentop-ng; do
    [ -f "$OUT/$f" ] || { echo "error: $OUT/$f missing; run the build scripts first" >&2; exit 1; }
done

files=("$OUT/libxenstat.so.4.17.0" "$OUT/xentop-ng")
[ -f "$OUT/xenstat-ext-test" ] && files+=("$OUT/xenstat-ext-test")

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10)
STAGE="$PREFIX/.staging.$$"

echo "==> Deploying to $HOST:$PREFIX"
# Root-owned and not group/world-writable even if $PREFIX already existed:
# xtop puts $PREFIX/lib on LD_LIBRARY_PATH for a program running as root.
ssh "${SSH_OPTS[@]}" -- "$HOST" "install -d -m 0755 -o root -g root '$PREFIX' '$PREFIX/lib' '$PREFIX/bin' '$STAGE'"
scp "${SSH_OPTS[@]}" -p -- "${files[@]}" "$HOST:$STAGE/"

# Move into place atomically and write the wrapper.
ssh "${SSH_OPTS[@]}" -- "$HOST" bash -s -- "$PREFIX" "$STAGE" <<'REMOTE'
set -euo pipefail
PREFIX="$1"
STAGE="$2"

install -m 0755 "$STAGE/libxenstat.so.4.17.0" "$PREFIX/lib/.libxenstat.so.4.17.0.new"
mv -f "$PREFIX/lib/.libxenstat.so.4.17.0.new" "$PREFIX/lib/libxenstat.so.4.17.0"
ln -sfn libxenstat.so.4.17.0 "$PREFIX/lib/libxenstat.so.4.17"

for b in xentop-ng xenstat-ext-test; do
    [ -f "$STAGE/$b" ] || continue
    install -m 0755 "$STAGE/$b" "$PREFIX/bin/.$b.new"
    mv -f "$PREFIX/bin/.$b.new" "$PREFIX/bin/$b"
done

cat > "$PREFIX/bin/.xtop.new" <<EOF
#!/bin/sh
# xentop-ng launcher: use the patched libxenstat from $PREFIX/lib
LD_LIBRARY_PATH=$PREFIX/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}
export LD_LIBRARY_PATH
exec $PREFIX/bin/xentop-ng "\$@"
EOF
chmod 0755 "$PREFIX/bin/.xtop.new"
mv -f "$PREFIX/bin/.xtop.new" "$PREFIX/bin/xtop"

rm -rf "$STAGE"
ls -l "$PREFIX/lib" "$PREFIX/bin"
REMOTE

echo "==> Done. Run: $PREFIX/bin/xtop   (test: $PREFIX/bin/xenstat-ext-test)"
