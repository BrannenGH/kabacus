// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Through `/mcp`: the shared write conventions (decimal
//! amounts, error codes, head checks, who made each write) and compact
//! previews.

mod support;

use kabacus_testkit::{spawn_server, spawn_server_with_journal};

use serde_json::{json, Value};
use support::*;

fn groceries(head: &str, amount: Value) -> Value {
    json!({
        "date": "2026-02-01",
        "description": "Grocery Store",
        "postings": [
            {"account": "expenses:groceries", "amount": amount},
            {"account": "assets:checking"}
        ],
        "head": head,
    })
}

/// Every write tool takes decimal strings, and refuses a JSON number with
/// `amount_format`, naming the field.
#[tokio::test]
async fn amounts_are_decimal_strings_and_never_floats() {
    let server = spawn_server(rewrites).await;
    let client = connect(&server).await;
    let mut head = head(&client).await;

    for (amount, rendered) in [
        ("100.00", "USD100.00"),
        ("-0.04", "-USD0.04"),
        ("0.5", "USD0.50"),
    ] {
        let preview = call(&client, "add_transaction", groceries(&head, json!(amount))).await;
        assert!(
            preview["after"].as_str().unwrap().contains(rendered),
            "{amount}: {preview}"
        );
        (_, head) = commit(&client, "add_transaction", groceries(&head, json!(amount))).await;
    }

    let error = fail(&client, "add_transaction", groceries(&head, json!(100.0))).await;
    assert_eq!(error["code"], "amount_format", "{error}");
    assert_eq!(error["path"], "postings[0].amount");
    let error = fail(&client, "add_transaction", groceries(&head, json!("1e3"))).await;
    assert_eq!(error["code"], "amount_format");

    // The legacy exact object still works.
    call(
        &client,
        "add_transaction",
        groceries(
            &head,
            json!({"commodity": "USD", "mantissa": 1234, "places": 2}),
        ),
    )
    .await;

    // And so do edit, correct and set_balance_assertion.
    let found = call(
        &client,
        "search_transactions",
        json!({"query": "Grocery", "limit": 1}),
    )
    .await;
    let id = found["transactions"][0]["id"].as_u64().unwrap();
    let mut edit = groceries(&head, json!("12.34"));
    edit["id"] = json!(id);
    let edited = call(&client, "edit_transaction", edit.clone()).await;
    assert!(
        edited["after"].as_str().unwrap().contains("USD12.34"),
        "{edited}"
    );
    edit["postings"][0]["amount"] = json!(12.34);
    assert_eq!(
        fail(&client, "edit_transaction", edit).await["code"],
        "amount_format"
    );
    let correct = json!({
        "id": id, "description": "Grocery Store", "head": head, "today": "2026-03-01",
        "postings": [
            {"account": "expenses:groceries", "amount": "7.00"},
            {"account": "assets:checking", "amount": "-7.00"}
        ],
    });
    call(&client, "correct_transaction", correct).await;
    let assertion = fail(
        &client,
        "set_balance_assertion",
        json!({"transaction": 1, "account": "assets:checking", "balance": 5, "head": head}),
    )
    .await;
    assert_eq!(assertion["code"], "amount_format");
    let preview = call(
        &client,
        "set_balance_assertion",
        json!({"transaction": 1, "account": "assets:checking", "balance": "1000.00", "head": head}),
    )
    .await;
    assert!(
        preview["diff"].as_str().unwrap().contains("= USD1000.00"),
        "{preview}"
    );
    client.cancel().await.unwrap();
}

