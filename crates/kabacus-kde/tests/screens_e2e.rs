// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The screens' view model (views) and every form's preview-then-commit
//! (writes) against a real kabacus-server on synthetic books with a bank
//! account, two funds, a card with its reserve, and one card purchase
//! missing its reserve lines.

use kabacus_client::KabacusClient;
use kabacus_kde::views::{self, RegisterFilter};
use kabacus_kde::writes::{self, Draft, WriteError};
use kabacus_store::BookStore;
use kabacus_testkit::{spawn_server_with_journal, TestServer};

const BOOKS: &str = "
account assets:bank:checking   ; ynab-account:Checking
account assets:bank:checking:insurance   ; ynab-envelope:Insurance
account assets:bank:checking:gifts   ; ynab-envelope:Gifts
account assets:bank:checking:card-visa
account liabilities:credit-card:visa   ; ynab-account:Visa
account expenses:dining
account income:salary

2026-09-01 Opening
    assets:bank:checking  2000.00 USD
    equity:opening

2026-09-02 Set aside
    assets:bank:checking:insurance  300.00 USD
    assets:bank:checking  -300.00 USD

2026-09-10 Coffee Roasters
    expenses:dining  20.00 USD
    liabilities:credit-card:visa  -20.00 USD
";

const CHECKING: &str = "assets:bank:checking";
const INSURANCE: &str = "assets:bank:checking:insurance";
const GIFTS: &str = "assets:bank:checking:gifts";
const RESERVE: &str = "assets:bank:checking:card-visa";
const VISA: &str = "liabilities:credit-card:visa";
// Row ids follow the journal's order, after the scratch books' opening.
const OPENING: &str = "2";
const SET_ASIDE: &str = "3";
const COFFEE: &str = "4";

