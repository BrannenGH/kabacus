// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reversing a transaction (a void). A correction is an edit in place, so
//! it needs nothing here. A reversal says in its comment what it reverses;
//! nothing links the two transactions.

use crate::domain::{Posting, Status, Transaction};
use chrono::NaiveDate;

/// Every posting with its amount negated, keeping accounts and status
/// marks. A reversal never carries the original's balance assertion
/// forward: it described the original date's state.
fn negate_postings(postings: &[Posting]) -> Vec<Posting> {
    postings
        .iter()
        .map(|p| Posting {
            amount: crate::money::Amount::new(p.amount.commodity.clone(), -p.amount.money.clone()),
            cost: None,
            balance_assertion: None,
            ..p.clone()
        })
        .collect()
}

/// The note a reversal of `original` carries in its comment.
pub fn reversal_note(original: &Transaction) -> String {
    format!(
        "Reverses #{} of {}",
        original.id,
        original.date.format("%Y-%m-%d")
    )
}

/// The reversal of `original`, dated `today`: every amount negated, so the
/// two together change no balance.
pub fn build_reversal(original: &Transaction, today: NaiveDate) -> Transaction {
    Transaction {
        id: 0,
        date: today,
        status: Status::Unmarked,
        code: String::new(),
        description: original.description.clone(),
        comment: reversal_note(original),
        tags: Vec::new(),
        postings: negate_postings(&original.postings),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AccountName, PostingKind};
    use crate::money::{Amount, Money};

    fn usd(mantissa: i128) -> Amount {
        Amount::new("USD", Money::from_decimal(mantissa, 2))
    }

    fn posting(account: &str, mantissa: i128, status: Status) -> Posting {
        Posting {
            account: AccountName::new(account),
            amount: usd(mantissa),
            status,
            kind: PostingKind::Real,
            comment: String::new(),
            tags: vec![],
            cost: None,
            balance_assertion: None,
        }
    }

    fn original_tx(id: u64, amount: i128) -> Transaction {
        Transaction {
            id,
            date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: "Grocery Store".into(),
            comment: String::new(),
            tags: vec![],
            postings: vec![
                posting("expenses:groceries", amount, Status::Cleared),
                posting("assets:checking", -amount, Status::Cleared),
            ],
        }
    }

    #[test]
    fn a_reversal_negates_every_posting_is_dated_today_and_says_what_it_reverses() {
        let orig = original_tx(42, 4500);
        let today = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let reversal = build_reversal(&orig, today);

        assert_eq!(reversal.id, 0);
        assert_eq!(reversal.date, today);
        assert_eq!(reversal.postings[0].amount.money.to_string(), "-45.00");
        assert_eq!(reversal.postings[1].amount.money.to_string(), "45.00");
        assert_eq!(reversal.postings[0].status, Status::Cleared);
        assert_eq!(reversal.comment, "Reverses #42 of 2026-01-05");
        assert!(reversal.tags.is_empty());
    }

    #[test]
    fn a_reversal_drops_balance_assertions() {
        let mut orig = original_tx(7, 100);
        orig.postings[1].balance_assertion = Some(crate::domain::BalanceAssertion::new(usd(0)));
        let reversal = build_reversal(&orig, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
        assert!(reversal
            .postings
            .iter()
            .all(|p| p.balance_assertion.is_none()));
    }

    proptest::proptest! {
        #[test]
        fn a_reversal_always_nets_the_original_to_zero(amount in -1_000_000i64..1_000_000) {
            let orig = original_tx(1, amount as i128);
            let today = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
            let reversal = build_reversal(&orig, today);
            let net: Money = orig.postings.iter().zip(&reversal.postings).map(|(o, v)| o.amount.money.clone() + v.amount.money.clone()).sum();
            proptest::prop_assert!(net.is_zero());
        }
    }
}
