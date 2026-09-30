// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Through `/mcp`: structured search with paging, Cash on Hand's
//! summary, the paged register, list_accounts' filters, and unreserved
//! detection over correction chains.

mod support;

use kabacus_testkit::spawn_server_with_journal;
use serde_json::{json, Value};
use std::collections::HashSet;
use support::*;

const CARDS: &str = "account assets:bank:checking\n\
    account liabilities:credit-card:visa\n\
    account assets:bank:checking:card-visa\n\
    account liabilities:credit-card:amex\n\
    account assets:bank:checking:card-amex\n\
    account expenses:dining\n\
    2026-01-02 Opening bank\n    assets:bank:checking  USD5000.00\n    equity:opening\n";

fn purchase(i: usize, card: &str, reserved: bool, tags: Value) -> Value {
    let mut postings = vec![
        json!({"account": "expenses:groceries", "amount": format!("{}.00", 1 + i % 50)}),
        json!({"account": format!("liabilities:credit-card:{card}"), "amount": format!("-{}.00", 1 + i % 50)}),
    ];
    if reserved {
        postings.push(
            json!({"account": "assets:bank:checking", "amount": format!("-{}.00", 1 + i % 50)}),
        );
        postings.push(json!({"account": format!("assets:bank:checking:card-{card}"), "amount": format!("{}.00", 1 + i % 50)}));
    }
    json!({"tool": "add_transaction", "args": {
        "date": format!("2026-03-{:02}", 1 + i % 28),
        "description": format!("Store {i} | aisle {}", i % 7),
        "postings": postings,
        "tags": tags,
    }})
}

/// 60 purchases tagged `reconciled`, 40 more not, some reserved.
async fn spawn_shop() -> (kabacus_testkit::TestServer, Client) {
    let server = spawn_server_with_journal(CARDS, |_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let ops: Vec<Value> = (0..100)
        .map(|i| {
            let tags = if i < 60 {
                json!([{"key": "reconciled", "value": "2026-03-31"}])
            } else {
                json!([])
            };
            purchase(
                i,
                if i % 2 == 0 { "visa" } else { "amex" },
                i % 3 == 0,
                tags,
            )
        })
        .collect();
    call(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "ops": ops}),
    )
    .await;
    (server, client)
}

/// A page of 25 tagged rows and a cursor that continues without gaps or
/// repeats; count_only; fields; sort.
#[tokio::test]
async fn search_pages_through_without_gaps_or_repeats() {
    let (_server, client) = spawn_shop().await;
    let mut seen = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut args = json!({"tag": {"key": "reconciled"}, "limit": 25});
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let page = call(&client, "search_transactions", args).await;
        assert_eq!(page["total_count"], 60);
        let rows = page["transactions"].as_array().unwrap();
        pages += 1;
        if pages == 1 {
            assert_eq!(rows.len(), 25);
            assert_eq!(page["truncated"], true);
        }
        for row in rows {
            assert!(
                seen.insert(row["id"].as_u64().unwrap().to_string()),
                "repeated {row}"
            );
        }
        match page["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => {
                assert_eq!(page["truncated"], false);
                break;
            }
        }
    }
    assert_eq!((pages, seen.len()), (3, 60));

    let count = call(
        &client,
        "search_transactions",
        json!({"tag": [{"key": "reconciled", "value": "2026-03-31"}], "count_only": true}),
    )
    .await;
    assert_eq!(count, json!({"total_count": 60}));
    let projected = call(
        &client,
        "search_transactions",
        json!({"limit": 2, "fields": ["id", "date", "cash_effect"], "sort": "date_desc"}),
    )
    .await;
    let row = &projected["transactions"][0];
    let keys: Vec<&String> = row.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 3, "{row}");
    assert!(row["cash_effect"]["exact"].is_string());
    assert_eq!(row["date"], "2026-03-28", "newest first");
}

/// Filters combine with AND.
#[tokio::test]
async fn search_filters_combine() {
    let (_server, client) = spawn_shop().await;
    let count = |args: Value| {
        let client = &client;
        async move {
            let mut args = args;
            args["count_only"] = json!(true);
            call(client, "search_transactions", args).await["total_count"]
                .as_u64()
                .unwrap()
        }
    };
    assert_eq!(
        count(json!({"account": "liabilities:credit-card:visa"})).await,
        50
    );
    assert_eq!(
        count(json!({"account": "liabilities:credit-card"})).await,
        100
    );
    assert_eq!(
        count(json!({"account": ["liabilities:credit-card:visa", "assets:bank:checking"]})).await,
        17
    );
    assert_eq!(count(json!({"kind": "debt-reserve"})).await, 34);
    // Reserved purchases move free money; the rest don't.
    assert_eq!(
        count(json!({"cash_effect": "nonzero", "kind": "revolving-debt"})).await,
        34
    );
    assert_eq!(
        count(json!({"cash_effect": "zero", "kind": "revolving-debt"})).await,
        66
    );
    assert_eq!(
        count(json!({"amount_min": "49.00", "amount_max": "50.00"})).await,
        4
    );
    assert_eq!(count(json!({"payee": "store 1"})).await, 11);
    assert_eq!(count(json!({"payee": "/^Store 1$/"})).await, 1);
    assert_eq!(count(json!({"memo": "aisle 3"})).await, 14);
    assert_eq!(
        count(json!({"has_tag": ["reconciled"], "lacks_tag": "void"})).await,
        60
    );
    assert_eq!(count(json!({"status": "cleared"})).await, 0);
    assert_eq!(
        count(json!({"status": "unmarked", "payee": "Store 99"})).await,
        1
    );
    let error = fail(&client, "search_transactions", json!({"payee": "/(/"})).await;
    assert_eq!(error["code"], "validation_failed");
}

