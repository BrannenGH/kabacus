<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# kabacus-service

Shared book operations over `kabacus-store`, with validation and reports from
`kabacus-core` and `kabacus-ledger`. This crate is GPL-3.0-or-later.

`Service` owns a store, ledger, revision cache, operation settings and account-kind
table. Construct it with an already opened store and ledger. The `service`,
`reports`, `search`, `import` and `batch` modules expose operations; `ops` and
`books` implement working copies, validation and revision writes. `commands`
decodes the arguments used by both single writes and batches.

Callers implement `caller::Caller` to supply authorization decisions, a store user
and audit metadata. Sign-in, role mapping, HTTP/MCP transport and execution
deadlines belong to the caller. `kabacus-server` supplies those adapters and
re-exports the operation modules under their existing paths.

Operations are async functions without a runtime dependency of their own. The
native store backends use Tokio; the WebAssembly backend executes SQLite
in the page. Portable import/export uses the same validated SQLite images.
`catalog` maps command ids to HTTP routes, MCP tools and schemas; `script`
turns named steps and references into an atomic batch.

Run the library's tests against in-memory SQLite:

```sh
cargo test -p kabacus-service --lib
```
