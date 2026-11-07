<!-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me> -->
<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# React and Tauri builds

Requires Node.js 22. The React interface runs in the browser and in a
bundled Tauri webview. Native requests use the Rust `Books` implementation;
browser files use `kabacus-wasm` directly in the page. Remote requests use
the server API on its origin.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-bindgen-cli --version 0.2.129
cd apps/kabacus-ui
npm ci
# A clang/llvm toolchain is needed by SQLite's WebAssembly build.
npm run build:pwa
npm test
npx playwright install chromium
npm run test:e2e
```

`Containerfile` builds the portable engine before the web bundle. Point the
server's `web_dir` at `apps/kabacus-ui/dist`, or serve that directory as a
static PWA. The service worker caches only application assets, never API
responses or books. Chromium's file handles are kept in IndexedDB; other
browsers keep the SQLite images there. Native windows keep sources and view
state in the shell's app-data directory. Refresh tokens stay in the native
keyring rather than the workspace file.

Register `https://<server>:<port>/app/` as an allowed redirect URI for the
public OAuth client configured in `auth.clients.app`. Enable Authorization
Code + PKCE and CORS from that origin on the provider's discovery and
token endpoints. No client secret belongs in this application. The web
session and sign-in redirect state use localStorage to survive app restarts.
The app requests `openid profile offline_access`, clears callback parameters,
and renews access tokens when the provider supplies a refresh token, on
reopening as well as while open. How long a web login lasts is then set by
the provider's refresh token and SSO session lifetimes (offline or idle
maximums), not by the app.
Sign out removes the persisted user; the identity provider session is managed
separately.

The desktop shell requires Rust and Tauri 2's platform prerequisites
(WebKitGTK 4.1 development libraries on Linux). Build and run:

```sh
npm run tauri build -- --no-bundle
src-tauri/target/release/kabacus-desktop
```

The build hook compiles the CLI sidecar from the same checkout. Desktop
bundles include it, and the Flatpak exposes it as `kabacus cli …`. The shell
loads its bundled interface, restricts navigation to that trusted origin,
and checks that origin on every filesystem command. It connects remote
books through `kabacus-client` using PKCE in the system browser and a
loopback redirect. Register `http://127.0.0.1:<port>/callback` with the
provider. Linux uses Secret Service for refresh tokens; other platforms
keep them for the session. No token is passed into JavaScript.

For attached MCP, enable Automation in a window or launch with
`--control PORT`. The listener binds only IPv4 loopback and drops when the
window closes. `kabacus-cli mcp --attach PORT` bridges stdio to that window.

The App builds workflow (`.github/workflows/apps.yml`) builds unsigned
packages to try: a Windows installer, a macOS disk image, a Flatpak
(`flatpak/`), an iOS simulator app and a debug-signed Android APK. It
generates the platform icons and the mobile projects on each run:

```sh
npx tauri icon src-tauri/icons/icon.png
npx tauri android init --ci && npx tauri android build --debug --apk
npx tauri ios init --ci && npx tauri ios build --debug --target aarch64-sim
```

`catalogs.js` compiles the interface's selected messages from the existing
66 `po/*.po` catalogs. `src/messages.js` lists existing whole
messages; the build refuses missing or fuzzy translations. No separate
translation catalog is maintained. The browser chooses its preferred
available language from the bundled catalogs; layout direction follows the catalog's `LTR` message.

Vite's development server can open scratch local books after `build:wasm`.
Server API development needs a same-origin reverse proxy. Browser tests use
synthetic API fixtures and a real portable-engine local-file flow. Server asset isolation tests live
in `crates/kabacus-server/tests/web_e2e.rs`. CI checks both the web build and
native shell separately from the existing KDE jobs.
