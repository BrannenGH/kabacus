// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The strict check: every transaction balances, every balance assertion
//! holds at the end of its day, and every commodity and account used is
//! declared. Every failure is reported, by kind and then position.
//! Implements the specification's "Ledger spec: strict checks and error
//! reporting" and "Ledger spec: balance assertions and order within a
//! date" (its engine rules), and the engine implementation specification,
//! sections 1.3, 2.2 (`check`, `check_change`) and 3.1.

use crate::balance::{check_balanced, Group, Sums, Unbalanced};
use crate::book::{Book, Styles};
use crate::render::render_money;
use chrono::NaiveDate;
use kabacus_core::{Amount, Money, Transaction};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

/// The kinds of check, in the order failures are reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureKind {
    Readable,
    Balanced,
    Assertion,
    Commodity,
    Account,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureKind::Readable => "readable",
            FailureKind::Balanced => "balanced",
            FailureKind::Assertion => "assertion",
            FailureKind::Commodity => "commodity",
            FailureKind::Account => "account",
        }
    }
}

/// Where a failure is: a line of a journal on import, a transaction (and
/// posting) on a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Position {
    Line { file: String, line: usize },
    Transaction { id: u64, posting: Option<usize> },
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Position::Line { file, line } => write!(f, "{file}:{line}"),
            Position::Transaction { id, posting: None } => write!(f, "transaction #{id}"),
            Position::Transaction {
                id,
                posting: Some(p),
            } => write!(f, "transaction #{id}, posting {}", p + 1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckFailure {
    pub kind: FailureKind,
    pub position: Position,
    /// One plain sentence, without the position.
    pub sentence: String,
    /// The raw detail behind "Details".
    pub details: String,
}

impl CheckFailure {
    /// The sentence with its position.
    pub fn message(&self) -> String {
        match self.kind {
            FailureKind::Readable => format!("{}: {}", self.position, self.sentence),
            _ => format!("{} ({})", self.sentence, self.position),
        }
    }
}

impl fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// Where each transaction is, for the positions of its failures. The
/// default places every transaction by id.
pub trait Locate {
    fn transaction(&self, tx: &Transaction) -> Position;
    fn posting(&self, tx: &Transaction, posting: usize) -> Position;
}

pub struct ById;

impl Locate for ById {
    fn transaction(&self, tx: &Transaction) -> Position {
        Position::Transaction {
            id: tx.id,
            posting: None,
        }
    }

    fn posting(&self, tx: &Transaction, posting: usize) -> Position {
        Position::Transaction {
            id: tx.id,
            posting: Some(posting),
        }
    }
}

fn list(amounts: &[Amount], styles: &Styles) -> String {
    amounts
        .iter()
        .map(|a| render_money(a, styles))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn unbalanced_failure(
    tx: &Transaction,
    problem: &Unbalanced,
    at: Position,
    styles: &Styles,
) -> CheckFailure {
    let date = tx.date.format("%Y-%m-%d");
    let (sentence, details) = match problem {
        Unbalanced::TwoOmitted { group } => (
            match group {
                Group::Real => format!(
                    "Only one posting in the transaction on {date} can leave out its amount."
                ),
                Group::BalancedVirtual => format!(
                    "Only one bracketed posting in the transaction on {date} can leave out its amount."
                ),
            },
            String::new(),
        ),
        Unbalanced::LeftOver { group, left_over } => {
            let first = render_money(&left_over[0], styles);
            (
                match group {
                    Group::Real => format!(
                        "The transaction on {date} doesn't balance: {first} is left over."
                    ),
                    Group::BalancedVirtual => format!(
                        "The bracketed postings of the transaction on {date} don't balance: {first} is left over."
                    ),
                },
                format!("Left over: {}", list(left_over, styles)),
            )
        }
    };
    CheckFailure {
        kind: FailureKind::Balanced,
        position: at,
        sentence,
        details,
    }
}

/// A posting with an assertion must state its amount.
pub fn assertion_without_amount(account: &str, at: Position) -> CheckFailure {
    CheckFailure {
        kind: FailureKind::Assertion,
        position: at,
        sentence: format!("The posting on {account} has an assertion but no amount."),
        details: String::new(),
    }
}

/// Each account's balance, alone and with its subaccounts, as postings are
/// added.
#[derive(Default)]
struct Balances {
    exclusive: HashMap<String, Sums>,
    inclusive: HashMap<String, Sums>,
}

impl Balances {
    fn add(&mut self, account: &str, amount: &Amount) {
        self.exclusive
            .entry(account.to_string())
            .or_default()
            .add(amount);
        let mut end = account.len();
        loop {
            self.inclusive
                .entry(account[..end].to_string())
                .or_default()
                .add(amount);
            match account[..end].rfind(':') {
                Some(i) => end = i,
                None => break,
            }
        }
    }
}

/// `account` and every account above it.
fn ancestors(account: &str) -> impl Iterator<Item = &str> {
    let mut end = Some(account.len());
    std::iter::from_fn(move || {
        let e = end?;
        let out = &account[..e];
        end = out.rfind(':');
        Some(out)
    })
}

/// Which assertions to evaluate: all, or only those on or after a date on
/// some accounts (and those in some transactions).
struct Scope {
    from: Option<NaiveDate>,
    exclusive: HashSet<String>,
    inclusive: HashSet<String>,
    always: HashSet<u64>,
}

impl Scope {
    fn wants(&self, tx: &Transaction, account: &str, inclusive: bool) -> bool {
        let Some(from) = self.from else {
            return true;
        };
        if self.always.contains(&tx.id) {
            return true;
        }
        tx.date >= from
            && if inclusive {
                self.inclusive.contains(account)
            } else {
                self.exclusive.contains(account)
            }
    }
}

fn assertion_failures(
    txs: &[&Transaction],
    unbalanced: &HashSet<u64>,
    styles: &Styles,
    locate: &dyn Locate,
    scope: &Scope,
) -> Vec<CheckFailure> {
    let mut ordered: Vec<&Transaction> = txs.to_vec();
    ordered.sort_by_key(|t| (t.date, t.id));
    let mut balances = Balances::default();
    let mut failures: Vec<(NaiveDate, String, CheckFailure)> = Vec::new();
    let mut day_start = 0;
    while day_start < ordered.len() {
        let date = ordered[day_start].date;
        let day_end = ordered[day_start..]
            .iter()
            .position(|t| t.date != date)
            .map_or(ordered.len(), |n| day_start + n);
        let day = &ordered[day_start..day_end];
        for tx in day.iter().filter(|t| !unbalanced.contains(&t.id)) {
            for p in &tx.postings {
                balances.add(p.account.as_str(), &p.amount);
            }
        }
        for tx in day {
            for (i, p) in tx.postings.iter().enumerate() {
                let Some(assertion) = &p.balance_assertion else {
                    continue;
                };
                let account = p.account.as_str();
                if !scope.wants(tx, account, assertion.inclusive) {
                    continue;
                }
                let held = if assertion.inclusive {
                    balances.inclusive.get(account)
                } else {
                    balances.exclusive.get(account)
                }
                .map(Sums::amounts)
                .unwrap_or_default();
                let asserted = &assertion.amount;
                let in_commodity = held
                    .iter()
                    .find(|a| a.commodity == asserted.commodity)
                    .map(|a| a.money.clone())
                    .unwrap_or_else(Money::zero);
                let others: Vec<&Amount> = held
                    .iter()
                    .filter(|a| a.commodity != asserted.commodity && !a.money.is_zero())
                    .collect();
                let holds =
                    in_commodity == asserted.money && (!assertion.total || others.is_empty());
                if holds {
                    continue;
                }
                let shown = if assertion.total {
                    let mut all: Vec<Amount> = held
                        .iter()
                        .filter(|a| !a.money.is_zero())
                        .cloned()
                        .collect();
                    if all.is_empty() {
                        all.push(Amount::new(
                            asserted.commodity.clone(),
                            Money::from_decimal(0, asserted.money.places()),
                        ));
                    }
                    list(&all, styles)
                } else {
                    render_money(
                        &Amount::new(asserted.commodity.clone(), in_commodity.clone()),
                        styles,
                    )
                };
                let whose = if assertion.inclusive {
                    format!("{account} and its subaccounts together hold")
                } else {
                    format!("{account} holds")
                };
                let mut details = format!(
                    "{} {}; calculated {}",
                    assertion.operator(),
                    render_money(asserted, styles),
                    shown
                );
                if assertion.total && !others.is_empty() {
                    details.push_str(&format!(
                        "; other commodities: {}",
                        others
                            .iter()
                            .map(|a| a.commodity.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !unbalanced.is_empty() {
                    details.push_str("; transactions that don't balance are left out");
                }
                failures.push((
                    date,
                    account.to_string(),
                    CheckFailure {
                        kind: FailureKind::Assertion,
                        position: locate.posting(tx, i),
                        sentence: format!(
                            "At the end of {}, {whose} {shown}, not the {} you asserted.",
                            date.format("%Y-%m-%d"),
                            render_money(asserted, styles)
                        ),
                        details,
                    },
                ));
            }
        }
        day_start = day_end;
    }
    // By date, then account; ties keep their order.
    failures.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    failures.into_iter().map(|(_, _, f)| f).collect()
}

/// Undeclared commodities and accounts in `txs`, each reported once, at
/// its first use.
fn declaration_failures(
    book: &Book,
    txs: &[&Transaction],
    locate: &dyn Locate,
) -> Vec<CheckFailure> {
    let mut commodities: Vec<(String, Position, usize)> = Vec::new();
    let mut accounts: Vec<(String, Position, usize)> = Vec::new();
    let note = |seen: &mut Vec<(String, Position, usize)>, name: &str, at: Position| match seen
        .iter_mut()
        .find(|(n, _, _)| n == name)
    {
        Some(entry) => entry.2 += 1,
        None => seen.push((name.to_string(), at, 0)),
    };
    for tx in txs {
        for (i, p) in tx.postings.iter().enumerate() {
            let used = std::iter::once(&p.amount)
                .chain(p.cost.as_ref().map(|c| &c.amount))
                .chain(p.balance_assertion.as_ref().map(|a| &a.amount));
            for amount in used {
                if !book.commodities.contains_key(&amount.commodity) {
                    note(&mut commodities, &amount.commodity, locate.posting(tx, i));
                }
            }
            if !book.is_declared(p.account.as_str()) {
                note(&mut accounts, p.account.as_str(), locate.posting(tx, i));
            }
        }
    }
    let more = |n: usize| match n {
        0 => String::new(),
        1 => "Used once more.".to_string(),
        n => format!("Used {n} more times."),
    };
    let mut out: Vec<CheckFailure> = commodities
        .into_iter()
        .map(|(symbol, position, n)| CheckFailure {
            kind: FailureKind::Commodity,
            position,
            sentence: if symbol.is_empty() {
                "A number with no commodity hasn't been declared as one.".to_string()
            } else {
                format!("The commodity {symbol} hasn't been declared.")
            },
            details: more(n),
        })
        .collect();
    out.extend(
        accounts
            .into_iter()
            .map(|(name, position, n)| CheckFailure {
                kind: FailureKind::Account,
                position,
                sentence: format!("The account {name} hasn't been declared."),
                details: more(n),
            }),
    );
    out
}

fn run(
    book: &Book,
    checked: &[&Transaction],
    locate: &dyn Locate,
    scope: &Scope,
) -> Vec<CheckFailure> {
    let styles = book.styles();
    let mut out = Vec::new();
    let mut unbalanced = HashSet::new();
    for tx in checked {
        if let Err(problem) = check_balanced(&tx.postings) {
            unbalanced.insert(tx.id);
            out.push(unbalanced_failure(
                tx,
                &problem,
                locate.transaction(tx),
                &styles,
            ));
        }
    }
    // Balances count every transaction that balances, not only the
    // checked ones.
    if scope.from.is_some() {
        for tx in &book.transactions {
            if check_balanced(&tx.postings).is_err() {
                unbalanced.insert(tx.id);
            }
        }
    }
    let all: Vec<&Transaction> = book.transactions.iter().collect();
    out.extend(assertion_failures(
        &all,
        &unbalanced,
        &styles,
        locate,
        scope,
    ));
    out.extend(declaration_failures(book, checked, locate));
    out.sort_by_key(|f| f.kind);
    out
}

/// The strict check of a whole book. Empty means valid.
pub fn check(book: &Book) -> Vec<CheckFailure> {
    check_located(book, &ById)
}

/// [`check`] with failures placed by `locate`.
pub fn check_located(book: &Book, locate: &dyn Locate) -> Vec<CheckFailure> {
    let all: Vec<&Transaction> = book.transactions.iter().collect();
    run(
        book,
        &all,
        locate,
        &Scope {
            from: None,
            exclusive: HashSet::new(),
            inclusive: HashSet::new(),
            always: HashSet::new(),
        },
    )
}

/// What a write changed, for [`check_change`].
#[derive(Debug, Default, Clone)]
pub struct Change {
    /// The transactions as they were before, for those changed or deleted.
    pub before: Vec<Transaction>,
    /// The ids of the transactions added or changed, as they are in the
    /// book now.
    pub after: BTreeSet<u64>,
    /// Account and commodity declarations taken away.
    pub undeclared_accounts: BTreeSet<String>,
    pub undeclared_commodities: BTreeSet<String>,
}

/// The strict check of `book`, the book after a change, looking only at
/// what `change` can affect: the transactions it adds or changes, the
/// assertions on the accounts it touches from its earliest date on, and the
/// uses of any declaration it takes away. On a book that passed the check
/// before the change, this gives the same verdict as [`check`].
pub fn check_change(book: &Book, change: &Change) -> Vec<CheckFailure> {
    let mut checked: Vec<&Transaction> = book
        .transactions
        .iter()
        .filter(|t| change.after.contains(&t.id))
        .collect();
    if !change.undeclared_accounts.is_empty() || !change.undeclared_commodities.is_empty() {
        let uses = |t: &Transaction| {
            t.postings.iter().any(|p| {
                change.undeclared_accounts.contains(p.account.as_str())
                    || std::iter::once(&p.amount)
                        .chain(p.cost.as_ref().map(|c| &c.amount))
                        .chain(p.balance_assertion.as_ref().map(|a| &a.amount))
                        .any(|a| change.undeclared_commodities.contains(&a.commodity))
            })
        };
        for tx in &book.transactions {
            if !change.after.contains(&tx.id) && uses(tx) {
                checked.push(tx);
            }
        }
        checked.sort_by_key(|t| t.id);
    }
    let touched: Vec<&Transaction> = change
        .before
        .iter()
        .chain(
            checked
                .iter()
                .copied()
                .filter(|t| change.after.contains(&t.id)),
        )
        .collect();
    let mut scope = Scope {
        from: touched.iter().map(|t| t.date).min(),
        exclusive: HashSet::new(),
        inclusive: HashSet::new(),
        always: change.after.iter().copied().collect(),
    };
    if scope.from.is_none() {
        // Nothing dated changed: no assertion can change its verdict.
        scope.from = Some(NaiveDate::MAX);
    }
    for tx in &touched {
        for p in &tx.postings {
            scope.exclusive.insert(p.account.as_str().to_string());
            for a in ancestors(p.account.as_str()) {
                scope.inclusive.insert(a.to_string());
            }
        }
    }
    run(book, &checked, &ById, &scope)
}

/// A commodity whose places must grow, and the amount that needs the most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacesNeeded {
    pub places: u32,
    pub amount: Amount,
    pub account: String,
    pub transaction: u64,
}

/// The commodities whose places must grow to fit `txs`, by symbol. A
/// commodity declared without an example takes any number of places.
pub fn places_needed<'a>(
    book: &Book,
    txs: impl IntoIterator<Item = &'a Transaction>,
) -> BTreeMap<String, PlacesNeeded> {
    let mut out: BTreeMap<String, PlacesNeeded> = BTreeMap::new();
    for tx in txs {
        for p in &tx.postings {
            let used = std::iter::once(&p.amount)
                .chain(p.cost.as_ref().map(|c| &c.amount))
                .chain(p.balance_assertion.as_ref().map(|a| &a.amount));
            for amount in used {
                let Some(declared) = book
                    .commodities
                    .get(&amount.commodity)
                    .and_then(|c| c.style.places)
                else {
                    continue;
                };
                let places = amount.money.places();
                let current = out.get(&amount.commodity).map_or(declared, |e| e.places);
                if places > current {
                    out.insert(
                        amount.commodity.clone(),
                        PlacesNeeded {
                            places,
                            amount: amount.clone(),
                            account: p.account.as_str().to_string(),
                            transaction: tx.id,
                        },
                    );
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
