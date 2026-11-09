// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drive kabacus-server's `/mcp` endpoint with an `rmcp` client
//! over streamable HTTP, the way an MCP client would, against
//! a real server on synthetic books.

mod support;

use kabacus_server::api::ENDPOINTS;
use kabacus_server::mcp::REWRITE_TOOLS;
use kabacus_testkit::{spawn_server, spawn_server_with_journal, TestServer};
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams, ResourceContents};
use serde_json::{json, Value};
use support::{call, connect, object, rewrite, store_head, Client};

async fn try_call(client: &Client, name: &'static str, args: Value) -> Result<Value, String> {
    client
        .call_tool(CallToolRequestParams::new(name).with_arguments(object(args)))
        .await
        .map(|r| r.structured_content.expect("structured content"))
        .map_err(|e| e.to_string())
}

async fn tool_names(client: &Client) -> Vec<String> {
    client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

fn groceries(head: &str, dry_run: bool) -> Value {
    json!({
        "date": "2026-02-01",
        "description": "Grocery Store",
        "postings": [
            {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 2000, "places": 2}},
            {"account": "assets:checking"}
        ],
        "head": head,
        "dry_run": dry_run,
    })
}

/// A server whose rewrite gates are open for the local principal.
async fn spawn_rewrite_server() -> TestServer {
    spawn_server(support::rewrites).await
}

/// Every API route has its MCP tool(s): the MCP endpoint can do whatever
/// the app can.
#[tokio::test]
async fn there_is_a_tool_for_every_api_endpoint() {
    let server = spawn_rewrite_server().await;
    let client = connect(&server).await;
    let names = tool_names(&client).await;

    for endpoint in ENDPOINTS {
        for tool in endpoint.mcp_tools {
            assert!(
                names.iter().any(|n| n == tool),
                "{} {} has no MCP tool {tool}; tools: {names:?}",
                endpoint.method,
                endpoint.path
            );
        }
    }

    // Annotations match read-only vs write; void and delete
    // are flagged destructive.
    let tools = client.list_all_tools().await.unwrap();
    for name in ["allocate", "add_transaction"] {
        let description = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap()
            .description
            .as_deref()
            .unwrap();
        assert!(description.contains("physical accounts"), "{name}");
        assert!(description.contains("explicitly confirms"), "{name}");
        assert!(description.contains("already happened"), "{name}");
    }
    let hint = |name: &str| {
        tools
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .annotations
            .clone()
            .unwrap()
    };
    for read in [
        "list_accounts",
        "cash_on_hand",
        "account_kinds",
        "card_reserves",
        "get_skill_guide",
        "get_history_diff",
    ] {
        assert_eq!(hint(read).read_only_hint, Some(true), "{read}");
    }
    for destructive in ["void_transaction", "delete_transaction"] {
        assert_eq!(
            hint(destructive).destructive_hint,
            Some(true),
            "{destructive}"
        );
    }
    client.cancel().await.unwrap();
}