/// Through `/mcp`: an amount with more decimal places than the
/// commodity takes is refused, in a single write as in a batch, and the
/// commodity keeps its places.
#[tokio::test]
async fn amounts_finer_than_the_commodity_are_refused() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;

    let commodity = call(&client, "get_commodity", json!({})).await;
    assert_eq!(commodity, json!({"symbol": "USD", "places": 2}));

    let error = fail(
        &client,
        "add_transaction",
        groceries(&head, json!("12.345")),
    )
    .await;
    assert_eq!(
        error["message"],
        "USD12.345 on expenses:groceries has more decimal places than USD takes (2) (rule: amount)",
        "{error}"
    );

    let mut batch_op = groceries(&head, json!("0.0001"));
    batch_op.as_object_mut().unwrap().remove("head");
    let done = try_call(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "ops": [{"tool": "add_transaction", "args": batch_op}]}),
    )
    .await;
    assert!(
        format!("{done:?}").contains("more decimal places than USD takes (2)"),
        "{done:?}"
    );
    let commodity = call(&client, "get_commodity", json!({})).await;
    assert_eq!(commodity["places"], 2);
    client.cancel().await.unwrap();
}

/// A stale head is a `head_conflict` naming the current head, for a dry
/// run as for a commit; other failures carry their own codes.
#[tokio::test]
async fn errors_carry_machine_readable_codes() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let (_, new_head) = commit(&client, "add_transaction", groceries(&head, json!("1.00"))).await;

    for dry_run in [true, false] {
        let mut args = groceries(&head, json!("2.00"));
        args["dry_run"] = json!(dry_run);
        let error = fail(&client, "add_transaction", args).await;
        assert_eq!(error["code"], "head_conflict", "{error}");
        assert_eq!(error["details"]["current_head"], new_head.as_str());
    }
    let error = fail(
        &client,
        "edit_transaction",
        json!({"id": 999, "date": "2026-01-01", "description": "x", "postings": [], "head": new_head}),
    )
    .await;
    assert_eq!(error["code"], "not_found", "{error}");
    let mut bad = groceries(&new_head, json!("1.00"));
    bad["postings"][0]["account"] = json!("expenses:nowhere");
    assert_eq!(
        fail(&client, "add_transaction", bad).await["code"],
        "validation_failed"
    );
    client.cancel().await.unwrap();

    let reader = spawn_server(|cfg| cfg.auth.no_auth_roles = vec!["ledger-read".to_string()]).await;
    let client = connect(&reader).await;
    let head = support::head(&client).await;
    let error = fail(&client, "add_transaction", groceries(&head, json!("1.00"))).await;
    assert_eq!(error["code"], "role_required", "{error}");
    client.cancel().await.unwrap();
}

/// Saving a transaction exactly as it already is writes nothing: no
/// revision, the same head, and an `unchanged` warning. A dry run says the
/// same, so a clean preview still means a clean save.
#[tokio::test]
async fn an_edit_that_changes_nothing_writes_nothing() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let (added, head) = commit(&client, "add_transaction", groceries(&head, json!("12.40"))).await;
    let id = added["id"].as_u64().unwrap();
    let commits = revision(&server).await;

    for dry_run in [true, false] {
        let mut args = groceries(&head, json!("12.40"));
        args["id"] = json!(id);
        args["dry_run"] = json!(dry_run);
        let result = call(&client, "edit_transaction", args).await;
        assert_eq!(result["head"], head.as_str(), "{result}");
        assert!(
            result["warnings"]
                .as_array()
                .is_some_and(|w| w.iter().any(|w| w["code"] == "unchanged")),
            "{result}"
        );
    }
    assert_eq!(revision(&server).await, commits, "nothing committed");
    client.cancel().await.unwrap();
}

/// Every write the server makes records who made it: writes, rewrites
/// and batches alike.
#[tokio::test]
async fn every_write_records_who_made_it() {
    let server = spawn_server(rewrites).await;
    let client = connect(&server).await;
    let imported = revision(&server).await;
    let head = head(&client).await;
    let (added, head) = commit(&client, "add_transaction", groceries(&head, json!("1.00"))).await;
    let mut edit = groceries(&head, json!("2.00"));
    edit["id"] = added["id"].clone();
    let (_, head) = commit(&client, "edit_transaction", edit).await;
    rewrite(
        &client,
        "declare_account",
        json!({"account": "expenses:pets", "head": head}),
    )
    .await;
    let head = support::head(&client).await;
    call(
        &client,
        "batch",
        json!({"head": head, "dry_run": false, "ops": [
            {"tool": "add_transaction", "args": groceries("", json!("3.00"))}
        ]}),
    )
    .await;

    let last = revision(&server).await;
    assert_eq!(last, imported + 4);
    let mut kinds = Vec::new();
    for rev in imported + 1..=last {
        let change = change(&server, &rev.to_string()).await;
        assert_eq!(change.actor, "mcp", "{change:?}");
        assert_eq!(change.user, "local", "{change:?}");
        assert_eq!(change.details["auth"], "none", "{change:?}");
        assert!(change.details["client"].is_string(), "{change:?}");
        assert!(change.details["session"].is_string(), "{change:?}");
        assert!(change.rows > 0, "{change:?}");
        kinds.push(change.kind);
    }
    assert_eq!(kinds, ["add", "edit", "rewrite", "batch"]);
    client.cancel().await.unwrap();
}

