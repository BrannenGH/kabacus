// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The stored rows of a ledger as plain values, the changes a write makes
//! to them, and how a change applies to a book either way. The audit log
//! records exactly these changes, so history, past revisions and replay
//! are all read from it.

use chrono::NaiveDate;
use kabacus_core::{
    AccountName, Amount, BalanceAssertion, Cost, CostKind, Money, Posting, PostingKind, Status,
    Transaction,
};
use kabacus_ledger::{Account, Book, Commodity, Side, Style};
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
#[error("a stored row is malformed: {0}")]
pub struct RowError(pub String);

fn bad(what: impl Into<String>) -> RowError {
    RowError(what.into())
}

/// A transaction and its postings as stored, without the revision columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxRow {
    pub id: u64,
    pub date: String,
    pub status: String,
    pub code: String,
    pub description: String,
    pub comment: String,
    pub postings: Vec<PostingRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostingRow {
    pub account: String,
    pub kind: String,
    pub status: String,
    pub mantissa: String,
    pub places: u32,
    pub commodity: String,
    pub cost_kind: Option<String>,
    pub cost_mantissa: Option<String>,
    pub cost_places: Option<u32>,
    pub cost_commodity: Option<String>,
    pub assert_op: Option<String>,
    pub assert_mantissa: Option<String>,
    pub assert_places: Option<u32>,
    pub assert_commodity: Option<String>,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRow {
    pub name: String,
    pub declared_tags: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommodityRow {
    pub symbol: String,
    pub style_side: String,
    pub style_space: bool,
    pub decimal_mark: String,
    pub group_mark: String,
    pub group_sizes: String,
    pub places: Option<u32>,
}

/// One stored row of any table a write changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "table", rename_all = "snake_case")]
pub enum Row {
    Transaction(TxRow),
    Account(AccountRow),
    Commodity(CommodityRow),
}

impl Row {
    /// Which row this is: its table and key.
    pub fn key(&self) -> RowKey {
        match self {
            Row::Transaction(t) => RowKey::Transaction(t.id),
            Row::Account(a) => RowKey::Account(a.name.clone()),
            Row::Commodity(c) => RowKey::Commodity(c.symbol.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RowKey {
    Transaction(u64),
    Account(String),
    Commodity(String),
}

/// A row added (`before` is none), changed, or removed (`after` is none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowChange {
    pub before: Option<Row>,
    pub after: Option<Row>,
}

impl RowChange {
    pub fn key(&self) -> RowKey {
        self.after
            .as_ref()
            .or(self.before.as_ref())
            .expect("a change has a row")
            .key()
    }

    pub fn reversed(&self) -> RowChange {
        RowChange {
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// Every row one write changes, in the order to apply them: declarations
/// added or changed first, then transactions, then declarations removed,
/// so no row ever points at one that isn't there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub changes: Vec<RowChange>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The changes that turn `before` into `after`.
    pub fn between(before: &Book, after: &Book) -> ChangeSet {
        let mut adds = Vec::new();
        let mut txs = Vec::new();
        let mut removals = Vec::new();
        let declarations = |book: &Book| -> BTreeMap<RowKey, Row> {
            book.accounts
                .values()
                .map(|a| Row::Account(account_row(a)))
                .chain(
                    book.commodities
                        .values()
                        .map(|c| Row::Commodity(commodity_row(c))),
                )
                .map(|r| (r.key(), r))
                .collect()
        };
        let (old, new) = (declarations(before), declarations(after));
        for (key, row) in &new {
            match old.get(key) {
                None => adds.push(RowChange {
                    before: None,
                    after: Some(row.clone()),
                }),
                Some(was) if was != row => adds.push(RowChange {
                    before: Some(was.clone()),
                    after: Some(row.clone()),
                }),
                _ => {}
            }
        }
        for (key, row) in &old {
            if !new.contains_key(key) {
                removals.push(RowChange {
                    before: Some(row.clone()),
                    after: None,
                });
            }
        }
        let old_txs: BTreeMap<u64, &Transaction> =
            before.transactions.iter().map(|t| (t.id, t)).collect();
        let new_txs: BTreeMap<u64, &Transaction> =
            after.transactions.iter().map(|t| (t.id, t)).collect();
        for (id, tx) in &new_txs {
            let row = tx_row(tx);
            match old_txs.get(id) {
                None => txs.push(RowChange {
                    before: None,
                    after: Some(Row::Transaction(row)),
                }),
                Some(was) => {
                    let was = tx_row(was);
                    if was != row {
                        txs.push(RowChange {
                            before: Some(Row::Transaction(was)),
                            after: Some(Row::Transaction(row)),
                        });
                    }
                }
            }
        }
        for (id, tx) in &old_txs {
            if !new_txs.contains_key(id) {
                txs.push(RowChange {
                    before: Some(Row::Transaction(tx_row(tx))),
                    after: None,
                });
            }
        }
        txs.sort_by_key(RowChange::key);
        ChangeSet {
            changes: adds.into_iter().chain(txs).chain(removals).collect(),
        }
    }

    /// The changes that add every row of `book` to an empty ledger.
    pub fn adding(book: &Book) -> ChangeSet {
        ChangeSet::between(&Book::default(), book)
    }
}

/// Apply `change` to `book`.
pub fn apply(book: &mut Book, change: &RowChange) -> Result<(), RowError> {
    if let Some(before) = &change.before {
        match before {
            Row::Transaction(t) => book.transactions.retain(|x| x.id != t.id),
            Row::Account(a) => {
                book.accounts.remove(&a.name);
            }
            Row::Commodity(c) => {
                book.commodities.remove(&c.symbol);
            }
        }
    }
    if let Some(after) = &change.after {
        match after {
            Row::Transaction(t) => {
                let tx = transaction(t)?;
                let at = book
                    .transactions
                    .iter()
                    .position(|x| x.id > tx.id)
                    .unwrap_or(book.transactions.len());
                book.transactions.insert(at, tx);
            }
            Row::Account(a) => {
                book.accounts.insert(a.name.clone(), account(a));
            }
            Row::Commodity(c) => {
                book.commodities.insert(c.symbol.clone(), commodity(c)?);
            }
        }
    }
    Ok(())
}

fn status_text(s: Status) -> &'static str {
    match s {
        Status::Unmarked => "unmarked",
        Status::Pending => "pending",
        Status::Cleared => "cleared",
    }
}

fn status_of(s: &str) -> Result<Status, RowError> {
    match s {
        "unmarked" => Ok(Status::Unmarked),
        "pending" => Ok(Status::Pending),
        "cleared" => Ok(Status::Cleared),
        other => Err(bad(format!("status {other:?}"))),
    }
}

fn money(mantissa: &str, places: u32) -> Result<Money, RowError> {
    let m: BigInt = mantissa
        .parse()
        .map_err(|_| bad(format!("mantissa {mantissa:?}")))?;
    if places > kabacus_ledger::MAX_PLACES {
        return Err(bad(format!("{places} places")));
    }
    Ok(Money::from_big(m, places))
}

pub fn tx_row(tx: &Transaction) -> TxRow {
    TxRow {
        id: tx.id,
        date: tx.date.format("%Y-%m-%d").to_string(),
        status: status_text(tx.status).to_string(),
        code: tx.code.clone(),
        description: tx.description.clone(),
        comment: tx.comment.clone(),
        postings: tx.postings.iter().map(posting_row).collect(),
    }
}

pub fn posting_row(p: &Posting) -> PostingRow {
    PostingRow {
        account: p.account.as_str().to_string(),
        kind: match p.kind {
            PostingKind::Real => "real",
            PostingKind::Virtual => "virtual",
            PostingKind::BalancedVirtual => "balanced_virtual",
        }
        .to_string(),
        status: status_text(p.status).to_string(),
        mantissa: p.amount.money.mantissa().to_string(),
        places: p.amount.money.places(),
        commodity: p.amount.commodity.clone(),
        cost_kind: p.cost.as_ref().map(|c| {
            match c.kind {
                CostKind::Unit => "unit",
                CostKind::Total => "total",
            }
            .to_string()
        }),
        cost_mantissa: p
            .cost
            .as_ref()
            .map(|c| c.amount.money.mantissa().to_string()),
        cost_places: p.cost.as_ref().map(|c| c.amount.money.places()),
        cost_commodity: p.cost.as_ref().map(|c| c.amount.commodity.clone()),
        assert_op: p
            .balance_assertion
            .as_ref()
            .map(|a| a.operator().to_string()),
        assert_mantissa: p
            .balance_assertion
            .as_ref()
            .map(|a| a.amount.money.mantissa().to_string()),
        assert_places: p
            .balance_assertion
            .as_ref()
            .map(|a| a.amount.money.places()),
        assert_commodity: p
            .balance_assertion
            .as_ref()
            .map(|a| a.amount.commodity.clone()),
        comment: p.comment.clone(),
    }
}

pub fn transaction(row: &TxRow) -> Result<Transaction, RowError> {
    let date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d")
        .map_err(|_| bad(format!("date {:?}", row.date)))?;
    let postings = row
        .postings
        .iter()
        .map(posting)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Transaction {
        id: row.id,
        date,
        status: status_of(&row.status)?,
        code: row.code.clone(),
        description: row.description.clone(),
        tags: kabacus_core::declared_tags(&row.comment),
        comment: row.comment.clone(),
        postings,
    })
}

pub fn posting(row: &PostingRow) -> Result<Posting, RowError> {
    let kind = match row.kind.as_str() {
        "real" => PostingKind::Real,
        "virtual" => PostingKind::Virtual,
        "balanced_virtual" => PostingKind::BalancedVirtual,
        other => return Err(bad(format!("posting kind {other:?}"))),
    };
    let cost = match (
        &row.cost_kind,
        &row.cost_mantissa,
        row.cost_places,
        &row.cost_commodity,
    ) {
        (Some(kind), Some(m), Some(p), Some(c)) => Some(Cost {
            kind: match kind.as_str() {
                "unit" => CostKind::Unit,
                "total" => CostKind::Total,
                other => return Err(bad(format!("cost kind {other:?}"))),
            },
            amount: Amount::new(c.clone(), money(m, p)?),
        }),
        (None, None, None, None) => None,
        _ => return Err(bad("a cost given in part")),
    };
    let balance_assertion = match (
        &row.assert_op,
        &row.assert_mantissa,
        row.assert_places,
        &row.assert_commodity,
    ) {
        (Some(op), Some(m), Some(p), Some(c)) => {
            if !matches!(op.as_str(), "=" | "==" | "=*" | "==*") {
                return Err(bad(format!("assertion {op:?}")));
            }
            Some(BalanceAssertion {
                amount: Amount::new(c.clone(), money(m, p)?),
                total: op.starts_with("=="),
                inclusive: op.ends_with('*'),
            })
        }
        (None, None, None, None) => None,
        _ => return Err(bad("an assertion given in part")),
    };
    Ok(Posting {
        account: AccountName::new(row.account.clone()),
        amount: Amount::new(row.commodity.clone(), money(&row.mantissa, row.places)?),
        status: status_of(&row.status)?,
        kind,
        tags: kabacus_core::declared_tags(&row.comment),
        comment: row.comment.clone(),
        cost,
        balance_assertion,
    })
}

pub fn account_row(a: &Account) -> AccountRow {
    AccountRow {
        name: a.name.as_str().to_string(),
        declared_tags: a.tags.clone(),
    }
}

pub fn account(row: &AccountRow) -> Account {
    Account {
        name: AccountName::new(row.name.clone()),
        tags: row.declared_tags.clone(),
    }
}

pub fn commodity_row(c: &Commodity) -> CommodityRow {
    let style = &c.style;
    CommodityRow {
        symbol: c.symbol.clone(),
        style_side: match style.side {
            Side::Left => "left",
            Side::Right => "right",
        }
        .to_string(),
        style_space: style.spaced,
        decimal_mark: style.decimal_mark.to_string(),
        group_mark: style.group_mark.map(String::from).unwrap_or_default(),
        group_sizes: style
            .group_sizes
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(","),
        places: style.places,
    }
}

pub fn commodity(row: &CommodityRow) -> Result<Commodity, RowError> {
    let side = match row.style_side.as_str() {
        "left" => Side::Left,
        "right" => Side::Right,
        other => return Err(bad(format!("side {other:?}"))),
    };
    let decimal_mark = match row.decimal_mark.as_str() {
        "." => '.',
        "," => ',',
        other => return Err(bad(format!("decimal mark {other:?}"))),
    };
    let mut marks = row.group_mark.chars();
    let group_mark = marks.next();
    if marks.next().is_some() {
        return Err(bad(format!("group mark {:?}", row.group_mark)));
    }
    let group_sizes = if row.group_sizes.is_empty() {
        Vec::new()
    } else {
        row.group_sizes
            .split(',')
            .map(|n| {
                n.parse::<u8>()
                    .map_err(|_| bad(format!("group sizes {:?}", row.group_sizes)))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(Commodity {
        symbol: row.symbol.clone(),
        style: Style {
            side,
            spaced: row.style_space,
            decimal_mark,
            group_mark,
            group_sizes,
            places: row.places,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kabacus_ledger::{import, MemorySource};
    use std::path::{Path, PathBuf};

    fn book(text: &str) -> Book {
        let src = MemorySource(
            [(PathBuf::from("/b/main.journal"), text.to_string())]
                .into_iter()
                .collect(),
        );
        import(&src, Path::new("/b/main.journal")).unwrap().book
    }

    const BOOK: &str = "commodity $1,000.00\ncommodity 1.000,00 EUR\naccount assets:checking  ; ynab-account:Checking\naccount expenses:food\naccount expenses:travel\n\n2026-01-02 * Trader Joe's  ; receipt\n    expenses:food  $45.17\n    assets:checking  = $954.83\n\n2026-01-03 Hotel\n    expenses:travel  100,00 EUR @@ $110.00\n    assets:checking\n";

    #[test]
    fn rows_round_trip_every_value() {
        let b = book(&BOOK.replace("  = $954.83", "  $-45.17 = $954.83"));
        for tx in &b.transactions {
            assert_eq!(&transaction(&tx_row(tx)).unwrap(), tx);
        }
        for c in b.commodities.values() {
            assert_eq!(&commodity(&commodity_row(c)).unwrap(), c);
        }
    }

    #[test]
    fn a_change_set_applied_turns_one_book_into_the_other_and_back() {
        let before = book(&BOOK.replace("  = $954.83", "  $-45.17 = $954.83"));
        let mut after = before.clone();
        after.transactions.remove(0);
        after.transactions[0].description = "Hotel Vancouver".into();
        after.accounts.remove("expenses:food");
        after.accounts.insert(
            "expenses:lodging".into(),
            Account {
                name: AccountName::new("expenses:lodging"),
                tags: String::new(),
            },
        );
        let changes = ChangeSet::between(&before, &after);
        let kinds: Vec<RowKey> = changes.changes.iter().map(RowChange::key).collect();
        assert_eq!(
            kinds,
            [
                RowKey::Account("expenses:lodging".into()),
                RowKey::Transaction(1),
                RowKey::Transaction(2),
                RowKey::Account("expenses:food".into()),
            ]
        );
        let mut replayed = before.clone();
        for c in &changes.changes {
            apply(&mut replayed, c).unwrap();
        }
        assert_eq!(replayed, after);
        for c in changes.changes.iter().rev() {
            apply(&mut replayed, &c.reversed()).unwrap();
        }
        assert_eq!(replayed, before);
    }

    #[test]
    fn malformed_rows_are_refused() {
        let mut row = tx_row(&book(&BOOK.replace("  = $954.83", "  $-45.17")).transactions[0]);
        row.postings[0].mantissa = "12.5".into();
        assert!(transaction(&row).is_err());
        row.postings[0].mantissa = "1".into();
        row.postings[0].cost_kind = Some("unit".into());
        assert!(transaction(&row).is_err());
    }
}
