// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::caller::{Caller, Permission};
use crate::state::{KindTable, Settings};
use kabacus_store::{AnyStore, SqliteStore};
use std::path::Path;

const BOOKS: &str = "commodity USD1000.00\n\
    account assets:checking\n\
    account expenses:groceries\n\
    account expenses:household\n\
    account equity:opening\n\
    \n\
    2026-01-01 Opening\n    assets:checking  1000.00 USD\n    equity:opening\n\n";

async fn state(extra: &str) -> Service {
    let store = AnyStore::Sqlite(SqliteStore::in_memory().unwrap());
    let main = Path::new("/books/main.journal");
    let src = kabacus_ledger::MemorySource(BTreeMap::from([(
        main.to_path_buf(),
        format!("{BOOKS}{extra}"),
    )]));
    let outcome = kabacus_ledger::import(&src, main).unwrap();
    let ledger = crate::state::find_or_create_ledger(&store, None)
        .await
        .unwrap();
    let user = store.local_user(crate::state::SERVER_USER).await.unwrap();
    let who = Who {
        user,
        actor: "import".to_string(),
        kind: "import".to_string(),
        summary: "import: main.journal".to_string(),
        details: serde_json::json!({}),
    };
    store.import(ledger, &outcome.book, &who).await.unwrap();
    Service::new(
        store,
        ledger,
        Settings::default(),
        KindTable {
            rules: kabacus_core::default_kind_rules(),
            reserve_template: kabacus_core::DEFAULT_RESERVE_TEMPLATE.to_string(),
        },
    )
}

struct LocalCaller;
impl Caller for LocalCaller {
    fn require(&self, _: Permission) -> Result<(), crate::error::Error> {
        Ok(())
    }
    fn user<'a>(
        &'a self,
        store: &'a AnyStore,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<kabacus_store::UserId>> + Send + 'a>,
    > {
        Box::pin(async move { Ok(store.local_user("Dana Okafor").await?) })
    }
    fn actor(&self, _: Option<&str>) -> String {
        "app".to_string()
    }
    fn details(&self) -> serde_json::Value {
        serde_json::json!({"user": "Dana Okafor", "client": "kabacus-app", "session": "none", "auth": "none"})
    }
}
fn local() -> LocalCaller {
    LocalCaller
}

fn usd(cents: i128) -> Amount {
    Amount::new("USD", Money::from_decimal(cents, 2))
}

fn posting(account: &str, cents: Option<i128>) -> DraftPosting {
    DraftPosting {
        cost: kabacus_core::CostEdit::Keep,
        account: AccountName::new(account),
        amount: cents.map(usd),
        status: Status::Unmarked,
        balance_assertion: AssertionEdit::Keep,
    }
}

fn date(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

fn groceries(cents: i128) -> AddRequest {
    AddRequest {
        date: date("2026-02-01"),
        description: "Fresh Thyme Market".to_string(),
        postings: vec![
            posting("expenses:groceries", Some(cents)),
            posting("assets:checking", None),
        ],
        tags: Vec::new(),
        new_accounts: Vec::new(),
    }
}

async fn head(state: &Service) -> String {
    state
        .store
        .revision(state.ledger)
        .await
        .unwrap()
        .to_string()
}

async fn books(state: &Service) -> kabacus_ledger::Book {
    state.store.load(state.ledger, None).await.unwrap().book
}

async fn add(state: &Service, req: AddRequest) -> (u64, String) {
    let done = write(
        state,
        &local(),
        WriteOp::Add(req),
        &head(state).await,
        false,
    )
    .await
    .unwrap();
    (done.id.unwrap(), done.head)
}

#[tokio::test]
async fn transaction_reads_filter_without_changing_the_cached_books() {
    let state = state("").await;
    let (id, _) = add(&state, groceries(6412)).await;
    let snapshot = state.books_snapshot().await.unwrap();
    let before = snapshot.transactions.clone();
    let loads = state.books_cache.loads();
    let query = serde_json::from_value(serde_json::json!({
        "account": "expenses:groceries", "payee": "Fresh Thyme", "sort": "date_desc"
    }))
    .unwrap();
    let rows = crate::search::matching(&state, &query).await.unwrap();
    assert_eq!(rows.len(), 1);
    let found = crate::search::get_transaction(&state, &local(), &id.to_string(), false)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&rows[0].0).unwrap(),
        serde_json::to_value(&found.transaction).unwrap()
    );
    assert!(found.history.is_none());
    assert!(matches!(
        crate::search::get_transaction(&state, &local(), "999", false).await,
        Err(crate::error::Error::NotFound(_))
    ));
    assert_eq!(snapshot.transactions, before);
    assert_eq!(state.books_cache.loads(), loads);
}

