// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's model against a real kabacus-server on synthetic books:
//! the MVP loop of sidebar → register → add → updated register.

use kabacus_client::KabacusClient;
use kabacus_kde::model::{self, SimpleDraft, ALL_TRANSACTIONS};
use kabacus_testkit::spawn_server;

fn groceries(amount: &str) -> SimpleDraft {
    SimpleDraft {
        date: "2026-02-03".to_string(),
        payee: "Corner Market".to_string(),
        memo: "milk".to_string(),
        from_account: "assets:checking".to_string(),
        to_account: "expenses:groceries".to_string(),
        amount: amount.to_string(),
    }
}
#[tokio::test]
async fn sidebar_lists_all_transactions_then_accounts_with_names_and_balances() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);

    let entries = model::sidebar(&client).await.unwrap();
    assert_eq!(entries[0].account, ALL_TRANSACTIONS);
    assert_eq!(entries[0].name, "All transactions");
    let checking = entries
        .iter()
        .find(|e| e.account == "assets:checking")
        .unwrap();
    assert_eq!(checking.name, "Everyday Checking");
    assert!(
        checking.balance.contains("1,000.00") || checking.balance.contains("1000.00"),
        "balance was {:?}",
        checking.balance
    );
}
#[tokio::test]
async fn adding_a_simple_transaction_shows_in_both_registers_from_each_side() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);

    let id = model::add_simple(&client, &groceries("45.67"))
        .await
        .unwrap();
    assert!(!id.is_empty());

    let checking = model::register(&client, "assets:checking").await.unwrap();
    let row = &checking[0];
    assert_eq!(row.id, id);
    assert_eq!(row.payee, "Corner Market");
    assert_eq!(row.account, "expenses:groceries");
    assert!(row.outflow.contains("45.67"), "{row:?}");
    assert!(row.inflow.is_empty());
    assert!(row.balance.contains("954.33"), "{row:?}");

    let envelope = model::register(&client, "expenses:groceries")
        .await
        .unwrap();
    assert_eq!(envelope[0].account, "assets:checking");
    assert!(envelope[0].inflow.contains("45.67"));

    let all = model::register(&client, ALL_TRANSACTIONS).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, id, "newest first");
}
#[tokio::test]
async fn invalid_drafts_are_refused_before_reaching_the_server() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let head_before = client.version().await.unwrap().head;

    for bad in [
        SimpleDraft {
            amount: "abc".into(),
            ..groceries("")
        },
        SimpleDraft {
            amount: "0".into(),
            ..groceries("")
        },
        SimpleDraft {
            date: "Feb 3".into(),
            ..groceries("1")
        },
        SimpleDraft {
            payee: " ".into(),
            ..groceries("1")
        },
        SimpleDraft {
            to_account: "assets:checking".into(),
            ..groceries("1")
        },
    ] {
        assert!(model::add_simple(&client, &bad).await.is_err(), "{bad:?}");
    }
    assert_eq!(client.version().await.unwrap().head, head_before);
}
#[tokio::test]
async fn editing_changes_date_payee_category_and_amount_in_place() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let id = model::add_simple(&client, &groceries("45.67"))
        .await
        .unwrap();

    let mut draft = model::edit_draft(&client, &id).await.unwrap();
    assert_eq!(draft.date, "2026-02-03");
    assert_eq!(draft.payee, "Corner Market");
    assert_eq!(draft.memo, "milk");
    assert_eq!(draft.from_account, "assets:checking");
    assert_eq!(draft.to_account, "expenses:groceries");
    assert_eq!(draft.amount, "45.67");

    draft.date = "2026-01-30".into();
    draft.payee = "Hardware Store".into();
    draft.to_account = "expenses:household".into();
    draft.amount = "12.50".into();
    model::save_edit(&client, &id, &draft).await.unwrap();

    let all = model::register(&client, ALL_TRANSACTIONS).await.unwrap();
    assert_eq!(all.len(), 2, "edited in place, nothing appended: {all:?}");
    let checking = model::register(&client, "assets:checking").await.unwrap();
    let row = checking.iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.date, "2026-01-30");
    assert_eq!(row.payee, "Hardware Store");
    assert_eq!(row.account, "expenses:household");
    assert!(row.outflow.contains("12.50"), "{row:?}");
    assert!(model::register(&client, "expenses:groceries")
        .await
        .unwrap()
        .is_empty());
}
#[tokio::test]
async fn invalid_edits_leave_the_books_unchanged() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let id = model::add_simple(&client, &groceries("45.67"))
        .await
        .unwrap();
    let head = client.version().await.unwrap().head;

    let mut draft = model::edit_draft(&client, &id).await.unwrap();
    draft.to_account = "expenses:undeclared".into();
    assert!(model::save_edit(&client, &id, &draft).await.is_err());
    draft.to_account = "expenses:household".into();
    draft.amount = "-3".into();
    assert!(model::save_edit(&client, &id, &draft).await.is_err());
    assert_eq!(client.version().await.unwrap().head, head);
}

/// Another client's in-place edit shows up in the app, attributed to it.
#[tokio::test]
async fn another_clients_edit_in_place_is_seen_and_attributed() {
    let server = spawn_server(|_| {}).await;
    let app = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    let id = model::add_simple(&app, &groceries("45.67")).await.unwrap();
    let draft = model::edit_draft(&app, &id).await.unwrap();

    let other = KabacusClient::connect_unix(&server.socket_path).with_client_hint("script");
    model::save_edit(
        &other,
        &id,
        &SimpleDraft {
            amount: "1.00".into(),
            ..draft
        },
    )
    .await
    .unwrap();

    let edited = model::edit_draft(&app, &id).await.unwrap();
    assert_eq!(edited.amount, "1.00");
    let last = &app.history().await.unwrap()[0];
    assert_eq!(last.actor.as_deref(), Some("script"));
    assert_eq!(last.kind.as_deref(), Some("edit"));
}
#[tokio::test]
async fn all_transactions_reads_from_source_to_destination_whatever_the_posting_order() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let id = model::add_simple(&client, &groceries("45.67"))
        .await
        .unwrap();
    let all = model::register(&client, ALL_TRANSACTIONS).await.unwrap();
    let row = all.iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.account, "assets:checking → expenses:groceries");
    // The opening balance is written checking-first; it still reads from
    // equity to checking.
    assert_eq!(
        all.last().unwrap().account,
        "equity:opening → assets:checking"
    );

    // Changing To keeps the direction, and the journal's posting order.
    let order_before: Vec<String> = posting_order(&client, &id).await;
    let mut draft = model::edit_draft(&client, &id).await.unwrap();
    draft.to_account = "expenses:household".into();
    model::save_edit(&client, &id, &draft).await.unwrap();
    let all = model::register(&client, ALL_TRANSACTIONS).await.unwrap();
    let row = all.iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.account, "assets:checking → expenses:household");
    let order_after = posting_order(&client, &id).await;
    assert_eq!(
        order_before.iter().position(|a| a == "assets:checking"),
        order_after.iter().position(|a| a == "assets:checking"),
        "posting order kept: {order_before:?} → {order_after:?}"
    );
}

async fn posting_order(client: &KabacusClient, id: &str) -> Vec<String> {
    let txs = client.list_transactions().await.unwrap();
    let tx = txs.iter().find(|t| t.id.to_string() == id).unwrap();
    tx.postings.iter().map(|p| p.account.clone()).collect()
}
