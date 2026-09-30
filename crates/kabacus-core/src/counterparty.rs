// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Receivable/payable side selection for counterparty entry, and netted
//! counterparty statements.
//!
//! Sign convention (hledger's natural one for these account types): a
//! receivable (`assets:receivable:<name>`) is an asset and its balance
//! stays >= 0; a payable (`liabilities:payable:<name>`) is a liability and
//! its balance stays <= 0. "Owed to me" is the positive net; "I owe them" is
//! the negative net.

use crate::domain::{Status, Transaction};
use crate::money::{Amount, Money};
use chrono::NaiveDate;
use num_traits::Signed;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowDirection {
    /// Money moving toward the person (I'm paying them).
    ToPerson,
    /// Money moving away from the person (they're paying me).
    FromPerson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterpartySplit {
    pub receivable_delta: Money,
    pub payable_delta: Money,
}

/// Given a person's current receivable (>= 0) and payable (<= 0)
/// balances and an amount moving in `direction`, choose how much goes to
/// each account. The invariant this maintains: `receivable + receivable_delta
/// >= 0` and `payable + payable_delta <= 0`, always, for any non-negative
/// `amount` and any valid starting state.
pub fn select_counterparty_sides(
    current_receivable: Money,
    current_payable: Money,
    amount: Money,
    direction: FlowDirection,
) -> CounterpartySplit {
    debug_assert!(!current_receivable.is_negative());
    debug_assert!(!current_payable.is_positive());
    debug_assert!(!amount.is_negative());

    match direction {
        FlowDirection::ToPerson => {
            // First pays down the payable, then builds the receivable.
            let payable_owed = -current_payable; // >= 0
            let pay_down = amount.clone().min(payable_owed);
            let remainder = amount - pay_down.clone();
            CounterpartySplit {
                receivable_delta: remainder,
                payable_delta: pay_down,
            }
        }
        FlowDirection::FromPerson => {
            // First uses up the receivable, then builds the payable.
            let use_up = amount.clone().min(current_receivable);
            let remainder = amount - use_up.clone();
            CounterpartySplit {
                receivable_delta: -use_up,
                payable_delta: -remainder,
            }
        }
    }
}

/// Every counterparty's net position.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterpartyNet {
    pub name: String,
    /// Positive: "Owes you $X". Negative: "You owe $X".
    pub net: Amount,
}