#[tokio::test]
async fn an_add_is_one_revision_recording_who_made_it() {
    let state = state("").await;
    let before = head(&state).await;
    let (id, after) = add(&state, groceries(6412)).await;
    assert_eq!(
        after.parse::<i64>().unwrap(),
        before.parse::<i64>().unwrap() + 1
    );
    assert_eq!(id, 2, "the opening balance is transaction 1");

    let change = state
        .store
        .change(state.ledger, after.parse().unwrap())
        .await
        .unwrap()
        .info;
    assert_eq!(change.kind, "add");
    assert_eq!(change.actor, "app");
    assert_eq!(change.user, "Dana Okafor");
    assert_eq!(change.details["auth"], "none");
    assert!(
        change
            .summary
            .starts_with("add: 2026-02-01 Fresh Thyme Market"),
        "{}",
        change.summary
    );

    let book = books(&state).await;
    let tx = book.transactions.iter().find(|t| t.id == id).unwrap();
    assert_eq!(tx.postings[1].amount, usd(-6412));
}

#[tokio::test]
async fn a_dry_run_writes_nothing_and_fails_like_the_commit() {
    let state = state(
        "2026-03-01 Statement\n    assets:checking  0.00 USD = 1000.00 USD\n    equity:opening  0.00 USD\n",
    )
    .await;
    let head0 = head(&state).await;
    let preview = write(&state, &local(), WriteOp::Add(groceries(500)), &head0, true).await;
    let Err(OpError::Check(failures)) = preview else {
        panic!("{preview:?}")
    };
    assert_eq!(failures[0].kind, kabacus_ledger::FailureKind::Assertion);
    let commit = write(
        &state,
        &local(),
        WriteOp::Add(groceries(500)),
        &head0,
        false,
    )
    .await;
    assert!(matches!(commit, Err(OpError::Check(_))), "{commit:?}");
    assert_eq!(head(&state).await, head0);

    let mut later = groceries(500);
    later.date = date("2026-03-02");
    let preview = write(&state, &local(), WriteOp::Add(later), &head0, true)
        .await
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(head(&state).await, head0, "a dry run writes nothing");
}

#[tokio::test]
async fn a_stale_head_is_a_conflict() {
    let state = state("").await;
    let stale = head(&state).await;
    add(&state, groceries(100)).await;
    let error = write(
        &state,
        &local(),
        WriteOp::Add(groceries(200)),
        &stale,
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OpError::Conflict { .. }), "{error:?}");
}

#[tokio::test]
async fn an_undeclared_account_is_refused() {
    let state = state("").await;
    let mut req = groceries(100);
    req.postings[0].account = AccountName::new("expenses:nowhere");
    let error = write(
        &state,
        &local(),
        WriteOp::Add(req),
        &head(&state).await,
        true,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, OpError::Validation(_) | OpError::ValidationMany(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_new_expense_category_is_declared_by_the_write_that_uses_it() {
    let state = state("").await;
    let mut req = groceries(4250);
    req.postings[0].account = AccountName::new("expenses:pets:vet");
    req.new_accounts = vec!["expenses:pets:vet".to_string()];
    let before = head(&state).await.parse::<i64>().unwrap();
    let (_, after) = add(&state, req).await;
    assert_eq!(after.parse::<i64>().unwrap(), before + 1);
    assert!(books(&state)
        .await
        .accounts
        .contains_key("expenses:pets:vet"));
}

#[tokio::test]
async fn only_an_expense_or_income_account_is_made_inline() {
    let state = state("").await;
    let mut req = groceries(100);
    req.postings[1].account = AccountName::new("assets:bank:savings");
    req.new_accounts = vec!["assets:bank:savings".to_string()];
    let error = write(
        &state,
        &local(),
        WriteOp::Add(req),
        &head(&state).await,
        true,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OpError::Invalid { .. }), "{error:?}");
    assert!(!books(&state)
        .await
        .accounts
        .contains_key("assets:bank:savings"));
}

#[tokio::test]
async fn a_description_that_would_break_its_journal_line_is_refused() {
    let state = state("").await;
    let head0 = head(&state).await;
    let mut req = groceries(1200);
    req.description = "Cub Foods\n    assets:checking  1000000.00 USD".to_string();
    let error = write(&state, &local(), WriteOp::Add(req), &head0, false)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            OpError::Validation(ValidationError::DescriptionBreaksLine)
        ),
        "{error:?}"
    );
    assert_eq!(head(&state).await, head0);
}

