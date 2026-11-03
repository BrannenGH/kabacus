<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Document sessions and automation

Build the local gateway and CLI together:

    scripts/dev cargo build -p kabacus-server --bins

Create synthetic books, then open a document session:

    kabacus-cli --file /tmp/household.kabacus create --name Household
    kabacus-server --auth=none --document /tmp/household.kabacus --socket /tmp/household.sock

The desktop package includes both binaries. Opening a `.kabacus` file with
`kabacus /tmp/household.kabacus` starts a private gateway and sets
`KABACUS_SOCKET` for that window. An ordinary launch retains the configured
connection. The app never opens SQLite itself.

Use the same session from an MCP client:

    kabacus-cli --socket /tmp/household.sock mcp

Or start a headless file session:

    kabacus-cli --file /tmp/household.kabacus mcp
    kabacus-cli --file /tmp/household.kabacus call get_version '{}'

For stdio MCP configuration, use `kabacus-cli` as the command and
`["--file", "/tmp/household.kabacus", "mcp"]` as its arguments. Stdout holds
only protocol messages; recovery locations are written to stderr. To work
alongside a running desktop session, use its socket instead of opening the
file a second time. The CLI calls the existing MCP tools, including their
default dry runs and role/rewrite gates. A TOML config can supply account
kind rules and `auth.no_auth_roles`; file mode explicitly disables network
listeners and uses the local principal.

## Nextcloud / WebDAV host

An existing WebDAV file can be opened with:

    kabacus-server --config /tmp/kabacus.toml --document-url https://cloud.example.org/remote.php/dav/files/robin/Household.kabacus

The host bearer credential is read from `KABACUS_DOCUMENT_TOKEN`; never put
it in a URL or command argument. The host must accept bearer authentication
and conditional PUT, and return strong ETags on GET and successful PUT.
Use a scoped OAuth token supported by the host. A Nextcloud app password
is not a bearer token.

`kabacus-cli --url URL mcp` runs the same host adapter headlessly.
The host credential and the gateway's API/MCP OAuth configuration are
separate. A remotely exposed gateway must retain OAuth and HTTPS; local
no-auth mode cannot enable the remote listener. The existing gateway's
role claims do not enforce Nextcloud's per-file ACLs: the adapter is not
a permission bridge. Keep hosted sessions private until a file-scoped
authorization and revocation integration is in place.

This is a WebDAV adapter, not an installed Nextcloud app or a WOPI
implementation. The Nextcloud file-action registration, permission-aware
launch/token handoff, document-session discovery and revocation bridge
are not implemented. Deploy one gateway per opened document, and route
all its editors to that gateway. Independent gateways contend through
conditional PUT; they do not form one collaborative session automatically.

The separation between storage and editing follows Microsoft's
[WOPI coauthoring integration](https://learn.microsoft.com/en-us/microsoft-365/cloud-storage-partner-program/online/scenarios/coauth).
The bundled, headless automation surface is informed by
[DesignCraft's CLI/MCP interface](https://github.com/storytold/designcraft/blob/main/docs/mcp.md).
No Office protocol compatibility is implied.

## Persistence and conflicts

`document::Session` validates the source through the existing untrusted-file
reader, then opens a private working SQLite database. The source is never
a live WAL database. Local sources with WAL, shared-memory or rollback-journal
sidecars are refused; close the legacy database gateway and export a consistent
snapshot before opening them in document mode. The HTTP router and MCP router share that working copy;
each write still goes through the existing service and accounting checks.

A checkpoint wraps each request, serializes document requests, and exports
all ledgers when their revisions change, including partial non-atomic
batches. Read-only requests and dry runs do not upload a new file.
The external storage ETag is independent of each ledger's numeric revision.

Local sessions hold a stable sidecar advisory lock. Cooperating Kabacus
processes must connect to the active socket. Before an atomic snapshot
replacement the adapter compares the source bytes with the opening/saved
snapshot. An unrelated writer or synchronization client does not honor this
lock, so this is not an atomic compare-and-swap against arbitrary external
programs. Use the WebDAV conditional-write adapter for concurrent remote
writers; do not live-sync a local session's working directory.

WebDAV saves send `If-Match` with the exact last storage ETag. Conflicts,
missing/weak ETags, or uncertain upload results stop further writes.
No automatic merge or retry is attempted. A failed checkpoint reports
`document_conflict`, `retry_write: false`, and whether changes were already
committed to the private working database. Check the file host after an
uncertain upload before deciding whether to apply the changes again.

Recovery directories are retained under
`$XDG_STATE_HOME/kabacus/recovery` (otherwise
`$HOME/.local/state/kabacus/recovery`), or `--recovery-dir`.
Each session has its own private directory containing the original snapshot,
the working database and WAL, source metadata without credentials, and
`pending.kabacus` after a checkpoint was prepared. Keep a live working
database together with its WAL. `pending.kabacus` is self-contained and can
be copied to a new filename and opened independently. Retained recovery
directories need manual cleanup after reconciliation; automatic retention,
resume and a conflict-resolution UI are not implemented.

## Existing PostgreSQL books

The original database-backed server remains available. Export one configured
ledger and its history without replacing an existing destination:

    kabacus-server --config /tmp/kabacus.toml export /tmp/household.kabacus

The export works for either backend. It excludes credentials and sharing
permissions, as existing portable exports do. Moving permissions and
switching production traffic require a separate migration; no existing
database is deleted or converted automatically.

## Checks

Synthetic document tests cover conditional writes, dry-run and stale-head
behavior, preservation of other ledgers, invalid file rejection, local
session locking and recovery after an external replacement:

    scripts/dev cargo test -p kabacus-server document::tests

Run the full existing CI gate before taking this draft out of review.
