// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: AGPL-3.0-or-later

use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use kabacus_server::{
    api,
    config::{AuthConfig, AuthMode, Config},
    state::AppState,
};
use kabacus_store::{AnyStore, SqliteStore};
use std::sync::Arc;
use tower::ServiceExt;

async fn app(web_dir: Option<std::path::PathBuf>) -> axum::Router {
    let config = Config {
        web_dir,
        auth: AuthConfig {
            mode: AuthMode::None,
            ..Default::default()
        },
        ..Default::default()
    };
    let store = AnyStore::Sqlite(SqliteStore::in_memory().unwrap());
    api::router(Arc::new(AppState::with_store(config, store).await.unwrap()))
}

#[tokio::test]
async fn hosts_assets_without_changing_api_routes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("index.html"), "<title>Kabacus</title>").unwrap();
    std::fs::write(dir.path().join("assets/app.js"), "export {};").unwrap();
    std::fs::write(dir.path().join("assets/de.json"), "{}").unwrap();
    std::fs::write(dir.path().join("private.kabacus"), "not an asset").unwrap();
    let app = app(Some(dir.path().to_path_buf())).await;
    for (path, status, mime) in [
        ("/app/", 200, Some("text/html; charset=utf-8")),
        (
            "/app/assets/app.js",
            200,
            Some("text/javascript; charset=utf-8"),
        ),
        (
            "/app/assets/de.json",
            200,
            Some("application/json; charset=utf-8"),
        ),
        ("/app/missing.js", 404, None),
        ("/app/private.kabacus", 404, None),
        ("/app/assets/../index.html", 404, None),
        ("/transactions/missing", 404, None),
        ("/version", 200, None),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status, "{path}");
        if let Some(mime) = mime {
            assert_eq!(response.headers()["content-type"], mime);
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert!(!response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty());
        }
    }
}

#[tokio::test]
async fn web_hosting_is_opt_in() {
    let response = app(None)
        .await
        .oneshot(Request::builder().uri("/app/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_symlinks_outside_the_bundle() {
    let bundle = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.js"), "private").unwrap();
    std::os::unix::fs::symlink(outside.path(), bundle.path().join("assets")).unwrap();
    let response = app(Some(bundle.path().to_path_buf()))
        .await
        .oneshot(
            Request::builder()
                .uri("/app/assets/secret.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}
