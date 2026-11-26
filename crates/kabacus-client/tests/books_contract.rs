// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use kabacus_client::{Books, BooksRequest, KabacusClient, LocalBooks};
use kabacus_service::local::LocalCaller;
use kabacus_store::{AnyStore, SqliteStore};
use serde_json::{json, Value};

const IMPORT_ACCOUNTS: &str = "account assets:bank:checking\n";

async fn request(
    books: &impl Books,
    method: &str,
    path: &str,
    head: Option<&str>,
    body: Value,
) -> (u16, Value) {
    let response = books
        .request(BooksRequest {
            method: method.into(),
            path: path.into(),
            head: head.map(str::to_string),
            body,
        })
        .await
        .unwrap();
    (
        response.status,
        serde_json::from_slice(&response.body).unwrap(),
    )
}

async fn contract(books: &impl Books) {
    let (_, v) = request(books, "GET", "/version", None, Value::Null).await;
    let head = v["head"].as_str().unwrap();
    let body = json!({"date": "2026-02-01", "description": "Fresh Thyme Market", "postings": [
        {"account": "expenses:groceries", "amount": "64.12"}, {"account": "assets:checking"}
    ], "dry_run": true});
    let (_, preview) = request(books, "POST", "/transactions", Some(head), body.clone()).await;
    assert_eq!(preview["head"], head);
    let (_, unchanged) = request(books, "GET", "/version", None, Value::Null).await;
    assert_eq!(unchanged["head"], head);
    let mut save = body.clone();
    save["dry_run"] = json!(false);
    let (status, saved) = request(books, "POST", "/transactions", Some(head), save.clone()).await;
    assert!(status < 300, "{saved}");
    assert_eq!(
        saved["head"].as_str().unwrap().parse::<i64>().unwrap(),
        head.parse::<i64>().unwrap() + 1
    );
    let (status, conflict) = request(books, "POST", "/transactions", Some(head), save).await;
    assert_eq!(status, 409);
    assert_eq!(conflict["code"], "head_conflict");
    assert_eq!(conflict["details"]["expected"], head);
    assert_eq!(conflict["details"]["current_head"], saved["head"]);
    let (_, rows) = request(books, "GET", "/transactions", None, Value::Null).await;
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[1]["postings"][1]["amount"]["exact"], "-64.12");
    let (_, history) = request(books, "GET", "/history", None, Value::Null).await;
    assert_eq!(history[0]["actor"], "app");
    let (_, report) = request(
        books,
        "GET",
        "/reports/balance-sheet?at=2026-02-01",
        None,
        Value::Null,
    )
    .await;
    assert!(report.is_object());
}

async fn statement_import_contract(books: &impl Books) {
    let (_, version) = request(books, "GET", "/version", None, Value::Null).await;
    let head = version["head"].as_str().unwrap();
    let path = "/accounts/assets%3Abank%3Achecking/import";
    let mut import = json!({
        "format": "csv",
        "content": "Date,Payee,Amount,ID\n2026-02-03,Harbor Street Cafe,-18.40,HC0261\n2026-02-04,Linden Corner Market,-62.18,HC0262\n",
        "mapping": {"date": 0, "description": [1], "amount": 2, "id": 3},
        "limit": 1,
    });
    let mut conflicting = import.clone();
    conflicting["content"] = json!(import["content"]
        .as_str()
        .unwrap()
        .replace("HC0262", "HC0261"));
    for dry_run in [true, false] {
        conflicting["dry_run"] = json!(dry_run);
        let (status, rejected) =
            request(books, "POST", path, Some(head), conflicting.clone()).await;
        assert_eq!(status, 422, "{rejected}");
        assert_eq!(rejected["code"], "statement_import");
        let (_, unchanged) = request(books, "GET", "/version", None, Value::Null).await;
        assert_eq!(unchanged["head"], head);
    }
    let (status, preview) = request(books, "POST", path, Some(head), import.clone()).await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["counts"]["new"], 2);
    assert_eq!(preview["rows"].as_array().unwrap().len(), 1);
    assert!(preview["rows"][0].get("transaction").is_none());
    import["cursor"] = preview["next_cursor"].clone();
    let (_, second) = request(books, "POST", path, Some(head), import.clone()).await;
    assert_eq!(second["rows"][0]["import_id"], "csv:HC0262");
    import["cursor"] = Value::Null;
    import["dry_run"] = json!(false);
    let (status, committed) = request(books, "POST", path, Some(head), import.clone()).await;
    assert_eq!(status, 200, "{committed}");
    let saved = committed["head"].as_str().unwrap();
    assert_eq!(
        saved.parse::<u64>().unwrap(),
        head.parse::<u64>().unwrap() + 1
    );
    assert!(committed["rows"][0]["transaction"].is_number());
    let (status, conflict) = request(books, "POST", path, Some(head), import.clone()).await;
    assert_eq!(status, 409, "{conflict}");
    let (_, repeated) = request(books, "POST", path, Some(saved), import).await;
    assert_eq!(repeated["head"], saved);
    assert_eq!(repeated["counts"]["duplicate"], 2);
    assert_eq!(repeated["rows"], json!([]));
}

