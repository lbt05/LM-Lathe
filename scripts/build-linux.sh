#!/usr/bin/env bash
#
# Builds the Linux installation files of LM-Lathe.
#
# The desktop application is a Tauri app and cannot be compiled on macOS, so
# the build runs inside a Linux container. The container image holds the
# WebKitGTK stack, the Rust toolchain and Node.js.
#
#   scripts/build-linux.sh                     # x86_64, .deb + .AppImage
#   scripts/build-linux.sh --arch aarch64      # arm64
#   scripts/build-linux.sh --bundles deb       # only the Debian package
#
# The packages land in dist/linux/<arch>/.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLI="${CONTAINER_CLI:-docker}"
# The command may carry more than one word, `sudo podman` for example.
read -r -a CLI_CMD <<< "$CLI"
ARCH="x86_64"
BUNDLES="deb,appimage"
# The download mirrors. Empty means the official addresses.
RUST_DIST_SERVER=""
RUST_UPDATE_ROOT=""
NPM_REGISTRY=""
CARGO_INDEX=""
# A mirror for the Ubuntu packages.
APT_MIRROR=""
# A mirror for the GitHub releases, for networks that cannot reach them.
MIRROR=""

while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --arch=*) ARCH="${1#*=}"; shift ;;
    --bundles) BUNDLES="$2"; shift 2 ;;
    --bundles=*) BUNDLES="${1#*=}"; shift ;;
    --rust-mirror) RUST_DIST_SERVER="$2"; RUST_UPDATE_ROOT="${2%/rustup}/rustup"; shift 2 ;;
    --rust-mirror=*) RUST_DIST_SERVER="${1#*=}"; RUST_UPDATE_ROOT="${1#*=}"; RUST_UPDATE_ROOT="${RUST_UPDATE_ROOT%/rustup}/rustup"; shift ;;
    --npm-registry) NPM_REGISTRY="$2"; shift 2 ;;
    --npm-registry=*) NPM_REGISTRY="${1#*=}"; shift ;;
    --cargo-index) CARGO_INDEX="$2"; shift 2 ;;
    --cargo-index=*) CARGO_INDEX="${1#*=}"; shift ;;
    --apt-mirror) APT_MIRROR="$2"; shift 2 ;;
    --apt-mirror=*) APT_MIRROR="${1#*=}"; shift ;;
    --mirror) MIRROR="$2"; shift 2 ;;
    --mirror=*) MIRROR="${1#*=}"; shift ;;
    --image) IMAGE="$2"; shift 2 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

case "$ARCH" in
  x86_64|amd64) PLATFORM="linux/amd64"; ARCH="x86_64" ;;
  aarch64|arm64) PLATFORM="linux/arm64"; ARCH="aarch64" ;;
  *) echo "unknown architecture: $ARCH" >&2; exit 2 ;;
esac

# Every platform gets its own image: a container runtime refuses to start an
# image that was built for another architecture.
IMAGE="${LM_LATHE_LINUX_IMAGE:-lm-lathe-build:linux-${ARCH}}"

OUT_DIR="$REPO_ROOT/dist/linux/$ARCH"
mkdir -p "$OUT_DIR"

echo "[linux] fetching the tools that write the AppImage"
MIRROR_ARG=()
[ -n "$MIRROR" ] && MIRROR_ARG=(--mirror "$MIRROR")
"$REPO_ROOT/scripts/linux/fetch-appimage-tools.sh" "$ARCH" ${MIRROR_ARG[@]+"${MIRROR_ARG[@]}"}

echo "[linux] building the image $IMAGE for $PLATFORM"
BUILD_ARGS=()
[ -n "$RUST_DIST_SERVER" ] && BUILD_ARGS+=(--build-arg "RUST_DIST_SERVER=$RUST_DIST_SERVER")
[ -n "$RUST_UPDATE_ROOT" ] && BUILD_ARGS+=(--build-arg "RUST_UPDATE_ROOT=$RUST_UPDATE_ROOT")
[ -n "$NPM_REGISTRY" ] && BUILD_ARGS+=(--build-arg "NPM_REGISTRY=$NPM_REGISTRY")
[ -n "$CARGO_INDEX" ] && BUILD_ARGS+=(--build-arg "CARGO_INDEX=$CARGO_INDEX")
[ -n "$APT_MIRROR" ] && BUILD_ARGS+=(--build-arg "APT_MIRROR=$APT_MIRROR")
BUILD_ARGS+=(--build-arg "TARGET_ARCH=$ARCH")
"${CLI_CMD[@]}" build --platform "$PLATFORM" ${BUILD_ARGS[@]+"${BUILD_ARGS[@]}"} \
  -t "$IMAGE" -f "$REPO_ROOT/scripts/linux/Dockerfile" "$REPO_ROOT/scripts/linux"

echo "[linux] building the packages for $ARCH ($BUNDLES)"
"${CLI_CMD[@]}" run --rm \
  --platform "$PLATFORM" \
  -v "$REPO_ROOT:/src:ro" \
  -v "$OUT_DIR:/out" \
  -v lm-lathe-cargo-registry:/usr/local/cargo/registry \
  -v lm-lathe-cargo-git:/usr/local/cargo/git \
  -v "lm-lathe-cargo-target-${ARCH}:/build/target" \
  -v "lm-lathe-tauri-target-${ARCH}:/build/apps/desktop/src-tauri/target" \
  "$IMAGE" \
  /bin/bash /src/scripts/linux/build-inside.sh "$BUNDLES"

echo "[linux] the packages are in $OUT_DIR"
ls -lh "$OUT_DIR"