#[tokio::test]
async fn an_edit_keeps_the_id_comments_tags_and_virtual_postings() {
    let state = state(
        "2026-02-03 * (1042) Corner Hardware | screws  ; receipt:2026-0042\n    ; kept for the warranty\n    expenses:household  12.50 USD  ; aisle:7\n    assets:checking\n    (expenses:household)  1.00 USD\n",
    )
    .await;
    let id = books(&state).await.transactions[1].id;
    let req = EditRequest {
        id,
        date: date("2026-02-03"),
        description: "Corner Hardware | screws".to_string(),
        postings: vec![
            posting("expenses:household", Some(1375)),
            posting("assets:checking", None),
            DraftPosting {
                cost: kabacus_core::CostEdit::Keep,
                account: AccountName::new("expenses:household"),
                amount: Some(usd(100)),
                ..posting("expenses:household", None)
            },
        ],
        tags: None,
    };
    let done = write(
        &state,
        &local(),
        WriteOp::Edit(req),
        &head(&state).await,
        false,
    )
    .await
    .unwrap();
    assert_eq!(done.id, Some(id));
    let book = books(&state).await;
    assert_eq!(book.transactions.len(), 2);
    let tx = &book.transactions[1];
    assert_eq!(tx.id, id);
    assert_eq!(tx.code, "1042");
    assert!(tx.comment.contains("receipt:2026-0042"), "{}", tx.comment);
    assert!(
        tx.comment.contains("kept for the warranty"),
        "{}",
        tx.comment
    );
    assert_eq!(tx.postings[0].amount, usd(1375));
    assert!(tx.postings[0].comment.contains("aisle:7"));
    assert_eq!(tx.postings[2].kind, PostingKind::Virtual);
}

#[tokio::test]
async fn an_edit_keeps_balance_assertions_unless_told_otherwise() {
    let state = state(
        "2026-02-10 Fresh Thyme Market\n    expenses:groceries  20.00 USD\n    assets:checking  -20.00 USD = 980.00 USD\n",
    )
    .await;
    let id = books(&state).await.transactions[1].id;
    let edit = |checking: AssertionEdit| EditRequest {
        id,
        date: date("2026-02-10"),
        description: "Fresh Thyme Market".to_string(),
        postings: vec![
            posting("expenses:groceries", Some(2000)),
            DraftPosting {
                cost: kabacus_core::CostEdit::Keep,
                balance_assertion: checking,
                ..posting("assets:checking", Some(-2000))
            },
        ],
        tags: None,
    };
    let kept = write(
        &state,
        &local(),
        WriteOp::Edit(edit(AssertionEdit::Keep)),
        &head(&state).await,
        false,
    )
    .await
    .unwrap();
    assert!(
        kept.warnings.iter().any(|w| w.code == "unchanged"),
        "{:?}",
        kept.warnings
    );
    write(
        &state,
        &local(),
        WriteOp::Edit(edit(AssertionEdit::Clear)),
        &head(&state).await,
        false,
    )
    .await
    .unwrap();
    let book = books(&state).await;
    assert!(book.transactions[1].postings[1].balance_assertion.is_none());
}

#[tokio::test]
async fn a_correction_is_an_edit_in_place_keeping_the_date() {
    let state = state("").await;
    let (id, head1) = add(&state, groceries(2000)).await;
    let op = WriteOp::Correct {
        id,
        request: CorrectionRequest::Correct {
            description: "Fresh Thyme Market".to_string(),
            postings: vec![
                posting("expenses:groceries", Some(2500)),
                posting("assets:checking", None),
            ],
        },
        today: date("2026-04-01"),
    };
    write(&state, &local(), op, &head1, false).await.unwrap();
    let book = books(&state).await;
    assert_eq!(book.transactions.len(), 2);
    let tx = book.transactions.iter().find(|t| t.id == id).unwrap();
    assert_eq!(tx.date, date("2026-02-01"));
    assert_eq!(tx.postings[0].amount, usd(2500));
}

