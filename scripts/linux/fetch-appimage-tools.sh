#!/usr/bin/env bash
#
# Fetches the tools that the Tauri bundler uses to write an AppImage.
#
# The bundler downloads them from GitHub into its cache
# (~/.cache/tauri) and reuses them afterwards. Some networks cannot reach
# GitHub, so the tools can be fetched here instead and put into the image.
#
#   scripts/linux/fetch-appimage-tools.sh x86_64
#   scripts/linux/fetch-appimage-tools.sh aarch64 --mirror https://gh-proxy.com
#
# The tools land in scripts/linux/tools/<arch>/ in the layout of the cache.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ARCH="${1:-x86_64}"
MIRROR=""
[ "${2:-}" = "--mirror" ] && MIRROR="$3"

# The commit of the linuxdeploy build the bundler expects.
LINUXDEPLOY_COMMIT="07333c6"

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "unknown architecture: $ARCH" >&2; exit 2 ;;
esac

# Every architecture keeps its own tools: the plugin has no architecture in
# its name, so they would overwrite each other.
TOOLS="$HERE/tools/$ARCH"

# A mirror takes the place of the address of the release.
fetch() {
  local url="$1" target="$2"
  if [ -s "$target" ]; then
    echo "[tools] $(basename "$target") is already there"
    return
  fi
  if [ -n "$MIRROR" ]; then
    url="$MIRROR/$url"
  fi
  echo "[tools] fetching $(basename "$target")"
  mkdir -p "$(dirname "$target")"
  curl -sSfL --retry 5 --retry-delay 3 --retry-all-errors -o "$target" "$url"
  chmod +x "$target"
}

fetch \
  "https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-${ARCH}" \
  "$TOOLS/AppRun-${ARCH}"

fetch \
  "https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy-${LINUXDEPLOY_COMMIT}/linuxdeploy-${ARCH}.AppImage" \
  "$TOOLS/linuxdeploy-${LINUXDEPLOY_COMMIT}-${ARCH}.AppImage"

# The plugin is optional: the bundler falls back to a built-in version.
fetch \
  "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-${ARCH}.AppImage" \
  "$TOOLS/linuxdeploy-plugin-appimage.AppImage" \
  || echo "[tools] the plugin is missing, the bundler uses its own version"

# Newer bundlers keep the tools in a directory of their own.
mkdir -p "$TOOLS/linuxdeploy-${LINUXDEPLOY_COMMIT}-${ARCH}"
cp -f "$TOOLS/linuxdeploy-${LINUXDEPLOY_COMMIT}-${ARCH}.AppImage" \
  "$TOOLS/linuxdeploy-${LINUXDEPLOY_COMMIT}-${ARCH}/"
mkdir -p "$TOOLS/linuxdeploy-plugin-appimage-${ARCH}"
cp -f "$TOOLS/linuxdeploy-plugin-appimage.AppImage" \
  "$TOOLS/linuxdeploy-plugin-appimage-${ARCH}/" 2>/dev/null || true

ls -lhR "$TOOLS"
