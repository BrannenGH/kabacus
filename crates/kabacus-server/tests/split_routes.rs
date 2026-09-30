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
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
#[tokio::test]
async fn shared_expense_nets_debt_and_is_previewed_without_a_write() {
    let store = AnyStore::Sqlite(SqliteStore::in_memory().unwrap());
    kabacus_testkit::import_journal(&store,None,&kabacus_testkit::scratch_journal("2026-01-02 Jordan paid for supplies\n    expenses:household  12.13 USD\n    liabilities:payable:jordan  -12.13 USD\n").replace("assets:checking", "assets:bank:checking")).await;
    let state = Arc::new(
        AppState::with_store(
            Config {
                auth: AuthConfig {
                    mode: AuthMode::None,
                    ..Default::default()
                },
                ..Default::default()
            },
            store,
        )
        .await
        .unwrap(),
    );
    let app = api::router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let version: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let body = json!({"date":"2026-01-03","description":"Cub Foods shared groceries","paid_with":"assets:bank:checking","category":"expenses:groceries","person":"jordan","total":"84.17","their_share":"42.08","dry_run":true});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/splits")
                .header("content-type", "application/json")
                .header("if-match", version["head"].as_str().unwrap())
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let result: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(result["head"], version["head"]);
    assert_eq!(result["dry_run"], true);
    let text = result["after"].as_str().unwrap();
    assert!(text.contains("29.95"), "{text}");
    assert!(text.contains("12.13"), "{text}");
    assert!(text.contains("42.09"), "{text}");
    assert_eq!(result["cash_on_hand"]["before"]["exact"], "1000.00");
    assert_eq!(result["cash_on_hand"]["after"]["exact"], "915.83");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let unchanged: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(unchanged["head"], version["head"]);
    let mut commit = body;
    commit["dry_run"] = json!(false);
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/splits")
            .header("content-type", "application/json")
            .header("if-match", version["head"].as_str().unwrap())
            .body(Body::from(commit.to_string()))
            .unwrap()
    };
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), 201);
    let saved: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(saved["id"], result["id"]);
    assert_ne!(saved["head"], version["head"]);
    // A repeated confirmation cannot create a second expense at the old revision.
    assert_eq!(app.oneshot(request()).await.unwrap().status(), 409);
}
