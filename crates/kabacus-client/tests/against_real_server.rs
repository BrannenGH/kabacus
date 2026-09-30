// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Proves `kabacus-client` and `kabacus-server`'s actual HTTP contract
//! agree, by running a real server (in-process, on scratch books) and
//! driving it only through the client.

use kabacus_client::types::{AddTransactionBody, DraftAmount, DraftPostingBody};
use kabacus_client::KabacusClient;
use kabacus_testkit::TestServer;

async fn spawn_server() -> (TestServer, KabacusClient) {
    let server = kabacus_testkit::spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(server.socket_path.clone()).with_client_hint("app");
    (server, client)
}
#[tokio::test]
async fn version_round_trips() {
    let (_dir, client) = spawn_server().await;
    let v = client.version().await.unwrap();
    assert_eq!(v.auth.mode, "none");
    assert!(v.head.is_some());
    assert_eq!(v.commit.len(), 40);
    assert!(v.source_url.ends_with(&v.commit), "{}", v.source_url);
}
#[tokio::test]
async fn add_transaction_and_list_round_trips_exact_money() {
    let (_dir, client) = spawn_server().await;
    let head = client.version().await.unwrap().head.unwrap();

    let body = AddTransactionBody {
        date: "2026-02-01".to_string(),
        description: "Grocery Store".to_string(),
        postings: vec![
            DraftPostingBody {
                account: "expenses:groceries".to_string(),
                amount: Some(DraftAmount::from_money(
                    "USD",
                    kabacus_core::Money::from_decimal(4568, 2),
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
        dry_run: false,
    };
    let result = client.add_transaction(&head, body).await.unwrap();
    assert_ne!(result.head, head);

    let txs = client.list_transactions().await.unwrap();
    let added = txs.iter().find(|t| Some(t.id) == result.id).unwrap();
    let checking = added
        .postings
        .iter()
        .find(|p| p.account == "assets:checking")
        .unwrap();
    assert_eq!(checking.amount.money().unwrap().to_string(), "-45.68");
}
#[tokio::test]
async fn stale_if_match_surfaces_as_conflict_error() {
    let (_dir, client) = spawn_server().await;
    let body = AddTransactionBody {
        date: "2026-02-01".to_string(),
        description: "x".to_string(),
        postings: vec![
            DraftPostingBody {
                account: "expenses:groceries".to_string(),
                amount: Some(DraftAmount::from_money(
                    "USD",
                    kabacus_core::Money::from_decimal(100, 2),
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
        dry_run: false,
    };
    let result = client
        .add_transaction("0000000000000000000000000000000000000000", body)
        .await;
    assert!(matches!(
        result,
        Err(kabacus_client::ClientError::Conflict { .. })
    ));
}
#[tokio::test]
async fn correct_and_void_round_trip() {
    let (_dir, client) = spawn_server().await;
    let head = client.version().await.unwrap().head.unwrap();

    let add_body = AddTransactionBody {
        date: "2026-02-01".to_string(),
        description: "Grocery Store".to_string(),
        postings: vec![
            DraftPostingBody {
                account: "expenses:groceries".to_string(),
                amount: Some(DraftAmount::from_money(
                    "USD",
                    kabacus_core::Money::from_decimal(2000, 2),
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
        dry_run: false,
    };
    let added = client.add_transaction(&head, add_body).await.unwrap();

    let id = added.id.unwrap();
    let corrected = client
        .correct_transaction(
            id,
            &added.head,
            "Grocery Store".to_string(),
            vec![
                DraftPostingBody {
                    account: "expenses:groceries".to_string(),
                    amount: Some(DraftAmount::from_money(
                        "USD",
                        kabacus_core::Money::from_decimal(2500, 2),
                    )),
                    status: String::new(),
                    balance_assertion: None,
                    clear_balance_assertion: false,
                },
                DraftPostingBody {
                    account: "assets:checking".to_string(),
                    amount: Some(DraftAmount::from_money(
                        "USD",
                        kabacus_core::Money::from_decimal(-2500, 2),
                    )),
                    status: String::new(),
                    balance_assertion: None,
                    clear_balance_assertion: false,
                },
            ],
            false,
        )
        .await
        .unwrap();

    // Corrected in place: the same transaction with the new amount.
    assert_eq!(corrected.id, Some(id));
    let tx = client.get_transaction(id).await.unwrap();
    assert_eq!(tx.postings[0].amount.money().unwrap().to_string(), "25.00");

    // A void adds the reversal; the original stays.
    let voided = client
        .void_transaction(id, &corrected.head, false)
        .await
        .unwrap();
    let reversal = voided.id.unwrap();
    assert_ne!(reversal, id);
    let txs = client.list_transactions().await.unwrap();
    assert!(txs.iter().any(|t| t.id == id));
    let reversal = txs.iter().find(|t| t.id == reversal).unwrap();
    assert!(
        reversal.comment.contains(&format!("#{id}")),
        "{}",
        reversal.comment
    );
}
#[tokio::test]
async fn missing_role_surfaces_as_forbidden_error() {
    let server = kabacus_testkit::spawn_server(|cfg| {
        cfg.auth.no_auth_roles = vec!["ledger-write".to_string()];
    })
    .await;
    let client = KabacusClient::connect_unix(server.socket_path.clone());
    let result = client.list_transactions().await;
    assert!(
        matches!(result, Err(kabacus_client::ClientError::Forbidden(role)) if role == "ledger-read")
    );
}
#[tokio::test]
async fn accounts_register_and_search_round_trip() {
    let (_dir, client) = spawn_server().await;
    let head = client.version().await.unwrap().head.unwrap();

    let add_body = AddTransactionBody {
        date: "2026-02-01".to_string(),
        description: "Grocery Store".to_string(),
        postings: vec![
            DraftPostingBody {
                account: "expenses:groceries".to_string(),
                amount: Some(DraftAmount::from_money(
                    "USD",
                    kabacus_core::Money::from_decimal(2000, 2),
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
        dry_run: false,
    };
    client.add_transaction(&head, add_body).await.unwrap();

    let accounts = client.list_accounts(None).await.unwrap();
    let checking = accounts
        .iter()
        .find(|a| a.account == "assets:checking")
        .unwrap();
    assert_eq!(checking.balances[0].money().unwrap().to_string(), "980.00");

    let register = client
        .get_register("assets:checking", None, None)
        .await
        .unwrap();
    assert_eq!(register.len(), 2);
    assert_eq!(register[1].other_account, "expenses:groceries");

    let found = client.search(&[("query", "grocery")]).await.unwrap();
    assert_eq!(found.len(), 1);
    let not_found = client
        .search(&[("query", "nonexistent-payee")])
        .await
        .unwrap();
    assert_eq!(not_found.len(), 0);
}
