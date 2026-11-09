// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The strict-checks and assertions pages' examples, with the engine's
//! rules.

use super::*;
use crate::import::{read, MemorySource};
use std::path::{Path, PathBuf};

const HEAD: &str = "commodity 1000.00 USD\ncommodity 1000.00 EUR\ncommodity 1000.00 GBP\naccount assets:a\naccount assets:a:sub\naccount assets:ab\naccount assets:b\naccount expenses:e\naccount income:i\n\n";

fn failures_of(body: &str) -> Vec<CheckFailure> {
    let src = MemorySource(
        [(PathBuf::from("/b/j.journal"), format!("{HEAD}{body}"))]
            .into_iter()
            .collect(),
    );
    read(&src, Path::new("/b/j.journal")).unwrap().check()
}

fn kinds(body: &str) -> Vec<FailureKind> {
    failures_of(body).iter().map(|f| f.kind).collect()
}

fn passes(body: &str) {
    let failures = failures_of(body);
    assert!(failures.is_empty(), "{body}\n{failures:#?}");
}

fn fails(body: &str, kind: FailureKind) -> CheckFailure {
    let failures = failures_of(body);
    assert!(!failures.is_empty(), "{body} passed");
    assert_eq!(failures[0].kind, kind, "{body}\n{failures:#?}");
    failures[0].clone()
}

const DEP: &str = "    assets:a  10.00 USD\n    income:i\n";

#[test]
fn a_balanced_declared_book_passes() {
    // S1, S8, S27
    passes("2026-01-02 ok\n    expenses:e  10.00 USD\n    assets:a\n");
    passes("2026-01-05 later\n    expenses:e  1.00 USD\n    assets:a\n\n2026-01-02 earlier\n    expenses:e  1.00 USD\n    assets:a\n");
    passes("2026-01-02\n    assets:a  0.00 USD\n\n2026-01-03 none\n");
}

#[test]
fn undeclared_commodities_and_accounts() {
    // S2, S3, S4, S16, S17, S24, S25, S26, S28, S30, S31, S32, S33, S34, S36
    let f = fails(
        "2026-01-02 x\n    expenses:zzz  10.00 USD\n    assets:a\n",
        FailureKind::Account,
    );
    assert_eq!(
        f.message(),
        "The account expenses:zzz hasn't been declared. (j.journal:12)"
    );
    let f = fails(
        "2026-01-02 x\n    expenses:e  10.00 CHF\n    assets:a\n",
        FailureKind::Commodity,
    );
    assert_eq!(
        f.message(),
        "The commodity CHF hasn't been declared. (j.journal:12)"
    );
    assert_eq!(
        kinds("2026-01-02 x\n    expenses:zzz  10.00 CHF\n    assets:yyy\n"),
        [
            FailureKind::Commodity,
            FailureKind::Account,
            FailureKind::Account
        ]
    );
    let f = fails(
        "2026-01-02 x\n    expenses:e  10\n    assets:a  -10\n",
        FailureKind::Commodity,
    );
    assert_eq!(
        f.sentence,
        "A number with no commodity hasn't been declared as one."
    );
    assert_eq!(f.details, "Used once more.");
    for account in ["Assets:a", "assets:abc", "assets", "assets:a:sub:x"] {
        fails(
            &format!("2026-01-02 x\n    {account}  1.00 USD\n    assets:a\n"),
            FailureKind::Account,
        );
    }
    fails(
        "2026-01-02 x\n    expenses:e  1.00 USD\n    assets:a\n    (zz)  1.00 USD\n",
        FailureKind::Account,
    );
    fails("2026-01-02 x\n    expenses:e  1.00 USD\n    assets:a\n    [zz:b]  1.00 USD\n    [expenses:e]  -1.00 USD\n", FailureKind::Account);
    fails(
        "2026-01-02 x\n    assets:q  0.00 USD = 0.00 USD\n    assets:a  0.00 USD\n",
        FailureKind::Account,
    );
    fails(
        "2026-01-02 x\n    expenses:e  1.00 USD\n    assets:a\n    expenses:e  0 CHF\n",
        FailureKind::Commodity,
    );
    fails(
        "2026-01-02 x\n    assets:a  0.00 USD = 0 CHF\n",
        FailureKind::Commodity,
    );
    fails(
        "2026-01-02 x\n    expenses:e  10.00 USD @ 2.00 CHF\n    assets:a  -20.00 CHF\n",
        FailureKind::Commodity,
    );
}

#[test]
fn declared_anywhere_counts() {
    // S22, S35, S37
    passes("2026-01-02 x\n    late:acct  1.00 EUR\n    assets:a\n\naccount late:acct\n");
    passes("commodity CHF\n\n2026-01-02 x\n    expenses:e  5 CHF\n    assets:a  -5 CHF\n");
    passes("commodity 1000.00\n\n2026-01-02 x\n    expenses:e  5\n    assets:a  -5\n");
}

