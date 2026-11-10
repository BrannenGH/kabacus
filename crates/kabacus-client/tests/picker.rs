// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Through the real server: the editor's account picker
//! and payee memory, as the app sees them.

use kabacus_client::types::{AddTransactionBody, DraftAmount, DraftPostingBody};
use kabacus_client::KabacusClient;
use kabacus_testkit::{spawn_server, TestServer};

async fn server_and_client() -> (TestServer, KabacusClient) {
    let server = spawn_server(|_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    (server, client)
}

async fn add(client: &KabacusClient, date: &str, description: &str, account: &str, cents: i64) {
    let head = client.version().await.unwrap().head.unwrap();
    client
        .add_transaction(
            &head,
            AddTransactionBody {
                new_accounts: vec![],
                date: date.to_string(),
                description: description.to_string(),
                postings: vec![
                    DraftPostingBody {
                        account: account.to_string(),
                        amount: Some(DraftAmount {
                            commodity: "USD".to_string(),
                            mantissa: cents.into(),
                            places: 2,
                        }),
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
            },
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn accounts_carry_friendly_names_and_rank_for_the_picker() {
    let (_server, client) = server_and_client().await;

    let all = client.list_accounts(None).await.unwrap();
    let checking = all.iter().find(|a| a.account == "assets:checking").unwrap();
    assert_eq!(checking.friendly_name.as_deref(), Some("Everyday Checking"));
    let equity = all.iter().find(|a| a.account == "equity:opening").unwrap();
    assert_eq!(equity.friendly_name, None);

    // Matches the friendly name even though the path doesn't contain it.
    let everyday = client.list_accounts(Some("everyday")).await.unwrap();
    assert_eq!(everyday.len(), 1);
    assert_eq!(everyday[0].account, "assets:checking");

    // An abbreviated path, ranked first.
    let groc = client.list_accounts(Some("exgro")).await.unwrap();
    assert_eq!(groc[0].account, "expenses:groceries");
}
#[tokio::test]
async fn a_known_payee_prefills_from_its_last_transaction() {
    let (_server, client) = server_and_client().await;
    add(
        &client,
        "2026-02-01",
        "Corner Grocer",
        "expenses:groceries",
        4200,
    )
    .await;
    add(
        &client,
        "2026-03-01",
        "Corner Grocer",
        "expenses:household",
        1500,
    )
    .await;
    add(
        &client,
        "2026-02-15",
        "Power Co",
        "expenses:household",
        8000,
    )
    .await;

    let payees = client.payees(None).await.unwrap();
    let names: Vec<&str> = payees.iter().map(|p| p.payee.as_str()).collect();
    assert_eq!(
        names[0], "Corner Grocer",
        "most recently used first: {names:?}"
    );

    let grocer = &client.payees(Some("corner")).await.unwrap()[0];
    assert_eq!(grocer.count, 2);
    assert_eq!(grocer.last.date, "2026-03-01");
    let expense = grocer
        .last
        .postings
        .iter()
        .find(|p| p.account != "assets:checking")
        .unwrap();
    assert_eq!(expense.account, "expenses:household");
}
