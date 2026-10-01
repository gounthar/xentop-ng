#!/usr/bin/env bash
# Build a patched libxenstat.so.4.17 for XCP-ng 8.3 (Xen 4.17.6, glibc 2.17).
#
#  1. Fetch Xen RELEASE-4.17.6 and the XCP-ng xen RPM sources pinned at the
#     commit matching xen-4.17.6-12.3.xcpng8.3 (cached under .cache/).
#  2. Inside the XCP-ng 8.3 build container: apply the whole XCP-ng patch
#     queue in spec order, then libxenstat/xcp-ng-4.17/0*.patch, configure the
#     tools like the RPM does, and build only the libraries libxenstat needs.
#     0003 also touches the hypervisor (a new domctl); those files are patched
#     but not built here: the hypervisor itself has to be rebuilt separately
#     (see libxenstat/README.md).
#  3. Copy libxenstat.so.4.17{,.0} and the xenstat-ext-test binary to out/.
#
# Environment overrides:
#   XEN_GIT            Xen git repo (URL or local path; a local clone is much
#                      faster, e.g. XEN_GIT=~/dev/xen)
#   CONTAINER_ENGINE   podman (default if present) or docker
#   JOBS               parallel make jobs (default: nproc)

source "$(dirname "$0")/common.sh"

XEN_GIT="${XEN_GIT:-https://xenbits.xen.org/git-http/xen.git}"
XEN_TAG="RELEASE-4.17.6"
XEN_COMMIT="8c80ec836310fb2be22b0a2437da002b6b031480"   # RELEASE-4.17.6^{commit}
XCPNG_RPM_GIT="${XCPNG_RPM_GIT:-https://github.com/xcp-ng-rpms/xen.git}"
XCPNG_RPM_COMMIT="08c0ed1fae7e551f57f7c5c957fbcea577857311"   # "Fixes for XSA-510 and XSA-512" = 4.17.6-12.3
JOBS="${JOBS:-$(nproc)}"

mkdir -p "$CACHE_DIR" "$OUT_DIR"

# ---------------------------------------------------------------- sources
if [ -d "$XEN_GIT" ]; then
    XEN_GIT="file://$(cd "$XEN_GIT" && pwd)"
fi

if [ ! -d "$CACHE_DIR/xen.git" ]; then
    log "Fetching Xen $XEN_TAG from $XEN_GIT"
    git init -q --bare "$CACHE_DIR/xen.git"
fi
if ! git -C "$CACHE_DIR/xen.git" rev-parse -q --verify "$XEN_TAG^{commit}" >/dev/null; then
    git -C "$CACHE_DIR/xen.git" fetch -q --depth 1 "$XEN_GIT" \
        "refs/tags/$XEN_TAG:refs/tags/$XEN_TAG"
fi
got="$(git -C "$CACHE_DIR/xen.git" rev-parse "$XEN_TAG^{commit}")"
if [ "$got" != "$XEN_COMMIT" ]; then
    echo "error: $XEN_TAG resolves to $got, expected $XEN_COMMIT" >&2
    exit 1
fi

if [ ! -d "$CACHE_DIR/xcpng-xen-rpm/.git" ]; then
    log "Cloning XCP-ng xen RPM sources"
    git clone -q "$XCPNG_RPM_GIT" "$CACHE_DIR/xcpng-xen-rpm"
fi
if ! git -C "$CACHE_DIR/xcpng-xen-rpm" rev-parse -q --verify "$XCPNG_RPM_COMMIT^{commit}" >/dev/null; then
    git -C "$CACHE_DIR/xcpng-xen-rpm" fetch -q origin
fi
git -C "$CACHE_DIR/xcpng-xen-rpm" -c advice.detachedHead=false checkout -q "$XCPNG_RPM_COMMIT"

log "Exporting pristine source tree"
SRC="$CACHE_DIR/src/xen-4.17.6"
rm -rf "$SRC"
mkdir -p "$SRC"
git -C "$CACHE_DIR/xen.git" archive "$XEN_TAG" | tar -x -C "$SRC"

# ------------------------------------------------------------------ build
ensure_image

