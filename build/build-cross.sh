#!/usr/bin/env bash
# Cross-build xentop-ng for arm64 and riscv64 Linux dom0s, then check each
# binary under qemu-user: the test suite, and the binary in demo mode.
#
# Built with cargo-zigbuild, which links against a chosen glibc version:
#   aarch64  glibc 2.17  (like the x86_64 build)
#   riscv64  glibc 2.27  (the first glibc with RISC-V)
# The script fails if a binary needs a newer glibc.
#
# Outputs: build/out/<arch>/xentop-ng
#
# Caches (reused across runs) live in build/.cache/cross/:
#   rustup/ cargo/ zig/ target/
#
# Environment overrides:
#   CROSS_ARCHS        archs to build (default: "aarch64 riscv64")
#   CROSS_TESTS        0 to skip the test suite under qemu (default: 1)
#   CONTAINER_ENGINE   podman (default if present) or docker

source "$(dirname "$0")/common.sh"

CROSS_ARCHS="${CROSS_ARCHS:-aarch64 riscv64}"
CROSS_TESTS="${CROSS_TESTS:-1}"
RUST_TOOLCHAIN="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$PROJECT_DIR/rust-toolchain.toml")"
RUSTUP_VERSION=1.29.1
RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
ZIG_VERSION=0.15.2
ZIG_SHA256=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
ZIGBUILD_VERSION=0.23.4

