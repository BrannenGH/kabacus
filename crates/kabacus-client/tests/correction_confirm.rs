// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Before a correction is sent, the dry run gives the
//! confirmation dialog everything it shows: the transaction exactly as
//! it'll be written, and each affected account's balance before and
//! after. Nothing is written until the real call.

use kabacus_client::types::{AddTransactionBody, DraftAmount, DraftPostingBody, WriteResultView};
use kabacus_client::KabacusClient;
use kabacus_testkit::spawn_server;

fn posting(account: &str, cents: Option<i128>) -> DraftPostingBody {
    DraftPostingBody {
        account: account.to_string(),
        amount: cents.map(|mantissa| DraftAmount {
            commodity: "USD".to_string(),
            mantissa: mantissa.into(),
            places: 2,
        }),
        status: String::new(),
        balance_assertion: None,
        clear_balance_assertion: false,
    }
}

fn balance(result: &WriteResultView, account: &str) -> (String, String) {
    let change = result
        .balances
        .iter()
        .find(|b| b.account == account)
        .unwrap_or_else(|| panic!("{account} missing from {:?}", result.balances));
    (change.before.exact.clone(), change.after.exact.clone())
}
#[tokio::test]
async fn correction_dry_run_shows_both_transactions_and_balance_changes() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    let head = client.version().await.unwrap().head.unwrap();
    let added = client
        .add_transaction(
            &head,
            AddTransactionBody {
                new_accounts: vec![],
                date: "2026-02-01".to_string(),
                description: "Grocery Store".to_string(),
                postings: vec![
                    posting("expenses:groceries", Some(2000)),
                    posting("assets:checking", None),
                ],
                tags: vec![],
                dry_run: false,
            },
        )
        .await
        .unwrap();

    let edited = vec![
        posting("expenses:household", Some(3000)),
        posting("assets:checking", None),
    ];
    let preview = client
        .correct_transaction(
            added.id.unwrap(),
            &added.head,
            "Grocery Store".into(),
            edited.clone(),
            true,
        )
        .await
        .unwrap();

    // Nothing written yet.
    assert_eq!(preview.head, added.head);
    assert_eq!(client.version().await.unwrap().head.unwrap(), added.head);

    // The transaction as it'll be: corrected in place.
    assert_eq!(preview.id, added.id);
    assert!(
        preview
            .preview
            .contains("+    expenses:household  USD30.00"),
        "{}",
        preview.preview
    );

    // Every affected balance, before and after.
    assert_eq!(
        balance(&preview, "assets:checking"),
        ("980.00".into(), "970.00".into())
    );
    assert_eq!(
        balance(&preview, "expenses:groceries"),
        ("20.00".into(), "0.00".into())
    );
    assert_eq!(
        balance(&preview, "expenses:household"),
        ("0.00".into(), "30.00".into())
    );

    // Then the real call.
    let committed = client
        .correct_transaction(
            added.id.unwrap(),
            &added.head,
            "Grocery Store".into(),
            edited,
            false,
        )
        .await
        .unwrap();
    assert_ne!(committed.head, added.head);
}
#[tokio::test]
async fn a_void_dry_run_shows_the_balances_it_restores() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    let head = client.version().await.unwrap().head.unwrap();
    let added = client
        .add_transaction(
            &head,
            AddTransactionBody {
                new_accounts: vec![],
                date: "2026-02-01".to_string(),
                description: "Grocery Store".to_string(),
                postings: vec![
                    posting("expenses:groceries", Some(2000)),
                    posting("assets:checking", None),
                ],
                tags: vec![],
                dry_run: false,
            },
        )
        .await
        .unwrap();
    let preview = client
        .void_transaction(added.id.unwrap(), &added.head, true)
        .await
        .unwrap();
    assert_eq!(
        balance(&preview, "assets:checking"),
        ("980.00".into(), "1000.00".into())
    );
    assert_eq!(
        balance(&preview, "expenses:groceries"),
        ("20.00".into(), "0.00".into())
    );
}