log "Patching and building inside $IMAGE"
run_in_container -e JOBS="$JOBS" -- bash -euo pipefail -c '
    set +u; source /opt/rh/devtoolset-11/enable; set -u
    SRC=/project/build/.cache/src/xen-4.17.6
    RPM=/project/build/.cache/xcpng-xen-rpm
    cd "$SRC"

    n=0
    for p in $(awk "/^Patch[0-9]+:/ {print \$2}" "$RPM/SPECS/xen.spec"); do
        patch -p1 --fuzz=0 -s --no-backup-if-mismatch < "$RPM/SOURCES/$p"
        n=$((n + 1))
    done
    echo "applied $n XCP-ng patches"

    for p in /project/libxenstat/xcp-ng-4.17/0*.patch; do
        echo "applying $(basename "$p")"
        patch -p1 --fuzz=0 --no-backup-if-mismatch < "$p"
    done

    export XEN_TARGET_ARCH=x86_64 PYTHON=python3
    # iasl is only needed for firmware; configure insists on it for x86.
    IASL=/bin/true ./configure -q --prefix=/usr --libdir=/usr/lib64 \
        --disable-qemu-traditional --disable-seabios --disable-stubdom \
        --disable-xsmpolicy --disable-pvshim --disable-rombios \
        --disable-ipxe --disable-ovmf --disable-systemd \
        --disable-ocamltools --disable-golang --disable-docs \
        --with-system-qemu=/usr/lib64/xen/bin/qemu-system-i386 \
        >/dev/null

    make -s -j"$JOBS" -C tools/include
    for l in toolcore toollog evtchn gnttab call foreignmemory devicemodel \
             ctrl store stat; do
        make -s -j"$JOBS" -C tools/libs/$l
    done

    # Validation helper, linked against the freshly built library.
    rpl=""
    for d in tools/libs/*/; do rpl="$rpl -Wl,-rpath-link,$SRC/$d"; done
    gcc -O2 -g -Wall -Wextra -Werror -std=gnu99 \
        -I tools/include -o tools/libs/stat/xenstat-ext-test \
        /project/build/xenstat-ext-test.c \
        -L tools/libs/stat -lxenstat $rpl \
        -Wl,-rpath,/opt/xentop-ng/lib
'

log "Collecting artefacts"
rm -f "$OUT_DIR"/libxenstat.so.4.17*
install -m 0755 "$SRC/tools/libs/stat/libxenstat.so.4.17.0" "$OUT_DIR/"
ln -sf libxenstat.so.4.17.0 "$OUT_DIR/libxenstat.so.4.17"
install -m 0755 "$SRC/tools/libs/stat/xenstat-ext-test" "$OUT_DIR/"
# Match the RPM (stripped); unstripped copies stay in the cached tree.
strip --strip-unneeded "$OUT_DIR/libxenstat.so.4.17.0" "$OUT_DIR/xenstat-ext-test"
mkdir -p "$OUT_DIR/include"
install -m 0644 "$SRC/tools/include/xenstat.h" "$OUT_DIR/include/"

soname="$(objdump -p "$OUT_DIR/libxenstat.so.4.17.0" | awk '/SONAME/ {print $2}')"
[ "$soname" = "libxenstat.so.4.17" ] || { echo "error: bad SONAME $soname" >&2; exit 1; }

for sym in xenstat_vbd_has_ext xenstat_vbd_rd_reqs_done xenstat_vbd_wr_reqs_done \
           xenstat_vbd_rd_usecs xenstat_vbd_wr_usecs xenstat_vbd_io_errors \
           xenstat_node_pcpu_idle_ns xenstat_node_num_pcpu_idle \
           xenstat_vcpu_has_runstate xenstat_vcpu_runnable_ns \
           xenstat_vcpu_blocked_ns xenstat_vcpu_offline_ns; do
    objdump -T "$OUT_DIR/libxenstat.so.4.17.0" | grep -qw "$sym" ||
        { echo "error: $sym not exported" >&2; exit 1; }
done

for f in "$OUT_DIR/libxenstat.so.4.17.0" "$OUT_DIR/xenstat-ext-test"; do
    max="$(objdump -T "$f" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
    log "$(basename "$f"): max glibc symbol version $max"
    if [ "$(printf '%s\n' "$max" GLIBC_2.17 | sort -V | tail -1)" != GLIBC_2.17 ]; then
        echo "error: $f requires $max (> GLIBC_2.17)" >&2
        exit 1
    fi
done

log "Done: $(cd "$OUT_DIR" && echo *)"
