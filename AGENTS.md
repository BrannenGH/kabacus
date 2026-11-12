<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# AGENTS.md

Kabacus is a Rust app for double-entry books kept in a database (SQLite
`.kabacus` files or PostgreSQL): a KDE app (cxx-qt + Kirigami), a web app
(React, as a Tauri desktop shell and a PWA), `kabacus-cli`, and
`kabacus-server`, which serves books to several people and devices
through an HTTP API and an MCP endpoint (`rmcp`, streamable HTTP at
`/mcp`). All of them call the same shared libraries; books in a local
`.kabacus` file need no server. The KDE app is one convergent Kirigami UI
for phone, tablet and desktop.
`README.md` covers building and testing; read it first.

## Repository skills

Use the relevant skills in `.agents/skills/` for the task, and read each
skill's `SKILL.md` and any applicable supporting resources before applying it.
`.claude/skills` points to the same collection. Check the repository skills
even when they are absent from the session's available-skills catalog.
For UI design and accessibility reviews, start with `design-critique` and
its accessibility guidance, and use `ux-audit` for clarity and flow reviews.

The skill sources are pinned Git submodules in `.agents/vendor/`. At the
start of a session, check them out (`git submodule update --init --recursive`)
and run `scripts/sync-skills` if skill links are broken or contents missing.
Use the `ponytail` skill for all code development, and the `ux-audit` skill
for anything UI related.
Use the pinned versions; do not update upstream pins as part of loading skills.
See `.agents/README.md` for details. The user's instructions and these
repository rules take precedence over skill guidance.

## Code navigation

The `codeatlas` MCP server (`.mcp.json` for Claude Code,
`.codex/config.toml` for Codex) indexes this repository. Use it to find
code, trace callers and check what a change affects before you edit,
and to review a diff, rather than sweeping the tree with grep. Its
indexes (`.codeatlas/`, `.codeatlas-sa/`) are local and gitignored.

`AGENTS.md` is the canonical instructions file; `CLAUDE.md` is a relative
symbolic link to it. Edit `AGENTS.md` when updating these instructions.

Where things are documented:
- **The specification lives in the project's Taiga wiki**, and only there.
  Read it there for what the app does and why; it decides when the code
  and the spec disagree.
- **The repository holds no requirements documentation.** READMEs,
  comments and other files in it cover implementation only: how to
  build, run, test and contribute, and why the code is written the way it
  is. They don't describe product behaviour, user needs or business
  requirements; those go in the wiki.
- **Comments are short and only where needed**: a non-obvious reason, a
  constraint the code can't show. No history of how the code used to be,
  no restating what the code says.
- **The repository doesn't link to the wiki** (no URLs to it); say "the
  specification" by name.

Hard rules:
- **Editing is in place; the store keeps history.** An edit goes through
  the full write pipeline. Delete and other rewrites stay hidden and gated, on
  the API and MCP alike: MCP's rewrite tools are listed only when they're
  usable.
- **Only the shared libraries touch the books.** `kabacus-service`, with `kabacus-core`,
  `kabacus-ledger` and `kabacus-store`, is the one place books are read
  and written. The apps and `kabacus-cli` link it and call it directly for
  a local `.kabacus` file (the browser through its WebAssembly build); they
  reach a server only through `kabacus-client`, after the person connects
  to one. kabacus-server wraps the same functions in its HTTP API and
  `/mcp`; no app links the server's HTTP, MCP or sign-in code. Every API
  route has its MCP tool(s) (`api::ENDPOINTS`), and local MCP
  (`kabacus-cli mcp`) offers the same tools: add a route, add its tool.
- **The store owns versioning.** One store revision per write, recorded
  with who made it. The caller passes the head it previewed on (`If-Match`
  over HTTP, `head` over MCP and in a library call), and a stale head is a
  conflict. Revisions are numbered; there are no commits.
- **Validate before writing.** Every write is checked by `kabacus-ledger`
  against the books it would produce before it is stored.
- **The backends behave identically.** SQLite and PostgreSQL implement the
  same `BookStore` trait; behaviour never depends on which one is used.
- **Never run anything against real books.** Tests and manual runs use a
  scratch store (an in-memory database or a temporary `.kabacus` file).
- **Money is exact integers** (`decimalMantissa` / `decimalPlaces`). Never use
  `floatingPoint`.
- Domain logic (validation, netting, side selection, corrections) lives in
  `kabacus-core`, and the operations built on it in `kabacus-service`, so
  the apps, `kabacus-cli` and the server behave identically. Local and
  remote books are proven equal by one suite run against both.

