// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Funds, card reserves and Cash on Hand through `/mcp`, against a real
//! server on synthetic books.

mod support;

use kabacus_core::{AccountKind, KindRule};
use kabacus_testkit::{spawn_server_with_journal, TestServer};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

type Client = RunningService<RoleClient, ()>;

async fn connect(server: &TestServer) -> Client {
    let config = StreamableHttpClientTransportConfig::with_uri("http://localhost/mcp");
    let transport = StreamableHttpClientTransport::from_unix_socket_with_config(
        server.socket_path.to_str().unwrap(),
        config,
    );
    ().serve(transport).await.unwrap()
}

fn args_object(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(m) => m,
        _ => panic!("expected a JSON object"),
    }
}

async fn try_call(client: &Client, name: &'static str, args: Value) -> Result<Value, String> {
    client
        .call_tool(CallToolRequestParams::new(name).with_arguments(args_object(args)))
        .await
        .map(|r| r.structured_content.expect("structured content"))
        .map_err(|e| e.to_string())
}

async fn call(client: &Client, name: &'static str, args: Value) -> Value {
    try_call(client, name, args)
        .await
        .unwrap_or_else(|e| panic!("{name} failed: {e}"))
}

/// Preview, then commit with the same arguments; the new head.
async fn commit(client: &Client, name: &'static str, mut args: Value) -> (Value, String) {
    let preview = call(client, name, args.clone()).await;
    assert_eq!(
        preview["head"], args["head"],
        "a dry run must not advance HEAD"
    );
    args["dry_run"] = json!(false);
    let result = call(client, name, args).await;
    let head = result["head"].as_str().unwrap().to_string();
    (result, head)
}

fn exact(value: &Value) -> &str {
    value["exact"].as_str().unwrap()
}

fn find<'a>(list: &'a Value, key: &str, value: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|item| item[key] == value)
        .unwrap_or_else(|| panic!("no {key} = {value} in {list}"))
}

fn usd(mantissa: i64) -> Value {
    json!({"commodity": "USD", "mantissa": mantissa, "places": 2})
}

const CARDS: [(&str, &str); 6] = [
    ("rewards-visa", "425.00"),
    ("cashback-mc", "310.75"),
    ("store-card", "120.00"),
    ("travel-card", "550.50"),
    ("spare-card", "0.00"),
    ("dept-store", "-25.00"),
];

const FUNDS: [(&str, &str); 6] = [
    ("assets:bank:savings:emergency-fund", "5000.00"),
    ("assets:bank:checking:home-mortgage", "900.00"),
    ("assets:bank:checking:insurance", "180.00"),
    ("assets:bank:checking:car", "250.00"),
    ("assets:bank:checking:cellphone", "60.00"),
    ("assets:bank:checking:subscriptions", "45.00"),
];

/// Synthetic books on 2026-10-01 with every kind of account, after the card history has been backfilled with reserve lines.
fn worked_example() -> String {
    let mut journal = String::from(
        "\naccount assets:bank:checking\n\
         account assets:bank:savings\n\
         account assets:bank:union-savings\n\
         account assets:cash\n\
         account assets:receivable:hsa\n\
         account liabilities:loans:car\n\
         account liabilities:mortgage:home  ; ynab-account:Home Mortgage\n\
         account expenses:household:shopping\n",
    );
    for (fund, _) in FUNDS {
        journal.push_str(&format!("account {fund}\n"));
    }
    for (card, _) in CARDS {
        journal.push_str(&format!(
            "account liabilities:credit-card:{card}\naccount assets:bank:checking:card-{card}\n"
        ));
    }
    // Checking pays for the card history from free money, leaving 3150.00.
    journal.push_str(
        "\n2026-09-01 Opening\n    assets:bank:checking  4531.25 USD\n    equity:opening\n",
    );
    for (card, amount) in CARDS {
        let negated = match amount.strip_prefix('-') {
            Some(positive) => positive.to_string(),
            None => format!("-{amount}"),
        };
        journal.push_str(&format!(
            "\n2026-09-15 History | {card}\n\
             \x20   expenses:household:shopping  {amount} USD\n\
             \x20   liabilities:credit-card:{card}  {negated} USD\n\
             \x20   assets:bank:checking  {negated} USD\n\
             \x20   assets:bank:checking:card-{card}  {amount} USD\n"
        ));
    }
    journal.push_str("\n2026-10-01 Opening balances\n");
    for (fund, amount) in FUNDS {
        journal.push_str(&format!("    {fund}  {amount} USD\n"));
    }
    journal.push_str(
        "    assets:bank:union-savings  12.00 USD\n\
         \x20   assets:receivable:hsa  1200.00 USD\n\
         \x20   equity:opening\n",
    );
    journal
}