#[tokio::test]
async fn a_void_adds_the_reversal_dated_today() {
    let state = state("").await;
    let (id, head1) = add(&state, groceries(2000)).await;
    let op = WriteOp::Correct {
        id,
        request: CorrectionRequest::Void,
        today: date("2026-04-01"),
    };
    let done = write(&state, &local(), op, &head1, false).await.unwrap();
    let book = books(&state).await;
    assert_eq!(book.transactions.len(), 3, "the original stays");
    let reversal = book
        .transactions
        .iter()
        .find(|t| Some(t.id) == done.id)
        .unwrap();
    assert_ne!(reversal.id, id);
    assert_eq!(reversal.date, date("2026-04-01"));
    assert!(
        reversal.comment.contains(&format!("#{id}")),
        "{}",
        reversal.comment
    );
    assert_eq!(reversal.postings[0].amount, usd(-2000));
}

#[tokio::test]
async fn set_status_changes_only_that_postings_mark() {
    let state = state("").await;
    let (id, head1) = add(&state, groceries(2000)).await;
    let op = WriteOp::SetStatus {
        id,
        account: AccountName::new("assets:checking"),
        status: Status::Cleared,
    };
    write(&state, &local(), op, &head1, false).await.unwrap();
    let book = books(&state).await;
    let tx = book.transactions.iter().find(|t| t.id == id).unwrap();
    assert_eq!(tx.status, Status::Unmarked);
    assert_eq!(tx.postings[0].status, Status::Unmarked);
    assert_eq!(tx.postings[1].status, Status::Cleared);
}

#[tokio::test]
async fn delete_removes_only_that_transaction_and_is_recorded_as_one() {
    let state = state("").await;
    let (id, head1) = add(&state, groceries(2000)).await;
    let done = write(&state, &local(), WriteOp::Delete { id }, &head1, false)
        .await
        .unwrap();
    let book = books(&state).await;
    assert_eq!(book.transactions.len(), 1);
    assert_eq!(book.transactions[0].description, "Opening");
    let change = state
        .store
        .change(state.ledger, done.head.parse().unwrap())
        .await
        .unwrap()
        .info;
    assert_eq!(change.kind, "delete");
    let missing = write(&state, &local(), WriteOp::Delete { id }, &done.head, true)
        .await
        .unwrap_err();
    assert!(matches!(missing, OpError::NotFound(_)), "{missing:?}");
}

#[tokio::test]
async fn more_places_than_the_commodity_takes_are_refused() {
    let state = state("").await;
    let head0 = head(&state).await;
    let mut req = groceries(0);
    req.postings[0].amount = Some(Amount::new("USD", Money::from_decimal(12345, 3)));
    let error = write(&state, &local(), WriteOp::Add(req), &head0, false)
        .await
        .unwrap_err();
    assert_eq!(
        refusal(error),
        "USD12.345 on expenses:groceries has more decimal places than USD takes (2)"
    );
    assert_eq!(head(&state).await, head0);
    assert_eq!(books(&state).await.commodities["USD"].style.places, Some(2));
}

/// A refused operation leaves the working copy as it was, so a batch
/// that carries on past it doesn't keep half of it.
#[tokio::test]
async fn a_refused_amount_leaves_the_working_copy_as_it_was() {
    let state = state("").await;
    let mut wc = WorkingCopy::load(&state, &head(&state).await)
        .await
        .unwrap();
    let before = wc.book.transactions.len();
    let mut req = groceries(0);
    req.postings[0].amount = Some(Amount::new("USD", Money::from_decimal(1, 3)));
    assert!(apply(&mut wc, WriteOp::Add(req)).is_err());
    assert_eq!(wc.book.transactions.len(), before);
    assert!(wc.changes().is_empty());
}

/// The message of an `amount` refusal.
fn refusal(error: OpError) -> String {
    match error {
        OpError::Invalid { rule, message } if rule == "amount" => message,
        other => panic!("{other:?}"),
    }
}

