// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Through `/mcp`: `batch`, many writes as one, checked once and
//! committed once.

mod support;

use kabacus_testkit::{spawn_server_with_journal, TestServer};
use serde_json::{json, Value};
use support::*;

const BANK: &str = "account assets:bank:checking\n\
    account liabilities:credit-card:visa\n\
    account assets:bank:checking:card-visa\n\
    2026-01-02 Opening bank\n    assets:bank:checking  USD500.00\n    equity:opening\n";

async fn spawn_bank(customize: impl FnOnce(&mut kabacus_server::config::Config)) -> TestServer {
    spawn_server_with_journal(BANK, customize).await
}

fn groceries(amount: &str) -> Value {
    json!({
        "date": "2026-02-01",
        "description": "Grocery Store",
        "postings": [
            {"account": "expenses:groceries", "amount": amount},
            {"account": "assets:checking"}
        ],
    })
}

/// A fund declared and allocated into in one batch: one dry run, one
/// commit.
#[tokio::test]
async fn declare_then_allocate_in_one_commit() {
    let server = spawn_bank(rewrites).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let commits = revision(&server).await;
    let batch = json!({
        "head": head,
        "ops": [
            {"tool": "declare_account", "args": {"account": "assets:bank:checking:vacation"}, "client_ref": "fund"},
            {"tool": "allocate", "client_ref": "move", "args": {
                "from": "assets:bank:checking", "to": "assets:bank:checking:vacation",
                "amount": "100.00", "date": "2026-02-01"}},
        ],
    });
    let preview = call(&client, "batch", batch.clone()).await;
    assert_eq!(preview["counts"], json!({"ok": 2, "error": 0}), "{preview}");
    assert!(preview.get("diff").is_none());
    assert_eq!(revision(&server).await, commits, "a preview writes nothing");
    let vacation = preview["balance_changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["account"] == "assets:bank:checking:vacation")
        .unwrap();
    assert_eq!(exact(&vacation["after"]), "100.00");

    let mut commit = batch.clone();
    commit["dry_run"] = json!(false);
    let done = call(&client, "batch", commit).await;
    assert_eq!(done["dry_run"], false);
    assert_ne!(done["head"], head.as_str());
    assert_eq!(revision(&server).await, commits + 1);
    let report = call(&client, "cash_on_hand", json!({})).await;
    let fund = report["funds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["account"] == "assets:bank:checking:vacation")
        .unwrap();
    assert_eq!(exact(&fund["balance"]), "100.00");
    let change = change(&server, done["head"].as_str().unwrap()).await;
    assert_eq!(change.kind, "batch", "{change:?}");
    client.cancel().await.unwrap();
}

/// A dry run is optional: a batch holding a rewrite commits on its first
/// call with `dry_run: false` and the current head.
#[tokio::test]
async fn a_rewrite_batch_commits_without_a_dry_run_first() {
    let server = spawn_bank(rewrites).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let commits = revision(&server).await;
    let coffee = json!({
        "date": "2026-02-03",
        "description": "Blue Bottle Coffee",
        "postings": [
            {"account": "expenses:coffee", "amount": "6.75"},
            {"account": "assets:checking"}
        ],
    });
    let done = call(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "ops": [
            {"tool": "declare_account", "args": {"account": "expenses:coffee"}},
            {"tool": "add_transaction", "args": groceries("42.18"), "client_ref": "groceries"},
            {"tool": "add_transaction", "args": coffee, "client_ref": "coffee"},
        ]}),
    )
    .await;
    assert_eq!(done["counts"], json!({"ok": 3, "error": 0}), "{done}");
    assert_eq!(done["dry_run"], false);
    assert!(done.get("confirm_token").is_none(), "{done}");
    assert_ne!(done["head"], head.as_str());
    assert_eq!(revision(&server).await, commits + 1);
    let id = done["ops"][2]["id"].as_u64().unwrap();
    let tx = call(&client, "get_transaction", json!({"id": id})).await;
    assert_eq!(tx["postings"][0]["account"], "expenses:coffee");
    client.cancel().await.unwrap();
}