#[tokio::test]
async fn local_and_remote_books_share_previews_revisions_errors_and_reports() {
    let store = AnyStore::Sqlite(SqliteStore::in_memory().unwrap());
    kabacus_testkit::import_journal(
        &store,
        None,
        &kabacus_testkit::scratch_journal(IMPORT_ACCOUNTS),
    )
    .await;
    let books = LocalBooks::from_store(store, LocalCaller::new("Dana Okafor", "app"))
        .await
        .unwrap();
    contract(&books).await;
    let (_, history) = request(&books, "GET", "/history", None, Value::Null).await;
    assert_eq!(history[0]["user"], "Dana Okafor");
    let server = kabacus_testkit::spawn_server_with_journal(IMPORT_ACCOUNTS, |_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    contract(&client).await;
    statement_import_contract(&books).await;
    statement_import_contract(&client).await;
}

#[tokio::test]
async fn local_files_reopen_and_copies_keep_history_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Household.kabacus");
    let books = LocalBooks::create(
        &path,
        "Household",
        "USD",
        "assets:bank:checking",
        LocalCaller::new("Dana Okafor", "cli"),
    )
    .await
    .unwrap();
    let (_, before) = request(&books, "GET", "/version", None, Value::Null).await;
    let copy = dir.path().join("Household copy.kabacus");
    books.save_copy(&copy).await.unwrap();
    let reopened = LocalBooks::open(copy, books.caller.clone()).await.unwrap();
    let (_, after) = request(&reopened, "GET", "/version", None, Value::Null).await;
    assert_eq!(before["head"], after["head"]);
    assert!(LocalBooks::create(
        &path,
        "Household",
        "USD",
        "assets:bank:checking",
        books.caller.clone()
    )
    .await
    .is_err());
}

#[tokio::test]
async fn opening_an_empty_file_refuses_it_without_creating_books() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Empty.kabacus");
    std::fs::write(&path, []).unwrap();
    assert!(
        LocalBooks::open(&path, LocalCaller::new("Dana Okafor", "app"))
            .await
            .is_err()
    );
    assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
}

#[tokio::test]
async fn save_copy_never_replaces_an_existing_file() {
    let folder = tempfile::tempdir().unwrap();
    let books = LocalBooks::create(
        folder.path().join("Household.kabacus"),
        "Cedar Household",
        "USD",
        "assets:checking",
        LocalCaller::for_app("Dana Okafor"),
    )
    .await
    .unwrap();
    let copy = folder.path().join("Saved copy.kabacus");
    std::fs::write(&copy, b"A previously saved document").unwrap();
    assert!(books.save_copy(&copy).await.is_err());
    assert_eq!(std::fs::read(copy).unwrap(), b"A previously saved document");
}

