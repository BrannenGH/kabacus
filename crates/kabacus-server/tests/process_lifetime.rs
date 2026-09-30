// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The real kabacus-server binary and the signals it handles: SIGTERM and
//! SIGINT stop it cleanly, SIGHUP reloads the account kind rules.

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Synthetic books imported into a `.kabacus` file by the binary's own
/// `import`, and a config serving them on `socket`.
fn write_books(dir: &Path, socket: &Path) -> std::path::PathBuf {
    std::fs::write(
        dir.join("main.journal"),
        "commodity USD1000.00\naccount assets:checking\naccount equity:opening\n\n\
         2026-01-01 Opening\n    assets:checking  10.00 USD\n    equity:opening\n",
    )
    .unwrap();
    let config = dir.join("kabacus.toml");
    std::fs::write(
        &config,
        format!(
            "database = {:?}\nsocket_path = {:?}\n",
            dir.join("books.kabacus").display().to_string(),
            socket.display().to_string()
        ),
    )
    .unwrap();
    assert!(Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
        .arg("--config")
        .arg(&config)
        .arg("import")
        .arg(dir.join("main.journal"))
        .status()
        .unwrap()
        .success());
    config
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// In a container the server is PID 1, which gets no default action for
/// SIGTERM or SIGINT: it has to handle them itself, or `podman stop` hangs
/// until it resorts to SIGKILL.
#[test]
fn sigterm_shuts_the_server_down_cleanly() {
    for signal in ["TERM", "INT"] {
        let dir = tempfile::tempdir().unwrap();
        let socket =
            std::env::temp_dir().join(format!("khl-stop-{}.sock", uuid::Uuid::new_v4().simple()));
        let config = write_books(dir.path(), &socket);
        let mut server: Child = Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
            .arg("--config")
            .arg(&config)
            .arg("--auth=none")
            .spawn()
            .unwrap();
        wait_until("the server socket", || socket.exists());

        assert!(Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(server.id().to_string())
            .status()
            .unwrap()
            .success());
        let status = server.wait().unwrap();

        assert!(status.success(), "SIG{signal}: {status}");
        assert!(!socket.exists(), "SIG{signal} left the socket behind");
    }
}

/// The account kinds of `GET /accounts/kinds`, by account.
fn account_kind(socket: &Path, account: &str) -> Option<String> {
    use http_body_util::{BodyExt, Full};
    use hyper_util::client::legacy::Client;
    use hyperlocal::UnixClientExt;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let client: Client<hyperlocal::UnixConnector, Full<bytes::Bytes>> = Client::unix();
        let response = client
            .get(hyperlocal::Uri::new(socket, "/accounts/kinds").into())
            .await
            .ok()?;
        let body = response.into_body().collect().await.ok()?.to_bytes();
        let kinds: serde_json::Value = serde_json::from_slice(&body).ok()?;
        kinds["accounts"]
            .as_array()?
            .iter()
            .find(|a| a["account"] == account)
            .and_then(|a| a["kind"].as_str().map(str::to_string))
    })
}

/// SIGHUP re-reads the account kind rules from the config file, without
/// a restart.
#[test]
fn sighup_reloads_the_account_kind_rules() {
    let dir = tempfile::tempdir().unwrap();
    let socket =
        std::env::temp_dir().join(format!("khl-hup-{}.sock", uuid::Uuid::new_v4().simple()));
    let config = write_books(dir.path(), &socket);
    let base = std::fs::read_to_string(&config).unwrap();
    let with_rule = |kind: &str| {
        format!("{base}\n[[account_kinds]]\npattern = \"assets:checking\"\nkind = \"{kind}\"\n")
    };
    std::fs::write(&config, with_rule("money")).unwrap();
    let mut server: Child = Command::new(env!("CARGO_BIN_EXE_kabacus-server"))
        .arg("--config")
        .arg(&config)
        .arg("--auth=none")
        .spawn()
        .unwrap();
    wait_until("the server socket", || socket.exists());
    assert_eq!(
        account_kind(&socket, "assets:checking").as_deref(),
        Some("money")
    );

    std::fs::write(&config, with_rule("non-liquid")).unwrap();
    assert!(Command::new("kill")
        .arg("-HUP")
        .arg(server.id().to_string())
        .status()
        .unwrap()
        .success());
    wait_until("the reloaded rules", || {
        account_kind(&socket, "assets:checking").as_deref() == Some("non-liquid")
    });

    let _ = Command::new("kill").arg(server.id().to_string()).status();
    server.wait().unwrap();
    let _ = std::fs::remove_file(&socket);
}
