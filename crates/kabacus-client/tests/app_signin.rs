// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's browser sign-in (Authorization Code + PKCE, loopback
//! redirect) against a stand-in OIDC provider and a real kabacus-server in
//! OAuth mode. The "browser" is an HTTP client that follows the provider's
//! redirect back to the app's loopback port, as a real one would.

use kabacus_client::oauth::app::{BrowserSignIn, UserSession};
use kabacus_client::oauth::{self, TokenSource};
use kabacus_client::types::{AddTransactionBody, DraftAmount, DraftPostingBody};
use kabacus_client::{ClientError, KabacusClient};
use kabacus_testkit::provider::{FakeProvider, ProviderOptions};
use kabacus_testkit::{spawn_oauth_server, TestServer};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Sign in the way the app does: issuer and client id from the server's
/// `/version`, endpoints from discovery, the user in the browser. Returns
/// the session and the page the browser ended on.
async fn sign_in(server: &TestServer) -> Result<(UserSession, String), ClientError> {
    let plain = KabacusClient::connect_unix(server.socket_path.clone());
    let version = plain.version().await?;
    assert_eq!(version.auth.mode, "oauth");
    let issuer = version.auth.issuer.unwrap();
    let client_id = version.auth.app_client_id.unwrap();
    let discovery = oauth::discover(&issuer).await?;
    let sign_in = BrowserSignIn::start(&discovery, &client_id, None).await?;

    let url = sign_in.authorize_url().to_string();
    let browser = tokio::spawn(async move {
        let page = reqwest::get(url).await.unwrap();
        page.text().await.unwrap()
    });
    let session = sign_in.finish().await;
    let page = browser.await.unwrap();
    Ok((session?, page))
}

fn client_for(server: &TestServer, session: &UserSession) -> KabacusClient {
    KabacusClient::connect_unix(server.socket_path.clone())
        .with_token_source(Arc::new(session.clone()))
}

async fn add_one(client: &KabacusClient) {
    let head = client.version().await.unwrap().head.unwrap();
    let body = AddTransactionBody {
        new_accounts: vec![],
        date: "2026-02-01".to_string(),
        description: "Grocery Store".to_string(),
        postings: vec![
            DraftPostingBody {
                account: "expenses:groceries".to_string(),
                amount: Some(DraftAmount::from_money(
                    "USD",
                    kabacus_core::Money::from_decimal(2000, 2),
                )),
                status: String::new(),
                balance_assertion: None,
                clear_balance_assertion: false,
            },
            DraftPostingBody {
                account: "assets:checking".to_string(),
                amount: None,
                status: String::new(),
                balance_assertion: None,
                clear_balance_assertion: false,
            },
        ],
        tags: vec![],
        dry_run: false,
    };
    client.add_transaction(&head, body).await.unwrap();
}

#[tokio::test]
async fn signs_in_through_the_browser_and_writes_as_that_user() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;

    let (session, page) = sign_in(&server).await.unwrap();
    assert!(page.contains("Signed in to Kabacus"), "{page}");

    // Without the session the server refuses; with it, reads and writes work.
    let anonymous = KabacusClient::connect_unix(server.socket_path.clone());
    assert!(matches!(
        anonymous.list_accounts(None).await,
        Err(ClientError::Unauthorized)
    ));
    let client = client_for(&server, &session);
    assert!(!client.list_accounts(None).await.unwrap().is_empty());
    add_one(&client).await;

    let last = client.history().await.unwrap().remove(0);
    assert_eq!(last.user, "alex");
    assert_eq!(last.actor.as_deref(), Some("app"));
    assert_eq!(session.username().await.as_deref(), Some("alex"));
    assert!(session.refresh_token().await.is_some());
}
#[tokio::test]
async fn a_refused_sign_in_is_reported() {
    let provider = FakeProvider::start(ProviderOptions {
        sign_in_as: None,
        ..Default::default()
    })
    .await;
    let server = spawn_oauth_server(&provider).await;

    let err = sign_in(&server).await.err().unwrap();
    assert!(err.to_string().contains("access_denied"), "{err}");
}
#[tokio::test]
async fn tokens_refresh_silently_and_the_new_refresh_token_is_announced() {
    let provider = FakeProvider::start(ProviderOptions {
        token_lifetime_secs: 1,
        ..Default::default()
    })
    .await;
    let server = spawn_oauth_server(&provider).await;
    let (session, _) = sign_in(&server).await.unwrap();
    let first_refresh = session.refresh_token().await.unwrap();
    let announced = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = announced.clone();
    session.on_new_refresh_token(move |token| sink.lock().unwrap().push(token.to_string()));
    let client = client_for(&server, &session);
    client.list_accounts(None).await.unwrap();
    let issued = provider.tokens_issued();

    tokio::time::sleep(Duration::from_millis(1100)).await;
    client.list_accounts(None).await.unwrap();

    assert_eq!(provider.tokens_issued(), issued + 1, "one silent refresh");
    let announced = announced.lock().unwrap().clone();
    assert_eq!(announced.len(), 1);
    assert_ne!(announced[0], first_refresh);
    assert_eq!(session.refresh_token().await, Some(announced[0].clone()));
}
#[tokio::test]
async fn a_session_ended_at_the_provider_asks_to_sign_in_again() {
    let provider = FakeProvider::start(ProviderOptions {
        token_lifetime_secs: 1,
        ..Default::default()
    })
    .await;
    let server = spawn_oauth_server(&provider).await;
    let (session, _) = sign_in(&server).await.unwrap();
    let client = client_for(&server, &session);
    client.list_accounts(None).await.unwrap();

    provider.end_all_sessions();
    tokio::time::sleep(Duration::from_millis(1100)).await;

    assert!(matches!(
        client.list_accounts(None).await,
        Err(ClientError::SessionExpired(_))
    ));
    // And it stays ended; no retry storm against the provider.
    let issued = provider.tokens_issued();
    assert!(matches!(
        session.token().await,
        Err(ClientError::SessionExpired(_))
    ));
    assert_eq!(provider.tokens_issued(), issued);
}
#[tokio::test]
async fn a_stored_refresh_token_resumes_the_session() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let (session, _) = sign_in(&server).await.unwrap();
    let stored = session.refresh_token().await.unwrap();
    drop(session);

    // The app starts again with only what the secret store kept.
    let discovery = oauth::discover(&provider.issuer).await.unwrap();
    let resumed = UserSession::resume(&discovery, "kabacus-app", &stored).unwrap();
    let client = client_for(&server, &resumed);
    assert!(!client.list_accounts(None).await.unwrap().is_empty());
    assert_eq!(resumed.username().await.as_deref(), Some("alex"));

    // A refresh token the provider no longer knows can't be resumed.
    let stale = UserSession::resume(&discovery, "kabacus-app", &stored).unwrap();
    assert!(matches!(
        client_for(&server, &stale).list_accounts(None).await,
        Err(ClientError::SessionExpired(_))
    ));
}
#[tokio::test]
async fn signing_out_revokes_the_refresh_token_at_the_provider() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let (session, _) = sign_in(&server).await.unwrap();
    let refresh_token = session.refresh_token().await.unwrap();

    session.sign_out().await.unwrap();

    assert_eq!(provider.revoked(), vec![refresh_token.clone()]);
    assert!(!provider.refresh_token_is_live(&refresh_token));
    assert!(session.refresh_token().await.is_none());
    assert!(matches!(
        client_for(&server, &session).list_accounts(None).await,
        Err(ClientError::SessionExpired(_))
    ));
}
