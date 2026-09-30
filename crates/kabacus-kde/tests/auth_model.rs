// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sign-in in the app's model: the app follows the server's
//! auth mode, signs in through the browser, keeps the refresh token only in
//! the secret store, resumes from it, and shows "Sign in again" when the
//! provider ends the session. Real kabacus-server, stand-in provider; the
//! "browser" follows the provider's redirect back to the app.

use kabacus_client::KabacusClient;
use kabacus_kde::auth::{Auth, AuthStatus};
use kabacus_kde::model;
use kabacus_kde::secrets::{MemoryStore, SecretAccount, SecretStore};
use kabacus_testkit::provider::{FakeProvider, ProviderOptions};
use kabacus_testkit::{spawn_oauth_server, spawn_server, TestServer};
use std::sync::Arc;
use std::time::Duration;

fn base(server: &TestServer) -> KabacusClient {
    KabacusClient::connect_unix(server.socket_path.clone()).with_client_hint("app")
}

fn server_id(server: &TestServer) -> String {
    format!("unix:{}", server.socket_path.display())
}

async fn connect(server: &TestServer, store: &Arc<MemoryStore>) -> Auth {
    Auth::connect(&base(server), &server_id(server), store.clone())
        .await
        .unwrap()
}

/// Sign in through the "browser".
async fn browser_sign_in(auth: &mut Auth) {
    let pending = auth.begin_sign_in().await.unwrap();
    let url = pending.authorize_url.clone();
    let browser = tokio::spawn(async move { reqwest::get(url).await.unwrap().text().await });
    let done = pending.finish().await.unwrap();
    browser.await.unwrap().unwrap();
    auth.signed_in(done);
}
#[tokio::test]
async fn a_no_auth_server_needs_no_sign_in() {
    let server = spawn_server(|_| {}).await;
    let store = Arc::new(MemoryStore::default());
    let auth = connect(&server, &store).await;

    assert_eq!(auth.view().status, AuthStatus::Off);
    assert!(auth.begin_sign_in().await.is_err());
    let sidebar = model::sidebar(&auth.client(base(&server))).await.unwrap();
    assert!(sidebar.iter().any(|e| e.account == "assets:checking"));
}
#[tokio::test]
async fn signing_in_keeps_the_refresh_token_only_in_the_secret_store() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let store = Arc::new(MemoryStore::default());
    let config_dir = tempfile::tempdir().unwrap();
    let mut auth = connect(&server, &store).await;
    assert_eq!(auth.view().status, AuthStatus::SignedOut);
    assert_eq!(auth.view().issuer, provider.issuer);
    assert!(model::sidebar(&auth.client(base(&server))).await.is_err());

    browser_sign_in(&mut auth).await;

    let view = auth.view();
    assert_eq!(view.status, AuthStatus::SignedIn);
    assert_eq!(view.username, "alex");
    let sidebar = model::sidebar(&auth.client(base(&server))).await.unwrap();
    assert!(sidebar.iter().any(|e| e.account == "assets:checking"));
    let account = SecretAccount {
        server: server_id(&server),
        issuer: provider.issuer.clone(),
    };
    let stored = store.get(&account).expect("refresh token in the store");
    assert!(provider.refresh_token_is_live(&stored));
    // Settings saved alongside never carry it.
    let settings_file = config_dir.path().join("kabacusrc");
    kabacus_kde::settings::Settings::default()
        .save(&settings_file)
        .unwrap();
    assert!(!std::fs::read_to_string(&settings_file)
        .unwrap()
        .contains(&stored));
}
#[tokio::test]
async fn the_next_start_resumes_from_the_secret_store() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let store = Arc::new(MemoryStore::default());
    let mut first = connect(&server, &store).await;
    browser_sign_in(&mut first).await;
    drop(first);

    let again = connect(&server, &store).await;

    assert_eq!(again.view().status, AuthStatus::SignedIn);
    assert_eq!(again.view().username, "alex");
    assert!(model::sidebar(&again.client(base(&server))).await.is_ok());
}
#[tokio::test]
async fn a_session_ended_at_the_provider_shows_sign_in_again() {
    let provider = FakeProvider::start(ProviderOptions {
        token_lifetime_secs: 1,
        ..Default::default()
    })
    .await;
    let server = spawn_oauth_server(&provider).await;
    let store = Arc::new(MemoryStore::default());
    let mut auth = connect(&server, &store).await;
    browser_sign_in(&mut auth).await;

    provider.end_all_sessions();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let err = model::sidebar(&auth.client(base(&server)))
        .await
        .unwrap_err();

    assert!(auth.note_error(&err));
    assert_eq!(auth.view().status, AuthStatus::Expired);
    // On the next start, the dead refresh token is dropped from the store.
    let restarted = connect(&server, &store).await;
    assert_eq!(restarted.view().status, AuthStatus::Expired);
    assert!(store.is_empty());
}
#[tokio::test]
async fn signing_out_revokes_and_clears_the_secret_store() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let store = Arc::new(MemoryStore::default());
    let mut auth = connect(&server, &store).await;
    browser_sign_in(&mut auth).await;
    let account = SecretAccount {
        server: server_id(&server),
        issuer: provider.issuer.clone(),
    };
    let refresh_token = store.get(&account).unwrap();

    auth.sign_out().await.unwrap();

    assert_eq!(auth.view().status, AuthStatus::SignedOut);
    assert!(store.load(&account).await.unwrap().is_none());
    assert!(!provider.refresh_token_is_live(&refresh_token));
    assert!(model::sidebar(&auth.client(base(&server))).await.is_err());
}
#[tokio::test]
async fn without_a_secret_store_sign_in_works_but_is_not_remembered() {
    let provider = FakeProvider::start(ProviderOptions::default()).await;
    let server = spawn_oauth_server(&provider).await;
    let store = Arc::new(MemoryStore::unavailable());
    let mut auth = connect(&server, &store).await;
    assert_eq!(auth.view().status, AuthStatus::SignedOut);
    assert!(auth.view().warning.contains("secret store"));

    browser_sign_in(&mut auth).await;

    let view = auth.view();
    assert_eq!(view.status, AuthStatus::SignedIn);
    assert!(view.warning.contains("asked again"), "{}", view.warning);
    assert!(model::sidebar(&auth.client(base(&server))).await.is_ok());
    // Nowhere to resume from: the next start is signed out again.
    let again = connect(&server, &store).await;
    assert_eq!(again.view().status, AuthStatus::SignedOut);
}