/// The worked example's figures, exactly.
#[tokio::test]
async fn cash_on_hand_matches_the_worked_example() {
    let server = spawn_server_with_journal(&worked_example(), |_| {}).await;
    let client = connect(&server).await;

    let report = call(&client, "cash_on_hand", json!({"as_of": "2026-10-01"})).await;
    assert_eq!(report["as_of"], "2026-10-01");
    assert_eq!(exact(&report["money_on_hand"]), "12178.25");
    // dept-store is in credit: its -25.00 reserve sets nothing aside and
    // comes off checking's free money.
    assert_eq!(exact(&report["future_allocations"]), "7841.25");
    assert_eq!(exact(&report["receivables"]), "1200.00");
    assert_eq!(exact(&report["cash_on_hand"]), "3137.00");
    assert_eq!(report["cash_on_hand"]["formatted"], "$3,137.00");

    let by_kind = &report["by_kind"];
    let money = find(by_kind, "kind", "money");
    assert_eq!(exact(&money["total"]), "3162.00");
    for (account, amount) in [
        ("assets:bank:checking", "3150.00"),
        ("assets:bank:savings", "0.00"),
        ("assets:bank:union-savings", "12.00"),
        ("assets:cash", "0.00"),
    ] {
        assert_eq!(
            exact(&find(&money["accounts"], "account", account)["balance"]),
            amount,
            "{account}"
        );
    }
    assert_eq!(
        exact(&find(by_kind, "kind", "pre-allocation")["total"]),
        "6435.00"
    );
    assert_eq!(
        exact(&find(by_kind, "kind", "debt-reserve")["total"]),
        "1381.25"
    );
    assert_eq!(
        exact(&find(by_kind, "kind", "receivable")["total"]),
        "1200.00"
    );

    let checking = find(&report["homes"], "account", "assets:bank:checking");
    assert_eq!(exact(&checking["free"]), "3125.00");
    assert_eq!(exact(&checking["funds"]), "1435.00");
    assert_eq!(exact(&checking["reserves"]), "1406.25");
    assert_eq!(exact(&checking["inclusive"]), "5966.25");

    for (fund, amount) in FUNDS {
        let line = find(&report["funds"], "account", fund);
        assert_eq!(exact(&line["balance"]), amount, "{fund}");
        assert_eq!(line["home"], fund.rsplit_once(':').unwrap().0);
    }
    for (card, amount) in CARDS {
        let line = find(
            &report["cards"],
            "card",
            &format!("liabilities:credit-card:{card}"),
        );
        assert_eq!(exact(&line["owed"]), amount, "{card}");
        assert_eq!(exact(&line["reserve_balance"]), amount, "{card}");
        assert_eq!(exact(&line["difference"]), "0.00", "{card}");
        assert_eq!(line["unreserved_count"], 0);
    }
    assert_eq!(report["unreserved_count"], 0);
    // dept-store in credit, and the fixture's own `assets:checking`,
    // which no rule matches.
    let codes: Vec<&str> = report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        ["negative-reserve", "unclassified"],
        "{}",
        report["warnings"]
    );

    // The day before, the funds and receivables weren't opened yet.
    let before = call(&client, "cash_on_hand", json!({"as_of": "2026-09-30"})).await;
    assert_eq!(exact(&before["cash_on_hand"]), "3125.00");
    assert_eq!(exact(&before["future_allocations"]), "1406.25");
    let bad = try_call(&client, "cash_on_hand", json!({"as_of": "October"})).await;
    assert!(bad.is_err());
    client.cancel().await.unwrap();
}

