#!/usr/bin/env bash
#
# The part of the Linux build that runs inside the container.
#
# The repository arrives read-only under /src, the work happens in /build and
# the finished packages are written to /out.

set -euo pipefail

BUNDLES="${1:-deb,appimage}"
BUNDLE_DIR="/build/apps/desktop/src-tauri/target/release/bundle"

echo "[inside] copying the sources"
# The target directories are volumes that carry the compilation over to the
# next run, so they must survive the clean up.
find /build -xdev -mindepth 1 -depth -delete 2>/dev/null || true
mkdir -p /build
tar -C /src \
  --exclude=./.git \
  --exclude=./target \
  --exclude=./dist \
  --exclude=./apps/desktop/target \
  --exclude=./apps/desktop/node_modules \
  --exclude=./apps/desktop/dist \
  -cf - . | tar -C /build -xf -

cd /build/apps/desktop

echo "[inside] installing the frontend dependencies"
npm ci

echo "[inside] building the packages ($BUNDLES)"
# The AppImage step runs linuxdeploy, which unpacks itself and sometimes
# trips over a busy /tmp. A second attempt gets over it.
if ! npm run tauri:build -- --bundles "$BUNDLES"; then
  echo "[inside] the bundling failed, trying once more"
  sleep 10
  npm run tauri:build -- --bundles "$BUNDLES"
fi

echo "[inside] collecting the packages"
mkdir -p /out
find "$BUNDLE_DIR" -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.rpm' \) \
  -exec cp -f {} /out/ \;

ls -lh /out
