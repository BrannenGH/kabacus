// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The books are read once for each state they're in and kept in memory
//! (`cache.rs`): reads of unchanged books don't read them again, and a
//! write, this server's or another's on the same store, is seen on the
//! very next read.

mod support;

use kabacus_server::config::Database;
use kabacus_server::ops;
use kabacus_store::{BookStore, ChangeSet, SqliteStore, Who};
use kabacus_testkit::spawn_server_with_journal;
use serde_json::json;
use support::*;

fn has_id(txs: &[kabacus_core::Transaction], id: u64) -> bool {
    txs.iter().any(|t| t.id == id)
}

#[tokio::test]
async fn unchanged_books_are_read_once() {
    let server = spawn_server_with_journal("", |_| {}).await;
    let state = &server.state;
    let first = ops::read_all_transactions(state).await.unwrap();
    let declared = state.declared_accounts().await.unwrap();
    let loads = state.books_cache.loads();
    for _ in 0..5 {
        assert_eq!(ops::read_all_transactions(state).await.unwrap(), first);
        assert_eq!(state.declared_accounts().await.unwrap(), declared);
    }
    assert_eq!(state.books_cache.loads(), loads);
}

/// Another server writing to the same store: each of its writes is seen
/// by this server's very next read, and is never answered from memory.
#[tokio::test]
async fn another_servers_write_is_seen_by_the_very_next_read() {
    let server = spawn_server_with_journal("", |_| {}).await;
    let state = &server.state;
    let Database::Sqlite(path) = &state.config.store else {
        unreachable!("the test server keeps its books in a file")
    };
    let other = SqliteStore::open(path).unwrap();
    let ledger = state.ledger;
    let user = other.local_user("Marisol Vega").await.unwrap();
    for i in 1..=25u32 {
        ops::read_all_transactions(state).await.unwrap();
        let loaded = other.load(ledger, None).await.unwrap();
        let mut after = loaded.book.clone();
        let mut tx = after.transactions[0].clone();
        tx.id = loaded.next_id;
        tx.date = chrono::NaiveDate::from_ymd_opt(2026, 3, i).unwrap();
        tx.description = format!("Corner Market {i}");
        after.transactions.push(tx);
        let who = Who {
            user,
            actor: "other".to_string(),
            kind: "add".to_string(),
            summary: format!("add: Corner Market {i}"),
            details: json!({}),
        };
        other
            .commit(
                ledger,
                loaded.revision,
                ChangeSet::between(&loaded.book, &after),
                &who,
            )
            .await
            .unwrap();
        let txs = ops::read_all_transactions(state).await.unwrap();
        assert!(has_id(&txs, loaded.next_id), "write {i} not seen");
    }
}

#[tokio::test]
async fn a_write_is_seen_by_the_next_read() {
    let server = spawn_server_with_journal("", |_| {}).await;
    let client = connect(&server).await;
    let state = &server.state;
    ops::read_all_transactions(state).await.unwrap();
    let (result, _) = commit(
        &client,
        "add_transaction",
        json!({
            "head": head(&client).await,
            "date": "2026-02-03",
            "description": "Fresh Thyme Market",
            "postings": [
                {"account": "expenses:groceries", "amount": "64.12"},
                {"account": "assets:checking", "amount": "-64.12"}
            ]
        }),
    )
    .await;
    let id = result["id"].as_u64().unwrap();
    let txs = ops::read_all_transactions(state).await.unwrap();
    assert!(has_id(&txs, id), "{result}");
}

/// About ten years of a household's books: pay every other week split
/// into funds, rent from its fund, and groceries and dining most days.
fn ten_years() -> String {
    let mut out = String::from(
        "account assets:bank:checking   ; ynab-account:Checking\n\
         account assets:bank:checking:rent\n\
         account assets:bank:checking:vacation\n\
         account income:salary\n\
         account expenses:rent\n\
         account expenses:dining\n\n\
         2016-01-01 Opening\n    assets:bank:checking  4200.00 USD\n    equity:opening\n\n",
    );
    let shops = [
        "Fresh Thyme Market",
        "Aldi",
        "Kroger",
        "Blue Door Pub",
        "Lucky Dragon",
    ];
    let start = chrono::NaiveDate::from_ymd_opt(2016, 1, 1).unwrap();
    for day in 0..3650u32 {
        let date = start + chrono::Duration::days(day.into());
        if day % 14 == 0 {
            out.push_str(&format!(
                "{date} Northwind Logistics payroll\n    assets:bank:checking  2480.00 USD\n    income:salary\n\n\
                 {date} Allocate paycheck\n    assets:bank:checking:rent  825.00 USD\n    assets:bank:checking:vacation  120.00 USD\n    assets:bank:checking\n\n"
            ));
        }
        if day % 28 == 3 {
            out.push_str(&format!(
                "{date} Maple Court Apartments\n    expenses:rent  1650.00 USD\n    assets:bank:checking:rent\n\n"
            ));
        }
        let shop = shops[(day % 5) as usize];
        let category = if day % 5 < 3 { "groceries" } else { "dining" };
        let cents = 800 + (day * 3727) % 9000;
        out.push_str(&format!(
            "{date} {shop}\n    expenses:{category}  {}.{:02} USD\n    assets:bank:checking\n\n",
            cents / 100,
            cents % 100
        ));
    }
    out
}

/// Once the books are read, Cash on Hand comes from memory: repeated
/// requests on unchanged books never load them again.
#[tokio::test]
async fn cash_on_hand_is_served_from_memory_once_the_books_are_read() {
    let server = spawn_server_with_journal(&ten_years(), |_| {}).await;
    let client = connect(&server).await;
    let first = call(&client, "cash_on_hand", json!({})).await;
    let loads = server.state.books_cache.loads();
    for _ in 0..5 {
        assert_eq!(call(&client, "cash_on_hand", json!({})).await, first);
    }
    assert_eq!(server.state.books_cache.loads(), loads, "read again");
}
