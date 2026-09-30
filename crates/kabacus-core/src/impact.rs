// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! What a write will do to account balances, for the confirmation
//! dialog ("affected account balances before and after").

use crate::money::{Amount, Money};
use crate::Transaction;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceChange {
    pub account: String,
    pub before: Amount,
    pub after: Amount,
}

/// For every (account, commodity) that `added` touches: its balance over
/// `existing` and after `added` is appended. A reversal and its original
/// net to zero. Sorted by account, then commodity; accounts
/// whose balance doesn't change (e.g. both sides of a void net out) are
/// still listed, so the dialog shows everything the write touches.
pub fn balance_impact(existing: &[Transaction], added: &[&Transaction]) -> Vec<BalanceChange> {
    let mut touched: BTreeMap<(String, String), (Money, Money)> = BTreeMap::new();
    for tx in added {
        for p in &tx.postings {
            let key = (p.account.as_str().to_string(), p.amount.commodity.clone());
            let entry = touched.entry(key).or_insert((Money::zero(), Money::zero()));
            entry.1 = entry.1.clone() + p.amount.money.clone();
        }
    }
    for tx in existing {
        for p in &tx.postings {
            let key = (p.account.as_str().to_string(), p.amount.commodity.clone());
            if let Some(entry) = touched.get_mut(&key) {
                entry.0 = entry.0.clone() + p.amount.money.clone();
            }
        }
    }
    touched
        .into_iter()
        .map(|((account, commodity), (before, delta))| {
            // A zero "before" has no decimal places of its own; show both
            // sides at the same precision.
            let places = before.places().max(delta.places());
            BalanceChange {
                account,
                before: Amount::new(commodity.clone(), before.rescaled(places)),
                after: Amount::new(commodity, (before + delta).rescaled(places)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountName, Posting, PostingKind, Status};
    use chrono::NaiveDate;

    fn tx(postings: &[(&str, i128)]) -> Transaction {
        Transaction {
            date: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: "x".into(),
            comment: String::new(),
            tags: vec![],
            postings: postings
                .iter()
                .map(|(account, cents)| Posting {
                    account: AccountName::new(*account),
                    amount: Amount::new("USD", Money::from_decimal(*cents, 2)),
                    status: Status::Unmarked,
                    kind: PostingKind::Real,
                    comment: String::new(),
                    tags: vec![],
                    cost: None,
                    balance_assertion: None,
                })
                .collect(),
            id: 0,
        }
    }

    fn usd(cents: i128) -> Amount {
        Amount::new("USD", Money::from_decimal(cents, 2))
    }
    #[test]
    fn a_correction_shows_each_touched_balance_before_and_after() {
        let existing = [
            tx(&[("assets:checking", 100000), ("equity:opening", -100000)]),
            tx(&[("expenses:groceries", 2000), ("assets:checking", -2000)]),
        ];
        // Correct the $20 grocery purchase to $30 of household.
        let reversal = tx(&[("expenses:groceries", -2000), ("assets:checking", 2000)]);
        let replacement = tx(&[("expenses:household", 3000), ("assets:checking", -3000)]);
        let impact = balance_impact(&existing, &[&reversal, &replacement]);
        assert_eq!(
            impact,
            [
                BalanceChange {
                    account: "assets:checking".into(),
                    before: usd(98000),
                    after: usd(97000),
                },
                BalanceChange {
                    account: "expenses:groceries".into(),
                    before: usd(2000),
                    after: usd(0),
                },
                BalanceChange {
                    account: "expenses:household".into(),
                    before: usd(0),
                    after: usd(3000),
                },
            ]
        );
    }
    #[test]
    fn a_new_account_starts_from_zero_at_the_same_precision() {
        let added = tx(&[("expenses:household", 3000), ("assets:checking", -3000)]);
        let impact = balance_impact(&[], &[&added]);
        assert_eq!(impact[1].before.money.to_string(), "0.00");
    }
    #[test]
    fn untouched_accounts_are_left_out() {
        let existing = [tx(&[("assets:savings", 500), ("equity:opening", -500)])];
        let added = tx(&[("expenses:groceries", 100), ("assets:checking", -100)]);
        let accounts: Vec<String> = balance_impact(&existing, &[&added])
            .into_iter()
            .map(|c| c.account)
            .collect();
        assert_eq!(accounts, ["assets:checking", "expenses:groceries"]);
    }
}
