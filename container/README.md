<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# kabacus-server image

`scripts/build-image` builds a redistributable image of kabacus-server
on openSUSE Tumbleweed
(`Containerfile`, stage `server`). It holds no settings for any one
machine:

| In the container              | What it is                                     |
|-------------------------------|------------------------------------------------|
| `/data` (volume)              | the books' database                             |
| `/run/kabacus` (volume)      | where `server.sock` appears                    |
| `/etc/kabacus/server.toml`   | the config (`server.toml` here); mount your own |
| `KABACUS_DATABASE`            | the `.kabacus` file (default `/data/books.kabacus`) |
| `TZ`                          | the books' timezone, for "today" (default UTC) |

The database is made on first start. To use PostgreSQL instead, set
`database_url` in the config.

Run it as the user who owns the data directory, so the database stays
theirs. With rootless podman:

    podman run -d --name kabacus-server \
      --userns=keep-id --read-only --tmpfs /tmp --network none \
      -v /path/to/data:/data \
      -v "$XDG_RUNTIME_DIR/kabacus:/run/kabacus" \
      -v /path/to/server.toml:/etc/kabacus/server.toml:ro \
      localhost/kabacus-server:latest --auth=none

`--auth=none` is for one user on one computer: unix socket only.
For OAuth or the remote HTTPS listener, configure
`[auth]` and `[remote]` in the config, publish the port and drop
`--auth=none`.

The server also serves MCP at `/mcp` (streamable HTTP), so an MCP client
such as an AI assistant can work with the books: point it at
`https://<host>:<port>/mcp` and it signs in with the same OAuth provider
as the app. To start it at login, wrap the same settings in a systemd
unit, e.g. a Quadlet `.container` file.

The server logs one line per request (`RUST_LOG` changes the level;
default `info`). To give the host journal each line's priority, so
`journalctl -p warning` works, mount the journal's socket and the
server writes to it directly; without it, logs go to stderr:

    -v /run/systemd/journal/socket:/run/systemd/journal/socket

In a Quadlet file that's `Volume=/run/systemd/journal/socket:/run/systemd/journal/socket`.
On an SELinux host the container may also need `SecurityLabelDisable=true`.

`--save FILE` writes an OCI archive to load elsewhere (`podman load -i
FILE`); `IMAGE=registry/name scripts/build-image` tags it for a registry.