/// A synthetic year of hand-kept books in another style: `-$9.93` signs,
/// thousands separators and amounts aligned at a column. 1,000
/// transactions, 5,000 lines.
fn hand_kept_books(crlf: bool) -> String {
    let mut out = String::from("commodity $1,000.00\n\n");
    for i in 0..1000 {
        let day = 1 + i % 28;
        let month = 1 + (i / 28) % 12;
        let cents = 100 + i * 7;
        let amount = format!("${}.{:02}", cents / 100, cents % 100);
        out.push_str(&format!(
            "2022-{month:02}-{day:02} McDonald's {i}\n    expenses:groceries          {amount}\n    assets:checking            -{amount}\n    ; receipt kept\n\n"
        ));
    }
    if crlf {
        out = out.replace('\n', "\r\n");
    }
    out
}

/// The id of the transaction whose description is `description`.
async fn id_of(server: &kabacus_testkit::TestServer, description: &str) -> u64 {
    support::book(server)
        .await
        .transactions
        .iter()
        .find(|t| t.description == description)
        .unwrap()
        .id
}

/// Editing one transaction of a year of books changes only that
/// transaction's row, in the books' own amount style; the preview is a
/// few hundred bytes and the audit log names it.
#[tokio::test]
async fn an_edit_changes_only_its_own_row_in_the_books_style() {
    let server = spawn_server_with_journal(&hand_kept_books(false), |_| {}).await;
    let client = connect(&server).await;
    let head = head(&client).await;
    let id = id_of(&server, "McDonald's 1").await;
    let edit = json!({
        "id": id,
        "date": "2022-01-02",
        "description": "McDonald's 1",
        "postings": [
            {"account": "expenses:groceries", "amount": "1234.56"},
            {"account": "assets:checking"}
        ],
        "head": head,
    });
    let preview = call(&client, "edit_transaction", edit.clone()).await;
    let size = preview.to_string().len();
    assert!(size < 4096, "a {size}-byte preview: {preview}");
    assert!(preview.get("diff").is_none());
    let after = preview["after"].as_str().unwrap();
    assert!(after.contains("expenses:groceries  $1,234.56"), "{after}");
    assert!(after.contains("assets:checking  -$1,234.56"), "{after}");
    assert!(after.contains("; receipt kept"), "{after}");

    let mut with_diff = edit.clone();
    with_diff["include_diff"] = json!(true);
    let diff = call(&client, "edit_transaction", with_diff).await["diff"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        diff.lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count(),
        2,
        "{diff}"
    );

    let (_, new_head) = commit(&client, "edit_transaction", edit).await;
    let change = change(&server, &new_head).await;
    assert_eq!(change.rows, 1, "{change:?}");
    assert_eq!(
        change.summary,
        format!("edit: 2022-01-02 McDonald's 1 [#{id}]")
    );
    assert_eq!(support::book(&server).await.transactions.len(), 1001);
    client.cancel().await.unwrap();
}

/// Books kept with Windows line endings read exactly as with Unix ones.
#[tokio::test]
async fn crlf_books_import_like_lf_books() {
    let lf = spawn_server_with_journal(&hand_kept_books(false), |_| {}).await;
    let crlf = spawn_server_with_journal(&hand_kept_books(true), |_| {}).await;
    assert_eq!(support::book(&crlf).await, support::book(&lf).await);
}
