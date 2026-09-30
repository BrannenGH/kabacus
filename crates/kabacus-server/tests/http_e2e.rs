// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! End-to-end HTTP tests against the real server on scratch books,
//! driven the way the app or kabacus-client would — over the unix
//! socket, in no-auth mode.

mod support;

use http_body_util::BodyExt;
use hyper_util::client::legacy::Client as HyperClient;
use hyperlocal::UnixClientExt;
use kabacus_server::config::{AuthConfig, Config};
use kabacus_testkit::TestServer;
use serde_json::{json, Value};

async fn spawn_server() -> TestServer {
    spawn_server_with(|_| {}).await
}

async fn spawn_server_with(customize_auth: impl FnOnce(&mut AuthConfig)) -> TestServer {
    spawn_server_with_config(|cfg| customize_auth(&mut cfg.auth)).await
}

async fn spawn_server_with_config(customize: impl FnOnce(&mut Config)) -> TestServer {
    kabacus_testkit::spawn_server(customize).await
}

trait Http {
    async fn request(
        &self,
        method: &str,
        path: &str,
        if_match: Option<&str>,
        body: Option<Value>,
    ) -> (u16, Value, http::HeaderMap);
    async fn head(&self) -> String;
}

impl Http for TestServer {
    async fn request(
        &self,
        method: &str,
        path: &str,
        if_match: Option<&str>,
        body: Option<Value>,
    ) -> (u16, Value, http::HeaderMap) {
        let client = HyperClient::unix();
        let uri: hyper::Uri = hyperlocal::Uri::new(&self.socket_path, path).into();
        let mut builder = hyper::Request::builder().method(method).uri(uri);
        if let Some(h) = if_match {
            builder = builder.header("if-match", h);
        }
        let body_bytes = body
            .map(|v| serde_json::to_vec(&v).unwrap())
            .unwrap_or_default();
        if !body_bytes.is_empty() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(http_body_util::Full::new(bytes::Bytes::from(body_bytes)))
            .unwrap();
        let response = client.request(request).await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or(Value::Null)
        };
        (status, json, headers)
    }

    async fn head(&self) -> String {
        support::store_head(self).await
    }
}
#[tokio::test]
async fn version_is_unauthenticated_and_reports_auth_mode() {
    let server = spawn_server().await;
    let (status, body, _) = server.request("GET", "/version", None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["auth"]["mode"], "none");
}

/// The running commit and where its source is (AGPL §13).
#[tokio::test]
async fn version_names_the_running_commit_and_its_source() {
    let server = spawn_server().await;
    let (_, body, _) = server.request("GET", "/version", None, None).await;
    let commit = body["commit"].as_str().unwrap();
    assert!(
        commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "{commit}"
    );
    let url = body["source_url"].as_str().unwrap();
    assert!(url.starts_with("https://"), "{url}");
    assert!(url.ends_with(&format!("/tree/{commit}")), "{url}");
}