async fn books() -> (TestServer, KabacusClient) {
    let server = spawn_server_with_journal(BOOKS, |_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    (server, client)
}

/// Commit a draft the way the Review page does: preview, then save with
/// the preview's head.
async fn store_head(server: &TestServer) -> String {
    let state = &server.state;
    state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string()
}

async fn changes(server: &TestServer) -> usize {
    let state = &server.state;
    state.store.history(state.ledger, 50).await.unwrap().len()
}

async fn save(client: &KabacusClient, draft: &Draft) -> writes::Saved {
    let preview = writes::preview(client, draft).await.unwrap();
    writes::commit(client, draft, &preview.head).await.unwrap()
}

fn card_purchase(amount: &str, source: &str) -> Draft {
    Draft::CardPurchase {
        date: "2026-10-01".into(),
        payee: "Corner Market".into(),
        memo: String::new(),
        card: VISA.into(),
        category: "expenses:dining".into(),
        source: source.into(),
        amount: amount.into(),
    }
}

// --- views -------------------------------------------------------------------

/// Overview leads with Cash on Hand and its parts, and each warning gets
/// the one action that fixes it.
#[tokio::test]
async fn overview_shows_cash_on_hand_and_warnings_with_their_fix() {
    let (_server, client) = books().await;
    let o = views::overview(&client).await.unwrap();
    assert_eq!(o.cash_on_hand.exact, "1700.00", "{o:?}");
    assert_eq!(o.future_allocations.exact, "300.00");
    let home = o.homes.iter().find(|h| h.account == CHECKING).unwrap();
    assert_eq!(home.free.exact, "1700.00");
    assert_eq!(home.inclusive.exact, "2000.00");
    assert!((home.free_share - 0.85).abs() < 1e-9, "{home:?}");
    let visa = o.cards.iter().find(|c| c.card == VISA).unwrap();
    assert!(!visa.covered);
    assert_eq!(visa.short.exact, "20.00");
    let mismatch = o
        .warnings
        .iter()
        .find(|w| w.code == "reserve-mismatch")
        .expect("reserve-mismatch warning");
    assert_eq!(mismatch.action, "Fix");
    assert_eq!(mismatch.target, "card");
    assert_eq!(mismatch.account, VISA);
}

/// The Overview lists only what holds money: a bank account with nothing
/// free and a fund with nothing in it are left off it, and stay on the
/// Funds page.
#[tokio::test]
async fn overview_lists_only_free_money_and_funds_with_a_balance() {
    let journal = format!(
        "{BOOKS}
account assets:bank:savings   ; ynab-account:High-Yield Savings
account assets:bank:savings:emergency   ; ynab-envelope:Emergency Fund

2026-09-03 Emergency fund deposit
    assets:bank:savings:emergency  500.00 USD
    equity:opening
"
    );
    let server = spawn_server_with_journal(&journal, |_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    let o = views::overview(&client).await.unwrap();

    let homes: Vec<&str> = o.homes.iter().map(|h| h.account.as_str()).collect();
    assert!(homes.contains(&CHECKING), "{homes:?}");
    assert!(
        !homes.contains(&"assets:bank:savings"),
        "nothing free: {homes:?}"
    );
    let funds: Vec<&str> = o.funds.iter().map(|f| f.account.as_str()).collect();
    assert!(funds.contains(&INSURANCE), "{funds:?}");
    assert!(
        funds.contains(&"assets:bank:savings:emergency"),
        "{funds:?}"
    );
    assert!(!funds.contains(&GIFTS), "empty fund: {funds:?}");
    // A reserve row opens the set-aside form from its own bank account.
    let insurance = o.funds.iter().find(|f| f.account == INSURANCE).unwrap();
    assert_eq!(insurance.home, CHECKING, "{insurance:?}");

    let page = views::funds(&client).await.unwrap();
    let savings = page
        .iter()
        .find(|h| h.home.account == "assets:bank:savings")
        .expect("the Funds page keeps a bank account with nothing free");
    assert_eq!(savings.home.free.exact, "0.00", "{savings:?}");
    assert!(
        page.iter()
            .any(|h| h.funds.iter().any(|f| f.account == GIFTS)),
        "the Funds page keeps an empty fund"
    );
}

/// The Loans page lists each loan as the Accounts page does, with what's
/// still owed; a credit card isn't a loan.
#[tokio::test]
async fn loans_lists_each_loan_with_what_is_owed() {
    let journal = format!(
        "{BOOKS}
account liabilities:loans:car   ; ynab-account:Auto Loan
account liabilities:mortgage:home   ; ynab-account:Home Mortgage

2026-09-01 Auto loan balance from Lakeside Credit Union
    liabilities:loans:car  -14250.00 USD
    equity:opening

2026-09-01 Mortgage balance from Harbor Federal
    liabilities:mortgage:home  -238400.00 USD
    equity:opening
"
    );
    let server = spawn_server_with_journal(&journal, |_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path).with_client_hint("app");
    let loans = views::loans(&client).await.unwrap();
    let names: Vec<&str> = loans.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Auto Loan", "Home Mortgage"], "{loans:?}");
    assert_eq!(loans[0].account, "liabilities:loans:car");
    assert_eq!(loans[0].balance.exact, "-14250.00");
    assert_eq!(loans[1].balance.exact, "-238400.00");

    // As LoansPage loads it.
    let loaded = views::load(&client, "loans", "{}").await.unwrap();
    assert_eq!(loaded[1]["name"], "Home Mortgage", "{loaded}");
}

/// Accounts are grouped by kind; funds and reserves fold into their home,
/// whose row shows what the bank shows and how it splits.
#[tokio::test]
async fn accounts_are_grouped_by_kind_with_funds_under_their_home() {
    let (_server, client) = books().await;
    let groups = views::accounts(&client).await.unwrap();
    let kinds: Vec<&str> = groups.iter().map(|g| g.kind.as_str()).collect();
    assert_eq!(kinds[..2], ["money", "revolving-debt"], "{kinds:?}");
    let money = &groups[0];
    let checking = money.rows.iter().find(|r| r.account == CHECKING).unwrap();
    assert_eq!(checking.name, "Checking");
    assert_eq!(checking.balance.exact, "2000.00");
    assert!(
        checking.detail.starts_with("Free $1,700.00"),
        "{checking:?}"
    );
    assert!(
        groups
            .iter()
            .all(|g| g.rows.iter().all(|r| r.account != INSURANCE)),
        "funds aren't top-level rows"
    );
    let visa = &groups[1].rows[0];
    assert_eq!(visa.balance.tone, "neg");
    assert!(visa.warn, "reserve short: {visa:?}");
}

/// The register lists real transactions; moves between a fund and its
/// bank account come back when asked for, and are marked.
#[tokio::test]
async fn the_register_hides_fund_transfers_until_asked() {
    let (_server, client) = books().await;
    let real = views::register(&client, CHECKING, &RegisterFilter::default())
        .await
        .unwrap();
    let payees: Vec<&str> = real.iter().map(|r| r.payee.as_str()).collect();
    assert_eq!(payees, ["Opening"], "{real:?}");
    assert_eq!(real[0].balance.tone, "pos");

    let rows = views::register(
        &client,
        CHECKING,
        &RegisterFilter {
            fund_transfers: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(rows[0].payee, "Set aside", "newest first: {rows:?}");
    assert_eq!(rows[0].kind, "move");
}

/// Every real transaction together, across accounts, without the fund
/// transfers unless asked for.
#[tokio::test]
async fn all_transactions_lists_real_ones_across_accounts() {
    let (_server, client) = books().await;
    let rows = views::register(&client, views::ALL_ACCOUNTS, &RegisterFilter::default())
        .await
        .unwrap();
    let payees: Vec<&str> = rows.iter().map(|r| r.payee.as_str()).collect();
    // The scratch books open with an opening balance of their own.
    assert_eq!(
        payees,
        ["Coffee Roasters", "Opening", "Opening"],
        "{rows:?}"
    );
    assert_eq!(rows[0].amount.exact, "-20.00");
    assert!(rows[0].unreserved);
    assert_eq!(rows[1].amount.exact, "2000.00");

    let with = views::register(
        &client,
        views::ALL_ACCOUNTS,
        &RegisterFilter {
            fund_transfers: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let payees: Vec<&str> = with.iter().map(|r| r.payee.as_str()).collect();
    assert_eq!(
        payees,
        ["Coffee Roasters", "Set aside", "Opening", "Opening"],
        "{with:?}"
    );

    let found = views::register(
        &client,
        views::ALL_ACCOUNTS,
        &RegisterFilter {
            text: "coffee".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(found.len(), 1);
}

#[tokio::test]
async fn the_register_filters_by_text_and_unreserved() {
    let (_server, client) = books().await;
    let found = views::register(
        &client,
        CHECKING,
        &RegisterFilter {
            text: "open".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].payee, "Opening");

    let visa = views::register(
        &client,
        VISA,
        &RegisterFilter {
            unreserved_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(visa.len(), 1);
    assert!(visa[0].unreserved);
    assert_eq!(visa[0].amount.tone, "neg");
}

/// Funds: each bank account with its funds and card reserves.
#[tokio::test]
async fn funds_list_each_home_with_its_funds_and_reserves() {
    let (_server, client) = books().await;
    let homes = views::funds(&client).await.unwrap();
    let checking = homes.iter().find(|h| h.home.account == CHECKING).unwrap();
    let names: Vec<&str> = checking.funds.iter().map(|f| f.name.as_str()).collect();
    assert!(
        names.contains(&"Insurance") && names.contains(&"Gifts"),
        "{names:?}"
    );
    let reserve = &checking.reserves[0];
    assert_eq!(reserve.account, RESERVE);
    assert_eq!(reserve.short.exact, "20.00");
}

/// A card's page: what the reserve holds and what's missing its lines.
#[tokio::test]
async fn a_card_lists_its_unreserved_transactions() {
    let (_server, client) = books().await;
    let cards = views::cards(&client).await.unwrap();
    let visa = cards.iter().find(|c| c.card.card == VISA).unwrap();
    assert_eq!(visa.reserve_account, RESERVE);
    assert_eq!(visa.unreserved.len(), 1);
    assert_eq!(visa.unreserved[0].id, COFFEE);
    assert_eq!(visa.unreserved[0].missing.exact, "20.00");
}

/// Reports: the cash-flow bridge and where the money went.
#[tokio::test]
async fn reports_bridge_cash_on_hand_and_list_categories() {
    let (_server, client) = books().await;
    let (from, to) = views::month_range(2026, 9);
    let flow = views::cash_flow(&client, &from, &to).await.unwrap();
    assert_eq!(flow.closing.exact, "1700.00", "{flow:?}");
    assert_eq!(flow.bars.first().unwrap().name, "Opening");
    assert_eq!(flow.bars.last().unwrap().kind, "base");
    assert!(flow.bars.iter().all(|b| b.lo <= b.hi && b.hi <= 1.0));
    assert!(
        flow.lines.iter().any(|l| l.name == "Set aside in reserves"),
        "{:?}",
        flow.lines
    );

    let cats = views::categories(&client, &from, &to).await.unwrap();
    let dining = cats
        .iter()
        .find(|c| c.account == "expenses:dining")
        .unwrap();
    assert_eq!(dining.amount.exact, "20.00");
    assert_eq!(dining.share, 1.0);

    let is = views::income_statement(&client, &from, &to, true)
        .await
        .unwrap();
    assert_eq!(is.expenses.exact, "20.00");
}

/// Reconcile checks without writing and says whether it matches.
#[tokio::test]
async fn reconcile_checks_a_statement_without_writing() {
    let (server, client) = books().await;
    let head = store_head(&server).await;
    let ok = views::reconcile_check(&client, CHECKING, "2026-09-30", "2,000.00")
        .await
        .unwrap();
    assert!(ok.matched, "{ok:?}");
    let off = views::reconcile_check(&client, CHECKING, "2026-09-30", "1950.00")
        .await
        .unwrap();
    assert!(!off.matched);
    assert_eq!(off.difference.exact, "50.00");
    assert_eq!(store_head(&server).await, head, "nothing written");
}

/// `Ledger.load` reads the args each page's `Fetch` sends (the keys are
/// plain strings on both sides, so a wrong one would quietly load the
/// wrong data), and the views `refresh` preloads take none.
#[tokio::test]
async fn load_reads_the_args_each_page_sends() {
    let (_server, client) = books().await;
    let load = |view: &'static str, args: serde_json::Value| {
        let client = &client;
        async move { views::load(client, view, &args.to_string()).await }
    };

    // RegisterPage.
    let visa = load(
        "register",
        serde_json::json!({ "account": VISA, "text": "", "unreservedOnly": true, "fundTransfers": false }),
    )
    .await
    .unwrap();
    assert_eq!(visa.as_array().unwrap().len(), 1, "{visa}");
    assert_eq!(visa[0]["unreserved"], true);
    let found = load(
        "register",
        serde_json::json!({ "account": CHECKING, "text": "open", "unreservedOnly": false }),
    )
    .await
    .unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
    assert_eq!(found[0]["payee"], "Opening");

    // ReportsPage, one per tab.
    let flow = load(
        "report",
        serde_json::json!({ "kind": "cash_flow", "year": 2026, "month": 9 }),
    )
    .await
    .unwrap();
    assert_eq!(flow["closing"]["exact"], "1700.00", "{flow}");
    let income = load(
        "report",
        serde_json::json!({ "kind": "income", "year": 2026, "month": 9, "principal": true }),
    )
    .await
    .unwrap();
    assert_eq!(income["expenses"]["exact"], "20.00", "{income}");
    let cats = load(
        "report",
        serde_json::json!({ "kind": "categories", "year": 2026, "month": 9 }),
    )
    .await
    .unwrap();
    assert_eq!(cats[0]["account"], "expenses:dining", "{cats}");

    // ReconcilePage.
    let check = load(
        "reconcile",
        serde_json::json!({ "account": CHECKING, "date": "2026-09-30", "balance": "2,000.00" }),
    )
    .await
    .unwrap();
    assert_eq!(check["matched"], true, "{check}");

    // TransactionForm's payee memory.
    let payees = load("payees", serde_json::json!({ "query": "coffee" }))
        .await
        .unwrap();
    assert_eq!(payees[0]["payee"], "Coffee Roasters", "{payees}");

    for view in [
        "overview", "accounts", "funds", "cards", "picker", "people", "activity",
    ] {
        load(view, serde_json::json!({})).await.unwrap();
    }
    assert!(load("nothing", serde_json::json!({})).await.is_err());
    assert!(load("report", serde_json::json!({ "kind": "nothing" }))
        .await
        .is_err());
}

// --- writes ------------------------------------------------------------------

/// A card purchase previews its reserve lines and what it does to Cash on
/// Hand, writes nothing, then commits on the preview's head.
#[tokio::test]
async fn a_card_purchase_previews_then_commits() {
    let (server, client) = books().await;
    let head = store_head(&server).await;
    let draft = card_purchase("42.18", "free");

    let p = writes::preview(&client, &draft).await.unwrap();
    assert_eq!(p.head, head);
    assert_eq!(store_head(&server).await, head, "dry run");
    assert!(p.summary.contains("Corner Market") && p.summary.contains("free money"));
    let coh = p.cash_on_hand.as_ref().unwrap();
    assert_eq!(
        (coh.before.exact.as_str(), coh.delta.exact.as_str()),
        ("1700.00", "-42.18")
    );
    assert!(
        p.journal
            .iter()
            .any(|l| l.kind == "add" && l.text.contains("card-visa")),
        "{:?}",
        p.journal
    );
    assert!(p.changes.iter().any(|c| c.account == RESERVE));

    let saved = writes::commit(&client, &draft, &p.head).await.unwrap();
    assert_ne!(saved.head, head);
    let o = views::overview(&client).await.unwrap();
    assert_eq!(o.cash_on_hand.exact, "1657.82");
}

/// Setting money aside lowers Cash on Hand; moving between funds doesn't.
#[tokio::test]
async fn setting_aside_lowers_cash_on_hand_and_moving_funds_does_not() {
    let (_server, client) = books().await;
    let aside = Draft::SetAside {
        date: "2026-10-01".into(),
        from: CHECKING.into(),
        to: GIFTS.into(),
        amount: "100.00".into(),
        description: String::new(),
    };
    let p = writes::preview(&client, &aside).await.unwrap();
    assert_eq!(p.cash_on_hand.unwrap().delta.exact, "-100.00");
    save(&client, &aside).await;

    let between = Draft::SetAside {
        date: "2026-10-02".into(),
        from: INSURANCE.into(),
        to: GIFTS.into(),
        amount: "50.00".into(),
        description: String::new(),
    };
    let p = writes::preview(&client, &between).await.unwrap();
    assert_eq!(p.cash_on_hand.unwrap().delta.exact, "0");
}

/// Setting aside more than an account's free money is refused under the
/// Amount field, naming the shortfall.
#[tokio::test]
async fn setting_aside_more_than_is_free_is_refused() {
    let (_server, client) = books().await;
    let too_much = Draft::SetAside {
        date: "2026-10-01".into(),
        from: CHECKING.into(),
        to: GIFTS.into(),
        amount: "2500.00".into(),
        description: String::new(),
    };
    match writes::preview(&client, &too_much).await {
        Err(WriteError::Field { field, message, .. }) => {
            assert_eq!(field, "amount");
            assert!(
                message.starts_with("Checking has only") && message.contains("800.00 short of"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }

    let fits = Draft::SetAside {
        date: "2026-10-01".into(),
        from: CHECKING.into(),
        to: GIFTS.into(),
        amount: "250.00".into(),
        description: String::new(),
    };
    let p = writes::preview(&client, &fits).await.unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

/// Setting money aside into a fund that doesn't exist yet: the name typed
/// in To becomes a new fund in the money account the money comes from,
/// made by the same save.
#[tokio::test]
async fn setting_aside_into_a_new_fund_makes_it() {
    let (_server, client) = books().await;
    let aside = Draft::SetAside {
        date: "2026-10-01".into(),
        from: CHECKING.into(),
        to: "Summer Vacation".into(),
        amount: "120.00".into(),
        description: String::new(),
    };
    let p = writes::preview(&client, &aside).await.unwrap();
    assert!(p.summary.contains("new reserve"), "{}", p.summary);
    assert!(
        p.changes.iter().any(|c| c.name == "Summer Vacation"),
        "{:?}",
        p.changes
    );
    assert_eq!(p.cash_on_hand.unwrap().delta.exact, "-120.00");
    save(&client, &aside).await;

    let o = views::overview(&client).await.unwrap();
    let fund = o
        .funds
        .iter()
        .find(|f| f.name == "Summer Vacation")
        .unwrap_or_else(|| panic!("{:?}", o.funds));
    assert_eq!(fund.amount.exact, "120.00");

    // A name with a colon can't be a fund's name.
    let bad = Draft::SetAside {
        date: "2026-10-01".into(),
        from: CHECKING.into(),
        to: "travel:japan".into(),
        amount: "1.00".into(),
        description: String::new(),
    };
    match writes::preview(&client, &bad).await {
        Err(WriteError::Field { field, .. }) => assert_eq!(field, "to"),
        other => panic!("{other:?}"),
    }
}

/// Paying a card from its reserve leaves Cash on Hand alone.
#[tokio::test]
async fn paying_a_card_leaves_cash_on_hand_alone() {
    let (_server, client) = books().await;
    save(&client, &card_purchase("30.00", "free")).await;
    let pay = Draft::PayCard {
        date: "2026-10-05".into(),
        card: VISA.into(),
        amount: "30.00".into(),
    };
    let p = writes::preview(&client, &pay).await.unwrap();
    assert_eq!(p.cash_on_hand.unwrap().delta.exact, "0");
    save(&client, &pay).await;
}

/// Fixing unreserved transactions is one batch: one preview, one commit,
/// and the card is covered again.
#[tokio::test]
async fn fixing_reserve_lines_is_one_commit_and_covers_the_card() {
    let (server, client) = books().await;
    let draft = Draft::FixReserves {
        card: VISA.into(),
        ids: vec![COFFEE.into()],
        source: "free".into(),
    };
    let p = writes::preview(&client, &draft).await.unwrap();
    assert!(p.changes.iter().any(|c| c.account == RESERVE), "{p:?}");
    let before = changes(&server).await;
    save(&client, &draft).await;
    assert_eq!(changes(&server).await, before + 1);
    let cards = views::cards(&client).await.unwrap();
    assert!(
        cards
            .iter()
            .find(|c| c.card.card == VISA)
            .unwrap()
            .card
            .covered
    );
}

/// A split nets against what you already owe the person first.
#[tokio::test]
async fn a_split_pays_down_what_you_owe_them_first() {
    let (_server, client) = books().await;
    // You owe Jordan 30.00.
    save(
        &client,
        &Draft::Simple {
            date: "2026-09-20".into(),
            payee: "Jordan paid the bill".into(),
            memo: String::new(),
            from: "liabilities:payable:jordan".into(),
            to: "expenses:dining".into(),
            amount: "30.00".into(),
        },
    )
    .await;
    let split = Draft::Split {
        date: "2026-10-01".into(),
        payee: "Dinner".into(),
        memo: String::new(),
        paid_with: CHECKING.into(),
        category: "expenses:dining".into(),
        person: "Jordan".into(),
        total: "80.00".into(),
        their_share: "40.00".into(),
    };
    let p = writes::preview(&client, &split).await.unwrap();
    let change = |a: &str| {
        p.changes
            .iter()
            .find(|c| c.account == a)
            .unwrap()
            .after
            .exact
            .clone()
    };
    assert_eq!(change("liabilities:payable:jordan"), "0.00", "{p:?}");
    assert_eq!(change("assets:receivable:jordan"), "10.00");
    save(&client, &split).await;
    let people = views::people(&client).await.unwrap();
    let jordan = people.iter().find(|p| p.name == "jordan").unwrap();
    assert_eq!(jordan.net.exact, "10.00");
    // Named as Accounts names them.
    assert_eq!(jordan.display, "Jordan");
}

/// Editing changes the transaction where it is; voiding adds a
/// reversal and leaves the original alone.
#[tokio::test]
async fn editing_in_place_and_voiding() {
    let (_server, client) = books().await;
    let edit = Draft::Edit {
        id: SET_ASIDE.into(),
        date: "2026-09-02".into(),
        payee: "Set aside".into(),
        memo: String::new(),
        from: CHECKING.into(),
        to: INSURANCE.into(),
        amount: "250.00".into(),
    };
    let p = writes::preview(&client, &edit).await.unwrap();
    assert!(p.summary.contains("in place"), "{}", p.summary);
    save(&client, &edit).await;
    let o = views::overview(&client).await.unwrap();
    assert_eq!(o.cash_on_hand.exact, "1750.00");

    let void = Draft::Void { id: COFFEE.into() };
    let p = writes::preview(&client, &void).await.unwrap();
    assert!(p.summary.contains("reversal"));
    save(&client, &void).await;
    let visa = views::register(&client, VISA, &RegisterFilter::default())
        .await
        .unwrap();
    // The original stays as it was and the reversal cancels it.
    assert_eq!(visa.len(), 2, "{visa:?}");
    assert!(visa.iter().any(|r| r.id == COFFEE), "{visa:?}");
    assert_eq!(visa[0].balance.exact, "0.00", "{visa:?}");
    let cards = views::cards(&client).await.unwrap();
    assert_eq!(cards[0].card.owed.exact, "0.00");
}

/// An edit saved without changing anything: the preview says there's
/// nothing to save (the form turns Save off), and saving anyway writes no
/// commit instead of failing with "The server had a problem."
#[tokio::test]
async fn an_unchanged_edit_has_nothing_to_save() {
    let (_server, client) = books().await;
    let id = SET_ASIDE;
    let d = kabacus_kde::model::edit_draft(&client, id).await.unwrap();
    let edit = Draft::Edit {
        id: id.into(),
        date: d.date,
        payee: d.payee,
        memo: d.memo,
        from: d.from_account,
        to: d.to_account,
        amount: d.amount,
    };
    let p = writes::preview(&client, &edit).await.unwrap();
    assert!(p.unchanged, "{p:?}");
    assert!(p.summary.contains("Nothing to save"), "{}", p.summary);
    let saved = writes::commit(&client, &edit, &p.head).await.unwrap();
    assert_eq!(saved.head, p.head, "no new commit");
}

/// A head that moved since the preview is a conflict naming who moved
/// it, never a silent retry.
#[tokio::test]
async fn a_stale_preview_is_a_conflict_that_names_who_changed_the_books() {
    let (_server, client) = books().await;
    let draft = card_purchase("10.00", "free");
    let p = writes::preview(&client, &draft).await.unwrap();

    // Meanwhile an MCP client changes the books.
    let other = KabacusClient::connect_unix(&_server.socket_path).with_client_hint("mcp");
    save(&other, &card_purchase("5.00", "free")).await;

    match writes::commit(&client, &draft, &p.head).await {
        Err(WriteError::Conflict { who, what }) => {
            assert_eq!(who, "MCP");
            assert!(what.contains("Corner Market"), "{what}");
        }
        other => panic!("{other:?}"),
    }
}

/// Problems a form can show next to the field: an account that
/// doesn't exist (with the closest one) and a decimal comma.
#[tokio::test]
async fn field_errors_point_at_the_field() {
    let (server, client) = books().await;
    let head = store_head(&server).await;
    let mut draft = card_purchase("23.40", "free");
    if let Draft::CardPurchase { category, .. } = &mut draft {
        *category = "expenses:dinning".into();
    }
    match writes::preview(&client, &draft).await {
        Err(WriteError::Field {
            field, suggestion, ..
        }) => {
            assert_eq!(field, "category");
            assert_eq!(suggestion, "", "no near match for a typo'd leaf");
        }
        other => panic!("{other:?}"),
    }
    let mut draft = card_purchase("23,40", "free");
    if let Draft::CardPurchase { category, .. } = &mut draft {
        *category = "expenses:other:dining".into();
    }
    match writes::preview(&client, &draft).await {
        Err(WriteError::Field {
            field, suggestion, ..
        }) => {
            assert_eq!(field, "category");
            assert_eq!(suggestion, "expenses:dining");
        }
        other => panic!("{other:?}"),
    }
    let draft = card_purchase("23,40", "free");
    match writes::preview(&client, &draft).await {
        Err(WriteError::Field { field, message, .. }) => {
            assert_eq!(field, "amount");
            assert!(message.contains("23.40"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(store_head(&server).await, head);
}

/// A write that would break a balance assertion says which one.
#[tokio::test]
async fn a_broken_balance_check_is_explained() {
    let journal =
        format!("{BOOKS}\n2026-09-30 Statement\n    assets:bank:checking  0 USD =* 2000.00 USD\n");
    let server = spawn_server_with_journal(&journal, |_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let draft = Draft::Simple {
        date: "2026-09-27".into(),
        payee: "Late fee".into(),
        memo: String::new(),
        from: CHECKING.into(),
        to: "expenses:dining".into(),
        amount: "40.00".into(),
    };
    match writes::preview(&client, &draft).await {
        Err(WriteError::Assertion {
            account, asserted, ..
        }) => {
            assert_eq!(account, "Checking");
            assert!(
                asserted.contains("2000") || asserted.contains("2,000"),
                "{asserted}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// Activity names who made each change.
#[tokio::test]
async fn activity_names_who_made_each_change() {
    let (_server, client) = books().await;
    save(&client, &card_purchase("12.00", "free")).await;
    let all = views::activity(&client, "").await.unwrap();
    assert_eq!(all[0].actor, "app", "{all:?}");
    assert_eq!(all[0].actor_label, "This app");
    assert!(!all[0].date.is_empty());
    assert!(!all[0].transaction_id.is_empty(), "{:?}", all[0]);
    let diff = views::commit_diff(&client, &all[0].hash).await.unwrap();
    assert!(diff
        .iter()
        .any(|l| l.kind == "add" && l.text.contains("Corner Market")));
    assert!(views::activity(&client, "mcp").await.unwrap().is_empty());
}

/// Once it matches, reconciling writes the balance assertion as one
/// reviewed commit.
#[tokio::test]
async fn a_matched_reconcile_writes_its_balance_assertion() {
    let (_server, client) = books().await;
    let draft = Draft::Reconcile {
        account: CHECKING.into(),
        date: "2026-09-30".into(),
        balance: "2000.00".into(),
    };
    let p = writes::preview(&client, &draft).await.unwrap();
    assert!(
        p.journal
            .iter()
            .any(|l| l.kind == "add" && l.text.contains("=*")),
        "an inclusive assertion: {:?}",
        p.journal
    );
    let saved = save(&client, &draft).await;
    let diff = client.history_diff(&saved.head).await.unwrap();
    assert!(diff.contains("=* "), "{diff}");
}

/// The card purchase form remembers payees: their usual category and last
/// amount.
#[tokio::test]
async fn payee_memory_and_pickers_come_from_the_books() {
    let (_server, client) = books().await;
    let memory = views::payees(&client, "coffee").await.unwrap();
    assert_eq!(memory[0].payee, "Coffee Roasters");
    assert_eq!(memory[0].category, "expenses:dining");
    assert_eq!(memory[0].last_amount.exact, "20.00");

    let picks = views::picker(&client).await.unwrap();
    assert_eq!((picks.commodity.as_str(), picks.places), ("USD", 2));
    let gifts = picks.accounts.iter().find(|p| p.account == GIFTS).unwrap();
    assert_eq!(gifts.kind, "pre-allocation");
    assert_eq!(gifts.home, CHECKING);
}

/// Every row carries its transaction's number, and an edit keeps it.
#[tokio::test]
async fn an_edit_keeps_the_transactions_number() {
    let (_server, client) = books().await;
    let rows = views::register(&client, CHECKING, &RegisterFilter::default())
        .await
        .unwrap();
    let opening = rows.iter().find(|r| r.payee == "Opening").unwrap();
    assert_eq!(opening.id, OPENING, "{opening:?}");

    let d = kabacus_kde::model::edit_draft(&client, &opening.id)
        .await
        .unwrap();
    assert_eq!(
        (d.from_account.as_str(), d.to_account.as_str()),
        ("equity:opening", CHECKING)
    );
    let edit = Draft::Edit {
        id: opening.id.clone(),
        date: d.date,
        payee: d.payee,
        memo: "by hand".into(),
        from: d.from_account,
        to: d.to_account,
        amount: d.amount,
    };
    save(&client, &edit).await;

    let rows = views::register(&client, CHECKING, &RegisterFilter::default())
        .await
        .unwrap();
    let opening = rows.iter().find(|r| r.payee == "Opening").unwrap();
    assert_eq!(opening.id, OPENING, "{opening:?}");
    assert_eq!(opening.memo, "by hand");
}

/// Income and equity show what was earned or put in, positive and
/// without sign colour, not hledger's negative credit balance.
#[tokio::test]
async fn income_and_equity_show_positive_and_neutral() {
    let journal = format!(
        "{BOOKS}\n2026-09-15 Payday\n    assets:bank:checking  2400.00 USD\n    income:salary\n"
    );
    let server = spawn_server_with_journal(&journal, |_| {}).await;
    let client = KabacusClient::connect_unix(&server.socket_path);
    let groups = views::accounts(&client).await.unwrap();
    let row = |account: &str| {
        groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.account == account)
            .unwrap_or_else(|| panic!("{account} in {groups:?}"))
            .clone()
    };
    let salary = row("income:salary");
    assert_eq!(
        (salary.balance.exact.as_str(), salary.balance.tone),
        ("2400.00", "zero")
    );
    assert!(!salary.balance.text.contains('−'), "{:?}", salary.balance);
    // The testkit's own opening (1000.00) and these books' (2000.00).
    let opening = row("equity:opening");
    assert_eq!(
        (opening.balance.exact.as_str(), opening.balance.tone),
        ("3000.00", "zero")
    );
    let checking = row(CHECKING);
    assert_eq!(checking.balance.tone, "pos", "others keep their colour");
}

#[tokio::test]
async fn split_posting_edits_preview_and_save_in_place() {
    let (_server, client) = books().await;
    let split: Draft = serde_json::from_value(serde_json::json!({
        "kind": "split", "date": "2026-09-12", "payee": "Harbor Cafe",
        "paid_with": CHECKING, "category": "expenses:dining", "person": "jordan",
        "total": "60.00", "their_share": "20.00"
    }))
    .unwrap();
    save(&client, &split).await;
    let tx = client
        .list_transactions()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.payee == "Harbor Cafe")
        .unwrap();
    assert_eq!(tx.postings.len(), 3);
    let mut postings: Vec<_> = tx
        .postings
        .iter()
        .enumerate()
        .map(|(i, p)| {
            serde_json::json!({ "account": p.account, "amount": p.amount.exact,
            "commodity": p.amount.commodity, "original": i })
        })
        .collect();
    postings[0]["amount"] = "35.00".into();
    postings[1]["amount"] = "25.00".into();
    let mut unbalanced = postings.clone();
    unbalanced[0]["amount"] = "36.00".into();
    let invalid: Draft = serde_json::from_value(serde_json::json!({
        "kind": "edit_postings", "id": tx.id.to_string(), "date": tx.date,
        "payee": "Harbor Cafe lunch", "postings": unbalanced
    }))
    .unwrap();
    assert!(writes::preview(&client, &invalid).await.is_err());
    let edit: Draft = serde_json::from_value(serde_json::json!({
        "kind": "edit_postings", "id": tx.id.to_string(), "date": tx.date,
        "payee": "Harbor Cafe lunch", "postings": postings
    }))
    .unwrap();
    let preview = writes::preview(&client, &edit).await.unwrap();
    assert!(client
        .list_transactions()
        .await
        .unwrap()
        .iter()
        .any(|t| t.payee == "Harbor Cafe"));
    writes::commit(&client, &edit, &preview.head).await.unwrap();
    let edited = client
        .list_transactions()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == tx.id)
        .unwrap();
    assert_eq!(edited.payee, "Harbor Cafe lunch");
    assert_eq!(edited.tags, tx.tags);
    assert_eq!(
        edited
            .postings
            .iter()
            .map(|p| p.amount.exact.as_str())
            .collect::<Vec<_>>(),
        vec!["35.00", "25.00", "-60.00"]
    );
    for (before, after) in tx.postings.iter().zip(&edited.postings) {
        assert_eq!(before.account, after.account);
        assert_eq!(before.status, after.status);
    }
}
