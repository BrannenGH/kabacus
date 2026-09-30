// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Phones and tablets reach the one kabacus-server over its
//! remote HTTPS listener, with an OAuth token on every request and a
//! certificate that chains to a private CA (no pinning).
//!
//! The provider is kabacus-testkit's stand-in OIDC provider; the server
//! itself runs for real (git and hledger).

use kabacus_client::{ClientError, KabacusClient};
use kabacus_testkit::spawn_remote_server;
#[tokio::test]
async fn remote_listener_serves_the_api_to_a_token_holder() {
    let server = spawn_remote_server().await;
    let client = KabacusClient::connect_https(&server.url, Some(server.pki.ca_pem.as_bytes()))
        .unwrap()
        .with_auth_token(server.provider.mint("tester", "kabacus-app"));

    let accounts = client.list_accounts(None).await.unwrap();
    assert!(
        accounts.iter().any(|a| a.account == "assets:checking"),
        "{accounts:?}"
    );
}
#[tokio::test]
async fn remote_listener_rejects_a_request_without_a_token() {
    let server = spawn_remote_server().await;
    let client =
        KabacusClient::connect_https(&server.url, Some(server.pki.ca_pem.as_bytes())).unwrap();
    assert!(matches!(
        client.list_accounts(None).await,
        Err(ClientError::Unauthorized)
    ));
}
#[tokio::test]
async fn client_refuses_a_certificate_outside_its_trust_store() {
    let server = spawn_remote_server().await;
    // No private CA configured: the test CA isn't in the system store.
    let client = KabacusClient::connect_https(&server.url, None)
        .unwrap()
        .with_auth_token(server.provider.mint("tester", "kabacus-app"));
    match client.list_accounts(None).await {
        Err(ClientError::Request(message)) => {
            assert!(message.contains("UnknownIssuer"), "{message}")
        }
        other => panic!("expected a TLS failure, got {other:?}"),
    }
}
#[test]
fn client_refuses_plain_http() {
    assert!(KabacusClient::connect_https("http://localhost:8443", None).is_err());
}