/// A later operation sees an earlier one and can name its id.
#[tokio::test]
async fn refs_name_an_earlier_operations_id() {
    let server = spawn_bank(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let mut edit = groceries("25.00");
    edit["id"] = json!({"$ref": "first.id"});
    edit["description"] = json!("Grocery Store | edited");
    let result = call(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "ops": [
            {"tool": "add_transaction", "args": groceries("20.00"), "client_ref": "first"},
            {"tool": "edit_transaction", "args": edit},
            {"tool": "set_cleared", "args": {"id": {"$ref": "first.id"}, "account": "assets:checking", "status": "cleared"}},
        ]}),
    )
    .await;
    let id = result["ops"][0]["id"].as_u64().unwrap();
    assert_eq!(result["ops"][1]["id"], id);
    let tx = call(&client, "get_transaction", json!({"id": id})).await;
    assert_eq!(tx["memo"], "edited");
    assert_eq!(exact(&tx["postings"][0]["amount"]), "25.00");
    assert_eq!(tx["postings"][1]["status"], "cleared");
    client.cancel().await.unwrap();
}

/// Atomic: a bad operation writes nothing and is named by index and
/// client_ref. continue_on_error lists every failure, in dry runs only.
/// Duplicate client_refs and unknown tools are refused.
#[tokio::test]
async fn a_failure_writes_nothing_and_names_the_operation() {
    let server = spawn_bank(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let mut bad = groceries("1.00");
    bad["postings"][0]["account"] = json!("expenses:nowhere");
    let ops = json!([
        {"tool": "add_transaction", "args": groceries("1.00"), "client_ref": "a"},
        {"tool": "add_transaction", "args": groceries("2.00"), "client_ref": "b"},
        {"tool": "add_transaction", "args": groceries("3.00"), "client_ref": "c"},
        {"tool": "add_transaction", "args": bad, "client_ref": "d"},
        {"tool": "add_transaction", "args": groceries("4.00"), "client_ref": "e"},
    ]);
    let commits = revision(&server).await;
    let error = fail(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "ops": ops}),
    )
    .await;
    assert_eq!(error["code"], "validation_failed", "{error}");
    assert_eq!(error["details"]["index"], 3);
    assert_eq!(error["details"]["client_ref"], "d");
    assert_eq!(error["path"], "ops[3]");
    assert_eq!(revision(&server).await, commits);
    assert_eq!(support::head(&client).await, head);

    let mut also_bad = ops.clone();
    also_bad[1]["args"]["postings"][0]["amount"] = json!(2.5);
    let listed = call(
        &client,
        "batch",
        json!({"head": head, "mode": "continue_on_error", "ops": also_bad}),
    )
    .await;
    assert_eq!(listed["counts"], json!({"ok": 3, "error": 2}), "{listed}");
    assert_eq!(listed["ops"][1]["error"]["code"], "amount_format");
    assert_eq!(listed["ops"][3]["error"]["code"], "validation_failed");
    assert_eq!(listed["ops"][4]["status"], "ok");

    let refused = fail(
        &client,
        "batch",
        json!({"head": head, "mode": "continue_on_error", "dry_run": false, "ops": ops}),
    )
    .await;
    assert_eq!(refused["code"], "validation_failed");
    let duplicate = fail(
        &client,
        "batch",
        json!({"head": head, "ops": [
            {"tool": "add_transaction", "args": groceries("1.00"), "client_ref": "x"},
            {"tool": "add_transaction", "args": groceries("1.00"), "client_ref": "x"},
        ]}),
    )
    .await;
    assert!(
        duplicate["message"]
            .as_str()
            .unwrap()
            .contains("used twice"),
        "{duplicate}"
    );
    let unknown = fail(
        &client,
        "batch",
        json!({"head": head, "ops": [{"tool": "get_version", "args": {}}]}),
    )
    .await;
    assert_eq!(unknown["details"]["index"], 0);
    client.cancel().await.unwrap();
}

/// A balance assertion the final state breaks fails the batch at its one
/// check, naming the operation when it wrote the failing line.
#[tokio::test]
async fn one_check_runs_on_the_final_state() {
    let server = spawn_server_with_journal(
        &format!("{BANK}\n2026-03-01 Check\n    assets:checking  USD0.00 = USD1000.00\n"),
        |_| {},
    )
    .await;
    let client = connect(&server).await;
    let head = head(&client).await;
    // Each one alone breaks the assertion; the two together don't.
    let result = call(
        &client,
        "batch",
        json!({"head": head, "ops": [
            {"tool": "add_transaction", "args": groceries("30.00")},
            {"tool": "add_transaction", "args": groceries("-30.00")},
        ]}),
    )
    .await;
    assert_eq!(result["counts"]["ok"], 2);
    let error = fail(
        &client,
        "batch",
        json!({"head": head, "ops": [{"tool": "add_transaction", "args": groceries("30.00")}]}),
    )
    .await;
    assert_eq!(error["code"], "validation_failed");
    assert_eq!(error["details"]["rule"], "assertion", "{error}");
    client.cancel().await.unwrap();
}