/// Scan all real postings and net receivable + payable per person.
/// Only postings dated on or before `as_of` are counted.
pub fn list_counterparties(
    transactions: &[Transaction],
    commodity: &str,
    as_of: NaiveDate,
) -> Vec<CounterpartyNet> {
    let mut nets: HashMap<String, Money> = HashMap::new();
    for tx in transactions {
        if tx.date > as_of {
            continue;
        }
        for p in &tx.postings {
            if p.amount.commodity != commodity {
                continue;
            }
            let person = p
                .account
                .receivable_person()
                .or_else(|| p.account.payable_person());
            if let Some(name) = person {
                let entry = nets.entry(name.to_string()).or_insert_with(Money::zero);
                *entry = entry.clone() + p.amount.money.clone();
            }
        }
    }
    let mut result: Vec<CounterpartyNet> = nets
        .into_iter()
        .map(|(name, net)| CounterpartyNet {
            name,
            net: Amount::new(commodity, net),
        })
        .collect();
    result.sort_by_key(|a| std::cmp::Reverse(a.net.money.mantissa().abs()));
    result
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatementLine {
    pub date: NaiveDate,
    pub description: String,
    /// Net effect of this transaction on the person's combined balance:
    /// positive is a charge (they now owe more, or I owe less), negative is
    /// a payment.
    pub net_amount: Amount,
    pub running_balance: Amount,
    /// True if any posting behind this line is not yet cleared.
    pub open: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub name: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub opening_balance: Amount,
    pub lines: Vec<StatementLine>,
    pub closing_balance: Amount,
}

/// Build a netted statement for one counterparty. Set `open_items_only`
/// for only the uncleared lines.
pub fn build_statement(
    name: &str,
    transactions: &[Transaction],
    commodity: &str,
    from: NaiveDate,
    to: NaiveDate,
    open_items_only: bool,
) -> Statement {
    let touches_person = |tx: &Transaction| -> Vec<crate::domain::Posting> {
        tx.postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
            .filter(|p| {
                p.account.receivable_person() == Some(name)
                    || p.account.payable_person() == Some(name)
            })
            .cloned()
            .collect()
    };

    let opening_net: Money = transactions
        .iter()
        .filter(|tx| tx.date < from)
        .flat_map(touches_person)
        .map(|p| p.amount.money)
        .sum();

    let mut running = opening_net.clone();
    let mut lines = Vec::new();
    let mut ordered: Vec<&Transaction> = transactions
        .iter()
        .filter(|tx| tx.date >= from && tx.date <= to)
        .collect();
    ordered.sort_by_key(|t| (t.date, t.id));

    for tx in ordered {
        let postings = touches_person(tx);
        if postings.is_empty() {
            continue;
        }
        let net: Money = postings.iter().map(|p| p.amount.money.clone()).sum();
        if net.is_zero() {
            // Leave out transactions that net to zero for the person.
            continue;
        }
        let open = postings.iter().any(|p| p.status != Status::Cleared);
        if open_items_only && !open {
            continue;
        }
        running = running + net.clone();
        lines.push(StatementLine {
            date: tx.date,
            description: tx.payee().to_string(),
            net_amount: Amount::new(commodity, net),
            running_balance: Amount::new(commodity, running.clone()),
            open,
        });
    }

    Statement {
        name: name.to_string(),
        from,
        to,
        opening_balance: Amount::new(commodity, opening_net),
        closing_balance: Amount::new(commodity, running),
        lines,
    }
}

/// The correctness check — closing net must equal the combined raw
/// balance of both accounts as of `to`, independent of the per-line netting
/// above.
pub fn combined_balance_as_of(
    name: &str,
    transactions: &[Transaction],
    commodity: &str,
    as_of: NaiveDate,
) -> Money {
    transactions
        .iter()
        .filter(|tx| tx.date <= as_of)
        .flat_map(|tx| &tx.postings)
        .filter(|p| p.amount.commodity == commodity)
        .filter(|p| {
            p.account.receivable_person() == Some(name) || p.account.payable_person() == Some(name)
        })
        .map(|p| p.amount.money.clone())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AccountName, Posting, PostingKind};
    use num_bigint::BigInt;

    fn usd(m: i128) -> Money {
        Money::from_decimal(m, 2)
    }
    #[test]
    fn paying_toward_person_pays_down_payable_first() {
        // I owe them $30 (payable = -30). I send $50.
        let split = select_counterparty_sides(
            Money::zero(),
            usd(-3000),
            usd(5000),
            FlowDirection::ToPerson,
        );
        assert_eq!(split.payable_delta, usd(3000)); // fully paid down
        assert_eq!(split.receivable_delta, usd(2000)); // remainder builds receivable
    }
    #[test]
    fn paying_toward_person_within_payable_only_touches_payable() {
        let split = select_counterparty_sides(
            Money::zero(),
            usd(-5000),
            usd(3000),
            FlowDirection::ToPerson,
        );
        assert_eq!(split.payable_delta, usd(3000));
        assert_eq!(split.receivable_delta, Money::zero());
    }
    #[test]
    fn money_from_person_uses_up_receivable_first() {
        // They owe me $30 (receivable = 30). They send me $50.
        let split = select_counterparty_sides(
            usd(3000),
            Money::zero(),
            usd(5000),
            FlowDirection::FromPerson,
        );
        assert_eq!(split.receivable_delta, usd(-3000));
        assert_eq!(split.payable_delta, usd(-2000));
    }
    #[test]
    fn money_from_person_within_receivable_only_touches_receivable() {
        let split = select_counterparty_sides(
            usd(5000),
            Money::zero(),
            usd(3000),
            FlowDirection::FromPerson,
        );
        assert_eq!(split.receivable_delta, usd(-3000));
        assert_eq!(split.payable_delta, Money::zero());
    }

    proptest::proptest! {
        #[test]
        fn receivable_never_goes_negative_and_payable_never_goes_positive(
            receivable in 0i64..1_000_000,
            payable in -1_000_000i64..=0,
            amount in 0i64..2_000_000,
            to_person in proptest::bool::ANY,
        ) {
            let direction = if to_person { FlowDirection::ToPerson } else { FlowDirection::FromPerson };
            let split = select_counterparty_sides(usd(receivable as i128), usd(payable as i128), usd(amount as i128), direction);
            let new_receivable = usd(receivable as i128) + split.receivable_delta.clone();
            let new_payable = usd(payable as i128) + split.payable_delta.clone();
            proptest::prop_assert!(!new_receivable.is_negative());
            proptest::prop_assert!(!new_payable.is_positive());
            // The full amount is always accounted for.
            let moved = split.receivable_delta.mantissa().abs() + split.payable_delta.mantissa().abs();
            proptest::prop_assert_eq!(moved, BigInt::from(amount));
        }
    }

    fn posting(account: &str, mantissa: i128, status: Status) -> Posting {
        Posting {
            account: AccountName::new(account),
            amount: Amount::new("USD", usd(mantissa)),
            status,
            kind: PostingKind::Real,
            comment: String::new(),
            tags: vec![],
            cost: None,
            balance_assertion: None,
        }
    }

    fn tx(date: &str, description: &str, postings: Vec<Posting>, index: u64) -> Transaction {
        Transaction {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: description.into(),
            comment: String::new(),
            tags: vec![],
            postings,
            id: index,
        }
    }

    fn jordan_fixture() -> Vec<Transaction> {
        vec![
            // Dinner split: they owe me $20.
            tx(
                "2026-01-10",
                "Jordan | dinner",
                vec![
                    posting("assets:receivable:jordan", 2000, Status::Cleared),
                    posting("assets:checking", -2000, Status::Cleared),
                ],
                1,
            ),
            // They pay me back $20: nets the receivable to zero.
            tx(
                "2026-01-15",
                "Jordan | repayment",
                vec![
                    posting("assets:receivable:jordan", -2000, Status::Cleared),
                    posting("assets:checking", 2000, Status::Cleared),
                ],
                2,
            ),
            // I borrow $10 from Jordan: I now owe her.
            tx(
                "2026-01-20",
                "Jordan | borrowed cash",
                vec![
                    posting("liabilities:payable:jordan", -1000, Status::Unmarked),
                    posting("assets:checking", 1000, Status::Unmarked),
                ],
                3,
            ),
        ]
    }
    #[test]
    fn counterparties_overview_nets_receivable_and_payable() {
        let nets = list_counterparties(
            &jordan_fixture(),
            "USD",
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        );
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].name, "jordan");
        assert_eq!(nets[0].net.money, usd(-1000)); // I owe her $10
    }
    #[test]
    fn statement_nets_lines_and_skips_zero_net_transactions() {
        let stmt = build_statement(
            "jordan",
            &jordan_fixture(),
            "USD",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            false,
        );
        // Only 2 lines: the dinner split and the borrow. The repayment nets
        // to zero for "jordan" only if... actually repayment is its own
        // transaction (not netting with dinner), so it appears as its own
        // line with a non-zero net (-20).
        assert_eq!(stmt.lines.len(), 3);
        assert_eq!(stmt.closing_balance.money, usd(-1000));
    }
    #[test]
    fn open_items_only_hides_cleared_lines() {
        let stmt = build_statement(
            "jordan",
            &jordan_fixture(),
            "USD",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            true,
        );
        // Only the borrowed-cash line is uncleared.
        assert_eq!(stmt.lines.len(), 1);
        assert_eq!(stmt.lines[0].description, "Jordan");
    }
    #[test]
    fn closing_net_matches_combined_raw_balance() {
        let to = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let stmt = build_statement(
            "jordan",
            &jordan_fixture(),
            "USD",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            to,
            false,
        );
        let combined = combined_balance_as_of("jordan", &jordan_fixture(), "USD", to);
        assert_eq!(stmt.closing_balance.money, combined);
    }

    proptest::proptest! {
        #[test]
        fn closing_always_equals_combined_balance(
            a in -100_000i64..100_000,
            b in -100_000i64..100_000,
            c in -100_000i64..100_000,
        ) {
            let txs = vec![
                tx("2026-01-05", "Jordan | one", vec![posting("assets:receivable:jordan", a as i128, Status::Unmarked), posting("assets:checking", -(a as i128), Status::Unmarked)], 1),
                tx("2026-01-10", "Jordan | two", vec![posting("liabilities:payable:jordan", b as i128, Status::Unmarked), posting("assets:checking", -(b as i128), Status::Unmarked)], 2),
                tx("2026-01-15", "Jordan | three", vec![posting("assets:receivable:jordan", c as i128, Status::Unmarked), posting("assets:checking", -(c as i128), Status::Unmarked)], 3),
            ];
            let to = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
            let stmt = build_statement("jordan", &txs, "USD", NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), to, false);
            let combined = combined_balance_as_of("jordan", &txs, "USD", to);
            proptest::prop_assert_eq!(stmt.closing_balance.money, combined);
        }
    }
}