const HOUSEHOLD: &str = "account assets:bank:checking\n\
    account assets:bank:checking:gifts\n\
    account expenses:dental\n\
    \n\
    2026-01-02 Opening\n    assets:bank:checking  1721.79 USD\n    equity:opening\n\n\
    2026-01-15 Allocate | to Gifts\n    assets:bank:checking:gifts  40.00 USD\n    assets:bank:checking\n\n\
    2026-02-17 Northside Dental Group\n    expenses:dental  1186.00 USD\n    assets:bank:checking\n\n";

fn set_aside(from: &str, to: &str, cents: i128, on: &str) -> WriteOp {
    WriteOp::Allocate {
        from: from.to_string(),
        to: to.to_string(),
        amount: Money::from_decimal(cents, 2),
        date: date(on),
        description: None,
        new_fund: false,
    }
}

/// Setting aside or moving more than the source holds is refused with the
/// shortfall; what it holds exactly is fine.
#[tokio::test]
async fn an_allocation_may_take_only_what_its_source_holds() {
    const CHECKING: &str = "assets:bank:checking";
    const GIFTS: &str = "assets:bank:checking:gifts";
    let state = state(HOUSEHOLD).await;
    let head0 = head(&state).await;
    let refused = |op| async {
        write(&state, &local(), op, &head(&state).await, false)
            .await
            .unwrap_err()
    };
    assert_eq!(
        refusal(refused(set_aside(CHECKING, GIFTS, 99999900, "2026-01-16")).await),
        "Checking (account) has only $495.79 on 2026-02-17, $999,503.21 short of $999,999.00"
    );
    // Free on its own date, but a later spend would be left overdrawn.
    assert_eq!(
        refusal(refused(set_aside(CHECKING, GIFTS, 50000, "2026-01-16")).await),
        "Checking (account) has only $495.79 on 2026-02-17, $4.21 short of $500.00"
    );
    assert_eq!(
        refusal(refused(set_aside(GIFTS, CHECKING, 4001, "2026-03-01")).await),
        "Gifts has only $40.00 on 2026-03-01, $0.01 short of $40.01"
    );
    assert_eq!(head(&state).await, head0, "nothing written");

    write(
        &state,
        &local(),
        set_aside(CHECKING, GIFTS, 49579, "2026-01-16"),
        &head0,
        false,
    )
    .await
    .unwrap();
}

fn edit_set_aside(id: u64, cents: i128) -> WriteOp {
    WriteOp::Edit(EditRequest {
        id,
        date: date("2026-01-15"),
        description: "Allocate | to Gifts".to_string(),
        postings: vec![
            posting("assets:bank:checking:gifts", Some(cents)),
            posting("assets:bank:checking", None),
        ],
        tags: None,
    })
}

async fn set_aside_id(state: &Service) -> u64 {
    books(state)
        .await
        .transactions
        .iter()
        .find(|t| t.description == "Allocate | to Gifts")
        .unwrap()
        .id
}

/// Editing a set-aside is held to the same rule as making one; an edit
/// that takes less from an account already short is still allowed.
#[tokio::test]
async fn an_edited_set_aside_may_take_only_what_its_source_holds() {
    let books_ok = state(HOUSEHOLD).await;
    let id = set_aside_id(&books_ok).await;
    let head0 = head(&books_ok).await;
    let error = write(
        &books_ok,
        &local(),
        edit_set_aside(id, 99999900),
        &head0,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(
        refusal(error),
        "Checking (account) has only $535.79 on 2026-02-17, $999,463.21 short of $999,999.00"
    );
    assert_eq!(head(&books_ok).await, head0, "nothing written");
    write(
        &books_ok,
        &local(),
        edit_set_aside(id, 53579),
        &head0,
        false,
    )
    .await
    .unwrap();

    let short = state(&format!(
        "{HOUSEHOLD}2026-03-01 Best Buy\n    expenses:dental  900.00 USD\n    assets:bank:checking\n\n"
    ))
    .await;
    let id = set_aside_id(&short).await;
    let error = write(
        &short,
        &local(),
        edit_set_aside(id, 4001),
        &head(&short).await,
        true,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OpError::Invalid { .. }), "{error:?}");
    write(
        &short,
        &local(),
        edit_set_aside(id, 3000),
        &head(&short).await,
        false,
    )
    .await
    .unwrap();
}

