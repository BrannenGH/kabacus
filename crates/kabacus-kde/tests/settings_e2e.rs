// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's connection settings against real servers: the
//! local unix socket in no-auth mode, and the remote HTTPS listener with a
//! private CA in OAuth mode.

use kabacus_kde::settings::{test_connection, ConnectionKind, ConnectionStatus, Settings};
use kabacus_testkit::{spawn_remote_server, spawn_server};

fn local(socket: &std::path::Path) -> Settings {
    Settings {
        kind: ConnectionKind::Local,
        socket_path: socket.display().to_string(),
        books_path: String::new(),
        ..Default::default()
    }
}
#[tokio::test]
async fn testing_a_local_socket_reports_the_server_and_its_accounts() {
    let server = spawn_server(|_| {}).await;

    let report = test_connection(&local(&server.socket_path)).await.unwrap();
    assert_eq!(report.status, ConnectionStatus::Connected);
    assert_eq!(report.auth_mode, "none");
    assert!(!report.server_version.is_empty());
    assert!(report.message.contains("6 accounts"), "{}", report.message);
}
#[tokio::test]
async fn testing_a_missing_socket_says_the_server_is_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let err = test_connection(&local(&dir.path().join("nothing.sock")))
        .await
        .unwrap_err();
    assert!(err.to_string().starts_with("Can't reach"), "{err}");
}
#[tokio::test]
async fn saved_settings_connect_the_app_to_the_server() {
    let server = spawn_server(|_| {}).await;
    let config = tempfile::tempdir().unwrap();
    let path = config.path().join("kabacusrc");
    local(&server.socket_path).save(&path).unwrap();

    let client = Settings::load(&path).connect().unwrap();
    let sidebar = kabacus_kde::model::sidebar(&client).await.unwrap();
    assert!(sidebar.iter().any(|e| e.account == "assets:checking"));
}
#[tokio::test]
async fn testing_a_remote_server_trusts_its_private_ca_and_asks_for_sign_in() {
    let remote = spawn_remote_server().await;
    let config = tempfile::tempdir().unwrap();
    let ca = config.path().join("ca.pem");
    std::fs::write(&ca, &remote.pki.ca_pem).unwrap();
    let settings = Settings {
        kind: ConnectionKind::Remote,
        server_url: remote.url.clone(),
        private_ca: ca.display().to_string(),
        ..Default::default()
    };

    let report = test_connection(&settings).await.unwrap();
    // Reachable and trusted; data needs an OAuth sign-in.
    assert_eq!(report.status, ConnectionStatus::SignInRequired);
    assert_eq!(report.auth_mode, "oauth");
}
#[tokio::test]
async fn testing_a_remote_server_without_its_ca_explains_the_certificate() {
    let remote = spawn_remote_server().await;
    let settings = Settings {
        kind: ConnectionKind::Remote,
        server_url: remote.url.clone(),
        ..Default::default()
    };

    let err = test_connection(&settings).await.unwrap_err();
    assert!(err.to_string().contains("certificate"), "{err}");
}
#[tokio::test]
async fn a_remote_address_must_be_https() {
    let settings = Settings {
        kind: ConnectionKind::Remote,
        server_url: "http://example.invalid:8443".to_string(),
        ..Default::default()
    };
    let err = test_connection(&settings).await.unwrap_err();
    assert!(err.to_string().contains("https://"), "{err}");
}