/// `books` moves on every write, and only then.
#[tokio::test]
async fn version_books_changes_on_a_write() {
    let server = spawn_server().await;
    let books = |body: &Value| body["books"].as_str().unwrap().to_string();
    let (_, before, _) = server.request("GET", "/version", None, None).await;
    let (_, again, _) = server.request("GET", "/version", None, None).await;
    assert_eq!(books(&before), books(&again), "nothing changed");
    assert_eq!(before["store"], "sqlite");
    assert!(before["schema_version"].as_u64().unwrap() >= 2);
    assert_eq!(before["engine_version"], kabacus_ledger::VERSION);

    let head = server.head().await;
    let (status, _, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    assert_eq!(status, 201);
    let (_, written, _) = server.request("GET", "/version", None, None).await;
    assert_ne!(before["head"], written["head"]);
    assert_ne!(books(&before), books(&written));
}
#[tokio::test]
async fn adds_a_transaction_over_http_and_it_shows_up_in_the_register() {
    let server = spawn_server().await;
    let head = server.head().await;

    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head),
            Some(json!({
                "date": "2026-02-01",
                "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    assert_eq!(status, 201, "{body:?}");
    assert!(body["head"].as_str().unwrap() != head);

    let (status, txs, etag_headers) = server.request("GET", "/transactions", None, None).await;
    assert_eq!(status, 200);
    assert_eq!(txs.as_array().unwrap().len(), 2);
    assert!(etag_headers.get("etag").is_some());
}
/// HTTP entity tags are quoted (RFC 9110 §8.8.3): the ETag is sent quoted,
/// and a client that echoes it back in If-Match, as HTTP clients do, is
/// accepted. The bare head the app sends still works.
#[tokio::test]
async fn the_etag_is_quoted_and_a_quoted_if_match_is_accepted() {
    let server = spawn_server().await;
    let head = server.head().await;
    let (status, _, headers) = server.request("GET", "/transactions", None, None).await;
    assert_eq!(status, 200);
    let etag = headers["etag"].to_str().unwrap().to_string();
    assert_eq!(etag, format!("\"{head}\""));

    let groceries = |description: &str| {
        json!({
            "date": "2026-02-01", "description": description,
            "postings": [
                {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 1845, "places": 2}},
                {"account": "assets:checking"}
            ]
        })
    };
    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&etag),
            Some(groceries("Corner Market")),
        )
        .await;
    assert_eq!(status, 201, "{body:?}");
    let head = server.head().await;
    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head),
            Some(groceries("Fresh Thyme Market")),
        )
        .await;
    assert_eq!(status, 201, "{body:?}");
    // A weak tag never matches (If-Match compares strongly).
    let head = server.head().await;
    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&format!("W/\"{head}\"")),
            Some(groceries("Aldi")),
        )
        .await;
    assert_eq!(status, 409, "{body:?}");
}