/// Kinds, homes, display names and both
/// balances, read-only from the rule table.
#[tokio::test]
async fn accounts_carry_their_kind_home_display_name_and_both_balances() {
    let server = spawn_server_with_journal(&worked_example(), |_| {}).await;
    let client = connect(&server).await;

    let accounts = call(&client, "list_accounts", json!({})).await;
    let checking = find(&accounts, "account", "assets:bank:checking");
    assert_eq!(checking["kind"], "money");
    assert_eq!(checking["home"], "");
    assert_eq!(exact(&checking["balance_exclusive"][0]), "3150.00");
    assert_eq!(exact(&checking["balance_inclusive"][0]), "5966.25");
    assert_eq!(checking["balances"], checking["balance_exclusive"]);

    let insurance = find(&accounts, "account", "assets:bank:checking:insurance");
    assert_eq!(insurance["kind"], "pre-allocation");
    assert_eq!(insurance["home"], "assets:bank:checking");
    assert_eq!(insurance["display_name"], "Insurance");
    let reserve = find(
        &accounts,
        "account",
        "assets:bank:checking:card-rewards-visa",
    );
    assert_eq!(reserve["kind"], "debt-reserve");
    assert_eq!(reserve["home"], "assets:bank:checking");
    assert_eq!(
        find(&accounts, "account", "liabilities:credit-card:spare-card")["kind"],
        "revolving-debt"
    );
    assert_eq!(
        find(&accounts, "account", "assets:receivable:hsa")["kind"],
        "receivable"
    );

    // Display names that collide, told apart by kind.
    let name = |account: &str| find(&accounts, "account", account)["display_name"].clone();
    assert_eq!(name("assets:bank:checking:car"), "Car (fund)");
    assert_eq!(name("liabilities:loans:car"), "Car (loan)");
    assert_eq!(
        name("assets:bank:checking:home-mortgage"),
        "Home Mortgage (fund)"
    );
    assert_eq!(name("liabilities:mortgage:home"), "Home Mortgage (loan)");
    assert_eq!(name("assets:receivable:jordan"), "Jordan (receivable)");
    assert_eq!(name("liabilities:payable:jordan"), "Jordan (payable)");

    let kinds = call(&client, "account_kinds", json!({})).await;
    let rules = kinds["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 13);
    assert_eq!(rules[0]["pattern"], "assets:bank:*:card-*");
    assert_eq!(rules[0]["kind"], "debt-reserve");
    assert_eq!(
        kinds["card_reserve_account"],
        "assets:bank:checking:card-{card}"
    );
    let reserve = find(
        &kinds["accounts"],
        "account",
        "assets:bank:checking:card-spare-card",
    );
    assert_eq!(reserve["rule"], 1);
    assert_eq!(reserve["pattern"], "assets:bank:*:card-*");
    for parent in ["assets", "assets:bank", "equity", "liabilities:credit-card"] {
        let account = find(&kinds["accounts"], "account", parent);
        assert_eq!(account["kind"], "parent", "{parent}");
        assert_eq!(account["rule"], Value::Null, "{parent}");
    }
    assert_eq!(kinds["unclassified"], json!(["assets:checking"]));
    let warning = find(&kinds["warnings"], "code", "unclassified");
    assert_eq!(warning["accounts"], json!(["assets:checking"]));
    client.cancel().await.unwrap();
}

const CARD_BOOKS: &str = "
account assets:bank:checking
account assets:bank:checking:insurance
account assets:bank:checking:card-rewards-visa
account assets:bank:savings
account assets:bank:savings:emergency-fund
account liabilities:credit-card:rewards-visa
account expenses:bills:insurance
account expenses:household:groceries

2026-09-01 Opening
    assets:bank:checking  2000.00 USD
    assets:bank:savings:emergency-fund  500.00 USD
    equity:opening
";

const VISA: &str = "liabilities:credit-card:rewards-visa";
const VISA_RESERVE: &str = "assets:bank:checking:card-rewards-visa";
const INSURANCE: &str = "assets:bank:checking:insurance";

/// Allocating checks kinds and homes; any posting may go to a fund.
#[tokio::test]
async fn allocate_moves_money_within_one_home_by_kind() {
    let server = spawn_server_with_journal(CARD_BOOKS, |_| {}).await;
    let client = connect(&server).await;
    let head = support::store_head(&server).await;

    let (allocated, head) = commit(
        &client,
        "allocate",
        json!({
            "from": "assets:bank:checking",
            "to": INSURANCE,
            "amount": "300.00",
            "date": "2026-09-02",
            "head": head,
        }),
    )
    .await;
    assert!(
        allocated["after"]
            .as_str()
            .unwrap()
            .contains("Allocate | to Insurance"),
        "{allocated}"
    );
    let last = &call(&client, "get_history", json!({})).await[0];
    assert_eq!(last["actor"], "mcp");
    assert_eq!(last["kind"], "add");

    for dry_run in [true, false] {
        let error = try_call(
            &client,
            "allocate",
            json!({
                "from": INSURANCE,
                "to": "assets:bank:savings:emergency-fund",
                "amount": "50.00",
                "date": "2026-09-02",
                "head": head,
                "dry_run": dry_run,
            }),
        )
        .await
        .unwrap_err();
        assert!(error.contains("different physical accounts"), "{error}");
        assert!(error.contains("explicitly confirms"), "{error}");
        assert!(error.contains("already happened"), "{error}");
        assert_eq!(support::store_head(&server).await, head);
    }

    for (to, expected) in [
        ("expenses:bills:insurance", "is expense"),
        (VISA, "is revolving-debt"),
    ] {
        let error = try_call(
            &client,
            "allocate",
            json!({"from": INSURANCE, "to": to, "amount": "1.00", "date": "2026-09-02", "head": head}),
        )
        .await
        .unwrap_err();
        assert!(error.contains(expected), "{to}: {error}");
    }
    let error = try_call(
        &client,
        "allocate",
        json!({"from": "assets:bank:checking", "to": INSURANCE, "amount": "-1", "date": "2026-09-02", "head": head}),
    )
    .await
    .unwrap_err();
    assert!(error.contains("more than zero"), "{error}");

    // An ordinary transaction may spend from a fund directly.
    let (_, _head) = commit(
        &client,
        "add_transaction",
        json!({
            "date": "2026-09-03",
            "description": "Insurer",
            "postings": [
                {"account": "expenses:bills:insurance", "amount": usd(12000)},
                {"account": INSURANCE},
            ],
            "head": head,
        }),
    )
    .await;
    let report = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(
        exact(&find(&report["funds"], "account", INSURANCE)["balance"]),
        "180.00"
    );
    assert_eq!(exact(&report["cash_on_hand"]), "1700.00");
    client.cancel().await.unwrap();
}

/// Setting money aside into a new fund declares the fund in the same
/// commit, as an ordinary write (not a rewrite): only a new
/// fund directly in the money account the money comes from.
#[tokio::test]
async fn allocate_can_set_money_aside_into_a_new_fund() {
    let server = spawn_server_with_journal(CARD_BOOKS, |c| c.allow_rewrite = false).await;
    let client = connect(&server).await;
    let head = support::store_head(&server).await;
    let vacation = "assets:bank:checking:vacation";

    let (allocated, _head) = commit(
        &client,
        "allocate",
        json!({
            "from": "assets:bank:checking",
            "to": "Vacation",
            "new_fund": true,
            "amount": "250.00",
            "date": "2026-09-02",
            "head": head,
        }),
    )
    .await;
    assert!(
        allocated["after"].as_str().unwrap().contains(vacation),
        "{allocated}"
    );
    assert!(
        support::book(&server).await.accounts.contains_key(vacation),
        "{vacation} isn't declared"
    );
    let report = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(
        exact(&find(&report["funds"], "account", vacation)["balance"]),
        "250.00"
    );
    let last = &call(&client, "get_history", json!({})).await[0];
    assert_eq!(last["kind"], "add", "one ordinary commit: {last}");

    let head = support::store_head(&server).await;
    for (from, to, expected) in [
        ("assets:bank:checking", vacation, "already exists"),
        (
            "assets:bank:checking",
            "assets:bank:savings:vacation",
            "directly in",
        ),
        (
            INSURANCE,
            "assets:bank:checking:insurance:extra",
            "directly in",
        ),
        (
            "assets:bank:checking",
            "assets:bank:checking:card-spare",
            "not a fund",
        ),
    ] {
        let error = try_call(
            &client,
            "allocate",
            json!({"from": from, "to": to, "new_fund": true, "amount": "1.00", "date": "2026-09-02", "head": head}),
        )
        .await
        .unwrap_err();
        assert!(error.contains(expected), "{to}: {error}");
    }
    // Without new_fund, an undeclared fund is still refused.
    let error = try_call(
        &client,
        "allocate",
        json!({"from": "assets:bank:checking", "to": "assets:bank:checking:garden", "amount": "1.00", "date": "2026-09-02", "head": head}),
    )
    .await
    .unwrap_err();
    assert!(error.contains("garden"), "{error}");
    client.cancel().await.unwrap();
}

/// A card transaction without its reserve lines is written, flagged
/// everywhere it shows up, and fixed with an ordinary edit.
#[tokio::test]
async fn unreserved_card_transactions_are_flagged_not_refused() {
    let server = spawn_server_with_journal(CARD_BOOKS, |_| {}).await;
    let client = connect(&server).await;
    let head = support::store_head(&server).await;
    let groceries = |head: &str, reserved: bool| {
        let mut postings = vec![
            json!({"account": "expenses:household:groceries", "amount": usd(4000)}),
            json!({"account": VISA, "amount": usd(-4000)}),
        ];
        if reserved {
            postings.push(json!({"account": "assets:bank:checking", "amount": usd(-4000)}));
            postings.push(json!({"account": VISA_RESERVE, "amount": usd(4000)}));
        }
        json!({
            "date": "2026-09-05",
            "description": "Corner Market",
            "postings": postings,
            "head": head,
        })
    };

    let preview = call(&client, "add_transaction", groceries(&head, false)).await;
    assert_eq!(preview["unreserved"], true, "{preview}");
    assert_eq!(preview["missing_reserve"][0]["card"], VISA);
    assert_eq!(preview["missing_reserve"][0]["reserve"], VISA_RESERVE);
    assert_eq!(exact(&preview["missing_reserve"][0]["amount"]), "40.00");
    let (added, head) = commit(&client, "add_transaction", groceries(&head, false)).await;
    assert_eq!(added["unreserved"], true);
    let id = added["id"].as_u64().unwrap();

    let all = call(&client, "search_transactions", json!({})).await;
    assert_eq!(all["transactions"].as_array().unwrap().len(), 3);
    assert_eq!(all["total_count"], 3);
    let unreserved = call(
        &client,
        "search_transactions",
        json!({"unreserved_only": true}),
    )
    .await;
    let unreserved = unreserved["transactions"].as_array().unwrap();
    assert_eq!(unreserved.len(), 1);
    assert_eq!(unreserved[0]["id"], id);
    assert_eq!(unreserved[0]["unreserved"], true);
    let register = call(&client, "get_register", json!({"account": VISA})).await;
    let lines = &register["lines"];
    assert_eq!(lines[0]["unreserved"], true);
    assert_eq!(exact(&lines[0]["missing_reserve"][0]["amount"]), "40.00");

    let report = call(&client, "cash_on_hand", json!({"detail": "full"})).await;
    assert_eq!(report["unreserved_count"], 1);
    let card = find(&report["cards"], "card", VISA);
    assert_eq!(card["unreserved_count"], 1);
    assert_eq!(card["unreserved"][0]["id"], id);
    // Owed minus reserve, which the flagged transaction explains.
    assert_eq!(exact(&card["difference"]), "40.00");
    let summary = call(&client, "cash_on_hand", json!({})).await;
    let card = find(&summary["cards"], "card", VISA);
    assert!(card.get("unreserved").is_none(), "{card}");
    assert_eq!(card["unreserved_count"], 1);
    let mismatch = find(&report["warnings"], "code", "reserve-mismatch");
    assert!(
        mismatch["message"]
            .as_str()
            .unwrap()
            .contains("1 transaction on the card doesn't move its reserve"),
        "{mismatch}"
    );

    // The fix is an ordinary in-place edit adding the reserve lines.
    let mut edit = groceries(&head, true);
    edit["id"] = json!(id);
    let (edited, _head) = commit(&client, "edit_transaction", edit).await;
    assert!(edited.get("unreserved").is_none(), "{edited}");
    let unreserved = call(
        &client,
        "search_transactions",
        json!({"unreserved_only": true}),
    )
    .await;
    assert_eq!(unreserved["transactions"], json!([]));
    assert_eq!(unreserved["total_count"], 0);
    let report = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(report["unreserved_count"], 0);
    assert!(report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|w| w["code"] != "reserve-mismatch"));
    client.cancel().await.unwrap();
}

