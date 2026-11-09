// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Through `/mcp`: the report tools, cash flow, receivables activity,
//! equity conversion and `reconcile`.

mod support;

use kabacus_testkit::{spawn_server_with_journal, TestServer};
use serde_json::json;
use std::collections::BTreeMap;
use support::*;

const ACCOUNTS: &str = "account assets:bank:checking\n\
    account assets:bank:checking:insurance\n\
    account assets:receivable:sam\n\
    account assets:investments:brokerage\n\
    account liabilities:loans:house\n\
    account liabilities:credit-card:visa\n\
    account assets:bank:checking:card-visa\n\
    account income:salary\n\
    account income:interest\n\
    account expenses:food:groceries\n\
    account expenses:food:dining\n\
    account expenses:interest:house\n\
    account equity:adjustments\n";

/// A year of synthetic books, Oct 2025 to Sep 2026.
fn year() -> String {
    let mut out = String::from(
        "2025-09-30 Opening\n    assets:bank:checking  USD10000.00\n    liabilities:loans:house  USD-100000.00\n    equity:opening\n\n",
    );
    for m in 0..12 {
        let (y, mo) = if m < 3 { (2025, 10 + m) } else { (2026, m - 2) };
        let d = |day: u32| format!("{y}-{mo:02}-{day:02}");
        out.push_str(&format!(
            "{} Pay\n    assets:bank:checking  USD{}.00\n    income:salary\n\n",
            d(1),
            3000 + m * 10
        ));
        out.push_str(&format!(
            "{} Groceries\n    expenses:food:groceries  USD{}.{:02}\n    assets:bank:checking\n\n",
            d(5),
            100 + m * 3,
            m * 7 % 100
        ));
        out.push_str(&format!(
            "{} Dinner, Sam's half\n    expenses:food:dining  USD40.00\n    assets:receivable:sam  USD40.00\n    assets:bank:checking  USD-80.00\n\n",
            d(9)
        ));
        out.push_str(&format!(
            "{} Sam repays\n    assets:bank:checking  USD40.00\n    assets:receivable:sam\n\n",
            d(20)
        ));
        out.push_str(&format!(
            "{} Mortgage\n    liabilities:loans:house  USD380.00\n    assets:bank:checking\n\n",
            d(15)
        ));
        out.push_str(&format!(
            "{} Co-owner's share\n    assets:receivable:sam  USD190.00\n    liabilities:loans:house\n\n",
            d(15)
        ));
        out.push_str(&format!(
            "{} Interest accrual\n    expenses:interest:house  USD300.00\n    liabilities:loans:house\n\n",
            d(28)
        ));
        out.push_str(&format!(
            "{} Interest\n    assets:bank:checking  USD0.04\n    income:interest\n\n",
            d(27)
        ));
    }
    out.push_str("2026-09-10 Invest\n    assets:investments:brokerage  USD100.00\n    assets:bank:checking\n\n");
    out.push_str("2026-09-11 Insurance fund\n    assets:bank:checking:insurance  USD250.00\n    assets:bank:checking\n\n");
    out.push_str("2026-09-12 Card dinner\n    expenses:food:dining  USD30.00\n    liabilities:credit-card:visa\n    assets:bank:checking  USD-30.00\n    assets:bank:checking:card-visa  USD30.00\n\n");
    out
}

async fn spawn_year() -> (TestServer, Client) {
    let server = spawn_server_with_journal(&format!("{ACCOUNTS}\n{}", year()), |_| {}).await;
    let client = connect(&server).await;
    (server, client)
}

fn cents(s: &str) -> i64 {
    let negative = s.starts_with('-');
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let places = s.split('.').nth(1).map_or(0, str::len);
    let n: i64 = digits.parse().unwrap_or(0) * 10i64.pow(2 - places.min(2) as u32);
    if negative {
        -n
    } else {
        n
    }
}

