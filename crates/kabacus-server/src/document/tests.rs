// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::automation::Gateway;
use crate::config::AuthMode;
use serde_json::json;

async fn seed(path: &Path) {
    let store = kabacus_store::SqliteStore::in_memory().unwrap();
    let owner = store.local_user("Robin Morgan").await.unwrap();
    let household = store.create_ledger("Household", owner).await.unwrap();
    let club = store.create_ledger("Garden club", owner).await.unwrap();
    store.export_file(&[household, club], path).await.unwrap();
}

fn config() -> Config {
    let mut config = Config::default();
    config.auth.mode = AuthMode::None;
    config.auth.no_auth_roles = vec![
        "ledger-read".into(),
        "ledger-write".into(),
        "ledger-rewrite".into(),
    ];
    config
}

#[tokio::test]
async fn api_and_mcp_save_the_same_portable_document_without_losing_other_ledgers() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    seed(&file).await;
    let original = std::fs::read(&file).unwrap();
    let session = Session::open(config(), Host::local(file.clone()).unwrap(), dir.path())
        .await
        .unwrap();
    let gateway = Gateway::Embedded(router(session));
    let preview = gateway
        .call(
            "declare_account",
            json!({
                "account": "assets:bank:checking", "head": "1"
            }),
        )
        .await
        .unwrap();
    assert!(preview.get("error").is_none(), "{preview}");
    assert_eq!(std::fs::read(&file).unwrap(), original);
    let result = gateway
        .call(
            "declare_account",
            json!({
                "account": "assets:bank:checking", "head": "1", "dry_run": false
            }),
        )
        .await
        .unwrap();
    assert!(result.get("error").is_none(), "{result}");
    let dumps = kabacus_store::file::read(&file).unwrap();
    assert_eq!(dumps.len(), 2);
    assert_eq!(dumps[0].revision, 2);
    assert_eq!(dumps[1].name, "Garden club");
    assert_eq!(dumps[1].revision, 1);
    assert!(dumps[0]
        .accounts
        .iter()
        .any(|a| a.name == "assets:bank:checking"));
    let saved = std::fs::read(&file).unwrap();
    let stale = gateway
        .call(
            "declare_account",
            json!({
                "account": "expenses:groceries", "head": "1", "dry_run": false
            }),
        )
        .await
        .unwrap();
    assert!(stale.get("error").is_some(), "{stale}");
    assert_eq!(std::fs::read(&file).unwrap(), saved);
}

#[tokio::test]
async fn external_replacement_blocks_writes_and_preserves_both_copies() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    seed(&file).await;
    let session = Session::open(config(), Host::local(file.clone()).unwrap(), dir.path())
        .await
        .unwrap();
    let recovery = session.recovery_path().to_path_buf();
    let gateway = Gateway::Embedded(router(session));
    let replacement = b"an external upload";
    std::fs::write(&file, replacement).unwrap();
    let result = gateway
        .call(
            "declare_account",
            json!({
                "account": "assets:bank:checking", "head": "1", "dry_run": false
            }),
        )
        .await
        .unwrap();
    assert_eq!(result["error"]["data"]["code"], "document_conflict");
    assert_eq!(
        result["error"]["data"]["details"]["committed_locally"],
        false
    );
    assert_eq!(std::fs::read(&file).unwrap(), replacement);
    assert!(recovery.join("working.kabacus").is_file());
    assert!(recovery.join("base.kabacus").is_file());
}

#[tokio::test]
async fn failed_checkpoint_keeps_a_self_contained_recovery_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    seed(&file).await;
    let mut session = Session::open(config(), Host::local(file.clone()).unwrap(), dir.path())
        .await
        .unwrap();
    let user = session
        .state
        .store
        .local_user("Robin Morgan")
        .await
        .unwrap();
    session
        .state
        .store
        .create_ledger("Community kitchen", user)
        .await
        .unwrap();
    std::fs::write(&file, b"an external replacement").unwrap();
    assert!(session.flush().await.is_err());
    let pending =
        kabacus_store::file::read(&session.recovery_path().join("pending.kabacus")).unwrap();
    assert_eq!(pending.len(), 3);
    assert_eq!(std::fs::read(file).unwrap(), b"an external replacement");
}

#[test]
fn a_file_has_one_local_session_and_host_urls_never_contain_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    let first = Host::local(file.clone()).unwrap();
    assert!(Host::local(file.clone()).is_err());
    drop(first);
    assert!(Host::local(file).is_ok());
    for url in [
        "http://cloud.example.org/books.kabacus",
        "https://robin:secret@cloud.example.org/books.kabacus",
        "https://cloud.example.org/books.kabacus?token=secret",
    ] {
        assert!(Host::webdav(url, None).is_err());
    }
}

#[tokio::test]
async fn untrusted_documents_are_validated_before_opening_a_working_database() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    std::fs::write(&file, b"not a SQLite document").unwrap();
    assert!(
        Session::open(config(), Host::local(file).unwrap(), dir.path())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn webdav_saves_use_the_exact_storage_etag_and_keep_conflicting_uploads() {
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::put;
    let app = Router::new().route(
        "/books.kabacus",
        put(|headers: HeaderMap| async move {
            assert_eq!(headers.get("if-match").unwrap(), "\"storage-revision-7\"");
            StatusCode::PRECONDITION_FAILED
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // Only the fixture bypasses the production HTTPS-only constructor.
    let host = Host::WebDav {
        url: format!("http://{address}/books.kabacus").parse().unwrap(),
        http: reqwest::Client::new(),
        token: None,
    };
    let base = Snapshot {
        bytes: b"base".to_vec(),
        version: "\"storage-revision-7\"".into(),
    };
    assert!(host.save(&base, b"edited".to_vec(), 1024).await.is_err());
    server.abort();
}

#[tokio::test]
async fn a_late_host_conflict_reports_local_commit_and_never_retries_on_a_read() {
    use axum::http::StatusCode;
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("household.kabacus");
    seed(&file).await;
    let source = std::fs::read(&file).unwrap();
    let uploads = Arc::new(AtomicUsize::new(0));
    let count = uploads.clone();
    let app = Router::new().route(
        "/books.kabacus",
        get(move || {
            let bytes = source.clone();
            async move { ([("etag", "\"storage-revision-7\"")], bytes) }
        })
        .put(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                StatusCode::PRECONDITION_FAILED
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let host = Host::WebDav {
        url: format!("http://{address}/books.kabacus").parse().unwrap(),
        http: reqwest::Client::new(),
        token: None,
    };
    let session = Session::open(config(), host, dir.path()).await.unwrap();
    let recovery = session.recovery_path().to_path_buf();
    let app = router(session);
    let gateway = Gateway::Embedded(app.clone());
    let result = gateway
        .call(
            "declare_account",
            json!({
                "account": "assets:bank:checking", "head": "1", "dry_run": false
            }),
        )
        .await
        .unwrap();
    assert_eq!(result["error"]["data"]["details"]["committed_locally"], true);
    assert_eq!(uploads.load(Ordering::SeqCst), 1);
    let pending = kabacus_store::file::read(&recovery.join("pending.kabacus")).unwrap();
    assert_eq!(pending[0].revision, 2);
    let response = app
        .oneshot(
            axum::http::Request::get("/version")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(uploads.load(Ordering::SeqCst), 1);
    server.abort();
}