#[test]
fn every_failure_reported_once_by_kind_then_position() {
    // S13, S16, S18, S19, S20, S21
    let failures = failures_of(
        "2026-01-02 one\n    expenses:yyy  5.00 USD\n    assets:a\n\n2026-01-03 two\n    expenses:aaa  5.00 USD\n    expenses:yyy  5.00 USD\n    assets:a  -9.00 USD\n\n2026-01-04 three\n    expenses:e  1.00 USD\n    expenses:e  1.00 CHF\n    expenses:e  1.00 XAU\n    assets:a  -1.00 USD\n    assets:a  -1.00 CHF\n    assets:a  -1.00 XAU\n",
    );
    let summary: Vec<String> = failures.iter().map(CheckFailure::message).collect();
    assert_eq!(
        summary,
        [
            "The transaction on 2026-01-03 doesn't balance: 1.00 USD is left over. (j.journal:15)",
            "The commodity CHF hasn't been declared. (j.journal:22)",
            "The commodity XAU hasn't been declared. (j.journal:23)",
            "The account expenses:yyy hasn't been declared. (j.journal:12)",
        ]
    );
    assert_eq!(failures[1].details, "Used once more.");
}

#[test]
fn balancing_failures_name_the_group_and_what_is_left_over() {
    // S5, S6, S7
    let f = fails(
        "2026-01-02 x\n    expenses:e  10.00 USD\n    assets:a  -9.00 USD\n",
        FailureKind::Balanced,
    );
    assert_eq!(
        f.sentence,
        "The transaction on 2026-01-02 doesn't balance: 1.00 USD is left over."
    );
    let f = fails(
        "2026-01-02 x\n    expenses:e  10.00 USD\n    assets:a  -10.00 USD\n    [expenses:e]  1.00 USD\n    [assets:a]  -2.00 USD\n",
        FailureKind::Balanced,
    );
    assert_eq!(
        f.sentence,
        "The bracketed postings of the transaction on 2026-01-02 don't balance: -1.00 USD is left over."
    );
    let f = fails(
        "2026-01-02 x\n    expenses:e\n    assets:a\n",
        FailureKind::Balanced,
    );
    assert_eq!(
        f.sentence,
        "Only one posting in the transaction on 2026-01-02 can leave out its amount."
    );
}

#[test]
fn the_four_operators() {
    // P1 to P10
    let p1 = "2026-01-01 open\n    assets:a  10.00 USD\n    assets:a  5.00 EUR\n    income:i  -10.00 USD\n    income:i  -5.00 EUR\n\n";
    passes(&format!(
        "{p1}2026-01-02 c\n    assets:a  0.00 USD = 10.00 USD\n"
    ));
    let f = fails(
        &format!("{p1}2026-01-02 c\n    assets:a  0.00 USD == 10.00 USD\n"),
        FailureKind::Assertion,
    );
    assert_eq!(
        f.sentence,
        "At the end of 2026-01-02, assets:a holds 5.00 EUR, 10.00 USD, not the 10.00 USD you asserted."
    );
    assert!(
        f.details.contains("other commodities: EUR"),
        "{}",
        f.details
    );
    let only_usd = "2026-01-01 open\n    assets:a  10.00 USD\n    income:i\n\n";
    passes(&format!(
        "{only_usd}2026-01-02 c\n    assets:a  0.00 USD == 10.00 USD\n"
    ));
    let tree =
        "2026-01-01 open\n    assets:a:sub  10.00 USD\n    assets:a  3.00 USD\n    income:i\n\n";
    passes(&format!(
        "{tree}2026-01-02 c\n    assets:a  0.00 USD = 3.00 USD\n\n2026-01-03 c\n    assets:a  0.00 USD =* 13.00 USD\n"
    ));
    let f = fails(
        &format!("{tree}2026-01-02 c\n    assets:a  0.00 USD = 13.00 USD\n"),
        FailureKind::Assertion,
    );
    assert_eq!(
        f.message(),
        "At the end of 2026-01-02, assets:a holds 3.00 USD, not the 13.00 USD you asserted. (j.journal:17)"
    );
    // P8, P9
    passes("2026-01-01 open\n    assets:a  3.00 USD\n    income:i\n\n2026-01-02 c\n    assets:a  0.00 EUR = 0.00 EUR\n");
    fails(
        "2026-01-01 open\n    assets:a  3.00 USD\n    income:i\n\n2026-01-02 c\n    assets:a  0.00 EUR == 0.00 EUR\n",
        FailureKind::Assertion,
    );
    // V4: assets:ab isn't under assets:a.
    passes("2026-01-01 open\n    assets:a  10.00 USD\n    assets:ab  2.00 USD\n    income:i\n\n2026-01-02 c\n    assets:a  0.00 USD =* 10.00 USD\n");
}