/// The monthly income statement matches the synthetic books to the cent,
/// and net = net_before_conversion -
/// equity conversion in every period.
#[tokio::test]
async fn income_statement_matches_monthly_entries() {
    let (_server, client) = spawn_year().await;
    let report = call(
        &client,
        "income_statement",
        json!({"from": "2025-10-01", "to": "2026-09-30", "interval": "month"}),
    )
    .await;
    assert_eq!(report["periods"].as_array().unwrap().len(), 12);
    let expected = BTreeMap::from([
        (
            "income:salary".to_string(),
            (0..12).map(|m| -(300000 + m * 1000)).collect::<Vec<i64>>(),
        ),
        ("income:interest".to_string(), vec![-4; 12]),
        (
            "expenses:food".to_string(),
            (0..12)
                .map(|m| 14000 + m * 300 + m * 7 % 100 + if m == 11 { 3000 } else { 0 })
                .collect(),
        ),
        ("expenses:interest".to_string(), vec![30000; 12]),
    ]);
    let mut ours: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (section, sign) in [("income", -1), ("expenses", 1)] {
        for row in report[section].as_array().unwrap() {
            ours.insert(
                row["account"].as_str().unwrap().to_string(),
                row["amounts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| sign * cents(a.as_str().unwrap()))
                    .collect(),
            );
        }
    }
    assert_eq!(ours, expected);
    let net = report["net"].as_array().unwrap();
    let before = report["net_before_conversion"].as_array().unwrap();
    let conversion = report["equity_conversion"]["totals"].as_array().unwrap();
    for i in 0..12 {
        assert_eq!(
            cents(before[i].as_str().unwrap()) - cents(conversion[i].as_str().unwrap()),
            cents(net[i].as_str().unwrap())
        );
    }
    // A payment of 380.00 less a 190.00 share less 300.00 of interest.
    assert_eq!(conversion[0], "-110.00");
    let plain = call(
        &client,
        "income_statement",
        json!({"from": "2025-10-01", "to": "2026-09-30", "interval": "month", "equity_conversion": false}),
    )
    .await;
    assert!(plain.get("equity_conversion").is_none());
    assert_eq!(plain["net"], plain["net_before_conversion"]);
    assert_eq!(plain["expenses"], report["expenses"]);
}

/// 380.00 paid, 190.00 of it shared with a co-owner: 190.00 of
/// conversion and no expense; non-liquid adds money moved into
/// investments.
#[tokio::test]
async fn equity_conversion_counts_only_the_own_share() {
    let (_server, client) = spawn_year().await;
    let report = call(
        &client,
        "income_statement",
        json!({"from": "2026-09-14", "to": "2026-09-16", "exclude": ["expenses:interest"]}),
    )
    .await;
    assert_eq!(report["equity_conversion"]["totals"], json!(["190.00"]));
    assert_eq!(report["expenses"], json!([]));
    let row = &report["equity_conversion"]["rows"][0];
    assert_eq!(row["cash"], json!(["380.00"]));
    assert_eq!(row["non_cash"], json!(["-190.00"]));
    let investing = call(
        &client,
        "income_statement",
        json!({"from": "2026-09-10", "to": "2026-09-10", "conversion_kinds": ["term-liability", "non-liquid"]}),
    )
    .await;
    assert_eq!(investing["equity_conversion"]["totals"], json!(["100.00"]));
}

/// Net worth is the sum of every asset and liability account; the last
/// point of the Cash on Hand history is Cash on Hand on that day.
#[tokio::test]
async fn balance_sheet_categories_and_history_agree_with_the_books() {
    let (_server, client) = spawn_year().await;
    let sheet = call(&client, "balance_sheet", json!({"as_of": "2026-09-30"})).await;
    let accounts = call(&client, "list_accounts", json!({})).await;
    let total: i64 = accounts
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| {
            let name = a["account"].as_str().unwrap();
            name.starts_with("assets") || name.starts_with("liabilities")
        })
        .filter_map(|a| {
            a["balance_exclusive"]
                .as_array()
                .unwrap()
                .first()
                .map(|b| cents(exact(b)))
        })
        .sum();
    assert_eq!(cents(sheet["net_worth"].as_str().unwrap()), total);
    let by_account = call(
        &client,
        "balance_sheet",
        json!({"as_of": "2026-09-30", "group_by": "account", "depth": 3}),
    )
    .await;
    assert_eq!(by_account["net_worth"], sheet["net_worth"]);

    let categories = call(
        &client,
        "category_breakdown",
        json!({"from": "2026-09-01", "to": "2026-09-30", "compare_previous": true, "limit": 2}),
    )
    .await;
    let rows = categories["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["account"], "expenses:interest");
    assert!(rows[0]["change"].is_string());

    let history = call(
        &client,
        "cash_on_hand_history",
        json!({"from": "2025-10-01", "to": "2026-09-30"}),
    )
    .await;
    let points = history["points"].as_array().unwrap();
    assert_eq!(points.len(), 12);
    let last = points.last().unwrap();
    assert_eq!(last["date"], "2026-09-30");
    let coh = call(&client, "cash_on_hand", json!({"as_of": "2026-09-30"})).await;
    assert_eq!(last["cash_on_hand"], coh["cash_on_hand"]["exact"]);
}

/// The cash flow bridge's invariants through MCP, and its exclusions.
#[tokio::test]
async fn cash_flow_bridges_cash_on_hand() {
    let (_server, client) = spawn_year().await;
    let flow = call(
        &client,
        "cash_flow",
        json!({"from": "2026-09-01", "to": "2026-09-30", "include_transactions": true, "limit": 2}),
    )
    .await;
    let opening = call(&client, "cash_on_hand", json!({"as_of": "2026-08-31"})).await;
    let closing = call(&client, "cash_on_hand", json!({"as_of": "2026-09-30"})).await;
    let change = cents(exact(&closing["cash_on_hand"])) - cents(exact(&opening["cash_on_hand"]));
    assert_eq!(cents(flow["change"].as_str().unwrap()), change);
    let lines = flow["lines"].as_array().unwrap();
    let sum = |key: &str| -> i64 { lines.iter().map(|l| cents(l[key].as_str().unwrap())).sum() };
    assert_eq!(sum("cash"), change);
    assert_eq!(sum("non_cash"), 0);
    assert_eq!(sum("total"), change);
    let line = |g: &str| lines.iter().find(|l| l["group"] == g).unwrap().clone();
    assert_eq!(line("equity_conversion")["cash"], "-380.00");
    assert_eq!(line("equity_conversion")["non_cash"], "490.00");
    assert_eq!(line("expense")["non_cash"], "-300.00");
    assert!(lines
        .iter()
        .all(|l| l["group"] != "revolving-debt" && l["group"] != "debt-reserve"));
    let expense = line("expense");
    assert_eq!(expense["transactions"].as_array().unwrap().len(), 2);
    assert_eq!(expense["truncated"], true);
    let more = call(
        &client,
        "cash_flow",
        json!({"from": "2026-09-01", "to": "2026-09-30", "cursor": expense["next_cursor"], "limit": 10}),
    )
    .await;
    assert_eq!(more["lines"].as_array().unwrap().len(), 1);
    assert_eq!(
        more["lines"][0]["transactions"].as_array().unwrap().len() + 2,
        expense["total_count"].as_u64().unwrap() as usize
    );

    let excluding = call(
        &client,
        "cash_flow",
        json!({"from": "2026-09-01", "to": "2026-09-30", "exclude": ["receivable", "payable"]}),
    )
    .await;
    let receivable = cents(line("receivable")["cash"].as_str().unwrap());
    assert_eq!(
        cents(excluding["change_excluding"].as_str().unwrap()),
        change - receivable
    );
}

/// opening + filed - collected = closing, per counterparty and in total,
/// and opening and closing match get_balance at the two dates.
#[tokio::test]
async fn receivables_activity_balances() {
    let (_server, client) = spawn_year().await;
    let activity = call(
        &client,
        "receivables_activity",
        json!({"from": "2026-09-01", "to": "2026-09-30", "include_entries": true}),
    )
    .await;
    let sam = &activity["counterparties"][0];
    let a = &sam["activity"];
    let c = |k: &str| cents(a[k].as_str().unwrap());
    assert_eq!(c("opening") + c("filed") - c("collected"), c("closing"));
    assert_eq!(c("filed"), 4000 + 19000);
    assert_eq!(c("collected"), 4000);
    let balance = |to: &str| {
        let client = &client;
        let to = to.to_string();
        async move {
            let b = call(
                client,
                "get_balance",
                json!({"account_prefix": "assets:receivable", "to": to}),
            )
            .await;
            cents(exact(&b[0]))
        }
    };
    assert_eq!(c("opening"), balance("2026-08-31").await);
    assert_eq!(c("closing"), balance("2026-09-30").await);
    let shares = sam["filed_by_counter_account"].as_array().unwrap();
    assert!(shares
        .iter()
        .any(|s| s["account"] == "liabilities:loans:house" && s["amount"] == "190.00"));
    assert_eq!(sam["entries"].as_array().unwrap().len(), 3);
    assert_eq!(activity["total"], *a);
}

/// A known 97.44 transaction left out: check says mismatch with a
/// difference of 97.44 and suggestions; once it's added, assert writes the
/// reconciled assertion and the books still check.
#[tokio::test]
async fn reconcile_finds_and_then_asserts_the_statement() {
    let (server, client) = spawn_year().await;
    let head = head(&client).await;
    let ledger = call(&client, "cash_on_hand", json!({"as_of": "2026-09-30"})).await;
    let inclusive = ledger["homes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["account"] == "assets:bank:checking")
        .unwrap()["inclusive"]
        .clone();
    let statement = cents(exact(&inclusive)) - 9744;
    let statement = format!("{}.{:02}", statement / 100, statement % 100);
    let args = json!({"account": "assets:bank:checking", "balance": statement, "date": "2026-09-30", "head": head});

    let check = call(&client, "reconcile", args.clone()).await;
    assert_eq!(check["status"], "mismatch", "{check}");
    assert_eq!(check["difference"], "97.44");
    assert!(check["suggestions"].is_array());
    assert!(check["later_transactions"].is_array());

    let mut assert = args.clone();
    assert["mode"] = json!("assert");
    let refused = fail(&client, "reconcile", assert.clone()).await;
    assert_eq!(refused["code"], "reconcile_mismatch");
    let mut adjust = args.clone();
    adjust["mode"] = json!("adjust");
    assert_eq!(
        fail(&client, "reconcile", adjust).await["code"],
        "validation_failed"
    );

    let (_, head) = commit(
        &client,
        "add_transaction",
        json!({"date": "2026-09-29", "description": "Hardware store", "head": head,
               "postings": [{"account": "expenses:food:groceries", "amount": "97.44"},
                            {"account": "assets:bank:checking"}]}),
    )
    .await;
    // An uncleared transaction that matches the difference is suggested.
    let suggested = call(
        &client,
        "reconcile",
        json!({"account": "assets:bank:checking", "balance": exact(&inclusive), "date": "2026-09-30"}),
    )
    .await;
    assert_eq!(suggested["difference"], "-97.44");
    assert!(
        suggested["suggestions"]
            .to_string()
            .contains("Hardware store"),
        "{suggested}"
    );

    assert["head"] = json!(head);
    assert["mark_cleared"] = json!(true);
    assert["dry_run"] = json!(false);
    let done = call(&client, "reconcile", assert).await;
    assert_eq!(done["status"], "matched");
    assert_eq!(done["dry_run"], false);
    let text = journal(&server).await;
    assert!(text.contains("reconciled:2026-09-30"), "{text}");
    assert!(
        text.contains("assets:bank:checking  USD0.00 =* USD"),
        "{text}"
    );
    let register = call(
        &client,
        "get_register",
        json!({"account": "assets:bank:checking", "to": "2026-09-30", "limit": 1000}),
    )
    .await;
    let lines = register["lines"].as_array().unwrap();
    assert!(lines.len() > 50);
    assert!(lines.iter().all(|l| l["status"] == "cleared"), "{register}");
    assert_checks(&server).await;
}

/// `adjust` with confirm_adjust posts the difference, then asserts.
#[tokio::test]
async fn reconcile_adjust_posts_the_difference() {
    let (_server, client) = spawn_year().await;
    let head = head(&client).await;
    let result = call(
        &client,
        "reconcile",
        json!({"account": "liabilities:loans:house", "balance": "-100000.00", "date": "2026-09-30",
               "mode": "adjust", "confirm_adjust": true, "head": head, "dry_run": false}),
    )
    .await;
    assert_eq!(result["status"], "mismatch");
    let loan = call(
        &client,
        "get_balance",
        json!({"account_prefix": "liabilities:loans:house", "to": "2026-09-30"}),
    )
    .await;
    assert_eq!(exact(&loan[0]), "-100000.00");
    let adjustments = call(
        &client,
        "get_balance",
        json!({"account_prefix": "equity:adjustments"}),
    )
    .await;
    assert_eq!(
        cents(exact(&adjustments[0])),
        cents(result["difference"].as_str().unwrap())
    );
}
