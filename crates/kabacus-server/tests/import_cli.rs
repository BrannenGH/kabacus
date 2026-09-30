// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `kabacus-server import`: hledger journals read into a ledger in the
//! configured store, once, and only books that check.

use std::path::Path;
use std::process::{Command, Output};

fn import(config: &Path, journal: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
        .arg("--config")
        .arg(config)
        .arg("import")
        .arg(journal)
        .output()
        .unwrap()
}

fn config(dir: &Path) -> std::path::PathBuf {
    let config = dir.join("kabacus.toml");
    std::fs::write(
        &config,
        format!(
            "database = {:?}\n",
            dir.join("books.kabacus").display().to_string()
        ),
    )
    .unwrap();
    config
}

#[test]
fn imports_a_journal_and_what_it_includes_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.journal"),
        "commodity USD1000.00\naccount assets:checking\naccount equity:opening\n\
         account expenses:groceries\ninclude 2026.journal\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("2026.journal"),
        "2026-01-01 Opening\n    assets:checking  1200.00 USD\n    equity:opening\n\n\
         2026-01-04 Fresh Thyme Market\n    expenses:groceries  64.12 USD\n    assets:checking\n",
    )
    .unwrap();
    let config = config(dir.path());

    let first = import(&config, &dir.path().join("main.journal"));
    let out = String::from_utf8_lossy(&first.stdout);
    assert!(
        first.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        out.contains("2 transaction(s), 3 account(s), 1 commodity"),
        "{out}"
    );
    assert!(dir.path().join("books.kabacus").is_file());

    let again = import(&config, &dir.path().join("main.journal"));
    assert!(
        !again.status.success(),
        "a second import into the same ledger"
    );
}

#[test]
fn refuses_books_that_dont_check() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.journal"),
        "account assets:checking\naccount equity:opening\n\n\
         2026-01-01 Opening\n    assets:checking  1200.00 USD\n    equity:opening  -1100.00 USD\n",
    )
    .unwrap();
    let out = import(&config(dir.path()), &dir.path().join("main.journal"));
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("main.journal"), "{err}");
}
