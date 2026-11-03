<!-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me> -->
<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# React and Tauri builds

Requires Node.js 22. The web build and the desktop shell use the same
React interface, served by kabacus-server at `/app/`. All book access goes
through the existing API on that origin.

```sh
cd apps/kabacus-ui
npm ci
npm test
npm run build
npx playwright install chromium
npm run test:e2e
```

Set `web_dir` at the top level of the server TOML configuration to the
absolute path of `apps/kabacus-ui/dist`, then restart the server. Open
`https://<server>:<port>/app/` using its configured HTTPS listener. The
server container builds and installs this bundle automatically. For a
local scratch server over a unix socket, use a local reverse proxy; do
not expose no-auth mode remotely. Web hosting is disabled when `web_dir`
is unset. Only entry-point and asset files are served; API routes keep
their existing authentication, roles and revision checks.

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
KABACUS_SERVER_URL=https://books.example.org:8443 src-tauri/target/release/kabacus-desktop
```

The local connection window remains available without the server and saves
only its HTTPS origin. `KABACUS_SERVER_URL` optionally supplies its initial
value, without credentials, paths, queries or fragments. Use a certificate
trusted by the operating system.
The shell discovers the configured provider through `kabacus-client`, opens
its Authorization Code + PKCE flow in the system browser, and receives the
redirect on a loopback port. Register the same loopback redirect pattern
as for the KDE app (`http://127.0.0.1:<port>/callback`).

The shell then opens the server's `/app/` in an ephemeral webview, so UI
updates deploy with the server. Only short-lived access tokens are passed
to the configured origin. On Linux the native shell reuses the KDE Secret
Service implementation to persist refresh tokens in KWallet or another
freedesktop keyring; on other platforms the refresh token lasts only for the
session. It renews access tokens through `kabacus-client`. Tokens are never
written to a settings file.
Sign out ends the native session and closes the window. If the provider
ends a session, use the connection window to sign in again. Only the bundled connection window may invoke the native connection command.
The shell exposes no filesystem or shell capabilities to remote pages, and
navigation is restricted to the configured server origin. `--no-bundle`
builds only the executable.

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
available language from the bundled catalogs (including the local connection
window); layout direction follows the catalog's `LTR` message.

Vite's development server is for UI development; API requests need a same-origin
reverse proxy to a scratch Kabacus-server. The browser tests use synthetic
API fixtures and do not touch real books. Server asset isolation tests live
in `crates/kabacus-server/tests/web_e2e.rs`. CI checks both the web build and
native shell separately from the existing KDE jobs.
