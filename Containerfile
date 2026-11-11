# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

# Kabacus's containers, on openSUSE Tumbleweed except deb. Stages:
# - dev:    Rust and test tools for building and testing without them on
#           the host (scripts/dev).
# - gui:    dev plus Qt 6, Kirigami, Xvfb and AT-SPI, for the GUI e2e
#           tests (scripts/gui-e2e).
# - deb:    Debian with Qt 6 and Kirigami, for the app's DEB (scripts/dev --deb).
# - server: the redistributable kabacus-server image (scripts/build-image).
#           Nothing in it is tied to one machine: the books are a
#           database in the /data volume ($KABACUS_DATABASE,
#           /data/books.kabacus unless set, or a PostgreSQL database_url),
#           the socket is /run/kabacus/server.sock, and any uid can run it.

ARG BASE=registry.opensuse.org/opensuse/tumbleweed:latest

FROM ${BASE} AS dev
# Test tools: pdftotext reads exported statements back. The PostgreSQL
# server lets the store suite start a throwaway cluster in each test run,
# with no port open. gdb is for the VS Code server debug config
# (.vscode/launch.json).
RUN zypper -n install --no-recommends rust cargo gcc gcc-c++ git-core \
    poppler-tools postgresql-server timezone ca-certificates gdb \
 && zypper clean -a

FROM dev AS gui
# The Kirigami app (cxx-qt builds against the system Qt), a private X
# display and D-Bus session, the AT-SPI stack the driver uses, and Plasma's
# Qt integration with Breeze icons, so the app looks as it does on Plasma.
# qt6-declarative-tools has qmllint (scripts/qmllint). KI18n translates the
# app; glibc-locale has the locales the translated GUI e2e test runs in.
# --force-resolution swaps the image's busybox tools for the real ones.
RUN zypper -n install --no-recommends --force-resolution qt6-base-devel qt6-declarative-devel qt6-declarative-tools \
    qt6-quickcontrols2-devel kf6-kirigami-devel kf6-kirigami-imports kirigami-addons6 kf6-ki18n-devel glibc-locale \
    kf6-qqc2-desktop-style kf6-breeze-icons plasma6-integration-plugin qt6-wayland xvfb-run xorg-x11-server-Xvfb dbus-1 \
    at-spi2-core gsettings-desktop-schemas python313-atspi python313-gobject \
    typelib-1_0-Atspi-2_0 \
    ImageMagick dejavu-fonts \
 && zypper clean -a
# The Secret Service for the app's refresh token: KWallet (ksecretd), as on
# Plasma. Other desktops' stores aren't tested.
# tests/gui/secret-service.py starts one, unlocked, in the private session.
RUN zypper -n install --no-recommends kwalletd6 && zypper clean -a

FROM docker.io/library/rust:1-trixie AS deb
# The app's DEB (scripts/build-packages), built against Debian's Qt 6 and
# Kirigami. The QML modules are the package's dependencies, installed here
# so a misnamed one fails the image build rather than the user's install.
RUN apt-get update && apt-get install -y --no-install-recommends \
    qt6-base-dev qt6-declarative-dev qmake6 libkf6i18n-dev \
    qml6-module-qtquick qml6-module-qtquick-controls qml6-module-qtquick-dialogs \
    qml6-module-qtquick-layouts qml6-module-qtquick-templates qml6-module-qtquick-window \
    qml6-module-qtqml-workerscript qml6-module-org-kde-kirigami \
    qml6-module-org-kde-kirigamiaddons-components qml6-module-org-kde-desktop \
 && rm -rf /var/lib/apt/lists/*
# Built into the image (in the rust image's PATH, whatever CARGO_HOME the
# build runs with), so a package build doesn't compile it again.
RUN cargo install --locked cargo-deb

FROM docker.io/library/node:22-bookworm-slim AS wasm-node

FROM docker.io/library/rust:1-trixie AS wasm
ARG SOURCE_COMMIT
ENV SOURCE_COMMIT=${SOURCE_COMMIT}
ENV CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar
COPY --from=wasm-node /usr/local/bin/node /usr/local/bin/node
RUN rustup target add wasm32-unknown-unknown \
 && cargo install --locked wasm-bindgen-cli --version 0.2.129
RUN apt-get update && apt-get install -y --no-install-recommends clang llvm \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY design ./design
COPY scripts/build-wasm ./scripts/build-wasm
RUN scripts/build-wasm \
 && CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test --locked --target wasm32-unknown-unknown -p kabacus-store --test portable

FROM docker.io/library/node:22-bookworm-slim AS web
WORKDIR /src/apps/kabacus-ui
COPY apps/kabacus-ui/package*.json ./
RUN npm ci --no-audit --no-fund
COPY apps/kabacus-ui/index.html apps/kabacus-ui/vite.config.js apps/kabacus-ui/catalogs.js ./
COPY po /src/po
COPY design/tokens.json /src/design/tokens.json
COPY apps/kabacus-ui/src ./src
COPY apps/kabacus-ui/public ./public
COPY apps/kabacus-ui/sw-source.js ./
COPY --from=wasm /src/apps/kabacus-ui/public/wasm ./public/wasm
ARG KABACUS_VERSION
RUN npm run build

FROM docker.io/library/rust:1-trixie AS tauri-test
RUN rustup component add rustfmt
COPY --from=wasm /src/apps/kabacus-ui/public/wasm /opt/kabacus-wasm
RUN apt-get update && apt-get install -y --no-install-recommends \
    nodejs npm pkg-config libwebkit2gtk-4.1-dev \
    libayatana-appindicator3-dev librsvg2-dev patchelf \
    dbus-x11 xvfb xauth python3-pyatspi at-spi2-core \
 && rm -rf /var/lib/apt/lists/*

FROM dev AS build
WORKDIR /src
COPY . .
ARG KABACUS_VERSION
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p kabacus-server \
 && cp target/release/kabacus-server /usr/local/bin/

FROM ${BASE} AS server
ARG VERSION=dev
RUN zypper -n install --no-recommends timezone ca-certificates \
 && zypper clean -a \
 && mkdir -p /data /run/kabacus && chmod 1777 /run/kabacus
COPY --from=build /usr/local/bin/kabacus-server /usr/local/bin/
COPY container/server.toml /etc/kabacus/server.toml
COPY --from=web /src/apps/kabacus-ui/dist /usr/share/kabacus/web
# Nothing but /data and /run/kabacus needs to be writable, so the root
# filesystem can be read-only.
ENV HOME=/tmp TZ=UTC KABACUS_DATABASE=/data/books.kabacus
VOLUME ["/data", "/run/kabacus"]
LABEL org.opencontainers.image.title="kabacus-server" \
      org.opencontainers.image.description="Gateway to double-entry books in SQLite or PostgreSQL: validated writes, one revision per change" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"
ENTRYPOINT ["/usr/local/bin/kabacus-server", "--config", "/etc/kabacus/server.toml"]