/// Purchases with their reserve lines, a payment from the
/// reserve, and the reserve split by source with payments FIFO.
#[tokio::test]
async fn card_purchases_and_payments_keep_the_reserve_equal_to_the_card() {
    let server = spawn_server_with_journal(CARD_BOOKS, |_| {}).await;
    let client = connect(&server).await;
    let head = support::store_head(&server).await;
    let (_, head) = commit(
        &client,
        "allocate",
        json!({"from": "assets:bank:checking", "to": INSURANCE, "amount": "300.00", "date": "2026-09-02", "head": head}),
    )
    .await;

    let purchase = |head: &str, expense: &str, amount: &str, source: &str, date: &str| {
        json!({
            "card": VISA,
            "expense_account": expense,
            "amount": amount,
            "date": date,
            "description": "Shop | test",
            "source": source,
            "tags": [{"key": "trip", "value": "coast"}],
            "head": head,
        })
    };
    let (groceries, head) = commit(
        &client,
        "record_card_purchase",
        purchase(
            &head,
            "expenses:household:groceries",
            "40.00",
            "free",
            "2026-09-05",
        ),
    )
    .await;
    let rendered = groceries["after"].as_str().unwrap();
    for line in [
        "expenses:household:groceries  USD40.00",
        "liabilities:credit-card:rewards-visa  -USD40.00",
        "assets:bank:checking  -USD40.00",
        "assets:bank:checking:card-rewards-visa  USD40.00",
        "trip:coast",
    ] {
        assert!(rendered.contains(line), "{line} in {rendered}");
    }
    assert!(groceries.get("unreserved").is_none());
    let (insurer, head) = commit(
        &client,
        "record_card_purchase",
        purchase(
            &head,
            "expenses:bills:insurance",
            "100.00",
            INSURANCE,
            "2026-09-06",
        ),
    )
    .await;
    assert!(insurer["after"]
        .as_str()
        .unwrap()
        .contains("assets:bank:checking:insurance  -USD100.00"));
    let (_, head) = commit(
        &client,
        "record_card_purchase",
        purchase(
            &head,
            "expenses:household:groceries",
            "25.00",
            "free",
            "2026-09-07",
        ),
    )
    .await;
    let error = try_call(
        &client,
        "record_card_purchase",
        purchase(
            &head,
            "expenses:household:groceries",
            "1.00",
            "assets:bank:checking",
            "2026-09-07",
        ),
    )
    .await
    .unwrap_err();
    assert!(error.contains("not a fund"), "{error}");
    let error = try_call(
        &client,
        "record_card_purchase",
        json!({"card": "assets:bank:checking", "expense_account": "expenses:household:groceries", "amount": "1", "date": "2026-09-07", "description": "x", "source": "free", "head": head}),
    )
    .await
    .unwrap_err();
    assert!(error.contains("not a card"), "{error}");

    let (paid, _head) = commit(
        &client,
        "pay_card",
        json!({"card": VISA, "amount": "50.00", "date": "2026-09-20", "head": head}),
    )
    .await;
    let rendered = paid["after"].as_str().unwrap();
    assert!(rendered.contains("Rewards Visa | Payment"), "{rendered}");
    assert!(rendered.contains("liabilities:credit-card:rewards-visa  USD50.00"));
    assert!(rendered.contains("assets:bank:checking:card-rewards-visa  -USD50.00"));

    let reserves = call(&client, "card_reserves", json!({"card": VISA})).await;
    assert!(reserves["rule"].as_str().unwrap().contains("FIFO"));
    let card = &reserves["cards"][0];
    assert_eq!(card["card"], VISA);
    assert_eq!(exact(&card["owed"]), "115.00");
    assert_eq!(exact(&card["reserve_balance"]), "115.00");
    assert_eq!(card["matches"], true);
    // The $50 payment paid the oldest first: all $40 of groceries, then
    // $10 of the insurance purchase.
    assert_eq!(card["sources"].as_array().unwrap().len(), 2);
    let free = find(&card["sources"], "source", "free");
    assert_eq!(exact(&free["amount"]), "25.00");
    let fund = find(&card["sources"], "source", "fund");
    assert_eq!(fund["fund"], INSURANCE);
    assert_eq!(fund["name"], "Insurance (fund)");
    assert_eq!(exact(&fund["amount"]), "90.00");
    assert!(
        try_call(&client, "card_reserves", json!({"card": INSURANCE}))
            .await
            .is_err()
    );
    assert_eq!(
        call(&client, "card_reserves", json!({})).await["cards"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let report = call(&client, "cash_on_hand", json!({})).await;
    // 2000 - 300 allocated - 40 - 25 spent from free money.
    assert_eq!(exact(&report["cash_on_hand"]), "1635.00");
    assert_eq!(exact(&report["future_allocations"]), "815.00");
    assert_eq!(exact(&report["money_on_hand"]), "2450.00");
    client.cancel().await.unwrap();
}

/// A server whose rewrite gates are open for the local principal.
async fn spawn_rewrite_server(journal: &str) -> TestServer {
    spawn_server_with_journal(journal, support::rewrites).await
}

const ASSERTED: &str = "
account assets:bank:checking
account assets:bank:checking:insurance
account liabilities:credit-card:spare-card

2026-09-01 Opening
    assets:bank:checking  1000.00 USD
    equity:opening

2026-09-28 Migration balance check
    assets:bank:checking  0.00 USD = 1000.00 USD
    liabilities:credit-card:spare-card  0.00 USD = 0.00 USD
";

/// Money account assertions become inclusive, so a fund no
/// longer breaks them; new ones are written inclusive.
#[tokio::test]
async fn money_assertions_are_converted_to_inclusive_and_written_inclusive() {
    let server = spawn_rewrite_server(ASSERTED).await;
    let client = connect(&server).await;
    let head = support::store_head(&server).await;

    let report = call(&client, "cash_on_hand", json!({})).await;
    let warning = find(&report["warnings"], "code", "exclusive-assertion");
    assert_eq!(warning["accounts"], json!(["assets:bank:checking"]));
    assert!(warning["message"]
        .as_str()
        .unwrap()
        .contains("convert_money_assertions"));

    // Before: moving money into a fund breaks the exclusive assertion.
    let allocation = |head: &str| json!({"from": "assets:bank:checking", "to": INSURANCE, "amount": "100.00", "date": "2026-09-20", "head": head});
    let error = try_call(&client, "allocate", allocation(&head))
        .await
        .unwrap_err();
    assert!(error.contains("you asserted"), "{error}");

    let preview = call(&client, "convert_money_assertions", json!({"head": head})).await;
    let diff = preview["diff"].as_str().unwrap();
    assert!(
        diff.contains("+    assets:bank:checking  USD0.00 =* USD1000.00"),
        "{diff}"
    );
    // Not a money account: unchanged.
    assert!(
        !diff.contains("+    liabilities:credit-card:spare-card"),
        "{diff}"
    );
    assert_eq!(
        support::store_head(&server).await,
        head,
        "a preview changes nothing"
    );
    let head = support::rewrite(&client, "convert_money_assertions", json!({"head": head})).await
        ["head"]
        .as_str()
        .unwrap()
        .to_string();
    let last = &call(&client, "get_history", json!({})).await[0];
    assert_eq!(last["kind"], "rewrite");

    let (_, head) = commit(&client, "allocate", allocation(&head)).await;
    let report = call(&client, "cash_on_hand", json!({})).await;
    assert!(report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|w| w["code"] != "exclusive-assertion"));
    let again = try_call(&client, "convert_money_assertions", json!({"head": head}))
        .await
        .unwrap_err();
    assert!(again.contains("already inclusive"), "{again}");

    // New assertions on a money account are inclusive; on others, plain.
    let preview = call(
        &client,
        "add_transaction",
        json!({
            "date": "2026-09-30",
            "description": "Check",
            "postings": [
                {"account": "assets:bank:checking", "amount": usd(0), "balance_assertion": usd(100000)},
                {"account": "liabilities:credit-card:spare-card", "amount": usd(0), "balance_assertion": usd(0)},
            ],
            "head": head,
        }),
    )
    .await;
    let rendered = preview["after"].as_str().unwrap();
    assert!(
        rendered.contains("assets:bank:checking  USD0.00 =* USD1000.00"),
        "{rendered}"
    );
    assert!(
        rendered.contains("liabilities:credit-card:spare-card  USD0.00 = USD0.00"),
        "{rendered}"
    );

    let opening = support::book(&server)
        .await
        .transactions
        .iter()
        .find(|t| t.description == "Opening" && t.date.to_string() == "2026-09-01")
        .unwrap()
        .id;
    let preview = call(
        &client,
        "set_balance_assertion",
        json!({"transaction": opening, "account": "assets:bank:checking", "balance": usd(100000), "head": head}),
    )
    .await;
    assert!(
        preview["diff"]
            .as_str()
            .unwrap()
            .contains("+    assets:bank:checking  USD1000.00 =* "),
        "{preview}"
    );
    client.cancel().await.unwrap();
}

/// The rule table is the config's, and a reload picks up a
/// changed one; a broken file keeps the rules in use.
#[tokio::test]
async fn kind_rules_come_from_the_config_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("kabacus.toml");
    let rules = |kind: &str| {
        format!("card_reserve_account = \"assets:reserves:{{card}}\"\n\n[[account_kinds]]\npattern = \"assets:checking\"\nkind = \"{kind}\"\n")
    };
    std::fs::write(&file, rules("money")).unwrap();
    let config = kabacus_server::config::Config::load_from_file(&file).unwrap();
    let server = spawn_server_with_journal("", move |cfg| {
        cfg.config_file = config.config_file.clone();
        cfg.account_kinds = config.account_kinds.clone();
        cfg.card_reserve_account = config.card_reserve_account.clone();
    })
    .await;
    assert_eq!(
        server.state.kind_table().rules,
        [KindRule::new("assets:checking", AccountKind::Money)]
    );
    let client = connect(&server).await;
    let kinds = call(&client, "account_kinds", json!({})).await;
    assert_eq!(kinds["rules"].as_array().unwrap().len(), 1);
    assert_eq!(kinds["card_reserve_account"], "assets:reserves:{card}");
    assert_eq!(
        find(&kinds["accounts"], "account", "assets:checking")["kind"],
        "money"
    );
    let report = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(exact(&report["cash_on_hand"]), "1000.00");

    std::fs::write(&file, rules("non-liquid")).unwrap();
    server.state.reload_kinds().unwrap();
    let kinds = call(&client, "account_kinds", json!({})).await;
    assert_eq!(
        find(&kinds["accounts"], "account", "assets:checking")["kind"],
        "non-liquid"
    );
    let report = call(&client, "cash_on_hand", json!({})).await;
    assert_eq!(exact(&report["cash_on_hand"]), "0.00");

    std::fs::write(&file, rules("parent")).unwrap();
    assert!(server.state.reload_kinds().is_err());
    let kinds = call(&client, "account_kinds", json!({})).await;
    assert_eq!(
        find(&kinds["accounts"], "account", "assets:checking")["kind"],
        "non-liquid"
    );
    client.cancel().await.unwrap();
}