/// Every route in [`ENDPOINTS`] is really served (a stale entry would let
/// the parity check above pass for a route that no longer exists).
#[tokio::test]
async fn every_listed_endpoint_is_routed() {
    use tower::ServiceExt as _;
    let server = spawn_server(|_| {}).await;
    let head = store_head(&server).await;
    for endpoint in ENDPOINTS {
        let path = endpoint
            .path
            .replace("{id}", "nope")
            .replace("{index}", "0")
            .replace("{rev}", &head)
            .replace("{name}", "jordan")
            .replace("{account}", "assets:checking");
        let request = axum::http::Request::builder()
            .method(endpoint.method)
            .uri(&path)
            .header("if-match", &head)
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{}"))
            .unwrap();
        let response = kabacus_server::api::router(server.state.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_ne!(status, 405, "{} {path} isn't routed", endpoint.method);
        assert!(
            !(status == 404 && body.is_empty()),
            "{} {path} isn't routed",
            endpoint.method
        );
    }
}

/// Hidden unless usable, and refused when called anyway.
#[tokio::test]
async fn rewrite_tools_are_hidden_and_refused_until_the_gates_are_open() {
    // The server bans rewrites (--no-rewrite), even for a caller with the role.
    let server = spawn_server(|cfg| {
        cfg.allow_rewrite = false;
        support::rewrites(cfg);
    })
    .await;
    let client = connect(&server).await;
    let names = tool_names(&client).await;
    for tool in REWRITE_TOOLS {
        assert!(!names.iter().any(|n| n == tool), "{tool} is listed");
    }
    assert!(names.iter().any(|n| n == "edit_transaction"));

    let head = store_head(&server).await;
    let err = try_call(
        &client,
        "delete_transaction",
        json!({"id": 1, "head": head}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("--no-rewrite"), "{err}");
    client.cancel().await.unwrap();

    // By default the server allows rewrites, but this principal lacks the role.
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let names = tool_names(&client).await;
    assert!(!names.iter().any(|n| n == "delete_transaction"));
    let err = try_call(
        &client,
        "delete_transaction",
        json!({"id": 1, "head": head, "dry_run": false}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("ledger-rewrite"), "{err}");
    client.cancel().await.unwrap();
}

/// Through MCP, a delete defaults to a dry run that changes nothing, and
/// commits in one call with `dry_run: false`, attributed to MCP.
#[tokio::test]
async fn delete_dry_runs_by_default_then_commits_in_one_call() {
    let server = spawn_rewrite_server().await;
    let client = connect(&server).await;
    let head = store_head(&server).await;
    let added = call(&client, "add_transaction", groceries(&head, false)).await;
    let id = added["id"].as_u64().unwrap().to_string();
    let head = added["head"].as_str().unwrap().to_string();

    let preview = call(
        &client,
        "delete_transaction",
        json!({"id": id, "head": head}),
    )
    .await;
    assert!(preview["diff"]
        .as_str()
        .unwrap()
        .contains("-2026-02-01 Grocery Store"));
    assert_eq!(store_head(&server).await, head, "a preview commits nothing");

    let done = rewrite(
        &client,
        "delete_transaction",
        json!({"id": id, "head": head}),
    )
    .await;
    assert_ne!(done["head"], head);
    let last = &call(&client, "get_history", json!({})).await[0];
    assert_eq!(last["actor"], "mcp");
    assert_eq!(last["kind"], "delete");
    let found = call(&client, "search_transactions", json!({})).await;
    assert!(!found["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == id));

    // The head moved, so the same call is a conflict.
    let err = try_call(
        &client,
        "delete_transaction",
        json!({"id": id, "head": head, "dry_run": false}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("head"), "{err}");
    client.cancel().await.unwrap();
}

/// Through MCP: declare an account.
#[tokio::test]
async fn declares_accounts_through_the_rewrite_tools() {
    let server = spawn_rewrite_server().await;
    let client = connect(&server).await;

    let head = store_head(&server).await;
    let preview = call(
        &client,
        "declare_account",
        json!({
            "account": "expenses:pets",
            "tags": [{"key": "ynab-envelope", "value": "Pets"}],
            "synthetic": true,
            "head": head,
        }),
    )
    .await;
    assert!(preview["diff"]
        .as_str()
        .unwrap()
        .contains("+account expenses:pets"));
    rewrite(
        &client,
        "declare_account",
        json!({
            "account": "expenses:pets",
            "tags": [{"key": "ynab-envelope", "value": "Pets"}],
            "synthetic": true,
            "head": head,
        }),
    )
    .await;
    let accounts = call(&client, "list_accounts", json!({"filter": "pets"})).await;
    assert_eq!(accounts[0]["account"], "expenses:pets");
    assert_eq!(accounts[0]["friendly_name"], "Pets");
    assert_eq!(accounts[0]["synthetic"], true);
    assert!(accounts[0]["physical_home"].is_null());
    client.cancel().await.unwrap();
}
#[tokio::test]
async fn list_accounts_tool_returns_real_balances() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let accounts = call(&client, "list_accounts", json!({})).await;
    let checking = accounts
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["account"] == "assets:checking")
        .unwrap();
    assert_eq!(checking["balances"][0]["exact"], "1000.00");
    assert_eq!(checking["friendly_name"], "Everyday Checking");
    client.cancel().await.unwrap();
}
#[tokio::test]
async fn get_version_carries_the_commit_and_source_url() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let version = call(&client, "get_version", json!({})).await;
    let commit = version["commit"].as_str().unwrap();
    assert_eq!(commit.len(), 40, "{version}");
    let url = version["source_url"].as_str().unwrap();
    assert!(url.ends_with(&format!("/tree/{commit}")), "{version}");
    client.cancel().await.unwrap();
}
#[tokio::test]
async fn add_transaction_dry_run_then_commit_and_actor_is_mcp() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let head = call(&client, "get_version", json!({})).await["head"]
        .as_str()
        .unwrap()
        .to_string();

    // dry_run defaults to true: nothing committed yet.
    let mut args = object(groceries(&head, false));
    args.remove("dry_run");
    let preview = call(&client, "add_transaction", Value::Object(args)).await;
    assert_eq!(preview["head"], head, "dry run must not advance HEAD");

    let committed = call(&client, "add_transaction", groceries(&head, false)).await;
    assert_ne!(committed["head"], head);

    let last = support::change(&server, committed["head"].as_str().unwrap()).await;
    assert_eq!(last.actor, "mcp");
    assert_eq!(last.kind, "add");
    assert_eq!(last.details["auth"], "none");
    client.cancel().await.unwrap();
}

/// The error a tool call failed with.
async fn call_error(client: &Client, name: &'static str, args: Value) -> rmcp::ErrorData {
    match client
        .call_tool(CallToolRequestParams::new(name).with_arguments(object(args)))
        .await
    {
        Err(rmcp::service::ServiceError::McpError(e)) => e,
        other => panic!("{name}: expected a tool error, got {other:?}"),
    }
}

/// A negative amount under one unit keeps its sign in the journal, whether
/// given or inferred.
#[tokio::test]
async fn add_transaction_commits_negative_amounts_under_one() {
    let server = spawn_server_with_journal("account income:interest\n", |_| {}).await;
    let client = connect(&server).await;
    let mut head = store_head(&server).await;

    for income in [
        json!({"commodity": "USD", "mantissa": -4, "places": 2}),
        Value::Null,
    ] {
        let mut interest = json!({"account": "income:interest"});
        if !income.is_null() {
            interest["amount"] = income;
        }
        let args = json!({
            "date": "2026-09-30",
            "description": "Interest Paid",
            "postings": [
                {"account": "assets:checking", "amount": {"commodity": "USD", "mantissa": 4, "places": 2}, "status": "cleared"},
                interest,
            ],
            "head": head,
            "dry_run": true,
        });
        let preview = call(&client, "add_transaction", args.clone()).await;
        assert!(
            preview["after"]
                .as_str()
                .unwrap()
                .contains("income:interest  -USD0.04"),
            "{preview}"
        );
        let mut commit = args;
        commit["dry_run"] = json!(false);
        head = call(&client, "add_transaction", commit).await["head"]
            .as_str()
            .unwrap()
            .to_string();
    }

    let balance = call(
        &client,
        "get_balance",
        json!({"account_prefix": "income:interest"}),
    )
    .await;
    assert!(balance.to_string().contains("\"-0.08\""), "{balance}");
    client.cancel().await.unwrap();
}

/// A dry run is checked like the commit: one that would break a later
/// balance assertion fails up front, naming the assertion.
#[tokio::test]
async fn add_transaction_dry_run_fails_like_the_commit_on_a_balance_assertion() {
    let server = spawn_server_with_journal(
        "\n2026-09-28 Migration balance check\n    assets:checking  0.00 USD = 1000.00 USD\n",
        |_| {},
    )
    .await;
    let client = connect(&server).await;
    let head = store_head(&server).await;

    for dry_run in [true, false] {
        let mut args = groceries(&head, dry_run);
        args["date"] = json!("2026-09-25");
        let error = call_error(&client, "add_transaction", args).await;
        assert!(
            error.message.starts_with(
                "At the end of 2026-09-28, assets:checking holds USD980.00, not the USD1000.00 you asserted. (transaction #2, posting 1)"
            ),
            "dry_run={dry_run}: {}",
            error.message
        );
        assert!(error.message.len() < 600, "{}", error.message);
        let data = error.data.expect("error data");
        assert_eq!(data["code"], "validation_failed");
        let details = &data["details"];
        assert_eq!(details["rule"], "assertion");
        assert_eq!(details["transaction"], 2);
        assert_eq!(details["posting"], 0);
        assert_eq!(
            details["failures"][0]["details"],
            "= USD1000.00; calculated USD980.00"
        );
        assert_eq!(details["fix_tool"], "set_balance_assertion");
    }
    assert_eq!(store_head(&server).await, head);

    // A transaction after the assertion's date still goes through.
    let mut args = groceries(&head, true);
    args["date"] = json!("2026-09-29");
    call(&client, "add_transaction", args).await;
    client.cancel().await.unwrap();
}

/// The client's way past an out-of-date assertion: the error names it,
/// set_balance_assertion removes it, the write is retried, and the
/// assertion is set again to the new balance.
#[tokio::test]
async fn an_out_of_date_assertion_is_removed_then_set_again_through_mcp() {
    let server = spawn_server_with_journal(
        "\n2026-09-28 Migration balance check\n    assets:checking  0.00 USD = 1000.00 USD\n",
        support::rewrites,
    )
    .await;
    let client = connect(&server).await;
    assert!(tool_names(&client)
        .await
        .iter()
        .any(|n| n == "set_balance_assertion"));
    let head = store_head(&server).await;
    let mut add = groceries(&head, false);
    add["date"] = json!("2026-09-25");

    let error = call_error(&client, "add_transaction", add.clone()).await;
    let details = error.data.unwrap()["details"].clone();
    assert_eq!(details["fix_tool"], "set_balance_assertion");

    let set = |balance: Value, head: &str| {
        let mut args = json!({
            "transaction": details["transaction"],
            "account": "assets:checking",
            "head": head,
        });
        if !balance.is_null() {
            args["balance"] = balance;
        }
        args
    };
    let preview = call(&client, "set_balance_assertion", set(Value::Null, &head)).await;
    assert!(
        preview["diff"]
            .as_str()
            .unwrap()
            .contains("-    assets:checking  USD0.00 = USD1000.00"),
        "{preview}"
    );
    assert_eq!(store_head(&server).await, head, "a preview changes nothing");
    let head = rewrite(&client, "set_balance_assertion", set(Value::Null, &head)).await["head"]
        .as_str()
        .unwrap()
        .to_string();

    add["head"] = json!(head);
    let head = call(&client, "add_transaction", add).await["head"]
        .as_str()
        .unwrap()
        .to_string();

    let balance = json!({"commodity": "USD", "mantissa": 98000, "places": 2});
    rewrite(&client, "set_balance_assertion", set(balance, &head)).await;
    let books = support::journal(&server).await;
    assert!(
        books.contains("assets:checking  USD0.00 = USD980.00"),
        "{books}"
    );
    client.cancel().await.unwrap();
}
/// A void keeps the transaction and adds its reversal, which says what it
/// reverses.
#[tokio::test]
async fn void_transaction_tool_works_end_to_end() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let head = store_head(&server).await;
    let added = call(&client, "add_transaction", groceries(&head, false)).await;
    let id = added["id"].as_u64().unwrap();

    let voided = call(
        &client,
        "void_transaction",
        json!({"id": id, "head": added["head"], "dry_run": false, "today": "2026-02-02"}),
    )
    .await;
    let txs = call(&client, "search_transactions", json!({})).await;
    let txs = txs["transactions"].as_array().unwrap();
    assert!(txs.iter().any(|t| t["id"] == id), "the original stays");
    let reversal = txs.iter().find(|t| t["id"] == voided["id"]).unwrap();
    assert_eq!(reversal["date"], "2026-02-02");
    assert!(
        reversal["comment"]
            .as_str()
            .unwrap()
            .contains(&format!("#{id}")),
        "{reversal}"
    );
    let balance = call(
        &client,
        "get_balance",
        json!({"account_prefix": "expenses:groceries"}),
    )
    .await;
    assert!(
        balance
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["exact"] == "0.00"),
        "{balance}"
    );
    client.cancel().await.unwrap();
}
#[tokio::test]
async fn edit_transaction_tool_previews_a_diff_then_edits_in_place() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let head = store_head(&server).await;
    let mut add = object(groceries(&head, false));
    add.insert("tags".into(), json!([{"key": "trip", "value": "coast"}]));
    let added = call(&client, "add_transaction", Value::Object(add)).await;
    let head = added["head"].as_str().unwrap().to_string();
    let id = added["id"].as_u64().unwrap();

    let args = json!({
        "id": id,
        "date": "2026-02-01",
        "description": "Grocery Store | milk",
        "postings": [
            {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": 1250, "places": 2}},
            {"account": "assets:checking"}
        ],
        "head": head,
    });
    let mut with_diff = args.clone();
    with_diff["include_diff"] = json!(true);
    let preview = call(&client, "edit_transaction", with_diff).await;
    assert_eq!(preview["head"], head, "dry run must not advance HEAD");
    assert_eq!(preview["dry_run"], true);
    assert!(preview["before"]
        .as_str()
        .unwrap()
        .contains("Grocery Store"));
    assert!(preview["after"].as_str().unwrap().contains("| milk"));
    let diff = preview["diff"].as_str().unwrap();
    assert!(diff.contains("+2026-02-01 Grocery Store | milk"), "{diff}");
    assert!(diff.contains("12.50"), "{diff}");

    let mut commit = object(args);
    commit.insert("dry_run".to_string(), json!(false));
    let committed = call(&client, "edit_transaction", Value::Object(commit)).await;
    assert_ne!(committed["head"], head);
    let last = &call(&client, "get_history", json!({})).await[0];
    assert_eq!(last["actor"], "mcp");
    assert_eq!(last["kind"], "edit");

    // Same id, new amount, and the tag it had is kept.
    let found = call(&client, "search_transactions", json!({"query": "Grocery"})).await;
    let tx = found["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    assert_eq!(tx["memo"], "milk");
    assert!(
        tx["tags"].to_string().contains("coast"),
        "tags were dropped: {tx}"
    );
    client.cancel().await.unwrap();
}

/// A synthetic transaction whose checking posting asserts the balance:
/// transaction 2, after the opening balance.
const ASSERTED: &str = "2026-02-01 Grocery Store\n    expenses:groceries  USD20.00\n    assets:checking  USD-20.00 = USD980.00\n";

/// Over MCP, an edit keeps a posting's balance assertion it doesn't
/// mention, and `balance_assertion` updates its amount.
#[tokio::test]
async fn edit_transaction_tool_keeps_and_updates_balance_assertions() {
    let server = spawn_server_with_journal(ASSERTED, |_| {}).await;
    let client = connect(&server).await;
    let head = store_head(&server).await;
    let edit = |groceries: i64, checking: Value, head: &str| {
        json!({
            "id": 2,
            "date": "2026-02-01",
            "description": "Grocery Store | milk",
            "postings": [
                {"account": "expenses:groceries", "amount": {"commodity": "USD", "mantissa": groceries, "places": 2}},
                checking,
            ],
            "head": head,
            "dry_run": false,
        })
    };

    let kept = call(
        &client,
        "edit_transaction",
        edit(2000, json!({"account": "assets:checking"}), &head),
    )
    .await;
    let head = kept["head"].as_str().unwrap().to_string();
    let diff = call(&client, "get_history_diff", json!({"rev": head})).await;
    let diff = diff["diff"].as_str().unwrap();
    assert!(diff.contains("+2026-02-01 Grocery Store | milk"), "{diff}");
    assert!(
        !diff.contains("-    assets:checking"),
        "the asserted posting must be left as it was: {diff}"
    );

    let refused = try_call(
        &client,
        "edit_transaction",
        edit(2500, json!({"account": "assets:checking"}), &head),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("assertion"), "{refused}");

    let checking = json!({
        "account": "assets:checking",
        "balance_assertion": {"commodity": "USD", "mantissa": 97500, "places": 2},
    });
    let updated = call(&client, "edit_transaction", edit(2500, checking, &head)).await;
    let diff = call(&client, "get_history_diff", json!({"rev": updated["head"]})).await;
    let diff = diff["diff"].as_str().unwrap();
    assert!(
        diff.contains("+    assets:checking  -USD25.00 = USD975.00"),
        "{diff}"
    );
    client.cancel().await.unwrap();
}

/// A synthetic transaction (2, after the opening balance) with a code,
/// comments and tags on several lines and a virtual posting. Its accounts
/// carry account tags (`ynab-account:`, `ynab-envelope:`) that aren't
/// copied onto the postings.
const ANNOTATED: &str = "account budget:groceries\n\n\
2026-02-01 * (#7) Grocery Store  ; trip comment, receipt-no:4471\n\
\x20   ; receipt:yes\n\
\x20   ! assets:checking  USD-20.00  ; card:visa\n\
\x20   expenses:groceries  USD20.00  ; plain words only\n\
\x20   (budget:groceries)  USD-20.00\n";

/// Over MCP, an edit that leaves out `tags` changes only the amounts: the
/// header, comments, tags and the virtual posting are written back as they
/// were, and no account tag is copied onto a posting.
#[tokio::test]
async fn edit_transaction_tool_keeps_comments_codes_and_virtual_postings() {
    let server = spawn_server_with_journal(ANNOTATED, |_| {}).await;
    let client = connect(&server).await;
    let head = store_head(&server).await;
    let usd = |cents: i64| json!({"commodity": "USD", "mantissa": cents, "places": 2});
    let edited = call(
        &client,
        "edit_transaction",
        json!({
            "id": 2,
            "date": "2026-02-01",
            "description": "Grocery Store",
            "postings": [
                {"account": "assets:checking", "status": "pending"},
                {"account": "expenses:groceries", "amount": usd(2500)},
                {"account": "budget:groceries", "amount": usd(-2500)},
            ],
            "head": head,
            "dry_run": false,
        }),
    )
    .await;
    let diff = call(&client, "get_history_diff", json!({"rev": edited["head"]})).await;
    let diff = diff["diff"].as_str().unwrap();
    let changed: Vec<&str> = diff
        .lines()
        .filter(|l| {
            (l.starts_with('-') || l.starts_with('+'))
                && !l.starts_with("---")
                && !l.starts_with("+++")
        })
        .collect();
    assert_eq!(
        changed,
        [
            "-    ! assets:checking  -USD20.00  ; card:visa",
            "-    expenses:groceries  USD20.00  ; plain words only",
            "-    (budget:groceries)  -USD20.00",
            "+    ! assets:checking  -USD25.00  ; card:visa",
            "+    expenses:groceries  USD25.00  ; plain words only",
            "+    (budget:groceries)  -USD25.00",
        ],
        "{diff}"
    );
    client.cancel().await.unwrap();
}

/// The read functions MCP gained for parity: payees, a revision's diff,
/// the books at a revision, and statement files.
#[tokio::test]
async fn reads_payees_history_and_exports_statements_like_the_app() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;
    let before = store_head(&server).await;
    let lent = json!({
        "date": "2026-02-03",
        "description": "Jordan | concert tickets",
        "postings": [
            {"account": "assets:receivable:jordan", "amount": {"commodity": "USD", "mantissa": 4000, "places": 2}},
            {"account": "assets:checking"}
        ],
        "head": before,
        "dry_run": false,
    });
    let added = call(&client, "add_transaction", lent).await;
    let rev = added["head"].as_str().unwrap().to_string();

    let payees = call(&client, "list_payees", json!({"query": "jor"})).await;
    assert_eq!(payees[0]["payee"], "Jordan");
    assert_eq!(payees[0]["last"]["memo"], "concert tickets");

    let diff = call(&client, "get_history_diff", json!({"rev": rev})).await;
    assert!(
        diff["diff"]
            .as_str()
            .unwrap()
            .contains("+2026-02-03 Jordan"),
        "{diff}"
    );

    let then = call(&client, "get_transactions_at", json!({"rev": before})).await;
    assert!(!then.to_string().contains("Jordan"), "{then}");

    let exported = client
        .call_tool(
            CallToolRequestParams::new("export_statement").with_arguments(object(json!({
                "name": "jordan", "from": "2026-01-01", "to": "2026-12-31", "format": "csv",
            }))),
        )
        .await
        .unwrap();
    let file = exported
        .content
        .iter()
        .find_map(|c| c.as_resource())
        .expect("an embedded file");
    let ResourceContents::TextResourceContents {
        uri,
        text,
        mime_type,
        ..
    } = &file.resource
    else {
        panic!("expected CSV text: {file:?}");
    };
    assert!(uri.ends_with(".csv") && uri.contains("jordan"), "{uri}");
    assert_eq!(mime_type.as_deref(), Some("text/csv"));
    assert!(text.contains("2026-02-03,Jordan,40.00"), "{text}");
    client.cancel().await.unwrap();
}
#[tokio::test]
async fn skills_are_resources_named_in_the_instructions() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;

    let instructions = client
        .peer_info()
        .and_then(|info| info.instructions.clone())
        .unwrap_or_default();
    assert!(
        instructions.contains("skill://kabacus-bookkeeping/SKILL.md"),
        "{instructions}"
    );
    assert!(instructions.contains("get_skill_guide"), "{instructions}");

    let resources = client.list_all_resources().await.unwrap();
    let uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    for expected in [
        "skill://kabacus-bookkeeping/SKILL.md",
        "skill://kabacus-bookkeeping/references/workflow.md",
        "skill://kabacus-bookkeeping/references/budget.md",
    ] {
        assert!(uris.contains(&expected), "missing {expected}: {uris:?}");
    }

    let read = client
        .read_resource(ReadResourceRequestParams::new(
            "skill://kabacus-bookkeeping/SKILL.md",
        ))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &read.contents[0] else {
        panic!("expected text: {:?}", read.contents);
    };
    assert!(
        text.starts_with("---\nname: kabacus-bookkeeping\n"),
        "{text}"
    );

    assert!(client
        .read_resource(ReadResourceRequestParams::new(
            "skill://kabacus-bookkeeping/../Cargo.toml",
        ))
        .await
        .is_err());
    client.cancel().await.unwrap();
}
#[tokio::test]
async fn get_skill_guide_serves_skill_md_then_only_the_files_it_links() {
    let server = spawn_server(|_| {}).await;
    let client = connect(&server).await;

    let main = call(&client, "get_skill_guide", json!({})).await;
    assert_eq!(main["file"], "SKILL.md");
    assert!(main["content"]
        .as_str()
        .unwrap()
        .contains("## Reference files"));

    let budget = call(
        &client,
        "get_skill_guide",
        json!({"file": "references/budget.md"}),
    )
    .await;
    assert!(budget["content"].as_str().unwrap().contains("Cash on Hand"));

    let err = try_call(&client, "get_skill_guide", json!({"file": "../Cargo.toml"}))
        .await
        .unwrap_err();
    assert!(err.contains("references/budget.md"), "{err}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn journal_import_is_not_exposed() {
    use tower::ServiceExt as _;
    let server = spawn_rewrite_server().await;
    let client = connect(&server).await;
    assert!(!tool_names(&client)
        .await
        .iter()
        .any(|name| name == "import_journal"));
    assert!(try_call(
        &client,
        "import_journal",
        json!({"main": "books.journal", "files": {}})
    )
    .await
    .is_err());
    assert!(!ENDPOINTS.iter().any(|endpoint| endpoint.path == "/import"));
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/import")
        .header("content-type", "application/json")
        .body(axum::body::Body::from("{}"))
        .unwrap();
    let response = kabacus_server::api::router(server.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    client.cancel().await.unwrap();
}
