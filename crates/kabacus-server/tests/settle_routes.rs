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
async fn settling_up_pays_down_what_you_owe_then_builds_what_they_owe() {
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
    let version = |app: axum::Router| async move {
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        v["head"].as_str().unwrap().to_string()
    };
    let settle = |head: String, direction: &str, amount: &str| {
        Request::builder()
            .method("POST")
            .uri("/settlements")
            .header("content-type", "application/json")
            .header("if-match", head)
            .body(Body::from(
                json!({"date":"2026-01-03","description":"Settled up with Jordan","account":"assets:bank:checking","person":"jordan","amount":amount,"direction":direction,"dry_run":true}).to_string(),
            ))
            .unwrap()
    };
    let head = version(app.clone()).await;
    let response = app
        .clone()
        .oneshot(settle(head.clone(), "paid", "20.00"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let result: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let text = result["after"].as_str().unwrap();
    assert!(text.contains("USD12.13"), "{text}");
    assert!(text.contains("USD7.87"), "{text}");
    assert!(text.contains("-USD20.00"), "{text}");
    assert_eq!(result["cash_on_hand"]["after"]["exact"], "980.00");
    // They paid us 5.00: the debt we owe them grows, nothing is owed to us yet.
    let response = app.oneshot(settle(head, "received", "5.00")).await.unwrap();
    assert_eq!(response.status(), 200);
    let result: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let text = result["after"].as_str().unwrap();
    assert!(text.contains("liabilities:payable:jordan"), "{text}");
    assert_eq!(result["cash_on_hand"]["after"]["exact"], "1005.00");
}
