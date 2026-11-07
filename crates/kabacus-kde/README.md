<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# kabacus-kde

Kabacus, the Kirigami app (its binary, package and settings are
`kabacus`). It talks to kabacus-server only through `kabacus-client`.

Settings live in `kabacusrc` (KConfig format); `KABACUS_SOCKET`
overrides the socket without saving it. `--size=WxH` opens the window at
a size (the GUI e2e tests use it for the phone, tablet and desktop form
factors).

The app ships as native packages, built against each distribution's Qt
and Kirigami. CI and every release build them:

    scripts/build-packages  # target/packages: RPM (openSUSE Tumbleweed), DEB (Debian)

The packages are versioned `<major>.<minor>.<commit count>`, so each master
build upgrades the last.
Their metadata (files, QML module dependencies) is in `Cargo.toml`.

The server is deployed from its image, `scripts/build-image`; see
`container/README.md`.

Tests run in openSUSE Tumbleweed containers (`Containerfile`), which have
Rust, the pinned hledger and the test tools, so none of that is needed on
the host:

    scripts/dev cargo test --workspace

- `src/views.rs` (what each screen shows) and `src/writes.rs` (every
  form's preview and commit): everything the UI does, in plain Rust (no
  Qt), tested by `tests/screens_e2e.rs` against a real server on synthetic
  books. `src/model.rs` holds the original sidebar/register/simple-editor
  model, tested by `tests/model_e2e.rs`; the app now uses only its
  `edit_draft` and `ModelError`.
- `src/settings.rs`: the connection settings and "Test connection";
  tested by `tests/settings_e2e.rs` against local and remote servers.
- `src/auth.rs`, `src/secrets.rs`: sign-in state and the secret store;
  tested by `tests/auth_model.rs` against a stand-in OIDC provider. The
  browser flow itself is `kabacus_client::oauth::app`. The GUI e2e run
  uses a real KWallet (`ksecretd`), started unlocked by
  `tests/gui/secret-service.py`. Other desktops' secret stores aren't tested.
- `src/i18n.rs`: the `i18n!` macros the Rust side words what it shows
  with; `src/translation.rs` and `translation.h` send them, and QML's
  `i18n()`, through KI18n (see "Translations" below). Without
  the gui they show the English text, so the tests read the source's.
- `src/bridge.rs`, `src/main.rs`, `qml/`: the cxx-qt bridge and UI, built
  only with `--features gui`. `qml/Theme.qml` links to
  `design/generated/Theme.qml`, generated from `design/tokens.json`.

Qt 6 and Kirigami come from the gui container, not the host;
`--wayland` shows its windows on your desktop:

    scripts/dev --gui cargo build -p kabacus-kde --features gui

Try it on throwaway books (never your real books):

    scripts/dev-server &
    scripts/dev --gui --wayland env KABACUS_SOCKET=$PWD/target/scratch/server.sock \
        target/dev/debug/kabacus

VS Code: `.vscode/` has these as tasks, and debug configs (gdb in the
containers; needs the C/C++ extension) for the app, the server, or both.

QML lint: `scripts/dev --gui scripts/qmllint`, as CI does, builds the bridge and runs qmllint over
`qml/` against its `me.hallb.kabacus` types; any warning fails. It also
writes `.qmlls.ini` (gitignored; qmlls only takes absolute import paths)
so qmlls in your editor resolves the same module. Point the editor at the
system or SDK qmlls; a standalone download can't see Kirigami.
`qml/qmldir` exists only for these tools: it repeats build.rs's
`Theme` singleton.

GUI e2e: `scripts/gui-e2e` builds the app against the gui container's Qt 6
and Kirigami and drives it through AT-SPI on a private Xvfb display.

`Ledger.load` answers from a cache kept per revision and fetches on the
tokio runtime (`Fetch.qml`); `refresh` preloads the main screens.
`src/bridge.rs` says which calls still block the GUI thread.

## Translations

One gettext catalog, `kabacus`, through KI18n: `po/kabacus.pot` is the
template and `po/<lang>.po` each language. QML uses KI18n's `i18n()`,
`i18nc()`, `i18np()` and `i18ncp()`; Rust uses macros of the same names
(`crates/kabacus-kde/src/i18n.rs`), which go through KI18n too.

    cargo run -p xtask -- i18n-extract        # update po/ after changing the app's text
    cargo run -p xtask -- i18n-build <dir>    # <dir>/<lang>/LC_MESSAGES/kabacus.mo

When writing messages:

- **Whole sentences.** Every message is one complete string with `%1`,
  `%2` placeholders, never pieces glued together. Placeholders stand only
  for numbers, names and amounts, and every message that has one says in
  its context (`i18nc`) what each one is. Separators (the list comma, the
  arrow in "Checking → Groceries") are messages too.
- **Layouts, not pixels.** Text sits in Qt Quick layouts that grow with
  it; a column holding translated text is as wide as its longest
  translation.
- **UTF-8** between Rust and Qt, file names from the file dialog included.
- **Right to left** comes from the catalog's `LTR` message (translated as
  `RTL`); mirror layouts, drawers, arrows and chart axes from it.

`i18n-extract --check` runs in CI. It fails if `po/` is out of date, if
any message is untranslated (or fuzzy) in any catalog, if a translation
drops its English text's placeholders or plural forms, or if a catalog
gets its language's direction wrong. Corrections to the machine-made
translations are welcome: edit `po/<lang>.po` in any gettext editor
(Lokalize, Poedit) and run `i18n-extract --check`.