/// 4.9: amounts under one keep their sign with a symbol commodity too.
#[tokio::test]
async fn a_dollar_amount_under_one_keeps_its_sign() {
    let server = spawn_server_with_journal(
        "commodity $1,000.00\naccount income:interest\n\n\
             2026-09-01 Cash\n    assets:checking  $10.00\n    equity:opening\n",
        |_| {},
    )
    .await;
    let client = connect(&server).await;
    let head = support::store_head(&server).await;
    let dollars = |m: i64| json!({"commodity": "$", "mantissa": m, "places": 2});
    let (added, _) = commit(
        &client,
        "add_transaction",
        json!({
            "date": "2026-09-30",
            "description": "Interest Paid",
            "postings": [
                {"account": "assets:checking", "amount": dollars(4)},
                {"account": "income:interest", "amount": dollars(-4)},
            ],
            "head": head,
        }),
    )
    .await;
    assert!(
        added["after"]
            .as_str()
            .unwrap()
            .contains("income:interest  -$0.04"),
        "{added}"
    );
    let books = support::journal(&server).await;
    assert!(books.contains("income:interest  -$0.04"), "{books}");
    let balance = call(
        &client,
        "get_balance",
        json!({"account_prefix": "income:interest"}),
    )
    .await;
    assert_eq!(balance[0]["exact"], "-0.04");
    assert_eq!(balance[0]["formatted"], "-$0.04");
    client.cancel().await.unwrap();
}