#[tokio::test]
async fn stale_if_match_returns_409() {
    let server = spawn_server().await;
    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some("0000000000000000000000000000000000000000"),
            Some(json!({
                "date": "2026-02-01", "description": "x",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 100, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    assert_eq!(status, 409, "{body:?}");
}
#[tokio::test]
async fn undeclared_account_returns_422_with_structured_error() {
    let server = spawn_server().await;
    let head = server.head().await;
    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head),
            Some(json!({
                "date": "2026-02-01", "description": "x",
                "postings": [
                    {"account": "expenses:nonexistent", "amount": {"commodity": "USD", "mantissa": 100, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    assert_eq!(status, 422, "{body:?}");
    assert!(body.get("rule").is_some());
    assert!(body.get("message").is_some());
}

/// One amount grammar: more places than `commodity USD1000.00` takes, and
/// a mantissa beyond i64 written as a JSON number, are refused. Nothing is
/// committed by a refusal.
#[tokio::test]
async fn finer_amounts_and_json_numbers_beyond_i64_are_refused() {
    let server = spawn_server().await;
    let head = server.head().await;
    let add = |amount: Value| {
        json!({
            "date": "2026-02-01", "description": "Grocery Store",
            "postings": [
                {"account": "expenses:groceries", "amount": amount},
                {"account": "assets:checking"}
            ]
        })
    };
    let (_, body, _) = server.request("GET", "/commodity", None, None).await;
    assert_eq!(body, json!({"symbol": "USD", "places": 2}));

    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head),
            Some(add(json!("12.345"))),
        )
        .await;
    assert_eq!(status, 422, "{body:?}");
    assert_eq!(body["rule"], "amount", "{body:?}");
    assert_eq!(
        body["message"],
        "USD12.345 on expenses:groceries has more decimal places than USD takes (2)",
        "{body:?}"
    );
    assert_eq!(server.head().await, head);
    let (_, commodity, _) = server.request("GET", "/commodity", None, None).await;
    assert_eq!(commodity["places"], 2);

    let huge: Value = serde_json::from_str(
        r#"{"commodity": "USD", "mantissa": 99999999999999999999, "places": 0}"#,
    )
    .unwrap();
    let (status, body, _) = server
        .request("POST", "/transactions", Some(&head), Some(add(huge)))
        .await;
    assert_eq!(status, 422, "{body:?}");
    assert_eq!(body["code"], "amount_format", "{body:?}");
    assert_eq!(
        body["message"], "write a mantissa beyond 64 bits as a string of digits",
        "{body:?}"
    );
    assert_eq!(server.head().await, head, "nothing committed");

    let (status, body, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head),
            Some(add(json!("$1,234.56"))),
        )
        .await;
    assert_eq!(status, 201, "the same grammar as the app: {body:?}");
}
#[tokio::test]
async fn a_correction_edits_the_transaction_in_place() {
    let server = spawn_server().await;
    let head0 = server.head().await;
    let (_, added, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    let id = added["id"].as_u64().unwrap().to_string();
    let head1 = added["head"].as_str().unwrap().to_string();

    let (status, corrected, _) = server
        .request(
            "POST",
            &format!("/transactions/{id}/corrections"),
            Some(&head1),
            Some(json!({
                "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2500, "places": 2}},
                    {"account": "assets:checking", "amount": {"commodity": "USD", "mantissa": -2500, "places": 2}}
                ]
            })),
        )
        .await;
    assert_eq!(status, 201, "{corrected:?}");
    assert_eq!(corrected["id"], added["id"]);

    // Corrected in place: the same transaction, same date, new amounts.
    let (_, txs, _) = server.request("GET", "/transactions", None, None).await;
    let txs = txs.as_array().unwrap();
    assert_eq!(txs.len(), 2);
    let grocery = txs.iter().find(|t| t["id"] == added["id"]).unwrap();
    assert_eq!(grocery["date"], "2026-02-01");
    assert_eq!(grocery["postings"][0]["amount"]["exact"], "25.00");
}
#[tokio::test]
async fn patches_status_and_it_is_reflected_in_the_register() {
    let server = spawn_server().await;
    let head0 = server.head().await;
    let (_, added, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    let id = added["id"].as_u64().unwrap();
    let head1 = added["head"].as_str().unwrap().to_string();

    let (status, result, _) = server
        .request(
            "PATCH",
            &format!("/transactions/{id}/status"),
            Some(&head1),
            Some(json!({"account": "assets:checking", "status": "cleared"})),
        )
        .await;
    assert_eq!(status, 200, "{result:?}");

    let (_, txs, _) = server.request("GET", "/transactions", None, None).await;
    let tx = txs
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    let checking_posting = tx["postings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["account"] == "assets:checking")
        .unwrap();
    assert_eq!(checking_posting["status"], "cleared");
}
#[tokio::test]
async fn counterparties_and_statement_endpoints_work() {
    let server = spawn_server().await;
    let head0 = server.head().await;
    let (_, added, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-03-01", "description": "Jordan | dinner",
                "postings": [
                    {"account": "assets:receivable:jordan", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    assert!(added["head"].as_str().is_some(), "{added:?}");

    let (status, counterparties, _) = server.request("GET", "/counterparties", None, None).await;
    assert_eq!(status, 200);
    let list = counterparties.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "jordan");
    assert_eq!(list[0]["label"], "Owes you $20.00");

    let (status, stmt, _) = server
        .request(
            "GET",
            "/counterparties/jordan/statement?from=2026-01-01&to=2026-12-31",
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{stmt:?}");
    assert_eq!(stmt["closing_balance"]["exact"], "20.00");
}
#[tokio::test]
async fn no_read_role_returns_403() {
    // `no_auth_roles: []` in no-auth mode means the single local principal
    // has none of Kabacus's three roles at all.
    let server =
        spawn_server_with(|auth| auth.no_auth_roles = vec!["ledger-write".to_string()]).await;
    let (status, body, _) = server.request("GET", "/transactions", None, None).await;
    assert_eq!(status, 403, "{body:?}");
    assert_eq!(body["role"], "ledger-read");
}
#[tokio::test]
async fn history_lists_commits_with_actor_and_kind() {
    let server = spawn_server().await;
    let head0 = server.head().await;
    server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-02-01", "description": "x",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 100, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;

    let (status, history, _) = server.request("GET", "/history", None, None).await;
    assert_eq!(status, 200);
    let entries = history.as_array().unwrap();
    assert_eq!(entries[0]["kind"], "add");
    assert_eq!(entries[0]["actor"], "unknown"); // no Kabacus-Client-Hint header sent
}
#[tokio::test]
async fn accounts_lists_declared_accounts_with_balances() {
    let server = spawn_server().await;
    let head0 = server.head().await;
    server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;

    let (status, accounts, _) = server.request("GET", "/accounts", None, None).await;
    assert_eq!(status, 200);
    let list = accounts.as_array().unwrap();
    let checking = list
        .iter()
        .find(|a| a["account"] == "assets:checking")
        .unwrap();
    assert_eq!(checking["balances"][0]["exact"], "980.00");

    let (status, filtered, _) = server
        .request("GET", "/accounts?filter=grocer", None, None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(filtered.as_array().unwrap().len(), 1);
}
#[tokio::test]
async fn register_shows_running_balance_and_other_account() {
    let server = spawn_server().await;
    let head0 = server.head().await;
    server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;

    let (status, register, _) = server
        .request("GET", "/register/assets:checking", None, None)
        .await;
    assert_eq!(status, 200, "{register:?}");
    let lines = register.as_array().unwrap();
    assert_eq!(lines.len(), 2); // Opening + Grocery Store
    let last = &lines[1];
    assert_eq!(last["other_account"], "expenses:groceries");
    assert_eq!(last["balance"]["exact"], "980.00");
}
#[tokio::test]
async fn rewrite_is_refused_when_the_server_bans_it_even_with_the_role() {
    // --no-rewrite (allow_rewrite = false): the role alone isn't enough
    // (gate 1).
    let server = spawn_server_with_config(|cfg| {
        cfg.allow_rewrite = false;
        support::rewrites(cfg);
    })
    .await;
    let head = server.head().await;
    let (status, body, _) = server
        .request(
            "PUT",
            "/transactions/1",
            Some(&head),
            Some(json!({"date": "2026-01-01", "description": "x", "postings": []})),
        )
        .await;
    assert_eq!(status, 403, "{body:?}");
}
#[tokio::test]
async fn rewrite_is_refused_without_the_role_by_default() {
    // Rewrites are on by default, but the caller has no ledger-rewrite role
    // (gate 2).
    let server = spawn_server().await;
    let head = server.head().await;
    let (status, body, _) = server
        .request(
            "PUT",
            "/transactions/1",
            Some(&head),
            Some(json!({"date": "2026-01-01", "description": "x", "postings": []})),
        )
        .await;
    assert_eq!(status, 403, "{body:?}");
}
#[tokio::test]
async fn an_assertion_is_removed_only_through_the_gated_rewrite() {
    let roles = vec![
        "ledger-read".to_string(),
        "ledger-write".to_string(),
        "ledger-rewrite".to_string(),
    ];
    let body = json!({"transaction": 1, "account": "assets:checking"});

    let banned = spawn_server_with_config(|cfg| {
        cfg.allow_rewrite = false;
        cfg.auth.no_auth_roles = roles.clone();
    })
    .await;
    let head = banned.head().await;
    let (status, _, _) = banned
        .request("PUT", "/assertions", Some(&head), Some(body.clone()))
        .await;
    assert_eq!(status, 403);

    let server = spawn_server_with_config(|cfg| cfg.auth.no_auth_roles = roles).await;
    let head = server.head().await;
    // The fixture's opening posting has no assertion yet: add one, then
    // remove it again.
    let mut set = body.clone();
    set["balance"] = json!({"commodity": "USD", "mantissa": 100000, "places": 2});
    let (status, done, _) = server
        .request("PUT", "/assertions", Some(&head), Some(set))
        .await;
    assert_eq!(status, 200, "{done:?}");

    let head = server.head().await;
    let mut dry = body.clone();
    dry["dry_run"] = json!(true);
    let (status, preview, _) = server
        .request("PUT", "/assertions", Some(&head), Some(dry))
        .await;
    assert_eq!(status, 200, "{preview:?}");
    assert!(
        preview["diff"]
            .as_str()
            .unwrap()
            .contains("+    assets:checking  USD1000.00\n"),
        "{preview:?}"
    );
}
#[tokio::test]
async fn rewrite_dry_runs_then_commits_in_one_call() {
    let server = spawn_server_with_config(support::rewrites).await;
    let head0 = server.head().await;
    let (_, added, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    let id = added["id"].as_u64().unwrap().to_string();
    let head1 = added["head"].as_str().unwrap().to_string();

    let rewrite = |dry_run: bool| {
        json!({
            "date": "2026-02-01", "description": "Corrected In Place",
            "postings": [
                {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 3000, "places": 2}},
                {"account": "assets:checking", "amount": {"commodity": "USD", "mantissa": -3000, "places": 2}}
            ],
            "dry_run": dry_run,
        })
    };
    let (status, preview, _) = server
        .request(
            "PUT",
            &format!("/transactions/{id}"),
            Some(&head1),
            Some(rewrite(true)),
        )
        .await;
    assert_eq!(status, 200, "{preview:?}");
    assert_eq!(preview["head"], head1.as_str());
    assert!(preview["diff"]
        .as_str()
        .unwrap()
        .contains("Corrected In Place"));

    // The original text must still be there: a dry run commits nothing.
    let (_, txs, _) = server.request("GET", "/transactions", None, None).await;
    assert!(txs
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["payee"] == "Grocery Store"));

    let (status, committed, _) = server
        .request(
            "PUT",
            &format!("/transactions/{id}"),
            Some(&head1),
            Some(rewrite(false)),
        )
        .await;
    assert_eq!(status, 200, "{committed:?}");

    let (_, txs, _) = server.request("GET", "/transactions", None, None).await;
    let txs = txs.as_array().unwrap();
    assert!(
        !txs.iter().any(|t| t["payee"] == "Grocery Store"),
        "the original text must be gone after a rewrite"
    );
    assert!(txs.iter().any(|t| t["payee"] == "Corrected In Place"));

    let (_, history, _) = server.request("GET", "/history", None, None).await;
    assert_eq!(history[0]["kind"], "rewrite");
}
#[tokio::test]
async fn delete_removes_the_transaction() {
    let server = spawn_server_with_config(support::rewrites).await;
    let head0 = server.head().await;
    let (_, added, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&head0),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    let id = added["id"].as_u64().unwrap().to_string();
    let head1 = added["head"].as_str().unwrap().to_string();

    let (status, preview, _) = server
        .request(
            "DELETE",
            &format!("/transactions/{id}"),
            Some(&head1),
            Some(json!({"dry_run": true})),
        )
        .await;
    assert_eq!(status, 200, "{preview:?}");
    assert_eq!(server.head().await, head1, "a dry run commits nothing");

    let (status, result, _) = server
        .request(
            "DELETE",
            &format!("/transactions/{id}"),
            Some(&head1),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 200, "{result:?}");

    let (_, txs, _) = server.request("GET", "/transactions", None, None).await;
    assert!(!txs
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["payee"] == "Grocery Store"));

    let (_, history, _) = server.request("GET", "/history", None, None).await;
    assert_eq!(history[0]["kind"], "delete");
}

async fn spawn_rewrite_server() -> TestServer {
    spawn_server_with_config(support::rewrites).await
}
#[tokio::test]
async fn declaring_an_account_is_a_rewrite() {
    let server = spawn_rewrite_server().await;
    let head = server.head().await;
    let body = json!({
        "account": "expenses:pets",
        "tags": [["ynab-envelope", "Pets"]],
    });

    let mut dry = body.clone();
    dry["dry_run"] = json!(true);
    let (status, preview, _) = server
        .request("POST", "/accounts", Some(&head), Some(dry))
        .await;
    assert_eq!(status, 200, "{preview:?}");
    let diff = preview["diff"].as_str().unwrap();
    assert!(
        diff.contains("+account expenses:pets  ; ynab-envelope:Pets"),
        "{diff}"
    );
    assert_eq!(server.head().await, head, "a dry run commits nothing");

    let (status, committed, _) = server
        .request("POST", "/accounts", Some(&head), Some(body))
        .await;
    assert_eq!(status, 200, "{committed:?}");

    let (_, accounts, _) = server.request("GET", "/accounts", None, None).await;
    let pets = accounts
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["account"] == "expenses:pets")
        .expect("declared account is listed");
    assert_eq!(pets["friendly_name"], "Pets");
    let (_, history, _) = server.request("GET", "/history", None, None).await;
    assert_eq!(history[0]["kind"], "rewrite");
    assert!(history[0]["summary"]
        .as_str()
        .unwrap()
        .contains("declare account: expenses:pets"));
}
#[tokio::test]
async fn declaring_an_account_needs_the_rewrite_gates() {
    // Default server: rewrites on, but no ledger-rewrite role.
    let server = spawn_server().await;
    let head = server.head().await;
    let (status, body, _) = server
        .request(
            "POST",
            "/accounts",
            Some(&head),
            Some(json!({ "account": "expenses:pets" })),
        )
        .await;
    assert_eq!(status, 403, "{body:?}");
}
#[tokio::test]
async fn declaring_rejects_bad_or_duplicate_accounts() {
    let server = spawn_rewrite_server().await;
    let head = server.head().await;
    for account in [
        "assets:checking",
        "",
        "expenses::gap",
        "expenses:a  b",
        "expenses:x ; y",
    ] {
        let (status, body, _) = server
            .request(
                "POST",
                "/accounts",
                Some(&head),
                Some(json!({ "account": account })),
            )
            .await;
        assert_eq!(status, 422, "{account:?}: {body:?}");
    }
}
/// A revision is a number; anything else is refused before the store is
/// asked.
#[tokio::test]
async fn a_revision_that_isnt_a_number_is_refused() {
    let server = spawn_server().await;
    for path in [
        "/history/--output=pwned.txt".to_string(),
        "/history/--output=pwned.txt/transactions".to_string(),
    ] {
        let (status, body, _) = server.request("GET", &path, None, None).await;
        assert_eq!(status, 422, "{path}: {body:?}");
    }
}

/// Every change is one revision that says who made it, and any
/// earlier state of the books can be listed, diffed and read back.
#[tokio::test]
async fn every_state_of_the_books_is_versioned_and_inspectable() {
    let server = spawn_server().await;
    let initial = server.head().await;
    let (_, history0, _) = server.request("GET", "/history", None, None).await;
    let commits0 = history0.as_array().unwrap().len();

    let (status, added, _) = server
        .request(
            "POST",
            "/transactions",
            Some(&initial),
            Some(json!({
                "date": "2026-02-01", "description": "Grocery Store",
                "postings": [
                    {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
                    {"account": "assets:checking"}
                ]
            })),
        )
        .await;
    assert_eq!(status, 201, "{added:?}");
    let after_add = added["head"].as_str().unwrap().to_string();

    // Exactly one new revision, attributed.
    let (_, history, _) = server.request("GET", "/history", None, None).await;
    let history = history.as_array().unwrap();
    assert_eq!(history.len(), commits0 + 1);
    assert_eq!(history[0]["hash"], after_add.as_str());
    // This raw request sends no client hint; the actor is still recorded.
    assert_eq!(history[0]["actor"], "unknown");
    assert_eq!(history[0]["kind"], "add");

    // Its diff shows the change.
    let client = HyperClient::unix();
    let uri: hyper::Uri =
        hyperlocal::Uri::new(&server.socket_path, &format!("/history/{after_add}")).into();
    let response = client
        .request(
            hyper::Request::get(uri)
                .body(http_body_util::Full::new(bytes::Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    let diff = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&diff).contains("+2026-02-01 Grocery Store"));

    // The books as they were before and after.
    let payees = |body: &Value| -> Vec<String> {
        body.as_array()
            .unwrap()
            .iter()
            .map(|t| t["payee"].as_str().unwrap().to_string())
            .collect()
    };
    let (status, before, _) = server
        .request(
            "GET",
            &format!("/history/{initial}/transactions"),
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{before:?}");
    assert_eq!(payees(&before), ["Opening"]);
    let (_, after, _) = server
        .request(
            "GET",
            &format!("/history/{after_add}/transactions"),
            None,
            None,
        )
        .await;
    assert_eq!(payees(&after), ["Opening", "Grocery Store"]);
}
#[tokio::test]
async fn an_unknown_revision_is_not_found() {
    let server = spawn_server().await;
    let missing = "9000";
    for path in [
        format!("/history/{missing}"),
        format!("/history/{missing}/transactions"),
    ] {
        let (status, body, _) = server.request("GET", &path, None, None).await;
        assert_eq!(status, 404, "{path}: {body:?}");
    }
}