const CARD_BOOKS: &str = "account assets:bank:checking\n\
    account assets:bank:checking:card-visa\n\
    account liabilities:credit-card:visa\n\
    account expenses:electronics\n\
    account income:salary\n\
    \n\
    2026-01-02 Opening\n    assets:bank:checking  8500.00 USD\n    equity:opening\n\n";

async fn cash_on_hand(state: &Service, as_of: Option<String>) -> crate::service::CashOnHandDto {
    let q = crate::service::CashOnHandQuery {
        as_of,
        ..Default::default()
    };
    crate::service::cash_on_hand(state, &local(), &q)
        .await
        .unwrap()
}

/// Cash on Hand is what can be spent today: a paycheque dated next month
/// isn't in it until that day, unless a later day is asked for.
#[tokio::test]
async fn cash_on_hand_leaves_out_future_transactions() {
    let payday = chrono::Local::now().date_naive() + chrono::Duration::days(30);
    let state = state(&format!(
        "{CARD_BOOKS}{payday} Northwind Logistics\n    assets:bank:checking  2184.33 USD\n    income:salary\n\n"
    ))
    .await;
    let today = cash_on_hand(&state, None).await;
    assert_eq!(today.cash_on_hand.exact, "8500.00");
    let on_payday = cash_on_hand(&state, Some(payday.to_string())).await;
    assert_eq!(on_payday.cash_on_hand.exact, "10684.33");
}

/// Paying a card more than it owes is realistic: the card goes into
/// credit and its reserve follows it below zero.
#[tokio::test]
async fn paying_a_card_more_than_it_owes_is_accepted() {
    let state = state(CARD_BOOKS).await;
    let purchase = WriteOp::CardPurchase {
        card: "liabilities:credit-card:visa".to_string(),
        expense_account: "expenses:electronics".to_string(),
        amount: Money::from_decimal(14999, 2),
        date: date("2026-01-21"),
        description: "Best Buy".to_string(),
        source: "free".to_string(),
        tags: Vec::new(),
    };
    write(&state, &local(), purchase, &head(&state).await, false)
        .await
        .unwrap();
    let before = cash_on_hand(&state, Some("2026-01-31".to_string())).await;
    let payment = WriteOp::PayCard {
        card: "liabilities:credit-card:visa".to_string(),
        amount: Money::from_decimal(500000, 2),
        date: date("2026-01-22"),
    };
    write(&state, &local(), payment, &head(&state).await, false)
        .await
        .unwrap();
    let after = cash_on_hand(&state, Some("2026-01-31".to_string())).await;
    let card = &after.cards[0];
    assert_eq!(card.balance.exact, "4850.01");
    assert_eq!(card.reserve_balance.exact, "-4850.01");
    assert_eq!(card.difference.exact, "0.00");
    assert_eq!(after.cash_on_hand.exact, before.cash_on_hand.exact);
    assert_eq!(
        before.money_on_hand.exact, "8500.00",
        "the purchase only moved free money into the reserve"
    );
    assert_eq!(after.money_on_hand.exact, "3500.00");
}

const WALLETS: &str = r#"commodity USD1,000.00
commodity ETH1.000000000000000000
commodity XMR1.000000000000
account assets:bank:checking  ; ynab-account:Checking
account assets:investments:ethereum  ; ynab-account:Ethereum wallet
account assets:investments:monero  ; ynab-account:Monero wallet
account expenses:fees:platform  ; ynab-envelope:Platform fees
account expenses:fees:broker  ; ynab-envelope:Broker fees
account equity:opening

2026-01-02 Opening balances
    assets:bank:checking  USD5218.46
    equity:opening  USD-5218.46
    assets:investments:ethereum  ETH1.234567890123456789
    equity:opening  ETH-1.234567890123456789
    assets:investments:monero  XMR12.345678901234
    equity:opening  XMR-12.345678901234

"#;