/// One HTTP request to the server's unix socket: status and JSON body.
async fn http(
    server: &TestServer,
    method: &str,
    path: &str,
    if_match: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    use http_body_util::BodyExt;
    use hyper_util::client::legacy::Client;
    use hyperlocal::UnixClientExt;
    let client: Client<hyperlocal::UnixConnector, http_body_util::Full<bytes::Bytes>> =
        Client::unix();
    let mut request = hyper::Request::builder()
        .method(method)
        .uri(hyperlocal::Uri::new(&server.socket_path, path))
        .header("content-type", "application/json");
    if let Some(head) = if_match {
        request = request.header("if-match", head);
    }
    let body = body
        .map(|b| serde_json::to_vec(&b).unwrap())
        .unwrap_or_default();
    let response = client
        .request(
            request
                .body(http_body_util::Full::new(bytes::Bytes::from(body)))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The app's API has the same reports and writes as the MCP tools.
#[tokio::test]
async fn the_http_api_serves_the_same_funds_and_cards() {
    let server = spawn_server_with_journal(CARD_BOOKS, |_| {}).await;
    let head = support::store_head(&server).await;

    let (status, allocated) = http(
        &server,
        "POST",
        "/allocations",
        Some(&head),
        Some(json!({"from": "assets:bank:checking", "to": INSURANCE, "amount": "300.00", "date": "2026-09-02"})),
    )
    .await;
    assert_eq!(status, 201, "{allocated}");
    let (status, bought) = http(
        &server,
        "POST",
        "/cards/purchases",
        allocated["head"].as_str(),
        Some(json!({"card": VISA, "expense_account": "expenses:bills:insurance", "amount": "100.00", "date": "2026-09-03", "description": "Insurer", "source": INSURANCE})),
    )
    .await;
    assert_eq!(status, 201, "{bought}");
    let (status, preview) = http(
        &server,
        "POST",
        "/cards/payments",
        bought["head"].as_str(),
        Some(json!({"card": VISA, "amount": "100.00", "date": "2026-09-20", "dry_run": true})),
    )
    .await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(preview["head"], bought["head"]);

    let (status, report) = http(&server, "GET", "/cash-on-hand?as_of=2026-09-30", None, None).await;
    assert_eq!(status, 200, "{report}");
    assert_eq!(exact(&report["cash_on_hand"]), "1700.00");
    let (status, reserves) = http(&server, "GET", "/cards/reserves", None, None).await;
    assert_eq!(status, 200, "{reserves}");
    assert_eq!(reserves["cards"][0]["sources"][0]["fund"], INSURANCE);
    let (status, kinds) = http(&server, "GET", "/accounts/kinds", None, None).await;
    assert_eq!(status, 200, "{kinds}");
    assert_eq!(
        find(&kinds["accounts"], "account", INSURANCE)["kind"],
        "pre-allocation"
    );
    let (status, error) = http(
        &server,
        "POST",
        "/allocations",
        bought["head"].as_str(),
        Some(json!({"from": INSURANCE, "to": "assets:bank:savings:emergency-fund", "amount": "1", "date": "2026-09-04"})),
    )
    .await;
    assert_eq!(status, 422, "{error}");
    assert_eq!(error["rule"], "allocate");
    // Without the rewrite role, the conversion is refused.
    let (status, _) = http(
        &server,
        "POST",
        "/assertions/inclusive",
        Some(&head),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 403);
}

/// R6.1: monthly budgeting is gone from both surfaces.
#[tokio::test]
async fn the_monthly_budget_is_gone() {
    let server = spawn_server_with_journal(CARD_BOOKS, |_| {}).await;
    let client = connect(&server).await;
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for gone in ["budget", "move_budget"] {
        assert!(!names.iter().any(|n| n == gone), "{gone} in {names:?}");
    }
    for present in ["allocate", "cash_on_hand", "card_reserves"] {
        assert!(names.iter().any(|n| n == present), "{present} missing");
    }
    let head = support::store_head(&server).await;
    let (status, _) = http(&server, "GET", "/budget?month=2026-09", None, None).await;
    assert_eq!(status, 404);
    let (status, _) = http(
        &server,
        "POST",
        "/budget/moves",
        Some(&head),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 404);
    client.cancel().await.unwrap();
}

/// R6.3: the bundled guide teaches funds, reserves, the patterns and Cash
/// on Hand, and nothing of the monthly budget.
#[tokio::test]
async fn the_skill_guide_describes_funds_reserves_and_cash_on_hand() {
    let server = spawn_server_with_journal("", |_| {}).await;
    let client = connect(&server).await;
    let guide = call(
        &client,
        "get_skill_guide",
        json!({"file": "references/budget.md"}),
    )
    .await;
    let text = guide["content"].as_str().unwrap();
    for wanted in [
        "= Money on Hand - Future Allocations - Receivables",
        "allocate(",
        "record_card_purchase(",
        "pay_card(",
        "card_reserves(",
        "unreserved",
        "convert_money_assertions",
        "Purchase from a fund on a card",
        "Mortgage or loan from a fund",
    ] {
        assert!(text.contains(wanted), "{wanted:?} missing from the guide");
    }
    for gone in ["Ready to Assign", "move_budget", "envelope:", "Overspent"] {
        assert!(!text.contains(gone), "{gone:?} still in the guide");
    }
    let main = call(&client, "get_skill_guide", json!({})).await;
    let main = main["content"].as_str().unwrap();
    assert!(main.contains("`allocate`"), "{main}");
    assert!(!main.contains("move_budget"), "{main}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn named_reserve_http_preview_does_not_write_and_save_keeps_totals_exact() {
    let server = spawn_server_with_journal(CARD_BOOKS, |_| {}).await;
    let head = support::store_head(&server).await;
    let (_, before) = http(&server, "GET", "/cash-on-hand", None, None).await;
    let mut body = json!({
        "from": "assets:bank:checking", "to": "Emergency Savings",
        "new_fund": true, "amount": "100.25", "date": "2026-09-02", "dry_run": true
    });
    let (status, preview) = http(
        &server,
        "POST",
        "/allocations",
        Some(&head),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(support::store_head(&server).await, head);
    assert!(!support::book(&server)
        .await
        .accounts
        .contains_key("assets:bank:checking:emergency-savings"));
    body["dry_run"] = json!(false);
    let (status, saved) = http(&server, "POST", "/allocations", Some(&head), Some(body)).await;
    assert_eq!(status, 201, "{saved}");
    let (_, after) = http(&server, "GET", "/cash-on-hand", None, None).await;
    assert_eq!(after["money_on_hand"], before["money_on_hand"]);
    assert_eq!(
        exact(
            &find(
                &after["funds"],
                "account",
                "assets:bank:checking:emergency-savings"
            )["balance"]
        ),
        "100.25"
    );
    support::assert_checks(&server).await;
}