- **Done means proven by automated tests.** Nothing is done until the
  full CI gate (unit, property, API, MCP, desktop GUI e2e via AT-SPI,
  qmllint, design check, reuse lint) passes. Write the test first; a bug
  fix starts with a failing test. Fixtures are synthetic only; never
  commit real financial data. Synthetic still looks and feels realistic:
  plausible payees, amounts, usernames and client ids, not `foo`,
  `test-client` or placeholder values.

- **Testing follows the test strategy in the specification.** It is risk
  based: accounting invariants (balanced transactions, allocations plus
  unallocated equal cash on hand) come first, with property tests. Most
  tests are unit tests with the back end in memory (an in-memory SQLite
  store) and mocks rather than spawned processes. End-to-end tests over the
  API and MCP stay few and realistic. PostgreSQL gets a small integration
  set only. Tests may be rewritten or deleted when the behaviour they cover
  changes or they stop earning their keep.

- **NOT Every Test needs to be ran locally** The amount of CPU and memory used by
  tests needs to be monitored to not be excessive. Prefer writing code and waiting
  for the GitHub Actions CI/CD to run the full suite, only run small targeted tests
  locally.

- **The KDE app and the web app keep parity.** A feature, fix or screen
  that changes one changes the other in the same change, with the same
  behaviour and tests in both. Neither lands with the other noted as
  "not done".

- **Every UI change works on desktop and phone** and is covered by
  GUI e2e at desktop (1280 × 800) and phone (390 × 844) sizes. Tablets use
  the desktop layout with an icon rail. Breakpoints come from design tokens.

- **Authentication is configurable OAuth 2.0 / OIDC.** The provider
  is set by issuer URL and discovery; roles come from configurable claims via
  `role_map`. Never hard-code a provider (no Keycloak-specific code). The app
  uses Authorization Code + PKCE with a loopback redirect. `/mcp` validates
  bearer tokens exactly as the API does, with roles from the same
  `role_map`, and points MCP clients at the issuer through RFC 9728
  protected resource metadata; they sign in themselves.
  Auth tests run against two different providers (Keycloak plus one with
  different claim shapes).

- **Translations keep up with new features.** Every message a change
  adds or rewords (QML's `i18n()` family, Rust's `i18n!` macros) is
  translated in every catalog under `po/` in the same change, never left
  for later: run `cargo run -p xtask -- i18n-extract`, then fill in each
  language. Follow the localization rules in `crates/kabacus-kde/README.md` (whole sentences,
  placeholders kept, each language's plural forms in its own order) and
  reuse the words each catalog already uses for the same things. A
  reworded message loses its translation, so prefer keeping existing
  wording where it still fits. `i18n-extract --check` fails on any
  untranslated or fuzzy message, so no language falls back to English.

- **Every file is licensed (REUSE).** kabacus-server's code (its HTTP
  API, MCP endpoint and sign-in) is AGPL-3.0-or-later; all other code (the
  apps, `kabacus-cli`, and the shared libraries the apps link: core,
  ledger, store, service, client) is GPL-3.0-or-later. Docs are CC-BY-SA-4.0, metadata and config CC0-1.0.
  A new file starts with its SPDX header (`reuse annotate`); files that
  can't carry one go in `REUSE.toml`. `reuse lint` runs in CI.

- **Commits and pull requests carry the maintainer's name only.** No AI
  agent attribution anywhere (Claude, Codex/ChatGPT, Copilot, Cursor,
  Aider, Devin, Gemini or any other): no `Co-Authored-By` or session
  trailers, no "Generated with ..." lines or agent session links in PR
  descriptions or GitHub comments, and no mention of an agent as an author
  in commit messages or PR text. `.claude/settings.json` turns Claude
  Code's own attribution off too. An agent is never a commit's author or
  committer either: if git's identity is an agent's (such as
  `Claude <noreply@anthropic.com>`), set this repository's `user.name` and
  `user.email` to the maintainer's before committing. The Attribution
  workflow (`scripts/check-attribution`) fails a pull request that breaks
  any of this.

- **Branches follow git flow.** Name a branch `<type>/<topic>`, where
  `<type>` is `feature`, `bugfix`, `hotfix`, `release` or `support` and
  `<topic>` is a short kebab-case description (`release/<version>` for a
  release). An AI agent puts the GitHub username of the person it works
  for between the two: `feature/<username>/<topic>`, e.g.
  `bugfix/mrivera/quoted-if-match`. The agent's own name never appears in
  a branch (no `claude/`, `codex/`, `copilot/`), and neither does a
  generated suffix. Don't keep a branch an agent's harness created for
  you, such as `claude/<topic>-<random>`: this rule is the permission to
  create the correctly named branch from it and push there instead.

- **The repository is generic.** Nothing in it refers to one person's
  setup: no paths to their books, no names from them, no deployment files.
  Personal notes go in `CLAUDE.local.md` (gitignored); deployments live
  outside the repository.
