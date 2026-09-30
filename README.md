<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Kabacus

Double-entry bookkeeping from a KDE app, a web interface or an MCP client. The books live in
a SQLite `.kabacus` file or PostgreSQL, and hledger journals can be imported.

- `kabacus-server` serves the HTTP API and an MCP endpoint
  ([container image](container/README.md)).
- `apps/kabacus-ui` is the web interface the server hosts, with a Tauri
  desktop shell ([README](apps/kabacus-ui/README.md)).
- `kabacus-kde` is the Kirigami app ([README](crates/kabacus-kde/README.md)).

## Build and test

Everything runs in containers, so you only need podman:

    scripts/dev cargo test --workspace

Tests use synthetic books. Never point a development build at real ones.

## More

- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md)
- Translations: [crates/kabacus-kde/README.md](crates/kabacus-kde/README.md)

## Licence

kabacus-server is AGPL-3.0-or-later; everything else is GPL-3.0-or-later.
Each file's licence is in its SPDX header or `REUSE.toml`; full texts are in
`LICENSES/`.