async fn wallet_request(
    s: &Service,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let head = s.store.revision(s.ledger).await.unwrap().to_string();
    let response = crate::dispatch::dispatch(
        s,
        &local(),
        crate::dispatch::Request {
            method: method.into(),
            path: path.into(),
            head: Some(head),
            body,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("{method} {path}: {e}"));
    serde_json::from_slice(&response.body).unwrap()
}

#[tokio::test]
async fn wallet_reporting_currency_follows_cash_and_lists_declared_precision() {
    let s = state(WALLETS).await;
    let info = wallet_request(&s, "GET", "/commodity", serde_json::Value::Null).await;
    assert_eq!(info["symbol"], "USD");
    assert!(info["commodities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["symbol"] == "ETH" && c["places"] == 18));
    let cash = wallet_request(&s, "GET", "/cash-on-hand", serde_json::Value::Null).await;
    assert_eq!(cash["cash_on_hand"]["exact"], "5218.46");
}

#[tokio::test]
async fn wallet_exchanges_keep_costs_fees_and_exact_balances_across_edits() {
    use serde_json::json;
    let s = state(WALLETS).await;
    let mut exchange = json!({"date":"2026-02-01", "description":"Cedar Exchange | Buy ETH", "postings":[
        {"account":"assets:bank:checking", "amount":"-1025.00"},
        {"account":"assets:investments:ethereum", "amount":{"commodity":"ETH","mantissa":"400000000000000000","places":18},
         "cost":{"kind":"total","amount":"1000.00"}},
        {"account":"expenses:fees:platform", "amount":"15.00"},
        {"account":"expenses:fees:broker", "amount":"10.00"}
    ], "dry_run":true});
    let before = s.store.revision(s.ledger).await.unwrap();
    wallet_request(&s, "POST", "/transactions", exchange.clone()).await;
    assert_eq!(s.store.revision(s.ledger).await.unwrap(), before);
    exchange["dry_run"] = json!(false);
    let saved = wallet_request(&s, "POST", "/transactions", exchange.clone()).await;
    let id = saved["id"].as_u64().unwrap();
    let path = format!("/transactions/{id}");
    let original = wallet_request(&s, "GET", &path, serde_json::Value::Null).await;
    assert_eq!(
        original["postings"][1]["cost"]["amount"]["exact"],
        "1000.00"
    );
    // An ordinary memo edit also preserves a cost omitted by an older client.
    exchange["description"] = json!("Cedar Exchange | Statement checked");
    exchange["postings"][1]
        .as_object_mut()
        .unwrap()
        .remove("cost");
    wallet_request(&s, "PATCH", &path, exchange).await;
    let edited = wallet_request(&s, "GET", &path, serde_json::Value::Null).await;
    assert_eq!(edited["postings"], original["postings"]);
    let register = wallet_request(
        &s,
        "GET",
        "/register/assets%3Ainvestments%3Aethereum",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(
        register.as_array().unwrap().last().unwrap()["balance"]["exact"],
        "1.634567890123456789"
    );
    let fees = wallet_request(
        &s,
        "GET",
        "/reports/categories?from=2026-02-01&to=2026-02-28&depth=3&commodity=USD",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(fees["rows"].as_array().unwrap().len(), 2);
    assert!(kabacus_ledger::check(&s.books_snapshot().await.unwrap().book).is_empty());
    wallet_request(
        &s,
        "POST",
        &format!("{path}/corrections"),
        json!({"void":true,"dry_run":false,"today":"2026-02-02"}),
    )
    .await;
    let register = wallet_request(
        &s,
        "GET",
        "/register/assets%3Ainvestments%3Aethereum",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(
        register.as_array().unwrap().last().unwrap()["balance"]["exact"],
        "1.234567890123456789"
    );
}

#[tokio::test]
async fn wallet_reconciliation_compares_and_asserts_the_selected_commodity() {
    use serde_json::json;
    let s = state(WALLETS).await;
    for (account, commodity, mantissa, places) in [
        ("assets:bank:checking", "USD", "521846", 2),
        (
            "assets:investments:ethereum",
            "ETH",
            "1234567890123456789",
            18,
        ),
        ("assets:investments:monero", "XMR", "12345678901234", 12),
    ] {
        let result = wallet_request(&s, "POST", "/reconcile", json!({"account":account,"date":"2026-02-01",
            "balance":{"commodity":commodity,"mantissa":mantissa,"places":places},"mode":"assert","dry_run":false})).await;
        assert_eq!(result["status"], "matched");
        assert_eq!(result["commodity"], commodity);
    }
}