async fn wallet_contract(books: &impl Books) {
    let (_, info) = request(books, "GET", "/commodity", None, Value::Null).await;
    assert_eq!(info["symbol"], "USD");
    let exact = |currency: &str, mantissa: &str, places: u32| json!({"commodity":currency,"mantissa":mantissa,"places":places});
    let checking = "assets:bank:checking";
    let ethereum = "assets:investments:ethereum";
    let monero = "assets:investments:monero";
    for (day, from, debit, to, credit, price, platform, broker) in [
        (
            "2026-02-01",
            checking,
            exact("USD", "-102500", 2),
            ethereum,
            exact("ETH", "4", 1),
            exact("USD", "100000", 2),
            exact("USD", "1500", 2),
            exact("USD", "1000", 2),
        ),
        (
            "2026-02-02",
            ethereum,
            exact("ETH", "-1025", 4),
            monero,
            exact("XMR", "15", 1),
            exact("ETH", "1", 1),
            exact("ETH", "15", 4),
            exact("ETH", "1", 3),
        ),
        (
            "2026-02-03",
            monero,
            exact("XMR", "-103", 2),
            checking,
            exact("USD", "16000", 2),
            exact("XMR", "1", 0),
            exact("XMR", "2", 2),
            exact("XMR", "1", 2),
        ),
    ] {
        let (_, v) = request(books, "GET", "/version", None, Value::Null).await;
        let head = v["head"].as_str().unwrap();
        let mut body = json!({"date":day,"description":"Cedar Exchange | Wallet transfer", "dry_run":true, "postings":[
            {"account":from,"amount":debit}, {"account":to,"amount":credit,"cost":{"kind":"total","amount":price}},
            {"account":"expenses:fees:platform","amount":platform}, {"account":"expenses:fees:broker","amount":broker}
        ]});
        let (status, preview) =
            request(books, "POST", "/transactions", Some(head), body.clone()).await;
        assert!((200..300).contains(&status), "{preview}");
        assert_eq!(preview["head"], head);
        body["dry_run"] = json!(false);
        let (status, saved) = request(books, "POST", "/transactions", Some(head), body).await;
        assert!((200..300).contains(&status), "{saved}");
        let path = format!("/transactions/{}", saved["id"]);
        let (_, tx) = request(books, "GET", &path, None, Value::Null).await;
        assert_eq!(tx["postings"][1]["cost"]["kind"], "total");
        let mut postings = tx["postings"].clone();
        for p in postings.as_array_mut().unwrap() {
            let a = &p["amount"];
            p["amount"] = exact(
                a["commodity"].as_str().unwrap(),
                &a["exact"].as_str().unwrap().replace('.', ""),
                a["exact"]
                    .as_str()
                    .unwrap()
                    .split('.')
                    .nth(1)
                    .map_or(0, |f| f.len() as u32),
            );
            p.as_object_mut().unwrap().remove("cost");
        }
        let (status, edit) = request(books,"PATCH",&path,saved["head"].as_str(),json!({"date":day,"description":"Cedar Exchange | Fees checked","postings":postings,"dry_run":false})).await;
        assert!((200..300).contains(&status), "{edit}");
        let (_, edited) = request(books, "GET", &path, None, Value::Null).await;
        assert_eq!(edited["postings"], tx["postings"]);
    }
    for (account, currency, expected) in [
        (checking, "USD", "4353.46"),
        (ethereum, "ETH", "1.532067890123456789"),
        (monero, "XMR", "12.815678901234"),
    ] {
        let path = format!("/register/{account}");
        let (status, lines) = request(books, "GET", &path, None, Value::Null).await;
        assert!((200..300).contains(&status), "{lines}");
        let balance = &lines.as_array().unwrap().last().unwrap()["balance"];
        assert_eq!(balance["commodity"], currency);
        assert_eq!(balance["exact"], expected);
        let (status, checked) = request(
            books,
            "POST",
            "/reconcile",
            None,
            json!({"account":account,"date":"2026-02-28","balance":expected,"mode":"check"}),
        )
        .await;
        assert!((200..300).contains(&status), "{checked}");
        assert_eq!(checked["status"], "matched");
    }
    for (currency, total) in [("USD", "25.00"), ("ETH", "0.0025"), ("XMR", "0.03")] {
        let path = format!(
            "/reports/categories?from=2026-02-01&to=2026-02-28&depth=3&commodity={currency}"
        );
        let (status, fees) = request(books, "GET", &path, None, Value::Null).await;
        assert!((200..300).contains(&status), "{fees}");
        assert_eq!(fees["commodity"], currency);
        assert_eq!(
            fees["total"]
                .as_str()
                .unwrap()
                .parse::<kabacus_core::Money>()
                .unwrap(),
            total.parse::<kabacus_core::Money>().unwrap()
        );
        assert_eq!(fees["rows"].as_array().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn wallet_transfers_fees_and_reconciliation_agree_in_local_and_remote_books() {
    let store = AnyStore::Sqlite(SqliteStore::in_memory().unwrap());
    kabacus_testkit::import_journal(
        &store,
        None,
        &kabacus_testkit::scratch_journal(kabacus_testkit::WALLETS),
    )
    .await;
    let books = LocalBooks::from_store(store, LocalCaller::new("Dana Okafor", "app"))
        .await
        .unwrap();
    wallet_contract(&books).await;
    let server = kabacus_testkit::spawn_server_with_journal(kabacus_testkit::WALLETS, |_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    wallet_contract(&client).await;
}