# Not the XCP-ng image: a recent Debian with qemu-user and the target
# libcs (for running the binaries; linking uses zig's own glibc stubs).
CONTAINERFILE_CONTENT="FROM docker.io/library/debian:trixie-slim@sha256:7792b1f7702a86946cd518db72b6a407302c3e9bc1635634368b878189e8221c
RUN apt-get update && apt-get install -y --no-install-recommends \\
        ca-certificates curl xz-utils gcc libc6-dev binutils python3 \\
        qemu-user libc6-arm64-cross libc6-riscv64-cross \\
    && rm -rf /var/lib/apt/lists/*"
IMAGE="localhost/xentop-ng-cross:$(printf '%s' "$CONTAINERFILE_CONTENT" | sha256sum | cut -c1-12)"

mkdir -p "$CACHE_DIR/cross/rustup" "$CACHE_DIR/cross/cargo" "$CACHE_DIR/cross/zig" \
    "$CACHE_DIR/cross/target"
ensure_image

log "Cross-building for: $CROSS_ARCHS (Rust $RUST_TOOLCHAIN, zig $ZIG_VERSION)"
run_in_container \
    -v "$PROJECT_DIR:/crate:ro" \
    -v "$CACHE_DIR/cross/rustup:/opt/rustup" \
    -v "$CACHE_DIR/cross/cargo:/opt/cargo" \
    -v "$CACHE_DIR/cross/zig:/opt/zig" \
    -v "$CACHE_DIR/cross/target:/target" \
    -v "$OUT_DIR:/out" \
    -e RUSTUP_HOME=/opt/rustup \
    -e CARGO_HOME=/opt/cargo \
    -e CARGO_TARGET_DIR=/target \
    -e RUST_TOOLCHAIN="$RUST_TOOLCHAIN" \
    -e RUSTUP_VERSION="$RUSTUP_VERSION" \
    -e RUSTUP_SHA256="$RUSTUP_SHA256" \
    -e ZIG_VERSION="$ZIG_VERSION" \
    -e ZIG_SHA256="$ZIG_SHA256" \
    -e ZIGBUILD_VERSION="$ZIGBUILD_VERSION" \
    -e CROSS_ARCHS="$CROSS_ARCHS" \
    -e CROSS_TESTS="$CROSS_TESTS" \
    -w /crate \
    -- bash -euo pipefail -c '
    log() { printf "\033[1;34m==>\033[0m %s\n" "$*" >&2; }
    export PATH="/opt/zig/zig-x86_64-linux-$ZIG_VERSION:$CARGO_HOME/bin:$PATH"

    if ! command -v rustup >/dev/null 2>&1; then
        curl --proto "=https" --tlsv1.2 -sSfo /tmp/rustup-init \
            "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/x86_64-unknown-linux-gnu/rustup-init"
        echo "$RUSTUP_SHA256  /tmp/rustup-init" | sha256sum -c -
        chmod +x /tmp/rustup-init
        /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none
    fi
    targets=""
    for a in $CROSS_ARCHS; do
        case "$a" in
            aarch64) targets="$targets,aarch64-unknown-linux-gnu" ;;
            riscv64) targets="$targets,riscv64gc-unknown-linux-gnu" ;;
            *) echo "error: unknown arch $a" >&2; exit 1 ;;
        esac
    done
    rustup toolchain install --profile minimal "$RUST_TOOLCHAIN" --target "${targets#,}" >/dev/null
    rustc --version

    if ! command -v zig >/dev/null 2>&1; then
        f="zig-x86_64-linux-$ZIG_VERSION.tar.xz"
        curl --proto "=https" --tlsv1.2 -sSfo "/tmp/$f" "https://ziglang.org/download/$ZIG_VERSION/$f"
        echo "$ZIG_SHA256  /tmp/$f" | sha256sum -c -
        tar -C /opt/zig -xJf "/tmp/$f"
    fi
    if [ "$(cargo-zigbuild --version 2>/dev/null)" != "cargo-zigbuild $ZIGBUILD_VERSION" ]; then
        cargo install --locked --version "$ZIGBUILD_VERSION" cargo-zigbuild
    fi

    for a in $CROSS_ARCHS; do
        case "$a" in
            aarch64) t=aarch64-unknown-linux-gnu glibc=2.17 qemu=qemu-aarch64 sysroot=/usr/aarch64-linux-gnu ;;
            riscv64) t=riscv64gc-unknown-linux-gnu glibc=2.27 qemu=qemu-riscv64 sysroot=/usr/riscv64-linux-gnu ;;
        esac
        log "$a: build ($t, glibc $glibc)"
        cargo zigbuild --release --locked --target "$t.$glibc"
        bin="/target/$t/release/xentop-ng"
        mkdir -p "/out/$a"
        install -m 0755 "$bin" "/out/$a/xentop-ng"
        bin="/out/$a/xentop-ng"

        max="$(readelf -V --wide "$bin" | grep -o "GLIBC_[0-9.]*" | sort -uV | tail -1)"
        log "$a: max glibc symbol version $max"
        if [ "$(printf "%s\n" "$max" "GLIBC_$glibc" | sort -V | tail -1)" != "GLIBC_$glibc" ]; then
            echo "error: $a binary requires $max (> GLIBC_$glibc)" >&2
            exit 1
        fi

        run=("$qemu" -L "$sysroot")
        if [ "$CROSS_TESTS" = 1 ]; then
            log "$a: test suite under $qemu"
            tests="$(cargo zigbuild --tests --release --locked --target "$t.$glibc" --message-format=json |
                python3 -c "
import json, sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get(\"reason\") == \"compiler-artifact\" and m[\"profile\"][\"test\"] and m.get(\"executable\"):
        print(m[\"executable\"])")"
            for tb in $tests; do
                "${run[@]}" "$tb" -q
            done
        fi

        log "$a: smoke test under $qemu"
        "${run[@]}" "$bin" --version
        "${run[@]}" "$bin" --demo-cpus 128 --demo-mem 1T --batch -n 2 -d 0.2 | tail -1 |
            python3 -m json.tool >/dev/null
        "${run[@]}" "$bin" --demo-stock --batch -n 1 -d 0.2 |
            python3 -c "import json,sys; s=json.load(sys.stdin)[\"sources\"]; assert s[\"pcpu\"]==\"missing\", s"
    done
'
for a in $CROSS_ARCHS; do
    log "Done: $OUT_DIR/$a/xentop-ng"
done