/// Committing a batch_id again returns the first result and adds no
/// write; the audit log records the id.
#[tokio::test]
async fn a_replayed_batch_id_writes_nothing() {
    let server = spawn_bank(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let batch = json!({"head": head, "dry_run": false, "batch_id": "backfill-1", "ops": [
        {"tool": "add_transaction", "args": groceries("1.00")},
    ]});
    let first = call(&client, "batch", batch.clone()).await;
    let commits = revision(&server).await;
    let again = call(&client, "batch", batch).await;
    assert_eq!(again["replayed"], true);
    assert_eq!(again["head"], first["head"]);
    assert_eq!(again["ops"], first["ops"]);
    assert_eq!(revision(&server).await, commits);
    let change = change(&server, first["head"].as_str().unwrap()).await;
    assert_eq!(change.details["batch_id"], "backfill-1", "{change:?}");
    client.cancel().await.unwrap();
}

/// `commit: "per_op"` makes one commit per operation.
#[tokio::test]
async fn per_op_commits_one_commit_per_operation() {
    let server = spawn_bank(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let commits = revision(&server).await;
    call(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "commit": "per_op", "ops": [
            {"tool": "add_transaction", "args": groceries("1.00")},
            {"tool": "add_transaction", "args": groceries("2.00")},
            {"tool": "add_transaction", "args": groceries("3.00")},
        ]}),
    )
    .await;
    assert_eq!(revision(&server).await, commits + 3);
    let history = call(&client, "get_history", json!({})).await;
    assert!(history[0]["summary"]
        .as_str()
        .unwrap()
        .starts_with("add: 2026-02-01 Grocery Store"));
    assert_eq!(history[0]["kind"], "add");
    client.cancel().await.unwrap();
}

/// The highest role among the operations is needed; over the limits is
/// `batch_too_large`.
#[tokio::test]
async fn roles_and_limits() {
    let server = spawn_bank(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let error = fail(
        &client,
        "batch",
        json!({"head": head, "ops": [
            {"tool": "add_transaction", "args": groceries("1.00")},
            {"tool": "declare_account", "args": {"account": "expenses:pets"}},
        ]}),
    )
    .await;
    assert_eq!(error["code"], "role_required", "{error}");
    let ops: Vec<Value> = (0..101)
        .map(|_| json!({"tool": "add_transaction", "args": {}}))
        .collect();
    let error = fail(&client, "batch", json!({"head": head, "ops": ops})).await;
    assert_eq!(error["code"], "batch_too_large");
    client.cancel().await.unwrap();
}

/// The card-reserve backfill: a full batch (100) of edits adding reserve
/// lines, one write, well within the time limit.
#[tokio::test]
async fn a_full_batch_of_edits_makes_one_commit() {
    const EDITS: usize = 100;
    let mut journal = BANK.to_string();
    for i in 0..EDITS {
        journal.push_str(&format!(
            "2025-{:02}-{:02} Store {i}\n    expenses:groceries  USD1.00\n    liabilities:credit-card:visa\n\n",
            1 + (i / 28) % 12,
            1 + i % 28
        ));
    }
    let server = spawn_server_with_journal(&journal, |_| {}).await;
    let ids: Vec<u64> = support::book(&server)
        .await
        .transactions
        .iter()
        .filter(|t| t.description.starts_with("Store "))
        .map(|t| t.id)
        .collect();
    assert_eq!(ids.len(), EDITS);
    let client = connect(&server).await;
    let head = head(&client).await;
    let before = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(before["unreserved_count"], EDITS);

    let ops: Vec<Value> = (0..EDITS)
        .map(|i| {
            json!({"tool": "edit_transaction", "args": {
                "id": ids[i],
                "date": format!("2025-{:02}-{:02}", 1 + (i / 28) % 12, 1 + i % 28),
                "description": format!("Store {i}"),
                "postings": [
                    {"account": "expenses:groceries", "amount": "1.00"},
                    {"account": "liabilities:credit-card:visa", "amount": "-1.00"},
                    {"account": "assets:bank:checking", "amount": "-1.00"},
                    {"account": "assets:bank:checking:card-visa", "amount": "1.00"},
                ],
            }})
        })
        .collect();
    let commits = revision(&server).await;
    let started = std::time::Instant::now();
    let result = call(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "ops": ops}),
    )
    .await;
    let took = started.elapsed();
    assert!(took < std::time::Duration::from_secs(120), "took {took:?}");
    assert_eq!(result["counts"]["ok"], EDITS);
    assert_eq!(revision(&server).await, commits + 1);
    let after = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(after["unreserved_count"], 0, "{}", after["warnings"]);
    client.cancel().await.unwrap();
}
