# Shared helpers for xentop-ng build scripts (sourced, not executed).
# Everything is built inside an XCP-ng 8.3 userland container so the
# resulting binaries only require glibc <= 2.17.

set -euo pipefail

BUILD_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$BUILD_DIR")"
CACHE_DIR="$BUILD_DIR/.cache"
OUT_DIR="$BUILD_DIR/out"

BASE_IMAGE="${BASE_IMAGE:-ghcr.io/xcp-ng/xcp-ng-build-env:8.3}"

if [ -z "${CONTAINER_ENGINE:-}" ]; then
    if command -v podman >/dev/null 2>&1; then
        CONTAINER_ENGINE=podman
    elif command -v docker >/dev/null 2>&1; then
        CONTAINER_ENGINE=docker
    else
        echo "error: neither podman nor docker found" >&2
        exit 1
    fi
fi

# Build dependencies baked into a derived image (built once, reused).
CONTAINERFILE_CONTENT="FROM $BASE_IMAGE
RUN yum install -y \\
        devtoolset-11-gcc devtoolset-11-binutils \\
        yajl-devel libuuid-devel ncurses-devel zlib-devel json-c-devel \\
        python3-devel perl patch which curl ca-certificates \\
    && yum clean all"

IMAGE_TAG="$(printf '%s' "$CONTAINERFILE_CONTENT" | sha256sum | cut -c1-12)"
IMAGE="localhost/xentop-ng-build:8.3-$IMAGE_TAG"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }

ensure_image() {
    if ! "$CONTAINER_ENGINE" image inspect "$IMAGE" >/dev/null 2>&1; then
        log "Building container image $IMAGE"
        printf '%s\n' "$CONTAINERFILE_CONTENT" |
            "$CONTAINER_ENGINE" build -t "$IMAGE" -f - "$BUILD_DIR"
    fi
}

# run_in_container [extra engine args...] -- command...
run_in_container() {
    local args=()
    while [ $# -gt 0 ] && [ "$1" != "--" ]; do args+=("$1"); shift; done
    [ "${1:-}" = "--" ] && shift

    local user_args=()
    # Rootless podman maps container root to the invoking user; with docker
    # run as the invoking user so created files are not root-owned.
    if [ "$CONTAINER_ENGINE" = docker ]; then
        user_args=(--user "$(id -u):$(id -g)" -e HOME=/tmp)
    fi

    "$CONTAINER_ENGINE" run --rm \
        --security-opt label=disable \
        "${user_args[@]}" \
        -v "$PROJECT_DIR:/project" \
        -w /project/build \
        "${args[@]}" \
        "$IMAGE" "$@"
}
