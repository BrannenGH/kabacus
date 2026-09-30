// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Through `/mcp`: renaming and removing accounts, and looking
//! up one transaction.

mod support;

use kabacus_testkit::{spawn_server, spawn_server_with_journal};
use serde_json::json;
use support::*;

const OLD: &str = "assets:bank:checking:card-sc";
const NEW: &str = "assets:bank:checking:card-store-card";

#[tokio::test]
async fn account_types_default_and_convert_without_changing_transactions() {
    let server = spawn_server_with_journal(
        "account assets:bank:checking  ; ynab-account:Everyday Checking\n\
         account assets:bank:checking:insurance\n\
         account equity:opening\n\n\
         2026-09-01 Opening\n    assets:bank:checking  800.00 USD\n    equity:opening\n",
        rewrites,
    )
    .await;
    let client = connect(&server).await;
    let mut revision = head(&client).await;
    let original = book(&server).await.transactions;
    let accounts = call(&client, "list_accounts", json!({})).await;
    let rows = accounts.as_array().unwrap();
    let checking = rows
        .iter()
        .find(|a| a["account"] == "assets:bank:checking")
        .unwrap();
    let reserve = rows
        .iter()
        .find(|a| a["account"] == "assets:bank:checking:insurance")
        .unwrap();
    assert_eq!(checking["synthetic"], false);
    assert_eq!(reserve["synthetic"], true);
    assert_eq!(reserve["physical_home"], "assets:bank:checking");

    for synthetic in [true, false] {
        let args = json!({
            "account": "assets:bank:checking",
            "synthetic": synthetic,
            "head": revision,
        });
        let preview = call(&client, "set_account_type", args.clone()).await;
        assert_eq!(preview["dry_run"], true);
        assert_eq!(head(&client).await, revision);
        let done = rewrite(&client, "set_account_type", args).await;
        revision = done["head"].as_str().unwrap().to_string();
        let accounts = call(&client, "list_accounts", json!({})).await;
        let rows = accounts.as_array().unwrap();
        let checking = rows
            .iter()
            .find(|a| a["account"] == "assets:bank:checking")
            .unwrap();
        let reserve = rows
            .iter()
            .find(|a| a["account"] == "assets:bank:checking:insurance")
            .unwrap();
        assert_eq!(checking["synthetic"], synthetic);
        assert_eq!(reserve["synthetic"], true);
        assert_eq!(
            reserve["physical_home"].as_str(),
            (!synthetic).then_some("assets:bank:checking")
        );
        let stored = book(&server).await;
        assert_eq!(stored.transactions, original);
        assert!(stored.accounts["assets:bank:checking"]
            .tags
            .contains("ynab-account:Everyday Checking"));
        let audit = change(&server, &revision).await;
        assert_eq!(audit.actor, "mcp");
    }
    let error = fail(
        &client,
        "set_account_type",
        json!({"account": "assets:bank:checking", "synthetic": true, "head": "0", "dry_run": false}),
    )
    .await;
    assert_eq!(error["code"], "head_conflict");
    assert_eq!(head(&client).await, revision);
    assert_checks(&server).await;
    client.cancel().await.unwrap();
}

/// A card reserve with 2,000 postings, in a posting, an assertion, a tag
/// and its declaration.
fn reserve_books() -> String {
    let mut out = String::new();
    for i in 0..2000 {
        out.push_str(&format!(
            "2025-{:02}-{:02} Store {i}\n    expenses:groceries  USD1.00\n    liabilities:credit-card:sc  USD-1.00\n    assets:bank:checking  USD-1.00\n    {OLD}  USD1.00  ; fund:{OLD}\n\n",
            1 + (i / 28) % 12,
            1 + i % 28
        ));
    }
    out.push_str(&format!(
        "2025-12-31 Check\n    {OLD}  USD0.00 = USD2000.00\n"
    ));
    out
}

async fn spawn_reserve() -> kabacus_testkit::TestServer {
    spawn_server_with_journal(
        &format!(
            "account assets:bank:checking\naccount liabilities:credit-card:sc\naccount {OLD}  ; note\naccount liabilities:credit-card:dept-store\n\n{}",
            reserve_books()
        ),
        rewrites,
    )
    .await
}