#[test]
fn a_total_inclusive_assertion_checks_every_commodity_of_the_subtree() {
    // The engine checks every commodity.
    let book = "2026-01-01 open\n    assets:a:sub  10.00 USD\n    assets:a  2.00 EUR\n    income:i  -10.00 USD\n    income:i  -2.00 EUR\n\n";
    let f = fails(
        &format!("{book}2026-01-02 c\n    assets:a  0.00 EUR ==* 2.00 EUR\n"),
        FailureKind::Assertion,
    );
    assert_eq!(
        f.sentence,
        "At the end of 2026-01-02, assets:a and its subaccounts together hold 2.00 EUR, 10.00 USD, not the 2.00 EUR you asserted."
    );
    passes(&format!(
        "{book}2026-01-02 c\n    assets:a  0.00 EUR =* 2.00 EUR\n"
    ));
}

#[test]
fn assertions_are_end_of_day_balances() {
    // Q1 and Q2 both pass: order within a day doesn't matter.
    passes(&format!(
        "2026-01-05 dep\n{DEP}\n2026-01-05 chk\n    assets:a  0.00 USD = 10.00 USD\n"
    ));
    passes(&format!(
        "2026-01-05 chk\n    assets:a  0.00 USD = 10.00 USD\n\n2026-01-05 dep\n{DEP}"
    ));
    // Q3 passes, Q4 fails: dates decide, not file order.
    passes(&format!(
        "2026-01-05 chk\n    assets:a  0.00 USD = 10.00 USD\n\n2026-01-04 dep\n{DEP}"
    ));
    fails(
        &format!("2026-01-04 chk\n    assets:a  0.00 USD = 10.00 USD\n\n2026-01-05 dep\n{DEP}"),
        FailureKind::Assertion,
    );
    // Q5 and Q6 both pass: the assertion is the day's end balance.
    passes("2026-01-05 t\n    assets:a  4.00 USD = 10.00 USD\n    assets:a  6.00 USD = 10.00 USD\n    income:i\n");
    // Q16, Q17
    passes(&format!("2026-01-05 dep\n{DEP}\n2026-01-06 more\n    assets:a  5.00 USD = 15.00 USD\n    income:i\n"));
    fails(
        &format!("2026-01-05 dep\n{DEP}\n2026-01-06 more\n    assets:a  5.00 USD = 10.00 USD\n    income:i\n"),
        FailureKind::Assertion,
    );
    // S9: comparison is by value.
    passes(&format!(
        "2026-01-05 dep\n{DEP}\n2026-01-06 c\n    assets:a  0 USD = 10 USD\n    assets:a  0 USD = 10.000 USD\n"
    ));
    // A posting's date: tag is an ordinary comment (Q12 fails here).
    fails(
        "2026-01-03 dep\n    assets:a  10.00 USD  ; date:2026-01-06\n    income:i\n\n2026-01-05 chk\n    assets:a  0.00 USD = 0.00 USD\n",
        FailureKind::Assertion,
    );
}

#[test]
fn an_assertion_counts_only_transactions_that_balance() {
    let failures = failures_of(
        "2026-01-05 bad\n    assets:a  10.00 USD\n    income:i  -9.00 USD\n\n2026-01-06 chk\n    assets:a  0.00 USD = 10.00 USD\n",
    );
    let kinds: Vec<FailureKind> = failures.iter().map(|f| f.kind).collect();
    assert_eq!(kinds, [FailureKind::Balanced, FailureKind::Assertion]);
    assert!(failures[1].details.contains("don't balance"));
}

#[test]
fn costs_are_ignored_by_assertions() {
    passes("2026-01-02 fx\n    assets:a  10.00 EUR @@ 11.00 USD\n    income:i\n\n2026-01-03 c\n    assets:a  0.00 EUR = 10.00 EUR\n");
}

fn book_of(body: &str) -> Book {
    let src = MemorySource(
        [(PathBuf::from("/b/j.journal"), format!("{HEAD}{body}"))]
            .into_iter()
            .collect(),
    );
    read(&src, Path::new("/b/j.journal")).unwrap().book
}

#[test]
fn check_change_sees_what_a_change_breaks() {
    let before = book_of(&format!(
        "2026-01-05 dep\n{DEP}\n2026-01-06 chk\n    assets:a  0.00 USD = 10.00 USD\n    income:i  0.00 USD\n"
    ));
    assert!(check(&before).is_empty());
    let mut after = before.clone();
    let old = after.transactions[0].clone();
    after.transactions[0].postings[0].amount.money = kabacus_core::Money::from_decimal(1200, 2);
    after.transactions[0].postings[1].amount.money = kabacus_core::Money::from_decimal(-1200, 2);
    let change = Change {
        before: vec![old],
        after: [1].into_iter().collect(),
        ..Default::default()
    };
    let failures = check_change(&after, &change);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].kind, FailureKind::Assertion);
    assert_eq!(
        failures[0].position,
        Position::Transaction {
            id: 2,
            posting: Some(0)
        }
    );
    assert_eq!(check(&after), failures);

    let mut undeclared = before.clone();
    undeclared.accounts.remove("income:i");
    let change = Change {
        undeclared_accounts: ["income:i".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let failures = check_change(&undeclared, &change);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].details, "Used once more.");
}

