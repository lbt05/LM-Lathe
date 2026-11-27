# Build environment for the Linux installers of LM-Lathe.
#
# The desktop application is a Tauri app: it needs the WebKitGTK stack of
# Linux, a Rust toolchain and Node.js. macOS cannot build it, so the build
# happens inside this image.
#
#   docker build -t lm-lathe-build:linux -f scripts/linux/Dockerfile scripts/linux

FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
# The AppImage runtime cannot use FUSE inside a container, it unpacks itself.
ENV APPIMAGE_EXTRACT_AND_RUN=1

# Some networks cannot reach the official servers. The build may point the
# downloads elsewhere; the defaults are the official addresses.
ARG RUST_DIST_SERVER=""
ARG RUST_UPDATE_ROOT=""
ARG NPM_REGISTRY=""
ARG CARGO_INDEX=""
ARG CARGO_DOWNLOAD=""
ARG APT_MIRROR=""
ARG TARGET_ARCH="x86_64"
ENV CARGO_SOURCE_MIRROR_INDEX="$CARGO_INDEX"
ENV CARGO_SOURCE_MIRROR_DOWNLOAD="$CARGO_DOWNLOAD"
ENV NPM_REGISTRY="$NPM_REGISTRY"

# The libraries the Tauri shell links against, the tools that write the
# packages (dpkg-deb, rpmbuild) and the usual build helpers.
RUN if [ -n "$APT_MIRROR" ]; then \
        sed -i "s|http://archive.ubuntu.com/ubuntu|${APT_MIRROR}|g; s|http://security.ubuntu.com/ubuntu|${APT_MIRROR}|g" \
            /etc/apt/sources.list; \
    fi \
    && apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        file \
        git \
        pkg-config \
        xz-utils \
        libwebkit2gtk-4.1-dev \
        libgtk-3-dev \
        libssl-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libxdo-dev \
        patchelf \
        dpkg-dev \
        fakeroot \
        rpm \
    && rm -rf /var/lib/apt/lists/*

# Node.js 20 — Vite 7 and the Tauri CLI do not run on the Node of Ubuntu.
RUN curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && if [ -n "$NPM_REGISTRY" ]; then npm config set registry "$NPM_REGISTRY"; fi \
    && node --version && npm --version

# The Rust toolchain. The backend asks for 1.82, the Tauri shell for 1.77.
RUN if [ -n "$RUST_DIST_SERVER" ]; then \
        export RUSTUP_DIST_SERVER="$RUST_DIST_SERVER" RUSTUP_UPDATE_ROOT="$RUST_UPDATE_ROOT"; \
    fi \
    && curl --proto '=https' --tlsv1.2 -sSf --retry 5 --retry-delay 3 --retry-all-errors https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path \
    && rustc --version && cargo --version && node --version && npm --version

# Cargo downloads the crates from the mirror when one is named.
RUN if [ -n "$CARGO_SOURCE_MIRROR_INDEX" ]; then \
        mkdir -p "$CARGO_HOME" \
        && printf '[source.crates-io]\nreplace-with = "mirror"\n\n[source.mirror]\nregistry = "%s"\n' \
            "$CARGO_SOURCE_MIRROR_INDEX" > "$CARGO_HOME/config.toml"; \
    fi

# The tools that write the AppImage. The bundler looks for them in its cache
# and downloads them from GitHub when they are missing. They are fetched by
# scripts/linux/fetch-appimage-tools.sh before the build.
COPY tools/${TARGET_ARCH}/ /root/.cache/tauri/
COPY tools/${TARGET_ARCH}/ /root/.tauri/

WORKDIR /build