/// Cash on Hand's summary has counts but no lists; full lists the first
/// few per card with a cursor that search_transactions continues; the
/// counts agree with unreserved_only; compare_to adds each change.
#[tokio::test]
async fn cash_on_hand_is_a_summary_by_default() {
    let (_server, client) = spawn_shop().await;
    let summary = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(summary["detail"], "summary");
    assert!(summary.to_string().len() < 50_000);
    let visa = summary["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["card"] == "liabilities:credit-card:visa")
        .unwrap();
    assert!(visa.get("unreserved").is_none());
    let unreserved: u64 = summary["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["unreserved_count"].as_u64().unwrap())
        .sum();
    assert_eq!(unreserved, 66);
    let counted = call(
        &client,
        "search_transactions",
        json!({"unreserved_only": true, "count_only": true}),
    )
    .await;
    assert_eq!(counted["total_count"], unreserved);

    let full = call(
        &client,
        "cash_on_hand",
        json!({"detail": "full", "unreserved_limit": 5}),
    )
    .await;
    let visa = full["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["card"] == "liabilities:credit-card:visa")
        .unwrap()
        .clone();
    assert_eq!(visa["unreserved"].as_array().unwrap().len(), 5);
    let rest = call(
        &client,
        "search_transactions",
        json!({"unreserved_only": true, "account": "liabilities:credit-card:visa",
               "cursor": visa["next_cursor"], "limit": 1}),
    )
    .await;
    let listed: Vec<&Value> = visa["unreserved"].as_array().unwrap().iter().collect();
    let first_of_rest = &rest["transactions"][0];
    assert!(listed.iter().all(|u| u["id"] != first_of_rest["id"]));

    let compared = call(
        &client,
        "cash_on_hand",
        json!({"as_of": "2026-03-31", "compare_to": "2026-02-28"}),
    )
    .await;
    let change = &compared["change"];
    assert_eq!(change["compare_to"], "2026-02-28");
    // Reserved purchases moved free money into the reserves.
    assert_eq!(
        exact(&change["cash_on_hand"]).parse::<f64>().unwrap(),
        -exact(&change["reserves"]).parse::<f64>().unwrap()
    );
}

/// A purchase on a card, corrected in place with no reserve lines either:
/// it is judged as it now is, and the card's difference is exactly its
/// missing amount.
#[tokio::test]
async fn a_corrected_purchase_is_judged_as_it_now_is() {
    let server = spawn_server_with_journal(CARDS, |_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let mut args = purchase(9, "visa", false, json!([]))["args"].clone();
    args["head"] = json!(head);
    let (added, head) = commit(&client, "add_transaction", args).await;
    let id = added["id"].as_u64().unwrap();
    let (corrected, _) = commit(
        &client,
        "correct_transaction",
        json!({"id": id, "description": "Store 9 | fixed", "head": head, "today": "2026-04-01",
        "postings": [
            {"account": "expenses:groceries", "amount": "12.00"},
            {"account": "liabilities:credit-card:visa", "amount": "-12.00"}
        ]}),
    )
    .await;
    assert_eq!(corrected["id"], id);
    let report = call(&client, "cash_on_hand", json!({"detail": "full"})).await;
    let visa = report["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["card"] == "liabilities:credit-card:visa")
        .unwrap()
        .clone();
    assert_eq!(visa["unreserved_count"], 1, "{visa}");
    assert_eq!(visa["unreserved"][0]["id"], id);
    assert_eq!(exact(&visa["difference"]), "12.00");
    assert_eq!(exact(&visa["unreserved"][0]["missing_reserve"]), "12.00");
    assert!(report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|w| w["code"] != "unreserved-unaccounted"));
    let found = call(
        &client,
        "search_transactions",
        json!({"unreserved_only": true}),
    )
    .await;
    assert_eq!(found["total_count"], 1);
    assert_eq!(found["transactions"][0]["id"], id);
}

/// The register pages; list_accounts filters by kind, depth and zero
/// balances and projects fields.
#[tokio::test]
async fn register_and_accounts_are_filtered_and_paged() {
    let (_server, client) = spawn_shop().await;
    let page = call(
        &client,
        "get_register",
        json!({"account": "liabilities:credit-card:visa", "limit": 20}),
    )
    .await;
    assert_eq!(page["total_count"], 50);
    assert_eq!(page["lines"].as_array().unwrap().len(), 20);
    let next = call(
        &client,
        "get_register",
        json!({"account": "liabilities:credit-card:visa", "limit": 40, "cursor": page["next_cursor"]}),
    )
    .await;
    assert_eq!(next["lines"].as_array().unwrap().len(), 30);
    assert_eq!(next["truncated"], false);
    // The running balance carries on across pages.
    let last = &page["lines"][19]["balance"];
    let first = &next["lines"][0];
    let expected =
        exact(last).parse::<f64>().unwrap() + exact(&first["amount"]).parse::<f64>().unwrap();
    assert!((exact(&first["balance"]).parse::<f64>().unwrap() - expected).abs() < 1e-9);

    let cards = call(
        &client,
        "list_accounts",
        json!({"kind": "revolving-debt", "fields": ["account", "kind"]}),
    )
    .await;
    let cards = cards.as_array().unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].as_object().unwrap().len(), 2);
    let shallow = call(&client, "list_accounts", json!({"depth": 1})).await;
    assert!(shallow
        .as_array()
        .unwrap()
        .iter()
        .all(|a| !a["account"].as_str().unwrap().contains(':')));
    let all = call(&client, "list_accounts", json!({})).await;
    let nonzero = call(&client, "list_accounts", json!({"include_zero": false})).await;
    assert!(nonzero.as_array().unwrap().len() < all.as_array().unwrap().len());
    assert!(nonzero
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["account"] != "expenses:dining"));
}