mod same_verdict {
    use super::*;
    use kabacus_core::{AccountName, BalanceAssertion, Money, Posting, PostingKind, Status};
    use proptest::prelude::*;

    const ACCOUNTS: [&str; 4] = ["assets:a", "assets:a:sub", "assets:b", "income:i"];

    fn tx(
        id: u64,
        day: u32,
        legs: &[(usize, i64)],
        assert: Option<(usize, i64, bool)>,
    ) -> Transaction {
        let mut postings: Vec<Posting> = legs
            .iter()
            .map(|&(a, cents)| Posting {
                account: AccountName::new(ACCOUNTS[a]),
                amount: Amount::new("USD", Money::from_decimal(cents as i128, 2)),
                status: Status::Unmarked,
                kind: PostingKind::Real,
                comment: String::new(),
                tags: vec![],
                cost: None,
                balance_assertion: None,
            })
            .collect();
        let total: i64 = legs.iter().map(|l| l.1).sum();
        postings.push(Posting {
            account: AccountName::new("income:i"),
            amount: Amount::new("USD", Money::from_decimal(-total as i128, 2)),
            status: Status::Unmarked,
            kind: PostingKind::Real,
            comment: String::new(),
            tags: vec![],
            cost: None,
            balance_assertion: None,
        });
        if let Some((a, cents, inclusive)) = assert {
            postings.push(Posting {
                account: AccountName::new(ACCOUNTS[a]),
                amount: Amount::new("USD", Money::from_decimal(0, 2)),
                status: Status::Unmarked,
                kind: PostingKind::Real,
                comment: String::new(),
                tags: vec![],
                cost: None,
                balance_assertion: Some(BalanceAssertion {
                    inclusive,
                    ..BalanceAssertion::new(Amount::new(
                        "USD",
                        Money::from_decimal(cents as i128, 2),
                    ))
                }),
            });
        }
        Transaction {
            id,
            date: NaiveDate::from_ymd_opt(2026, 1, day).unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: format!("t{id}"),
            comment: String::new(),
            tags: vec![],
            postings,
        }
    }

    fn base() -> Book {
        book_of("")
    }

    /// A valid book: assertions set to what the balances are.
    fn settle(book: &mut Book) {
        loop {
            let failures = check(book);
            let Some(f) = failures.iter().find(|f| f.kind == FailureKind::Assertion) else {
                return;
            };
            let Position::Transaction {
                id,
                posting: Some(p),
            } = f.position
            else {
                return;
            };
            let tx = book.transactions.iter_mut().find(|t| t.id == id).unwrap();
            tx.postings.remove(p);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn check_change_agrees_with_check(
            txs in proptest::collection::vec(
                (1u32..10, proptest::collection::vec((0usize..3, -5000i64..5000), 1..3),
                 proptest::option::of((0usize..3, -5000i64..5000, any::<bool>()))),
                1..8),
            edit in (0usize..8, 1u32..10, -5000i64..5000),
        ) {
            let mut book = base();
            for (i, (day, legs, assert)) in txs.iter().enumerate() {
                book.transactions.push(tx(i as u64 + 1, *day, legs, *assert));
            }
            settle(&mut book);
            prop_assume!(check(&book).is_empty());
            let (which, day, cents) = edit;
            let index = which % book.transactions.len();
            let old = book.transactions[index].clone();
            let mut after = book.clone();
            {
                let t = &mut after.transactions[index];
                t.date = NaiveDate::from_ymd_opt(2026, 1, day).unwrap();
                t.postings[0].amount.money = Money::from_decimal(cents as i128, 2);
                let total: Money = t.postings.iter().take(t.postings.len()).filter(|p| p.account.as_str() != "income:i").map(|p| p.amount.money.clone()).sum();
                if let Some(income) = t.postings.iter_mut().find(|p| p.account.as_str() == "income:i") {
                    income.amount.money = -total;
                }
            }
            let change = Change {
                before: vec![old.clone()],
                after: [old.id].into_iter().collect(),
                ..Default::default()
            };
            let full = check(&after);
            let incremental = check_change(&after, &change);
            prop_assert_eq!(full.is_empty(), incremental.is_empty(), "{:#?}\n{:#?}", full, incremental);
            prop_assert_eq!(full, incremental);
        }
    }
}
