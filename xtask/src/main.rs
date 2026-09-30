// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Developer tasks (`cargo xtask <command>`). Run from the workspace root:
//! `cargo run -p xtask -- <command>`.

mod design;
mod i18n;

const USAGE: &str = "usage: cargo xtask <command>
  design-generate [--check]   design/tokens.json -> design/generated/Theme.qml
  i18n-extract [--check]      the app's messages -> po/kabacus.pot and po/*.po
  i18n-build <dir>            po/*.po -> <dir>/<lang>/LC_MESSAGES/kabacus.mo";

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("design-generate") => design::generate(args.any(|a| a == "--check")),
        Some("i18n-extract") => {
            i18n::extract_all(&design::workspace_root()?, args.any(|a| a == "--check"))
        }
        Some("i18n-build") => {
            let Some(out) = args.next() else {
                anyhow::bail!("{USAGE}");
            };
            i18n::build_all(&design::workspace_root()?, std::path::Path::new(&out))
        }
        Some(other) => anyhow::bail!("unknown xtask command: {other}\n{USAGE}"),
        None => anyhow::bail!("{USAGE}"),
    }
}
