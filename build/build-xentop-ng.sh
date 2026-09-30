#!/usr/bin/env bash
# Build the xentop-ng Rust crate (cargo build --release) inside the XCP-ng 8.3
# build container so the binary only needs glibc <= 2.17.
#
# Caches (reused across runs) live in build/.cache/:
#   rustup/  RUSTUP_HOME (stable toolchain, installed on first run)
#   cargo/   CARGO_HOME  (registry, git checkouts)
#   target/<crate>/  CARGO_TARGET_DIR
#
# Environment overrides:
#   CRATE_DIR          crate to build (default: the xentop-ng project dir)
#   BIN_NAME           binary to collect (default: xentop-ng)
#   BIN_OUT_DIR        where to put the binary (default: build/out)
#   RUST_TOOLCHAIN     rustup toolchain (default: stable)
#   CARGO_ARGS         extra cargo build args
#   CONTAINER_ENGINE   podman (default if present) or docker

source "$(dirname "$0")/common.sh"

CRATE_DIR="$(cd "${CRATE_DIR:-$PROJECT_DIR}" && pwd)"
BIN_NAME="${BIN_NAME:-xentop-ng}"
BIN_OUT_DIR="${BIN_OUT_DIR:-$OUT_DIR}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-stable}"
CARGO_ARGS="${CARGO_ARGS:-}"
TARGET_CACHE="$CACHE_DIR/target/$(basename "$CRATE_DIR")"

[ -f "$CRATE_DIR/Cargo.toml" ] || { echo "error: no Cargo.toml in $CRATE_DIR" >&2; exit 1; }
mkdir -p "$CACHE_DIR/rustup" "$CACHE_DIR/cargo" "$TARGET_CACHE" "$BIN_OUT_DIR"

ensure_image

log "Building $CRATE_DIR ($BIN_NAME) with Rust $RUST_TOOLCHAIN in $IMAGE"
run_in_container \
    -v "$CRATE_DIR:/crate" \
    -v "$CACHE_DIR/rustup:/opt/rustup" \
    -v "$CACHE_DIR/cargo:/opt/cargo" \
    -v "$TARGET_CACHE:/target" \
    -e RUSTUP_HOME=/opt/rustup \
    -e CARGO_HOME=/opt/cargo \
    -e CARGO_TARGET_DIR=/target \
    -e RUST_TOOLCHAIN="$RUST_TOOLCHAIN" \
    -e CARGO_ARGS="$CARGO_ARGS" \
    -e XENSTAT_INCLUDE_DIR=/project/build/out/include \
    -e XENSTAT_LIB_DIR=/project/build/out \
    -w /crate \
    -- bash -euo pipefail -c '
    set +u; source /opt/rh/devtoolset-11/enable; set -u
    export PATH="$CARGO_HOME/bin:$PATH"
    if ! command -v rustup >/dev/null 2>&1; then
        curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs |
            sh -s -- -y --no-modify-path --profile minimal \
                --default-toolchain "$RUST_TOOLCHAIN"
    fi
    rustup toolchain install --profile minimal "$RUST_TOOLCHAIN" >/dev/null 2>&1 ||
        rustup toolchain install --profile minimal "$RUST_TOOLCHAIN"
    rustup default "$RUST_TOOLCHAIN" >/dev/null
    rustc --version
    # Plain gcc from devtoolset-11 as linker; glibc comes from the image (2.17).
    export CC=gcc CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc
    locked=""
    [ -f Cargo.lock ] && locked="--locked"
    cargo build --release $locked $CARGO_ARGS
'

src="$TARGET_CACHE/release/$BIN_NAME"
[ -x "$src" ] || { echo "error: $src not produced" >&2; exit 1; }
install -m 0755 "$src" "$BIN_OUT_DIR/$BIN_NAME"

max="$(objdump -T "$BIN_OUT_DIR/$BIN_NAME" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
log "$BIN_NAME: max glibc symbol version $max"
if [ "$(printf '%s\n' "$max" GLIBC_2.17 | sort -V | tail -1)" != GLIBC_2.17 ]; then
    echo "error: $BIN_NAME requires $max (> GLIBC_2.17)" >&2
    exit 1
fi
log "Done: $BIN_OUT_DIR/$BIN_NAME"
