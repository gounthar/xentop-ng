#!/bin/sh
# Install xentop-ng from this bundle on an XCP-ng 8.3 host (run as root).
#
#   ./install.sh [PREFIX]      default PREFIX: /opt/xentop-ng
#
# Installs only under PREFIX. The system xentop and libxenstat are not
# touched: the patched libxenstat is used solely by the `xtop` launcher.
set -eu

PREFIX="${1:-/opt/xentop-ng}"
case "$PREFIX" in
    /*) ;;
    *) echo "error: PREFIX must be an absolute path" >&2; exit 2 ;;
esac
if [ "$(id -u)" -ne 0 ]; then
    echo "error: run as root" >&2
    exit 1
fi
here="$(cd "$(dirname "$0")" && pwd)"

# Root-owned, not group/world-writable: the launcher puts $PREFIX/lib on
# LD_LIBRARY_PATH for a program that runs as root.
install -d -m 0755 -o root -g root "$PREFIX" "$PREFIX/bin" "$PREFIX/lib"
install -m 0755 -o root -g root "$here/lib/libxenstat.so.4.17.0" "$PREFIX/lib/libxenstat.so.4.17.0"
ln -sfn libxenstat.so.4.17.0 "$PREFIX/lib/libxenstat.so.4.17"
for b in xentop-ng xenstat-ext-test; do
    install -m 0755 -o root -g root "$here/bin/$b" "$PREFIX/bin/$b"
done

tmp="$PREFIX/bin/.xtop.$$"
cat > "$tmp" <<WRAP
#!/bin/sh
# xentop-ng launcher: use the patched libxenstat from $PREFIX/lib
LD_LIBRARY_PATH=$PREFIX/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}
export LD_LIBRARY_PATH
exec $PREFIX/bin/xentop-ng "\$@"
WRAP
chmod 0755 "$tmp"
chown root:root "$tmp"
mv -f "$tmp" "$PREFIX/bin/xtop"

echo "Installed to $PREFIX. Run: $PREFIX/bin/xtop"
