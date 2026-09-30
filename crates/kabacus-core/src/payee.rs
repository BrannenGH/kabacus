// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Payee memory: a known payee pre-fills the editor from its last
//! transaction.

use crate::Transaction;
use std::collections::HashMap;

/// One payee as the editor's autocomplete sees it.
#[derive(Debug, Clone, Copy)]
pub struct KnownPayee<'a> {
    /// The payee as last written.
    pub payee: &'a str,
    /// Its most recent transaction: what to pre-fill from.
    pub last: &'a Transaction,
    /// How many transactions it has.
    pub count: usize,
}

/// Payees are the same regardless of case and surrounding spaces.
fn payee_key(payee: &str) -> String {
    payee.trim().to_lowercase()
}

/// Later date wins; on the same date, the one added later.
fn is_later(a: &Transaction, b: &Transaction) -> bool {
    (a.date, a.id) > (b.date, b.id)
}

/// Every payee in `transactions`, most recently used first.
pub fn known_payees<'a>(transactions: &[&'a Transaction]) -> Vec<KnownPayee<'a>> {
    let mut by_key: HashMap<String, KnownPayee<'a>> = HashMap::new();
    for &tx in transactions {
        let payee = tx.payee().trim();
        if payee.is_empty() {
            continue;
        }
        by_key
            .entry(payee_key(payee))
            .and_modify(|known| {
                known.count += 1;
                if is_later(tx, known.last) {
                    known.last = tx;
                    known.payee = payee;
                }
            })
            .or_insert(KnownPayee {
                payee,
                last: tx,
                count: 1,
            });
    }
    let mut payees: Vec<KnownPayee<'a>> = by_key.into_values().collect();
    payees.sort_by(|a, b| {
        (b.last.date, b.last.id)
            .cmp(&(a.last.date, a.last.id))
            .then_with(|| a.payee.cmp(b.payee))
    });
    payees
}

/// The transaction to pre-fill from when the user picks `payee`.
pub fn last_transaction_for<'a>(
    transactions: &[&'a Transaction],
    payee: &str,
) -> Option<&'a Transaction> {
    let key = payee_key(payee);
    transactions
        .iter()
        .copied()
        .filter(|tx| payee_key(tx.payee()) == key)
        .reduce(|best, tx| if is_later(tx, best) { tx } else { best })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountName, Amount, Money, Posting, PostingKind, Status};
    use chrono::NaiveDate;

    fn tx(date: &str, description: &str, account: &str, cents: i128, index: u64) -> Transaction {
        let posting = |account: &str, cents: i128| Posting {
            account: AccountName::new(account),
            amount: Amount::new("USD".to_string(), Money::from_decimal(cents, 2)),
            status: Status::Unmarked,
            kind: PostingKind::Real,
            comment: String::new(),
            tags: vec![],
            cost: None,
            balance_assertion: None,
        };
        Transaction {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: description.to_string(),
            comment: String::new(),
            tags: vec![],
            postings: vec![posting(account, cents), posting("assets:checking", -cents)],
            id: index,
        }
    }
    #[test]
    fn prefills_from_the_payees_latest_transaction() {
        let txs = [
            tx(
                "2026-01-05",
                "Corner Grocer | weekly shop",
                "expenses:groceries",
                4200,
                0,
            ),
            tx("2026-02-09", "corner grocer", "expenses:household", 1500, 1),
            tx("2026-01-20", "Corner Grocer", "expenses:groceries", 3900, 2),
            tx("2026-03-01", "Power Co", "expenses:utilities", 8000, 3),
        ];
        let refs: Vec<&Transaction> = txs.iter().collect();
        // Case and the memo after `|` don't matter; the latest date wins.
        let last = last_transaction_for(&refs, "CORNER GROCER ").unwrap();
        assert_eq!(last.postings[0].account.as_str(), "expenses:household");
        assert_eq!(last.postings[0].amount.money, Money::from_decimal(1500, 2));
        assert!(last_transaction_for(&refs, "Nobody").is_none());
    }
    #[test]
    fn same_day_ties_go_to_the_one_added_later() {
        let txs = [
            tx("2026-01-05", "Cafe", "expenses:coffee", 400, 7),
            tx("2026-01-05", "Cafe", "expenses:lunch", 1200, 3),
        ];
        let refs: Vec<&Transaction> = txs.iter().collect();
        let last = last_transaction_for(&refs, "Cafe").unwrap();
        assert_eq!(last.postings[0].account.as_str(), "expenses:coffee");
    }
    #[test]
    fn lists_known_payees_most_recent_first() {
        let txs = [
            tx("2026-01-05", "Corner Grocer", "expenses:groceries", 4200, 0),
            tx("2026-03-01", "Power Co", "expenses:utilities", 8000, 1),
            tx("2026-02-09", "corner grocer", "expenses:groceries", 1500, 2),
        ];
        let refs: Vec<&Transaction> = txs.iter().collect();
        let payees = known_payees(&refs);
        let names: Vec<&str> = payees.iter().map(|p| p.payee).collect();
        assert_eq!(names, ["Power Co", "corner grocer"]);
        assert_eq!(payees[1].count, 2);
    }
}