/// Renaming a reserve with 2,000 postings: the books still check, balances
/// are the same under the new name, and the old name is gone everywhere.
/// The preview counts occurrences with three samples, no diff.
#[tokio::test]
async fn rename_reaches_every_reference() {
    let server = spawn_reserve().await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let before = call(&client, "get_balance", json!({"account_prefix": OLD})).await;
    let preview = call(
        &client,
        "rename_account",
        json!({"from": OLD, "to": NEW, "head": head}),
    )
    .await;
    assert!(preview.get("diff").is_none(), "{preview}");
    assert_eq!(preview["details"]["occurrences"], 2 * 2000 + 2);
    assert_eq!(preview["details"]["transactions"], 2001);
    assert_eq!(preview["details"]["samples"].as_array().unwrap().len(), 3);
    assert!(preview.to_string().len() < 4096);

    let done = rewrite(
        &client,
        "rename_account",
        json!({"from": OLD, "to": NEW, "head": head}),
    )
    .await;
    assert_ne!(done["head"], head.as_str());
    let after = call(&client, "get_balance", json!({"account_prefix": NEW})).await;
    assert_eq!(after, before);
    let text = journal(&server).await;
    assert!(!text.contains(OLD), "the books still name {OLD}");
    assert_eq!(support::book(&server).await.accounts[NEW].tags, "note");
    assert_checks(&server).await;
    client.cancel().await.unwrap();
}

/// `to` must not exist unless merging accounts of one kind; a new path
/// that changes an account's kind needs allow_kind_change.
#[tokio::test]
async fn rename_guards_merges_and_kind_changes() {
    let server = spawn_reserve().await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let error = fail(
        &client,
        "rename_account",
        json!({"from": "liabilities:credit-card:sc", "to": "liabilities:credit-card:dept-store", "head": head}),
    )
    .await;
    assert!(
        error["message"].as_str().unwrap().contains("merge"),
        "{error}"
    );
    let merge = call(
        &client,
        "rename_account",
        json!({"from": "liabilities:credit-card:sc", "to": "liabilities:credit-card:dept-store", "merge": true, "head": head}),
    )
    .await;
    assert_eq!(merge["details"]["merged"], true);
    let kind = fail(
        &client,
        "rename_account",
        json!({"from": OLD, "to": "expenses:store-card", "head": head}),
    )
    .await;
    assert_eq!(kind["details"]["rule"], "kind_change", "{kind}");
    let allowed = call(
        &client,
        "rename_account",
        json!({"from": OLD, "to": "expenses:store-card", "allow_kind_change": true, "head": head}),
    )
    .await;
    assert_eq!(allowed["warnings"][0]["code"], "kind_change");
    assert_eq!(
        allowed["details"]["kind_changes"][0]["old_kind"],
        "debt-reserve"
    );
    client.cancel().await.unwrap();
}

/// An empty declared account is removed; one with a posting is refused
/// with account_in_use and a count of each reference.
#[tokio::test]
async fn remove_only_an_unused_account() {
    let server = spawn_server_with_journal("account expenses:typo\n", rewrites).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let error = fail(
        &client,
        "remove_account",
        json!({"account": "assets:checking", "head": head}),
    )
    .await;
    assert_eq!(error["code"], "account_in_use", "{error}");
    assert_eq!(error["details"]["counts"]["postings"], 1);
    assert_eq!(error["details"]["counts"]["sub_accounts"], 0);

    rewrite(
        &client,
        "remove_account",
        json!({"account": "expenses:typo", "head": head}),
    )
    .await;
    assert!(!journal(&server).await.contains("expenses:typo"));
    let accounts = call(&client, "list_accounts", json!({"filter": "typo"})).await;
    assert!(accounts
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["account"] != "expenses:typo"));
    client.cancel().await.unwrap();
}

/// One transaction by id, with the writes that changed it.
#[tokio::test]
async fn get_transaction_by_id_with_history() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let add = json!({
        "date": "2026-02-01", "description": "Grocery Store", "head": head,
        "postings": [{"account": "expenses:groceries", "amount": "5.00"}, {"account": "assets:checking"}],
    });
    let (added, head) = commit(&client, "add_transaction", add.clone()).await;
    let id = added["id"].as_u64().unwrap();
    let mut edit = add;
    edit["id"] = json!(id);
    edit["head"] = json!(head);
    edit["postings"][0]["amount"] = json!("6.00");
    commit(&client, "edit_transaction", edit).await;

    let tx = call(
        &client,
        "get_transaction",
        json!({"id": id, "include_history": true}),
    )
    .await;
    assert_eq!(exact(&tx["postings"][0]["amount"]), "6.00");
    let history = tx["history"].as_array().unwrap();
    assert_eq!(history.len(), 2, "{history:?}");
    assert_eq!(history[0]["kind"], "edit");
    assert_eq!(history[1]["kind"], "add");
    assert!(history[0]["date"].as_str().unwrap().starts_with("20"));
    assert_eq!(history[0]["actor"], "mcp");

    let opening = call(&client, "get_transaction", json!({"id": "1"})).await;
    assert_eq!(opening["payee"], "Opening");
    assert_eq!(opening["id"], 1);
    let error = fail(&client, "get_transaction", json!({"id": 999})).await;
    assert_eq!(error["code"], "not_found");
    client.cancel().await.unwrap();
}
