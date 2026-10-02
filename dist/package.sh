#!/usr/bin/env bash
# Assemble release archives from build/out/ (run the build scripts first).
#
#   dist/package.sh VERSION      e.g. dist/package.sh v0.1.0
#
# Produces in build/out/release/:
#   xentop-ng-VERSION-x86_64-linux-gnu.tar.gz   the binary (glibc >= 2.17)
#   xentop-ng-VERSION-aarch64-linux-gnu.tar.gz  the arm64 binary (glibc >= 2.17)
#   xentop-ng-VERSION-riscv64-linux-gnu.tar.gz  the riscv64 binary (glibc >= 2.27)
#   xentop-ng-VERSION-xcp-ng-8.3.tar.gz         binary + patched libxenstat
#                                               + installer, for XCP-ng 8.3
#   SHA256SUMS
set -euo pipefail

ver="${1:?usage: $0 VERSION}"
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/build/out"
rel="$out/release"
rm -rf "$rel"
mkdir -p "$rel"

for f in xentop-ng xenstat-ext-test libxenstat.so.4.17.0 aarch64/xentop-ng riscv64/xentop-ng; do
    [ -f "$out/$f" ] || { echo "error: $out/$f missing; run build/build-*.sh first" >&2; exit 1; }
done

# Generic binaries.
g="xentop-ng-$ver-x86_64-linux-gnu"
mkdir -p "$rel/$g"
cp "$out/xentop-ng" "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$rel/$g/"
cross=()
for a in aarch64 riscv64; do
    d="xentop-ng-$ver-$a-linux-gnu"
    mkdir -p "$rel/$d"
    cp "$out/$a/xentop-ng" "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$rel/$d/"
    cross+=("$d")
done

# XCP-ng 8.3 bundle.
x="xentop-ng-$ver-xcp-ng-8.3"
mkdir -p "$rel/$x/bin" "$rel/$x/lib"
cp "$out/xentop-ng" "$out/xenstat-ext-test" "$rel/$x/bin/"
cp "$out/libxenstat.so.4.17.0" "$rel/$x/lib/"
cp "$root/dist/install.sh" "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$rel/$x/"
# libxenstat is LGPL-2.1+: ship the changes and where the rest comes from.
cp -r "$root/libxenstat" "$rel/$x/libxenstat"
{
    echo "# Corresponding source for lib/libxenstat.so.4.17.0"
    echo
    echo "libxenstat is part of Xen and licensed LGPL-2.1-or-later."
    echo "This build is Xen $(sed -n 's/^XEN_TAG="\([^"]*\)".*/\1/p' "$root/build/build-libxenstat.sh") +"
    echo "the XCP-ng xen RPM patch queue at commit"
    sed -n 's/^XCPNG_RPM_COMMIT="\([^"]*\)".*/\1/p' "$root/build/build-libxenstat.sh"
    echo "of https://github.com/xcp-ng-rpms/xen + the patches in libxenstat/xcp-ng-4.17/."
    echo "build/build-libxenstat.sh in the xentop-ng repository reproduces it."
} > "$rel/$x/libxenstat/SOURCES.md"

# Reproducible-ish archives: fixed owner, mtime and order.
epoch="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}"
for d in "$g" "${cross[@]}" "$x"; do
    tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
        -C "$rel" -czf "$rel/$d.tar.gz" "$d"
    rm -rf "${rel:?}/$d"
done
(cd "$rel" && sha256sum ./*.tar.gz > SHA256SUMS)
ls -l "$rel"
