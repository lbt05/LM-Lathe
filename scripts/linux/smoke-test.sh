#!/usr/bin/env bash
#
# Starts the built Linux package inside a container and checks that the window
# program comes up and that the backend it carries answers.
#
#   scripts/linux/smoke-test.sh [architecture]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CLI="${CONTAINER_CLI:-docker}"
# The command may carry more than one word, `sudo podman` for example.
read -r -a CLI_CMD <<< "$CLI"
ARCH="${1:-aarch64}"
APT_MIRROR="${2:-}"

case "$ARCH" in
  x86_64|amd64) PLATFORM="linux/amd64" ;;
  aarch64|arm64) PLATFORM="linux/arm64" ;;
  *) echo "unknown architecture: $ARCH" >&2; exit 2 ;;
esac

OUT_DIR="$REPO_ROOT/dist/linux/$ARCH"
if [ ! -d "$OUT_DIR" ] || [ -z "$(ls -A "$OUT_DIR" 2>/dev/null)" ]; then
  echo "no packages in $OUT_DIR — run scripts/build-linux.sh --arch $ARCH first" >&2
  exit 1
fi

echo "[smoke] starting the package of $ARCH"
"${CLI_CMD[@]}" run --rm --platform "$PLATFORM" \
  -v "$OUT_DIR:/w" \
  --entrypoint /bin/bash \
  "lm-lathe-build:linux-${ARCH}" \
  -c '
    set -e
    export DEBIAN_FRONTEND=noninteractive
    if [ -n "'"$APT_MIRROR"'" ]; then
      sed -i "s|http://archive.ubuntu.com/ubuntu|'"$APT_MIRROR"'|g; s|http://security.ubuntu.com/ubuntu|'"$APT_MIRROR"'|g" /etc/apt/sources.list
    fi
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends xvfb dbus-x11 curl procps >/dev/null

    cd /w
    chmod +x ./*.AppImage ./*.deb 2>/dev/null || true
    APPIMAGE=$(ls ./*.AppImage)

    dbus-run-session -- xvfb-run -a "$APPIMAGE" >/tmp/app.log 2>&1 &
    APP=$!

    # The window program pulls the backend up and talks to it on this port.
    for _ in $(seq 1 40); do
      if curl -fsS http://127.0.0.1:54321/health >/tmp/health.json 2>/dev/null; then
        break
      fi
      sleep 1
    done

    echo "--- backend ---"
    curl -fsS http://127.0.0.1:54321/health || echo "the backend did not answer"

    echo
    echo "--- processes ---"
    ps -eo comm | grep -E "lm-lathe" | sort -u || echo "no lm-lathe process"

    echo
    echo "--- log ---"
    tail -n 15 /tmp/app.log || true

    kill $APP 2>/dev/null || true
  '
