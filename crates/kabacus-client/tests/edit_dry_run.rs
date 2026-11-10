// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! In-place edits can be previewed: a dry run validates the
//! candidate on a scratch copy and returns the diff without committing, so
//! an MCP client can show it before the real call.

use kabacus_client::types::{AddTransactionBody, DraftAmount, DraftPostingBody};
use kabacus_client::{ClientError, KabacusClient};
use kabacus_testkit::spawn_server;

fn body(date: &str, expense: &str, cents: i128, dry_run: bool) -> AddTransactionBody {
    AddTransactionBody {
        new_accounts: vec![],
        date: date.to_string(),
        description: "Corner Market".to_string(),
        postings: vec![
            DraftPostingBody {
                account: expense.to_string(),
                amount: Some(DraftAmount::from_money(
                    "USD",
                    kabacus_core::Money::from_decimal(cents, 2),
                )),
                status: String::new(),
                balance_assertion: None,
                clear_balance_assertion: false,
            },
            DraftPostingBody {
                account: "assets:checking".to_string(),
                amount: None,
                status: String::new(),
                balance_assertion: None,
                clear_balance_assertion: false,
            },
        ],
        tags: vec![],
        dry_run,
    }
}

async fn head(client: &KabacusClient) -> String {
    client.version().await.unwrap().head.unwrap()
}
#[tokio::test]
async fn a_dry_run_edit_shows_the_diff_and_commits_nothing() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let added = client
        .add_transaction(
            &head(&client).await,
            body("2026-02-03", "expenses:groceries", 4567, false),
        )
        .await
        .unwrap();
    let id = added.id.unwrap();

    let preview = client
        .edit_transaction(
            id,
            &added.head,
            body("2026-02-03", "expenses:groceries", 999, true),
        )
        .await
        .unwrap();
    assert!(
        preview.preview.contains("-    expenses:groceries"),
        "{}",
        preview.preview
    );
    assert!(preview.preview.contains("9.99"), "{}", preview.preview);
    assert_eq!(head(&client).await, added.head, "nothing was committed");

    // The same edit for real.
    client
        .edit_transaction(
            id,
            &added.head,
            body("2026-02-03", "expenses:groceries", 999, false),
        )
        .await
        .unwrap();
    assert_ne!(head(&client).await, added.head);
}
#[tokio::test]
async fn a_dry_run_edit_is_validated_like_the_real_one() {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let added = client
        .add_transaction(
            &head(&client).await,
            body("2026-02-03", "expenses:groceries", 4567, false),
        )
        .await
        .unwrap();
    let id = added.id.unwrap();

    let err = client
        .edit_transaction(
            id,
            &added.head,
            body("2026-02-03", "expenses:undeclared", 999, true),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Invalid { .. }), "{err:?}");
    assert_eq!(head(&client).await, added.head);
}
