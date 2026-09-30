// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where kabacus-server keeps the books: the config's `database`, else
//! `$KABACUS_DATABASE`, else `books.kabacus` under the XDG data directory,
//! made on first start. It refuses a file that isn't Kabacus's.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn server_makes_the_default_database_on_first_start() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("kabacus.toml");
    std::fs::write(&config, "").unwrap();
    let socket = home.path().join("k.sock");
    let mut server = Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
        .arg("--config")
        .arg(&config)
        .arg("--auth=none")
        .arg("--socket")
        .arg(&socket)
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env_remove("KABACUS_DATABASE")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "the server didn't start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = Command::new("kill").arg(server.id().to_string()).status();
    server.wait().unwrap();
    assert!(home.path().join("data/kabacus/books.kabacus").is_file());
}

#[test]
fn server_refuses_a_file_that_isnt_kabacus() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.kabacus");
    let text = "2026-01-01 Opening\n    assets:checking  10.00 USD\n    equity:opening\n";
    std::fs::write(&file, text).unwrap();
    let config = dir.path().join("kabacus.toml");
    std::fs::write(
        &config,
        format!("database = {:?}\n", file.display().to_string()),
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
        .arg("--config")
        .arg(&config)
        .arg("--auth=none")
        .arg("--socket")
        .arg(dir.path().join("k.sock"))
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

/// `--no-rewrite` turns rewrites off; `--allow-rewrite` is still accepted
/// (rewrites are on by default) but can't be combined with it.
#[test]
fn no_rewrite_flag_is_offered_and_conflicts_with_allow_rewrite() {
    let help = Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--no-rewrite"), "{help}");
    assert!(!help.contains("--allow-rewrite"), "{help}");

    let both = Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
        .args(["--no-rewrite", "--allow-rewrite"])
        .output()
        .unwrap();
    assert!(!both.status.success());
    let err = String::from_utf8_lossy(&both.stderr);
    assert!(err.contains("cannot be used with"), "{err}");
}
