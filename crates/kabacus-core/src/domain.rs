// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core ledger domain types: transactions, postings, accounts. No I/O.

use crate::money::Amount;
use chrono::NaiveDate;

/// hledger's posting/transaction status mark: unmarked, pending (`!`) or
/// cleared (`*`). The migrated books carry YNAB's cleared flags as posting
/// marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Status {
    #[default]
    Unmarked,
    Pending,
    Cleared,
}

impl Status {
    /// Clicking cycles unmarked -> pending -> cleared -> unmarked.
    pub fn cycle(self) -> Status {
        match self {
            Status::Unmarked => Status::Pending,
            Status::Pending => Status::Cleared,
            Status::Cleared => Status::Unmarked,
        }
    }
}

pub type Tag = (String, String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountName(pub String);

impl AccountName {
    pub fn new(s: impl Into<String>) -> Self {
        AccountName(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split(':')
    }

    pub fn leaf(&self) -> &str {
        self.segments().last().unwrap_or("")
    }

    pub fn is_under(&self, prefix: &str) -> bool {
        self.0 == prefix || self.0.starts_with(&format!("{prefix}:"))
    }

    /// If this is a person's receivable account (`assets:receivable:<name>`),
    /// the person's leaf name.
    pub fn receivable_person(&self) -> Option<&str> {
        self.0.strip_prefix("assets:receivable:")
    }

    /// If this is a person's payable account (`liabilities:payable:<name>`),
    /// the person's leaf name.
    pub fn payable_person(&self) -> Option<&str> {
        self.0.strip_prefix("liabilities:payable:")
    }
}

impl std::fmt::Display for AccountName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostingKind {
    Real,
    Virtual,
    BalancedVirtual,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Posting {
    pub account: AccountName,
    pub amount: Amount,
    pub status: Status,
    pub kind: PostingKind,
    /// hledger's `pcomment`: the text of the posting's comment lines, tag
    /// declarations included.
    pub comment: String,
    /// The posting's own tags, the ones its comment declares (not those it
    /// inherits from its account).
    pub tags: Vec<Tag>,
    /// What the amount cost in another commodity (`@` or `@@`), if
    /// anything. Balancing counts the cost instead of the amount.
    pub cost: Option<Cost>,
    /// The asserted balance of this account at the end of the
    /// transaction's date, if any.
    pub balance_assertion: Option<BalanceAssertion>,
}

/// A posting's cost: `@` gives the price of one unit, `@@` of the whole
/// amount.
#[derive(Debug, Clone, PartialEq)]
pub struct Cost {
    pub kind: CostKind,
    pub amount: Amount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostKind {
    Unit,
    Total,
}

impl Cost {
    /// What the posting counts as when balancing: the total cost, signed
    /// like the posting's amount.
    pub fn total_for(&self, amount: &Amount) -> Amount {
        let total = match self.kind {
            CostKind::Total => self.amount.money.abs_value(),
            CostKind::Unit => self.amount.money.times(&amount.money).abs_value(),
        };
        Amount::new(
            self.amount.commodity.clone(),
            if amount.money.is_negative() {
                -total
            } else {
                total
            },
        )
    }
}

/// A posting's balance assertion: `= 100.00 USD`, or one of hledger's
/// stricter forms (`==` total, `=*` inclusive of subaccounts, `==*`).
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceAssertion {
    pub amount: Amount,
    /// `==`: no other commodity may have a balance in the account.
    pub total: bool,
    /// `=*`: the balance includes subaccounts.
    pub inclusive: bool,
}

impl BalanceAssertion {
    /// A plain `= <amount>` assertion.
    pub fn new(amount: Amount) -> Self {
        BalanceAssertion {
            amount,
            total: false,
            inclusive: false,
        }
    }

    /// The operator as hledger writes it: `=`, `==`, `=*` or `==*`.
    pub fn operator(&self) -> &'static str {
        match (self.total, self.inclusive) {
            (false, false) => "=",
            (true, false) => "==",
            (false, true) => "=*",
            (true, true) => "==*",
        }
    }
}

impl Posting {
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    /// The transaction's own number in its ledger, counting up as
    /// transactions are added; 0 until it is stored.
    pub id: u64,
    pub date: NaiveDate,
    pub status: Status,
    pub code: String,
    /// hledger's raw `Payee | memo` description, unsplit.
    pub description: String,
    pub comment: String,
    /// The tags written in `comment`. They mean nothing to the books.
    pub tags: Vec<Tag>,
    pub postings: Vec<Posting>,
}

impl Transaction {
    /// hledger's `Payee | memo` convention: split on the first
    /// ` | `, trimming both sides. With no separator the whole description
    /// is the payee and the memo is empty.
    pub fn payee(&self) -> &str {
        self.split_description().0
    }

    pub fn memo(&self) -> &str {
        self.split_description().1
    }

    fn split_description(&self) -> (&str, &str) {
        match self.description.split_once('|') {
            Some((payee, memo)) => (payee.trim(), memo.trim()),
            None => (self.description.trim(), ""),
        }
    }

    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits_payee_and_memo_on_pipe() {
        let tx = sample_tx("Grocery Store | weekly shop");
        assert_eq!(tx.payee(), "Grocery Store");
        assert_eq!(tx.memo(), "weekly shop");
    }
    #[test]
    fn description_without_pipe_is_all_payee() {
        let tx = sample_tx("Grocery Store");
        assert_eq!(tx.payee(), "Grocery Store");
        assert_eq!(tx.memo(), "");
    }
    #[test]
    fn status_cycles_unmarked_pending_cleared_unmarked() {
        assert_eq!(Status::Unmarked.cycle(), Status::Pending);
        assert_eq!(Status::Pending.cycle(), Status::Cleared);
        assert_eq!(Status::Cleared.cycle(), Status::Unmarked);
    }

    #[test]
    fn account_name_helpers() {
        let a = AccountName::new("assets:receivable:jordan");
        assert_eq!(a.leaf(), "jordan");
        assert_eq!(a.receivable_person(), Some("jordan"));
        assert_eq!(a.payable_person(), None);
        assert!(a.is_under("assets"));
        assert!(!a.is_under("assets:checking"));
    }

    fn sample_tx(description: &str) -> Transaction {
        Transaction {
            id: 1,
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: description.to_string(),
            comment: String::new(),
            tags: vec![],
            postings: vec![],
        }
    }
}
