// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Funds, card reserves and Cash on Hand.
//!
//! A fund (`pre-allocation`) is money set aside in a sub-account of the
//! money account holding it, its home. A card reserve (`debt-reserve`) is
//! the same for a credit card: it always equals what the card owes, so
//! every transaction touching a card also moves its reserve.
//! What's left in the money accounts themselves is free money:
//!
//! ```text
//! Cash on Hand = Money on Hand - Future Allocations - Receivables
//!              = Σ money (exclusive balances) - Σ overdrawn funds
//! ```
//!
//! An overdrawn fund sets nothing aside: its overdraft was paid out of its
//! home's free money, so it comes off that free money instead.

use crate::format::format_money;
use crate::kinds::{
    classify_accounts, duplicate_display_names, matching_rule, with_ancestors, AccountKind,
    ClassifiedAccount, KindRule,
};
use crate::{AccountName, Amount, Money, Transaction};
use chrono::NaiveDate;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The account whose balance means something wasn't categorized.
pub const UNCATEGORIZED: &str = "expenses:uncategorized";

/// The default card-reserve naming: the reserve for
/// `liabilities:credit-card:<name>` is `assets:bank:checking:card-<name>`.
pub const DEFAULT_RESERVE_TEMPLATE: &str = "assets:bank:checking:card-{card}";

/// Every account with its kind, home and display name, and the
/// rule naming each card's reserve.
#[derive(Debug, Clone)]
pub struct Chart {
    pub rules: Vec<KindRule>,
    /// Sorted by account.
    pub accounts: Vec<ClassifiedAccount>,
    /// `{card}` is replaced by the card account's last segment.
    pub reserve_template: String,
}

impl Chart {
    /// Apply persisted overrides after the rule-based defaults.
    pub fn with_account_types(mut self, types: &HashMap<String, bool>) -> Self {
        for account in &mut self.accounts {
            if let Some(synthetic) = types.get(account.account.as_str()) {
                account.synthetic = *synthetic;
            }
        }
        self
    }

    /// A synthetic reserve shares its home's physical account. A standalone
    /// synthetic account has no physical account of its own.
    pub fn physical_home(&self, account: &str) -> Option<AccountName> {
        let classified = self.get(account)?;
        if !classified.synthetic {
            return Some(classified.account.clone());
        }
        if let Some(home) = &classified.home {
            return self.physical_home(home.as_str());
        }
        let mut path = account;
        while let Some((parent, _)) = path.rsplit_once(':') {
            if let Some(c) = self.get(parent) {
                if c.kind == AccountKind::Money && !c.synthetic {
                    return Some(c.account.clone());
                }
            }
            path = parent;
        }
        None
    }

    /// A chart of `declared` accounts plus every account `transactions`
    /// post to, and all their parents.
    pub fn from_books(
        rules: Vec<KindRule>,
        reserve_template: String,
        declared: impl IntoIterator<Item = AccountName>,
        transactions: &[Transaction],
        friendly_names: &HashMap<String, String>,
    ) -> Chart {
        let posted: HashSet<AccountName> = transactions
            .iter()
            .flat_map(|tx| tx.postings.iter().map(|p| p.account.clone()))
            .collect();
        let mut named: BTreeSet<AccountName> = declared.into_iter().collect();
        named.extend(posted.iter().cloned());
        let accounts = classify_accounts(&rules, &with_ancestors(&named), &posted, friendly_names);
        Chart {
            rules,
            accounts,
            reserve_template,
        }
    }

    /// A chart that knows no accounts, only the rules: enough to tell
    /// cards and their reserves apart in a single transaction.
    pub fn from_rules(rules: Vec<KindRule>, reserve_template: String) -> Chart {
        Chart {
            rules,
            accounts: Vec::new(),
            reserve_template,
        }
    }

    pub fn get(&self, account: &str) -> Option<&ClassifiedAccount> {
        self.accounts
            .binary_search_by(|c| c.account.as_str().cmp(account))
            .ok()
            .map(|i| &self.accounts[i])
    }

    /// The account's kind; for one the chart doesn't know, its rule's.
    pub fn kind_of(&self, account: &AccountName) -> AccountKind {
        match self.get(account.as_str()) {
            Some(c) => c.kind,
            None => matching_rule(&self.rules, account.as_str())
                .map_or(AccountKind::Unclassified, |i| self.rules[i].kind),
        }
    }

    pub fn display_name(&self, account: &AccountName) -> String {
        self.get(account.as_str())
            .map_or_else(|| account.to_string(), |c| c.display_name.clone())
    }

    /// The reserve that must mirror `card`'s balance.
    pub fn reserve_for(&self, card: &AccountName) -> AccountName {
        AccountName::new(self.reserve_template.replace("{card}", card.leaf()))
    }

    /// The card whose reserve is `reserve`, if the chart has one.
    pub fn card_for_reserve(&self, reserve: &AccountName) -> Option<&AccountName> {
        self.of_kind(AccountKind::RevolvingDebt)
            .map(|c| &c.account)
            .find(|card| &self.reserve_for(card) == reserve)
    }

    fn of_kind(&self, kind: AccountKind) -> impl Iterator<Item = &ClassifiedAccount> {
        self.accounts.iter().filter(move |c| c.kind == kind)
    }

    /// For each card `tx` posts an amount A to, the amount still
    /// missing from its reserve if the reserve doesn't get -A in the same
    /// transaction. Empty when `tx` is reserved (or touches no card).
    pub fn missing_reserves(&self, tx: &Transaction) -> Vec<MissingReserve> {
        let mut cards: BTreeMap<(AccountName, String), Money> = BTreeMap::new();
        for p in &tx.postings {
            if self.kind_of(&p.account) == AccountKind::RevolvingDebt {
                let total = cards
                    .entry((p.account.clone(), p.amount.commodity.clone()))
                    .or_insert_with(Money::zero);
                *total = total.clone() + p.amount.money.clone();
            }
        }
        cards
            .into_iter()
            .filter_map(|((card, commodity), on_card)| {
                let reserve = self.reserve_for(&card);
                let reserved: Money = tx
                    .postings
                    .iter()
                    .filter(|p| p.account == reserve && p.amount.commodity == commodity)
                    .map(|p| p.amount.money.clone())
                    .sum();
                let missing = -on_card - reserved;
                (!missing.is_zero()).then(|| MissingReserve {
                    card,
                    reserve,
                    amount: Amount::new(commodity, missing),
                })
            })
            .collect()
    }
}

/// What a card transaction failed to post to the card's reserve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingReserve {
    pub card: AccountName,
    pub reserve: AccountName,
    /// What the reserve still needs in this transaction (`-A - posted`).
    pub amount: Amount,
}

/// Balances in one commodity, as of a date, from each account's own
/// postings (exclusive) or with its sub-accounts' (inclusive).
#[derive(Debug, Clone)]
pub struct Balances {
    exclusive: HashMap<AccountName, Money>,
    commodity: String,
    places: u32,
}

impl Balances {
    pub fn compute(
        transactions: &[Transaction],
        commodity: &str,
        as_of: Option<NaiveDate>,
    ) -> Self {
        let mut exclusive: HashMap<AccountName, Money> = HashMap::new();
        let mut places = 0;
        for tx in transactions {
            for p in tx
                .postings
                .iter()
                .filter(|p| p.amount.commodity == commodity)
            {
                places = places.max(p.amount.money.places());
                if as_of.is_some_and(|as_of| tx.date > as_of) {
                    continue;
                }
                let total = exclusive
                    .entry(p.account.clone())
                    .or_insert_with(Money::zero);
                *total = total.clone() + p.amount.money.clone();
            }
        }
        Balances {
            exclusive,
            commodity: commodity.to_string(),
            places,
        }
    }

    /// At the books' precision, so an empty account reads `0.00`.
    pub fn amount(&self, money: Money) -> Amount {
        Amount::new(
            &self.commodity,
            money.rescaled(self.places.max(money.places())),
        )
    }

    pub fn exclusive(&self, account: &AccountName) -> Amount {
        self.amount(
            self.exclusive
                .get(account)
                .cloned()
                .unwrap_or_else(Money::zero),
        )
    }

    pub fn inclusive(&self, account: &AccountName) -> Amount {
        self.amount(
            self.exclusive
                .iter()
                .filter(|(a, _)| a.is_under(account.as_str()))
                .map(|(_, m)| m.clone())
                .sum(),
        )
    }

    pub fn commodity(&self) -> &str {
        &self.commodity
    }
}

/// Something to tell the user about the funds. Never blocks a
/// write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    /// `negative-fund`, `negative-free-money`, `negative-reserve`,
    /// `reserve-mismatch`, `unreserved-unaccounted`,
    /// `unclassified`, `uncategorized` or `duplicate-display-name`.
    pub code: &'static str,
    pub message: String,
    pub accounts: Vec<String>,
}

/// A card transaction that doesn't move the card's reserve to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreservedTransaction {
    pub id: u64,
    pub date: NaiveDate,
    pub description: String,
    /// What the reserve still needs from this transaction.
    pub missing: Amount,
}

/// Which transactions are flagged unreserved, by their position in
/// the slice given to [`unreserved_flags`], with what each still owes its
/// reserves. Every transaction is judged on its own.
pub fn unreserved_flags(
    chart: &Chart,
    transactions: &[Transaction],
) -> BTreeMap<usize, Vec<MissingReserve>> {
    transactions
        .iter()
        .enumerate()
        .map(|(i, tx)| (i, chart.missing_reserves(tx)))
        .filter(|(_, missing)| !missing.is_empty())
        .collect()
}

/// One card and its reserve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardLine {
    pub card: AccountName,
    pub display_name: String,
    /// As booked: negative while the card is owed money.
    pub balance: Amount,
    /// What is owed on the card: `-balance`.
    pub owed: Amount,
    pub reserve: AccountName,
    /// Whether the reserve account exists in the books.
    pub reserve_exists: bool,
    pub reserve_balance: Amount,
    /// `owed - reserve_balance`: what the reserve still needs; zero when
    /// it covers the card.
    pub difference: Amount,
    /// Every flagged transaction, oldest
    /// first.
    pub unreserved: Vec<UnreservedTransaction>,
}

impl CardLine {
    /// The sum of every flagged transaction's missing amount.
    pub fn missing_total(&self) -> Money {
        self.unreserved
            .iter()
            .map(|u| u.missing.money.clone())
            .sum()
    }
}

fn card_lines(
    chart: &Chart,
    transactions: &[Transaction],
    balances: &Balances,
    as_of: Option<NaiveDate>,
) -> Vec<CardLine> {
    let dated: Vec<Transaction> = transactions
        .iter()
        .filter(|tx| as_of.is_none_or(|as_of| tx.date <= as_of))
        .cloned()
        .collect();
    let mut unreserved: HashMap<AccountName, Vec<(u64, UnreservedTransaction)>> = HashMap::new();
    for (i, missing) in unreserved_flags(chart, &dated) {
        let tx = &dated[i];
        for missing in missing {
            if missing.amount.commodity != balances.commodity() {
                continue;
            }
            unreserved.entry(missing.card).or_default().push((
                tx.id,
                UnreservedTransaction {
                    id: tx.id,
                    date: tx.date,
                    description: tx.description.clone(),
                    missing: missing.amount,
                },
            ));
        }
    }
    chart
        .of_kind(AccountKind::RevolvingDebt)
        .map(|card| {
            let balance = balances.exclusive(&card.account);
            let owed = balances.amount(-balance.money.clone());
            let reserve = chart.reserve_for(&card.account);
            let reserve_balance = balances.exclusive(&reserve);
            let mut unreserved = unreserved.remove(&card.account).unwrap_or_default();
            unreserved.sort_by_key(|(index, u)| (u.date, *index));
            let unreserved = unreserved.into_iter().map(|(_, u)| u).collect();
            CardLine {
                card: card.account.clone(),
                display_name: card.display_name.clone(),
                difference: balances.amount(owed.money.clone() - reserve_balance.money.clone()),
                balance,
                owed,
                reserve_exists: chart.get(reserve.as_str()).is_some(),
                reserve,
                reserve_balance,
                unreserved,
            }
        })
        .collect()
}

/// How far `fund` (a fund or card reserve) is overdrawn: zero unless its
/// balance is negative.
fn overdraft(balances: &Balances, fund: &AccountName) -> Money {
    let balance = balances.exclusive(fund).money;
    if balance.is_negative() {
        -balance
    } else {
        Money::zero()
    }
}

/// A money account's free money: its own balance less what its overdrawn
/// funds and card reserves have taken from it. An overdrawn fund's
/// spending, or a card payment past what its reserve held, was really paid
/// out of free money, so that money can't be spent again, even though the
/// fund or reserve carries the minus. Counting it as a negative allocation
/// instead would leave Cash on Hand too high by the overdraft.
fn free_money(chart: &Chart, balances: &Balances, home: &AccountName) -> Amount {
    let overdrawn = overdrawn_in(chart, balances, home, AccountKind::PreAllocation)
        + overdrawn_in(chart, balances, home, AccountKind::DebtReserve);
    balances.amount(balances.exclusive(home).money - overdrawn)
}

/// How far the accounts of `kind` in `home` are overdrawn, together.
fn overdrawn_in(
    chart: &Chart,
    balances: &Balances,
    home: &AccountName,
    kind: AccountKind,
) -> Money {
    chart
        .of_kind(kind)
        .filter(|c| c.home.as_ref() == Some(home))
        .map(|c| overdraft(balances, &c.account))
        .sum()
}

fn warnings(chart: &Chart, balances: &Balances, cards: &[CardLine]) -> Vec<Warning> {
    let mut out = Vec::new();
    for fund in chart.of_kind(AccountKind::PreAllocation) {
        let balance = balances.exclusive(&fund.account);
        if balance.money.is_negative() {
            out.push(Warning {
                code: "negative-fund",
                message: format!(
                    "The fund {} is overdrawn: {}",
                    fund.display_name,
                    format_money(&balance)
                ),
                accounts: vec![fund.account.to_string()],
            });
        }
    }
    let homes: BTreeSet<&AccountName> = chart
        .accounts
        .iter()
        .filter_map(|c| c.home.as_ref())
        .collect();
    for home in homes {
        let free = free_money(chart, balances, home);
        if free.money.is_negative() {
            out.push(Warning {
                code: "negative-free-money",
                message: format!(
                    "{}'s funds and card reserves add up to more than the money in it: its free money is {}",
                    chart.display_name(home),
                    format_money(&free)
                ),
                accounts: vec![home.to_string()],
            });
        }
    }
    for card in cards {
        let overdrawn = card.reserve_balance.money.is_negative();
        let in_credit = card.balance.money.is_positive();
        if !overdrawn && !in_credit {
            continue;
        }
        let mut parts = Vec::new();
        if in_credit {
            parts.push(format!("is in credit by {}", format_money(&card.balance)));
        }
        if overdrawn {
            parts.push(format!(
                "its reserve is overdrawn by {}",
                format_money(&balances.amount(-card.reserve_balance.money.clone()))
            ));
        }
        out.push(Warning {
            code: "negative-reserve",
            message: format!(
                "{} {}; a payment may be missing the purchases it paid for",
                card.display_name,
                parts.join(" and ")
            ),
            accounts: vec![card.card.to_string(), card.reserve.to_string()],
        });
    }
    for card in cards.iter().filter(|c| !c.difference.money.is_zero()) {
        let why = match (card.reserve_exists, card.unreserved.len()) {
            (false, _) => format!("its reserve {} doesn't exist", card.reserve),
            (true, 0) => "money was moved in or out of the reserve without the card".to_string(),
            (true, 1) => "1 transaction on the card doesn't move its reserve".to_string(),
            (true, n) => format!("{n} transactions on the card don't move its reserve"),
        };
        out.push(Warning {
            code: "reserve-mismatch",
            message: format!(
                "{} owes {} but its reserve holds {}: {why}",
                card.display_name,
                format_money(&card.owed),
                format_money(&card.reserve_balance),
            ),
            accounts: vec![card.card.to_string(), card.reserve.to_string()],
        });
    }
    for card in cards {
        let unaccounted = card.difference.money.clone() - card.missing_total();
        if !unaccounted.is_zero() {
            out.push(Warning {
                code: "unreserved-unaccounted",
                message: format!(
                    "{}'s flagged transactions explain {} of the {} its reserve is short; {} is unaccounted for",
                    card.display_name,
                    format_money(&balances.amount(card.missing_total())),
                    format_money(&card.difference),
                    format_money(&balances.amount(unaccounted)),
                ),
                accounts: vec![card.card.to_string(), card.reserve.to_string()],
            });
        }
    }
    for c in chart.of_kind(AccountKind::Unclassified) {
        out.push(Warning {
            code: "unclassified",
            message: format!("{} matches no account kind rule", c.account),
            accounts: vec![c.account.to_string()],
        });
    }
    let uncategorized = AccountName::new(UNCATEGORIZED);
    let balance = balances.inclusive(&uncategorized);
    if !balance.money.is_zero() {
        out.push(Warning {
            code: "uncategorized",
            message: format!(
                "{UNCATEGORIZED} holds {}; categorize those transactions",
                format_money(&balance)
            ),
            accounts: vec![UNCATEGORIZED.to_string()],
        });
    }
    for (name, accounts) in duplicate_display_names(&chart.accounts) {
        out.push(Warning {
            code: "duplicate-display-name",
            message: format!("{} accounts are all called {name:?}", accounts.len()),
            accounts: accounts.iter().map(|a| a.to_string()).collect(),
        });
    }
    out
}

/// The validation warnings, over every transaction.
pub fn validation_warnings(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
) -> Vec<Warning> {
    let balances = Balances::compute(transactions, commodity, None);
    let cards = card_lines(chart, transactions, &balances, None);
    warnings(chart, &balances, &cards)
}

/// An account and its balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBalance {
    pub account: AccountName,
    pub display_name: String,
    pub balance: Amount,
}

/// The accounts of one kind and their total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindGroup {
    pub kind: AccountKind,
    pub total: Amount,
    pub accounts: Vec<AccountBalance>,
}

/// A money account: its free money and what its funds and reserves hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeLine {
    pub account: AccountName,
    pub display_name: String,
    /// Money in no fund or reserve: the exclusive balance, less what
    /// overdrawn funds in it have taken.
    pub free: Amount,
    /// What its funds set aside; an overdrawn fund counts as zero.
    pub funds: Amount,
    /// What its card reserves set aside; a negative reserve counts as zero.
    pub reserves: Amount,
    /// The inclusive balance, which matches the bank statement.
    pub inclusive: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundLine {
    pub account: AccountName,
    pub display_name: String,
    pub home: AccountName,
    pub balance: Amount,
}

/// The Cash on Hand report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashOnHand {
    pub as_of: Option<NaiveDate>,
    pub money_on_hand: Amount,
    pub future_allocations: Amount,
    pub receivables: Amount,
    pub cash_on_hand: Amount,
    /// Money, funds, card reserves and receivables, account by account.
    pub by_kind: Vec<KindGroup>,
    pub homes: Vec<HomeLine>,
    pub funds: Vec<FundLine>,
    pub cards: Vec<CardLine>,
    pub warnings: Vec<Warning>,
}

/// Cash on Hand in `commodity` as of the end of `as_of`
/// (every transaction if `None`).
pub fn cash_on_hand(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    as_of: Option<NaiveDate>,
) -> CashOnHand {
    let balances = Balances::compute(transactions, commodity, as_of);
    let group = |kind: AccountKind| {
        let accounts: Vec<AccountBalance> = chart
            .of_kind(kind)
            .map(|c| AccountBalance {
                account: c.account.clone(),
                display_name: c.display_name.clone(),
                balance: balances.exclusive(&c.account),
            })
            .collect();
        KindGroup {
            kind,
            total: balances.amount(accounts.iter().map(|a| a.balance.money.clone()).sum()),
            accounts,
        }
    };
    let by_kind: Vec<KindGroup> = [
        AccountKind::Money,
        AccountKind::PreAllocation,
        AccountKind::DebtReserve,
        AccountKind::Receivable,
    ]
    .into_iter()
    .map(group)
    .collect();
    let total = |kinds: &[AccountKind]| -> Money {
        by_kind
            .iter()
            .filter(|g| kinds.contains(&g.kind))
            .map(|g| g.total.money.clone())
            .sum()
    };
    let money_on_hand = total(&[
        AccountKind::Money,
        AccountKind::PreAllocation,
        AccountKind::DebtReserve,
        AccountKind::Receivable,
    ]);
    let overdrafts: Money = [AccountKind::PreAllocation, AccountKind::DebtReserve]
        .into_iter()
        .flat_map(|kind| chart.of_kind(kind))
        .map(|c| overdraft(&balances, &c.account))
        .sum();
    // Overdrawn funds and reserves set nothing aside (see `free_money`).
    let future_allocations =
        total(&[AccountKind::PreAllocation, AccountKind::DebtReserve]) + overdrafts;
    let receivables = total(&[AccountKind::Receivable]);

    let homes = chart
        .of_kind(AccountKind::Money)
        .map(|home| {
            let held = |kind: AccountKind| -> Money {
                chart
                    .of_kind(kind)
                    .filter(|c| c.home.as_ref() == Some(&home.account))
                    .map(|c| balances.exclusive(&c.account).money)
                    .sum()
            };
            let overdrawn = |kind| overdrawn_in(chart, &balances, &home.account, kind);
            HomeLine {
                account: home.account.clone(),
                display_name: home.display_name.clone(),
                free: free_money(chart, &balances, &home.account),
                funds: balances.amount(
                    held(AccountKind::PreAllocation) + overdrawn(AccountKind::PreAllocation),
                ),
                reserves: balances
                    .amount(held(AccountKind::DebtReserve) + overdrawn(AccountKind::DebtReserve)),
                inclusive: balances.inclusive(&home.account),
            }
        })
        .collect();
    let funds = chart
        .of_kind(AccountKind::PreAllocation)
        .map(|fund| FundLine {
            account: fund.account.clone(),
            display_name: fund.display_name.clone(),
            home: fund.home.clone().unwrap_or_else(|| fund.account.clone()),
            balance: balances.exclusive(&fund.account),
        })
        .collect();
    let cards = card_lines(chart, transactions, &balances, as_of);
    let warnings = warnings(chart, &balances, &cards);
    CashOnHand {
        as_of,
        money_on_hand: balances.amount(money_on_hand.clone()),
        future_allocations: balances.amount(future_allocations.clone()),
        receivables: balances.amount(receivables.clone()),
        cash_on_hand: balances.amount(money_on_hand - future_allocations - receivables),
        by_kind,
        homes,
        funds,
        cards,
        warnings,
    }
}

/// Where money in a card reserve came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReserveSource {
    /// Discretionary spending, from free money.
    Free,
    /// Planned spending, from this fund.
    Fund(AccountName),
    /// Neither: no fund or money account on the other side (a payment
    /// larger than the reserve, or a reserve line from elsewhere).
    Unattributed,
}

/// How the split is worked out, stated in every response.
pub const RESERVE_SPLIT_RULE: &str = "The reserve is split by the counter-account of each posting to it since it last reached zero: a money account is free (discretionary) money, a fund is that fund (planned). Payments are taken from the oldest postings first (FIFO); a refund back to free money or a fund comes out of that source.";

/// A card, its reserve, and what the reserve holds by source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardReserve {
    pub card: CardLine,
    /// The date the reserve last reached zero, if it ever did.
    pub since: Option<NaiveDate>,
    /// Non-zero sources only; they add up to the reserve's balance.
    pub sources: Vec<(ReserveSource, Amount)>,
}

/// Every card (or only `card`) with its reserve split by source.
pub fn card_reserves(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    card: Option<&str>,
) -> Vec<CardReserve> {
    let balances = Balances::compute(transactions, commodity, None);
    let mut ordered: Vec<&Transaction> = transactions.iter().collect();
    ordered.sort_by_key(|tx| (tx.date, tx.id));
    card_lines(chart, transactions, &balances, None)
        .into_iter()
        .filter(|line| card.is_none_or(|c| line.card.as_str() == c))
        .map(|line| {
            let (since, lots) = reserve_lots(chart, &ordered, &line.reserve, commodity);
            let mut totals: BTreeMap<ReserveSource, Money> = BTreeMap::new();
            for (source, money) in lots {
                let total = totals.entry(source).or_insert_with(Money::zero);
                *total = total.clone() + money;
            }
            CardReserve {
                card: line,
                since,
                sources: totals
                    .into_iter()
                    .filter(|(_, m)| !m.is_zero())
                    .map(|(s, m)| (s, balances.amount(m)))
                    .collect(),
            }
        })
        .collect()
}

/// The reserve's postings since it last reached zero, as lots by source
/// (see [`RESERVE_SPLIT_RULE`]).
fn reserve_lots(
    chart: &Chart,
    ordered: &[&Transaction],
    reserve: &AccountName,
    commodity: &str,
) -> (Option<NaiveDate>, Vec<(ReserveSource, Money)>) {
    let touching: Vec<&Transaction> = ordered
        .iter()
        .cloned()
        .filter(|tx| {
            tx.postings
                .iter()
                .any(|p| &p.account == reserve && p.amount.commodity == commodity)
        })
        .collect();
    let mut running = Money::zero();
    let mut start = 0;
    let mut since = None;
    for (i, tx) in touching.iter().enumerate() {
        running = running
            + tx.postings
                .iter()
                .filter(|p| &p.account == reserve && p.amount.commodity == commodity)
                .map(|p| p.amount.money.clone())
                .sum::<Money>();
        if running.is_zero() {
            start = i + 1;
            since = Some(tx.date);
        }
    }

    let mut lots: Vec<(ReserveSource, Money)> = Vec::new();
    for tx in &touching[start..] {
        for posting in tx
            .postings
            .iter()
            .filter(|p| &p.account == reserve && p.amount.commodity == commodity)
        {
            let x = posting.amount.money.clone();
            // The free money or funds on the other side of this posting.
            let counters: Vec<(ReserveSource, Money)> = tx
                .postings
                .iter()
                .filter(|q| &q.account != reserve && q.amount.commodity == commodity)
                .filter(|q| {
                    q.amount.money.is_negative() == x.is_positive() && !q.amount.money.is_zero()
                })
                .filter_map(|q| {
                    let source = match chart.kind_of(&q.account) {
                        AccountKind::Money => ReserveSource::Free,
                        AccountKind::PreAllocation => ReserveSource::Fund(q.account.clone()),
                        _ => return None,
                    };
                    Some((source, abs(q.amount.money.clone())))
                })
                .collect();
            let mut left = abs(x.clone());
            if x.is_positive() {
                for (source, available) in counters {
                    if left.is_zero() {
                        break;
                    }
                    let take = left.clone().min(available);
                    lots.push((source, take.clone()));
                    left = left - take;
                }
                if !left.is_zero() {
                    lots.push((ReserveSource::Unattributed, left));
                }
            } else {
                for (source, available) in counters {
                    if left.is_zero() {
                        break;
                    }
                    let take = left.clone().min(available);
                    consume(&mut lots, Some(source), take.clone());
                    left = left - take;
                }
                if !left.is_zero() {
                    consume(&mut lots, None, left);
                }
            }
        }
    }
    (since, lots)
}

fn abs(m: Money) -> Money {
    if m.is_negative() {
        -m
    } else {
        m
    }
}

/// Take `amount` from the oldest positive lots of `source` (any source if
/// `None`); what they can't cover becomes a negative lot.
fn consume(lots: &mut Vec<(ReserveSource, Money)>, source: Option<ReserveSource>, amount: Money) {
    let mut left = amount;
    for (lot_source, money) in lots.iter_mut() {
        if left.is_zero() {
            return;
        }
        if !money.is_positive() || source.as_ref().is_some_and(|s| s != lot_source) {
            continue;
        }
        let take = left.clone().min(money.clone());
        *money = money.clone() - take.clone();
        left = left - take;
    }
    if !left.is_zero() {
        lots.push((source.unwrap_or(ReserveSource::Unattributed), -left));
    }
}

/// Why `allocate`, `record_card_purchase` or `pay_card` was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FundsError {
    #[error("the amount must be more than zero")]
    NotPositive,
    #[error("the amount must not be zero")]
    Zero,
    #[error("choose two different accounts")]
    SameAccount,
    #[error("{account} is {kind}; allocate only moves money between a money account and its own funds and card reserves")]
    NotAllocatable { account: String, kind: AccountKind },
    #[error("{from} and {to} have different allocation homes ({from_home} and {to_home}); allocate only works within one home. Check synthetic and physical_home before recording a transfer")]
    DifferentHomes {
        from: String,
        to: String,
        from_home: String,
        to_home: String,
    },
    #[error("{from} and {to} cross different physical accounts ({from_home} and {to_home}); allocation cannot move cash between them. Only record a transfer after the user explicitly confirms it already happened in the real world")]
    PhysicalTransfer {
        from: String,
        to: String,
        from_home: String,
        to_home: String,
    },
    #[error("{account} is {kind}, not a card (revolving-debt)")]
    NotACard { account: String, kind: AccountKind },
    #[error("{card}'s reserve {reserve} doesn't exist; declare it first")]
    NoReserve { card: String, reserve: String },
    #[error(
        "{reserve} sits in {home}, which isn't a money account, so there is no free money to take"
    )]
    NoFreeMoney { reserve: String, home: String },
    #[error("{account} is {kind}, not a fund; the source is \"free\" or a fund (pre-allocation)")]
    NotAFund { account: String, kind: AccountKind },
    #[error("{fund} already exists; set money aside in it without making it new")]
    FundExists { fund: String },
    #[error("a new fund goes directly in the money account the money comes from ({from}), not at {fund}")]
    NewFundElsewhere { fund: String, from: String },
    #[error("{0}")]
    FundName(String),
}

/// The account for a new fund called `name` (as typed: "Holiday Gifts")
/// in the money account `home`: `<home>:holiday-gifts`, the way funds are
/// named, lower case with dashes.
pub fn new_fund_account(home: &str, name: &str) -> Result<String, FundsError> {
    let segment = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    if segment.is_empty() {
        return Err(FundsError::FundName("Name the new fund".to_string()));
    }
    if segment.contains([':', ';']) {
        return Err(FundsError::FundName(format!(
            "{name:?} can't name a fund: no `:` or `;`"
        )));
    }
    Ok(format!("{home}:{segment}"))
}

/// Before setting money aside into a fund that doesn't exist yet, which
/// the same write declares. Only a new fund (pre-allocation, by its kind
/// rule) directly in the money account the money comes from: declaring it
/// can't change what anything already recorded means, so unlike other
/// account declarations it needs no rewrite.
pub fn check_new_fund(chart: &Chart, from: &str, fund: &str) -> Result<(), FundsError> {
    if chart.get(fund).is_some() {
        return Err(FundsError::FundExists {
            fund: fund.to_string(),
        });
    }
    let from_name = AccountName::new(from);
    let parent = fund.rsplit_once(':').map(|(parent, _)| parent);
    if chart.kind_of(&from_name) != AccountKind::Money || parent != Some(from) {
        return Err(FundsError::NewFundElsewhere {
            fund: fund.to_string(),
            from: from.to_string(),
        });
    }
    match chart.kind_of(&AccountName::new(fund)) {
        AccountKind::PreAllocation => Ok(()),
        kind => Err(FundsError::NotAFund {
            account: fund.to_string(),
            kind,
        }),
    }
}

/// Before a write declares a new expense or income account for what it
/// records. Like a new fund it changes nothing already recorded, so it
/// needs no rewrite; any other kind of account still does.
pub fn check_new_category(chart: &Chart, account: &str) -> Result<(), String> {
    if chart.get(account).is_some() {
        return Err(format!("{account} already exists"));
    }
    if account
        .split(':')
        .any(|s| s.trim().is_empty() || s.contains(';'))
    {
        return Err(format!("{account:?} can't name an account"));
    }
    match chart.kind_of(&AccountName::new(account)) {
        AccountKind::Expense | AccountKind::Income => Ok(()),
        kind => Err(format!(
            "{account} would be a {} account; only a new expense or income category can be made here",
            kind.as_str()
        )),
    }
}

/// A transaction to add: its description and `(account, amount)` lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTransaction {
    pub description: String,
    pub postings: Vec<(AccountName, Money)>,
}

/// Allocate `amount` from `from` to `to`, each either a money
/// account (its free money) or a fund or card reserve in it, both in the
/// same home.
pub fn plan_allocation(
    chart: &Chart,
    from: &str,
    to: &str,
    amount: Money,
    description: Option<&str>,
) -> Result<PlannedTransaction, FundsError> {
    if !amount.is_positive() {
        return Err(FundsError::NotPositive);
    }
    if from == to {
        return Err(FundsError::SameAccount);
    }
    let place = |account: &str| {
        let name = AccountName::new(account);
        match chart.kind_of(&name) {
            AccountKind::Money => Ok((name.clone(), true)),
            kind if kind.is_envelope() => match chart.get(account).and_then(|c| c.home.clone()) {
                Some(home) => Ok((home, false)),
                None => Err(FundsError::NotAllocatable {
                    account: account.to_string(),
                    kind,
                }),
            },
            kind => Err(FundsError::NotAllocatable {
                account: account.to_string(),
                kind,
            }),
        }
    };
    let (from_home, from_is_home) = place(from)?;
    let (to_home, to_is_home) = place(to)?;
    let from_physical = chart.physical_home(from);
    let to_physical = chart.physical_home(to);
    if from_physical != to_physical {
        return Err(FundsError::PhysicalTransfer {
            from: from.to_string(),
            to: to.to_string(),
            from_home: from_physical.map_or_else(|| "synthetic".to_string(), |a| a.to_string()),
            to_home: to_physical.map_or_else(|| "synthetic".to_string(), |a| a.to_string()),
        });
    }
    if from_home != to_home {
        return Err(FundsError::DifferentHomes {
            from: from.to_string(),
            to: to.to_string(),
            from_home: from_home.to_string(),
            to_home: to_home.to_string(),
        });
    }
    let from = AccountName::new(from);
    let to = AccountName::new(to);
    let description = match description.map(str::trim) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => match (from_is_home, to_is_home) {
            (true, _) => format!("Allocate | to {}", chart.display_name(&to)),
            (_, true) => format!("Release | from {}", chart.display_name(&from)),
            _ => format!(
                "Allocate | from {} to {}",
                chart.display_name(&from),
                chart.display_name(&to)
            ),
        },
    };
    Ok(PlannedTransaction {
        description,
        postings: vec![(to, amount.clone()), (from, -amount)],
    })
}

/// Where a card purchase's money is set aside from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurchaseSource {
    /// Free money in the reserve's home: a discretionary purchase.
    Free,
    /// A fund: a planned purchase.
    Fund(AccountName),
}

impl PurchaseSource {
    /// `"free"`, or a fund's account.
    pub fn parse(source: &str) -> PurchaseSource {
        if source.trim().eq_ignore_ascii_case("free") {
            PurchaseSource::Free
        } else {
            PurchaseSource::Fund(AccountName::new(source.trim()))
        }
    }
}

fn card_and_reserve(chart: &Chart, card: &str) -> Result<(AccountName, AccountName), FundsError> {
    let card = AccountName::new(card);
    let kind = chart.kind_of(&card);
    if kind != AccountKind::RevolvingDebt {
        return Err(FundsError::NotACard {
            account: card.to_string(),
            kind,
        });
    }
    let reserve = chart.reserve_for(&card);
    if chart.get(reserve.as_str()).is_none() {
        return Err(FundsError::NoReserve {
            card: card.to_string(),
            reserve: reserve.to_string(),
        });
    }
    Ok((card, reserve))
}

/// A purchase on `card`, as one transaction: the expense, the
/// charge to the card, and `amount` moved from `source` to the card's
/// reserve. A negative amount is a refund back to the same source.
pub fn plan_card_purchase(
    chart: &Chart,
    card: &str,
    expense_account: &str,
    amount: Money,
    source: &PurchaseSource,
    description: &str,
) -> Result<PlannedTransaction, FundsError> {
    if amount.is_zero() {
        return Err(FundsError::Zero);
    }
    let (card, reserve) = card_and_reserve(chart, card)?;
    let source = match source {
        PurchaseSource::Free => {
            let home = reserve
                .as_str()
                .rsplit_once(':')
                .map_or_else(|| reserve.clone(), |(h, _)| AccountName::new(h));
            if chart.kind_of(&home) != AccountKind::Money {
                return Err(FundsError::NoFreeMoney {
                    reserve: reserve.to_string(),
                    home: home.to_string(),
                });
            }
            home
        }
        PurchaseSource::Fund(fund) => {
            let kind = chart.kind_of(fund);
            if kind != AccountKind::PreAllocation {
                return Err(FundsError::NotAFund {
                    account: fund.to_string(),
                    kind,
                });
            }
            fund.clone()
        }
    };
    Ok(PlannedTransaction {
        description: description.to_string(),
        postings: vec![
            (AccountName::new(expense_account), amount.clone()),
            (card, -amount.clone()),
            (source, -amount.clone()),
            (reserve, amount),
        ],
    })
}

/// Pay `amount` off `card` from its reserve.
pub fn plan_card_payment(
    chart: &Chart,
    card: &str,
    amount: Money,
) -> Result<PlannedTransaction, FundsError> {
    if !amount.is_positive() {
        return Err(FundsError::NotPositive);
    }
    let (card, reserve) = card_and_reserve(chart, card)?;
    Ok(PlannedTransaction {
        description: format!("{} | Payment", chart.display_name(&card)),
        postings: vec![(card, amount.clone()), (reserve, -amount)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::default_kind_rules;
    use crate::{Posting, PostingKind, Status};
    use num_traits::ToPrimitive;

    fn usd(m: i128) -> Money {
        Money::from_decimal(m, 2)
    }

    #[test]
    fn synthetic_accounts_resolve_their_physical_boundary() {
        let savings = "assets:bank:savings";
        let emergency = "assets:bank:savings:emergency-fund";
        let chart = chart(
            &[],
            &[CHECKING, INSURANCE, VISA_RESERVE, savings, emergency],
        );
        assert!(!chart.get(CHECKING).unwrap().synthetic);
        assert!(chart.get(INSURANCE).unwrap().synthetic);
        assert!(chart.get(VISA_RESERVE).unwrap().synthetic);
        assert_eq!(
            chart.physical_home(INSURANCE),
            Some(AccountName::new(CHECKING))
        );
        assert_eq!(
            chart.physical_home(emergency),
            Some(AccountName::new(savings))
        );

        let chart = chart.with_account_types(&HashMap::from([
            (CHECKING.to_string(), true),
            (INSURANCE.to_string(), false),
        ]));
        assert_eq!(chart.physical_home(CHECKING), None);
        assert_eq!(chart.physical_home(VISA_RESERVE), None);
        assert_eq!(
            chart.physical_home(INSURANCE),
            Some(AccountName::new(INSURANCE))
        );
        assert_eq!(
            chart.kind_of(&AccountName::new(INSURANCE)),
            AccountKind::PreAllocation
        );
        assert!(matches!(
            plan_allocation(&chart, CHECKING, INSURANCE, usd(100), None),
            Err(FundsError::PhysicalTransfer { .. })
        ));
        assert!(plan_allocation(&chart, CHECKING, VISA_RESERVE, usd(100), None).is_ok());
        let chart = chart.with_account_types(&HashMap::from([(savings.to_string(), true)]));
        assert!(matches!(
            plan_allocation(&chart, VISA_RESERVE, emergency, usd(100), None),
            Err(FundsError::DifferentHomes { .. })
        ));
    }

    fn tx(date: &str, description: &str, postings: &[(&str, i128)]) -> Transaction {
        Transaction {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: description.into(),
            comment: String::new(),
            tags: vec![("id".into(), format!("{date}-{description}"))],
            postings: postings
                .iter()
                .map(|(account, mantissa)| Posting {
                    account: AccountName::new(*account),
                    amount: Amount::new("USD", usd(*mantissa)),
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

    fn chart(txs: &[Transaction], declared: &[&str]) -> Chart {
        Chart::from_books(
            default_kind_rules(),
            DEFAULT_RESERVE_TEMPLATE.to_string(),
            declared.iter().map(|a| AccountName::new(*a)),
            txs,
            &HashMap::new(),
        )
    }

    const CHECKING: &str = "assets:bank:checking";
    const VISA: &str = "liabilities:credit-card:rewards-visa";
    const VISA_RESERVE: &str = "assets:bank:checking:card-rewards-visa";
    const INSURANCE: &str = "assets:bank:checking:insurance";
    const PHONE: &str = "assets:bank:checking:cellphone";

    /// Synthetic books on 2026-10-01 with every kind of account:
    /// each card's reserve equals its balance after the backfill.
    fn example_books() -> Vec<Transaction> {
        let opening = |account: &str, cents: i128| {
            tx(
                "2026-10-01",
                &format!("Opening {account}"),
                &[(account, cents), ("equity:opening", -cents)],
            )
        };
        let mut txs = vec![
            opening(CHECKING, 315000),
            opening("assets:bank:savings", 0),
            opening("assets:bank:union-savings", 1200),
            opening("assets:cash", 0),
            opening("assets:bank:savings:emergency-fund", 500000),
            opening("assets:bank:checking:home-mortgage", 90000),
            opening(INSURANCE, 18000),
            opening("assets:bank:checking:car", 25000),
            opening("assets:bank:checking:cellphone", 6000),
            opening("assets:bank:checking:subscriptions", 4500),
            opening("assets:receivable:hsa", 120000),
        ];
        // Card history, backfilled with reserve lines.
        for (card, cents) in [
            ("rewards-visa", 42500),
            ("cashback-mc", 31075),
            ("store-card", 12000),
            ("travel-card", 55050),
            ("spare-card", 0),
            ("dept-store", -2500),
        ] {
            txs.push(tx(
                "2026-09-15",
                &format!("History {card}"),
                &[
                    ("expenses:household:shopping", cents),
                    (&format!("liabilities:credit-card:{card}"), -cents),
                    ("equity:opening", -cents),
                    (&format!("assets:bank:checking:card-{card}"), cents),
                ],
            ));
        }
        txs
    }
    #[test]
    fn cash_on_hand_matches_the_worked_example() {
        let txs = example_books();
        let chart = chart(&txs, &[]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        assert_eq!(report.money_on_hand.money, usd(1217825));
        // The store card in credit leaves its reserve at -25.00, which sets
        // nothing aside and comes off checking's free money.
        assert_eq!(report.future_allocations.money, usd(784125));
        assert_eq!(report.receivables.money, usd(120000));
        assert_eq!(report.cash_on_hand.money, usd(313700));
        assert_eq!(report.cash_on_hand.money.to_string(), "3137.00");

        let group = |kind| report.by_kind.iter().find(|g| g.kind == kind).unwrap();
        assert_eq!(group(AccountKind::Money).total.money, usd(316200));
        assert_eq!(group(AccountKind::PreAllocation).total.money, usd(643500));
        assert_eq!(group(AccountKind::DebtReserve).total.money, usd(138125));
        // Zero balances still show at the books' precision.
        let savings = group(AccountKind::Money)
            .accounts
            .iter()
            .find(|a| a.account.as_str() == "assets:bank:savings")
            .unwrap();
        assert_eq!(savings.balance.money.to_string(), "0.00");

        let checking = report
            .homes
            .iter()
            .find(|h| h.account.as_str() == CHECKING)
            .unwrap();
        assert_eq!(checking.free.money, usd(312500));
        assert_eq!(checking.funds.money, usd(143500));
        assert_eq!(checking.reserves.money, usd(140625));
        assert_eq!(checking.inclusive.money, usd(312500 + 143500 + 140625));

        let emergency = report
            .funds
            .iter()
            .find(|f| f.account.as_str() == "assets:bank:savings:emergency-fund")
            .unwrap();
        assert_eq!(emergency.home.as_str(), "assets:bank:savings");
        assert_eq!(emergency.balance.money, usd(500000));

        assert_eq!(report.cards.len(), 6);
        assert!(report.cards.iter().all(|c| c.difference.money.is_zero()));
        let dept_store = report
            .cards
            .iter()
            .find(|c| c.card.leaf() == "dept-store")
            .unwrap();
        assert_eq!(dept_store.owed.money, usd(-2500));
        assert_eq!(dept_store.reserve_balance.money, usd(-2500));
        let codes: Vec<&str> = report.warnings.iter().map(|w| w.code).collect();
        assert_eq!(codes, ["negative-reserve"], "{:?}", report.warnings);
        assert_eq!(
            report.warnings[0].accounts[0],
            "liabilities:credit-card:dept-store"
        );
    }
    #[test]
    fn as_of_leaves_out_later_transactions() {
        let txs = vec![
            tx(
                "2026-10-01",
                "Pay",
                &[(CHECKING, 100000), ("income:salary", -100000)],
            ),
            tx(
                "2026-10-05",
                "Pay",
                &[(CHECKING, 100000), ("income:salary", -100000)],
            ),
        ];
        let chart = chart(&txs, &[]);
        let before = cash_on_hand(&chart, &txs, "USD", NaiveDate::from_ymd_opt(2026, 10, 4));
        assert_eq!(before.cash_on_hand.money, usd(100000));
        let after = cash_on_hand(&chart, &txs, "USD", None);
        assert_eq!(after.cash_on_hand.money, usd(200000));
    }

    /// The effect of each common transaction pattern on Cash on Hand.
    #[test]
    fn each_pattern_moves_cash_on_hand_as_documented() {
        let mut txs = vec![tx(
            "2026-10-01",
            "Opening",
            &[(CHECKING, 100000), ("equity:opening", -100000)],
        )];
        let declared = [
            VISA,
            VISA_RESERVE,
            INSURANCE,
            PHONE,
            "assets:receivable:hsa",
        ];
        let coh = |txs: &[Transaction]| {
            cash_on_hand(&chart(txs, &declared), txs, "USD", None)
                .cash_on_hand
                .money
        };
        let expect = |txs: &mut Vec<Transaction>, t: Transaction, change: i128| {
            let before = coh(txs);
            txs.push(t);
            assert_eq!(
                coh(txs) - before,
                usd(change),
                "{}",
                txs.last().unwrap().description
            );
        };
        // Setting money aside leaves less free money, so Cash on Hand
        // goes down. Only moves between funds leave it unchanged.
        expect(
            &mut txs,
            tx(
                "2026-10-02",
                "Allocate",
                &[(INSURANCE, 10000), (CHECKING, -10000)],
            ),
            -10000,
        );
        expect(
            &mut txs,
            tx("2026-10-02", "Move", &[(PHONE, 2000), (INSURANCE, -2000)]),
            0,
        );
        expect(
            &mut txs,
            tx("2026-10-02", "Release", &[(CHECKING, 2000), (PHONE, -2000)]),
            2000,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-02",
                "Bill from the bank",
                &[("expenses:bills:insurance", 1000), (INSURANCE, -1000)],
            ),
            0,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-03",
                "Groceries",
                &[
                    ("expenses:household:groceries", 4000),
                    (VISA, -4000),
                    (CHECKING, -4000),
                    (VISA_RESERVE, 4000),
                ],
            ),
            -4000,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-04",
                "Insurance",
                &[
                    ("expenses:bills:insurance", 6000),
                    (VISA, -6000),
                    (INSURANCE, -6000),
                    (VISA_RESERVE, 6000),
                ],
            ),
            0,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-05",
                "Interest",
                &[
                    ("expenses:interest:visa", 200),
                    (VISA, -200),
                    (CHECKING, -200),
                    (VISA_RESERVE, 200),
                ],
            ),
            -200,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-06",
                "Refund",
                &[
                    ("expenses:household:groceries", -1000),
                    (VISA, 1000),
                    (CHECKING, 1000),
                    (VISA_RESERVE, -1000),
                ],
            ),
            1000,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-07",
                "Refund to fund",
                &[
                    ("expenses:bills:insurance", -500),
                    (VISA, 500),
                    (INSURANCE, 500),
                    (VISA_RESERVE, -500),
                ],
            ),
            0,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-08",
                "Pay card",
                &[(VISA, 8700), (VISA_RESERVE, -8700)],
            ),
            0,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-09",
                "Pharmacy",
                &[("assets:receivable:hsa", 3000), (CHECKING, -3000)],
            ),
            -3000,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-10",
                "HSA",
                &[(CHECKING, 3000), ("assets:receivable:hsa", -3000)],
            ),
            3000,
        );
        expect(
            &mut txs,
            tx(
                "2026-10-11",
                "Pay",
                &[(CHECKING, 50000), ("income:salary", -50000)],
            ),
            50000,
        );

        let report = cash_on_hand(&chart(&txs, &declared), &txs, "USD", None);
        let visa = &report.cards[0];
        assert!(visa.difference.money.is_zero());
        assert!(visa.unreserved.is_empty());
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }
    #[test]
    fn a_card_transaction_without_its_reserve_lines_is_unreserved() {
        let chart = Chart::from_rules(default_kind_rules(), DEFAULT_RESERVE_TEMPLATE.into());
        let reserved = tx(
            "2026-10-03",
            "Groceries",
            &[
                ("expenses:household:groceries", 4000),
                (VISA, -4000),
                (CHECKING, -4000),
                (VISA_RESERVE, 4000),
            ],
        );
        assert!(chart.missing_reserves(&reserved).is_empty());
        let purchase = tx(
            "2026-10-03",
            "Groceries",
            &[("expenses:household:groceries", 4000), (VISA, -4000)],
        );
        assert_eq!(
            chart.missing_reserves(&purchase),
            [MissingReserve {
                card: AccountName::new(VISA),
                reserve: AccountName::new(VISA_RESERVE),
                amount: Amount::new("USD", usd(4000)),
            }]
        );
        let payment = tx("2026-10-08", "Pay card", &[(VISA, 8700), (CHECKING, -8700)]);
        assert_eq!(chart.missing_reserves(&payment)[0].amount.money, usd(-8700));
        // A partial reserve line leaves the rest missing.
        let partial = tx(
            "2026-10-03",
            "Groceries",
            &[
                ("expenses:x", 4000),
                (VISA, -4000),
                (CHECKING, -3000),
                (VISA_RESERVE, 3000),
            ],
        );
        assert_eq!(chart.missing_reserves(&partial)[0].amount.money, usd(1000));
        // Money moved into a reserve without the card isn't a card transaction.
        let allocation = tx(
            "2026-10-02",
            "Allocate",
            &[(VISA_RESERVE, 1000), (CHECKING, -1000)],
        );
        assert!(chart.missing_reserves(&allocation).is_empty());
    }
    /// An overdrawn fund was paid for out of its home's free money: it
    /// sets nothing aside, and its overdraft isn't free to spend any more.
    /// Counting it as a negative allocation instead left Cash on Hand that
    /// much too high.
    #[test]
    fn an_overdrawn_fund_comes_out_of_its_homes_free_money() {
        let txs = vec![
            tx(
                "2026-10-01",
                "Opening",
                &[(CHECKING, 300000), ("equity:opening", -300000)],
            ),
            tx(
                "2026-10-02",
                "Allocate",
                &[(INSURANCE, 15000), (PHONE, 6000), (CHECKING, -21000)],
            ),
            tx(
                "2026-10-03",
                "State Farm",
                &[("expenses:insurance", 34500), (INSURANCE, -34500)],
            ),
        ];
        let chart = chart(&txs, &[]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        // The fund still shows what it really holds, with its warning.
        let insurance = report
            .funds
            .iter()
            .find(|f| f.account.as_str() == INSURANCE)
            .unwrap();
        assert_eq!(insurance.balance.money, usd(-19500));
        assert_eq!(report.warnings[0].code, "negative-fund");

        let checking = report
            .homes
            .iter()
            .find(|h| h.account.as_str() == CHECKING)
            .unwrap();
        assert_eq!(checking.inclusive.money, usd(265500));
        assert_eq!(checking.free.money, usd(279000 - 19500));
        assert_eq!(checking.funds.money, usd(6000));
        assert_eq!(
            checking.free.money.clone()
                + checking.funds.money.clone()
                + checking.reserves.money.clone(),
            checking.inclusive.money
        );
        assert_eq!(report.money_on_hand.money, usd(265500));
        assert_eq!(report.future_allocations.money, usd(6000));
        assert_eq!(report.cash_on_hand.money, usd(259500));

        // The totals behind the history and the cash flow bridge agree.
        let totals = crate::reports::cash_totals(&chart, &txs, "USD", None);
        assert_eq!(totals.money, usd(259500));
        assert_eq!(totals.funds, usd(6000));
        assert_eq!(totals.money_on_hand, report.money_on_hand.money);
        assert_eq!(totals.future_allocations, report.future_allocations.money);
        assert_eq!(totals.cash_on_hand, report.cash_on_hand.money);
    }

    /// Card payments made from a reserve whose purchases were never
    /// recorded leave the reserve negative and the card in credit. That
    /// money really left free money, so the reserve counts as zero and its
    /// shortfall comes off the home's free money, like an overdrawn fund.
    #[test]
    fn a_negative_card_reserve_comes_out_of_its_homes_free_money() {
        let mut txs = vec![
            tx(
                "2026-05-01",
                "Opening",
                &[(CHECKING, 40578), ("equity:opening", -40578)],
            ),
            tx(
                "2026-05-22",
                "Payment to Rewards Visa",
                &[(VISA, 1883), (VISA_RESERVE, -1883)],
            ),
            tx(
                "2026-09-09",
                "Payment to Rewards Visa",
                &[(VISA, 2549), (VISA_RESERVE, -2549)],
            ),
        ];
        let chart = chart(&txs, &[VISA_RESERVE]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        let checking = report
            .homes
            .iter()
            .find(|h| h.account.as_str() == CHECKING)
            .unwrap();
        assert_eq!(checking.inclusive.money, usd(36146));
        assert_eq!(checking.free.money, usd(36146));
        assert_eq!(checking.reserves.money, usd(0));
        assert_eq!(report.future_allocations.money, usd(0));
        assert_eq!(report.cash_on_hand.money, usd(36146));
        let warning = report
            .warnings
            .iter()
            .find(|w| w.code == "negative-reserve")
            .expect("negative-reserve warning");
        assert!(
            warning.message.contains("Rewards Visa"),
            "{}",
            warning.message
        );
        assert!(warning.message.contains("44.32"), "{}", warning.message);
        assert_eq!(warning.accounts, [VISA, VISA_RESERVE]);

        let totals = crate::reports::cash_totals(&chart, &txs, "USD", None);
        assert_eq!(totals.money, usd(36146));
        assert_eq!(totals.reserves, usd(0));
        assert_eq!(totals.future_allocations, report.future_allocations.money);
        assert_eq!(totals.cash_on_hand, report.cash_on_hand.money);

        // Recording the missing purchases doesn't change Cash on Hand.
        txs.push(tx(
            "2026-05-20",
            "Macy's",
            &[
                ("expenses:household:clothing", 4432),
                (VISA, -4432),
                (VISA_RESERVE, 4432),
                (CHECKING, -4432),
            ],
        ));
        let report = cash_on_hand(&chart, &txs, "USD", None);
        assert_eq!(report.cash_on_hand.money, usd(36146));
        assert!(report.warnings.iter().all(|w| w.code != "negative-reserve"));
    }

    /// A card in credit warns even while its reserve is at zero.
    #[test]
    fn a_card_in_credit_warns() {
        let txs = vec![
            tx(
                "2026-10-01",
                "Opening",
                &[(CHECKING, 120000), ("equity:opening", -120000)],
            ),
            tx(
                "2026-10-05",
                "Payment to Rewards Visa",
                &[(VISA, 2500), (CHECKING, -2500)],
            ),
        ];
        let chart = chart(&txs, &[VISA_RESERVE]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        let warning = report
            .warnings
            .iter()
            .find(|w| w.code == "negative-reserve")
            .expect("negative-reserve warning");
        assert!(warning.message.contains("in credit"), "{}", warning.message);
    }

    #[test]
    fn warnings_cover_overdrawn_funds_homes_reserves_and_names() {
        let txs = vec![
            tx(
                "2026-10-01",
                "Opening",
                &[(CHECKING, 10000), ("equity:opening", -10000)],
            ),
            tx(
                "2026-10-02",
                "Allocate",
                &[(INSURANCE, 15000), (CHECKING, -15000)],
            ),
            tx(
                "2026-10-03",
                "Bill",
                &[("expenses:bills:insurance", 20000), (INSURANCE, -20000)],
            ),
            tx(
                "2026-10-04",
                "Groceries",
                &[("expenses:household:groceries", 4000), (VISA, -4000)],
            ),
            tx(
                "2026-10-05",
                "Mystery",
                &[("expenses:uncategorized", 100), ("assets:other", -100)],
            ),
            tx(
                "2026-10-06",
                "Car",
                &[
                    ("liabilities:loans:car", 100),
                    ("assets:bank:checking:car", -100),
                ],
            ),
        ];
        let chart = chart(&txs, &[VISA_RESERVE]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        let codes: Vec<&str> = report.warnings.iter().map(|w| w.code).collect();
        assert_eq!(
            codes,
            [
                "negative-fund",
                "negative-fund",
                "negative-free-money",
                "reserve-mismatch",
                "unclassified",
                "uncategorized",
            ],
            "{:?}",
            report.warnings
        );
        let mismatch = &report.warnings[3];
        assert!(
            mismatch.message.contains("1 transaction"),
            "{}",
            mismatch.message
        );
        assert_eq!(report.cards[0].unreserved.len(), 1);
        assert_eq!(report.cards[0].unreserved[0].missing.money, usd(4000));
        assert_eq!(validation_warnings(&chart, &txs, "USD"), report.warnings);
        // Car the fund and Car the loan get told apart.
        assert_eq!(
            chart.display_name(&AccountName::new("assets:bank:checking:car")),
            "Car (fund)"
        );
    }
    #[test]
    fn allocations_stay_inside_one_home() {
        let txs = vec![tx(
            "2026-10-01",
            "Opening",
            &[(CHECKING, 10000), ("equity:opening", -10000)],
        )];
        let chart = chart(
            &txs,
            &[
                INSURANCE,
                VISA_RESERVE,
                "assets:bank:savings:emergency-fund",
                "assets:bank:savings",
                VISA,
            ],
        );
        let plan = plan_allocation(&chart, CHECKING, INSURANCE, usd(10000), None).unwrap();
        assert_eq!(plan.description, "Allocate | to Insurance");
        assert_eq!(
            plan.postings,
            [
                (AccountName::new(INSURANCE), usd(10000)),
                (AccountName::new(CHECKING), usd(-10000))
            ]
        );
        let plan = plan_allocation(&chart, INSURANCE, CHECKING, usd(500), None).unwrap();
        assert_eq!(plan.description, "Release | from Insurance");
        let plan = plan_allocation(
            &chart,
            INSURANCE,
            VISA_RESERVE,
            usd(500),
            Some("Cover the card"),
        )
        .unwrap();
        assert_eq!(plan.description, "Cover the card");

        let refused = |from: &str, to: &str, cents: i128| {
            plan_allocation(&chart, from, to, usd(cents), None).unwrap_err()
        };
        assert!(matches!(
            refused(CHECKING, "assets:bank:savings:emergency-fund", 1),
            FundsError::PhysicalTransfer { .. }
        ));
        assert!(matches!(
            refused(INSURANCE, "assets:bank:savings:emergency-fund", 500),
            FundsError::PhysicalTransfer { from_home, to_home, .. }
                if from_home == CHECKING && to_home == "assets:bank:savings"
        ));
        assert!(matches!(
            refused(CHECKING, "expenses:bills:insurance", 1),
            FundsError::NotAllocatable {
                kind: AccountKind::Expense,
                ..
            }
        ));
        assert!(matches!(
            refused(VISA, VISA_RESERVE, 1),
            FundsError::NotAllocatable {
                kind: AccountKind::RevolvingDebt,
                ..
            }
        ));
        assert_eq!(refused(CHECKING, INSURANCE, 0), FundsError::NotPositive);
        assert_eq!(refused(INSURANCE, INSURANCE, 1), FundsError::SameAccount);
    }
    #[test]
    fn a_new_fund_is_named_from_what_was_typed() {
        assert_eq!(
            new_fund_account(CHECKING, "  Holiday   Gifts "),
            Ok("assets:bank:checking:holiday-gifts".to_string())
        );
        assert_eq!(
            new_fund_account(CHECKING, "Car Repairs"),
            Ok("assets:bank:checking:car-repairs".to_string())
        );
        assert!(new_fund_account(CHECKING, "   ").is_err());
        assert!(new_fund_account(CHECKING, "travel:japan").is_err());
        assert!(new_fund_account(CHECKING, "gifts; note").is_err());
    }
    #[test]
    fn only_a_new_expense_or_income_category_can_be_made_inline() {
        let txs = vec![tx(
            "2026-10-01",
            "Opening",
            &[(CHECKING, 10000), ("equity:opening", -10000)],
        )];
        let chart = chart(&txs, &[INSURANCE, "expenses:groceries"]);
        assert_eq!(check_new_category(&chart, "expenses:pets:vet"), Ok(()));
        assert_eq!(check_new_category(&chart, "income:side-gigs"), Ok(()));
        assert!(check_new_category(&chart, "expenses:groceries").is_err());
        assert!(check_new_category(&chart, "assets:bank:savings").is_err());
        assert!(check_new_category(&chart, "expenses:pets;vet").is_err());
    }
    #[test]
    fn a_new_fund_sits_directly_in_the_money_it_comes_from() {
        let txs = vec![tx(
            "2026-10-01",
            "Opening",
            &[(CHECKING, 10000), ("equity:opening", -10000)],
        )];
        let chart = chart(
            &txs,
            &[INSURANCE, VISA_RESERVE, VISA, "assets:bank:savings"],
        );
        assert_eq!(
            check_new_fund(&chart, CHECKING, "assets:bank:checking:vacation"),
            Ok(())
        );
        let refused = |from: &str, fund: &str| check_new_fund(&chart, from, fund).unwrap_err();
        assert!(matches!(
            refused(CHECKING, INSURANCE),
            FundsError::FundExists { .. }
        ));
        assert!(matches!(
            refused(CHECKING, "assets:bank:savings:vacation"),
            FundsError::NewFundElsewhere { .. }
        ));
        assert!(matches!(
            refused(CHECKING, "assets:bank:checking:travel:japan"),
            FundsError::NewFundElsewhere { .. }
        ));
        assert!(matches!(
            refused(INSURANCE, "assets:bank:checking:insurance:extra"),
            FundsError::NewFundElsewhere { .. }
        ));
        // Named like a card's reserve, so not a fund.
        assert!(matches!(
            refused(CHECKING, "assets:bank:checking:card-spare"),
            FundsError::NotAFund { .. }
        ));
    }
    #[test]
    fn card_purchases_and_payments_carry_their_reserve_lines() {
        let txs = vec![tx(
            "2026-10-01",
            "Opening",
            &[(CHECKING, 10000), ("equity:opening", -10000)],
        )];
        let chart = chart(
            &txs,
            &[
                INSURANCE,
                VISA_RESERVE,
                VISA,
                "liabilities:credit-card:spare-card",
            ],
        );
        let free = plan_card_purchase(
            &chart,
            VISA,
            "expenses:household:groceries",
            usd(4000),
            &PurchaseSource::parse("free"),
            "Corner Market",
        )
        .unwrap();
        assert_eq!(
            free.postings,
            [
                (AccountName::new("expenses:household:groceries"), usd(4000)),
                (AccountName::new(VISA), usd(-4000)),
                (AccountName::new(CHECKING), usd(-4000)),
                (AccountName::new(VISA_RESERVE), usd(4000)),
            ]
        );
        let planned = plan_card_purchase(
            &chart,
            VISA,
            "expenses:bills:insurance",
            usd(10000),
            &PurchaseSource::parse(INSURANCE),
            "Insurer",
        )
        .unwrap();
        assert_eq!(
            planned.postings[2],
            (AccountName::new(INSURANCE), usd(-10000))
        );
        for plan in [&free, &planned] {
            let t = tx(
                "2026-10-02",
                "x",
                &plan
                    .postings
                    .iter()
                    .map(|(a, m)| (a.as_str(), m.mantissa().to_i128().unwrap()))
                    .collect::<Vec<_>>(),
            );
            assert!(chart.missing_reserves(&t).is_empty());
        }

        let refused = |card: &str, source: &str| {
            plan_card_purchase(
                &chart,
                card,
                "expenses:x",
                usd(1),
                &PurchaseSource::parse(source),
                "x",
            )
            .unwrap_err()
        };
        assert!(matches!(
            refused(CHECKING, "free"),
            FundsError::NotACard { .. }
        ));
        assert!(matches!(
            refused(VISA, CHECKING),
            FundsError::NotAFund {
                kind: AccountKind::Money,
                ..
            }
        ));
        assert!(matches!(
            refused("liabilities:credit-card:spare-card", "free"),
            FundsError::NoReserve { .. }
        ));

        let pay = plan_card_payment(&chart, VISA, usd(42500)).unwrap();
        assert_eq!(pay.description, "Rewards Visa | Payment");
        assert_eq!(
            pay.postings,
            [
                (AccountName::new(VISA), usd(42500)),
                (AccountName::new(VISA_RESERVE), usd(-42500))
            ]
        );
        assert_eq!(
            plan_card_payment(&chart, VISA, usd(-1)).unwrap_err(),
            FundsError::NotPositive
        );
    }
    #[test]
    fn a_reserve_splits_by_source_since_it_last_reached_zero_paying_oldest_first() {
        let cellphone = "assets:bank:checking:cellphone";
        let txs = vec![
            tx(
                "2026-09-01",
                "Opening",
                &[
                    (CHECKING, 100000),
                    ("equity:opening", -100000),
                    (INSURANCE, 50000),
                    ("equity:opening", -50000),
                ],
            ),
            // Before the reserve last reached zero: not counted.
            tx(
                "2026-09-02",
                "Old",
                &[
                    ("expenses:x", 7000),
                    (VISA, -7000),
                    (CHECKING, -7000),
                    (VISA_RESERVE, 7000),
                ],
            ),
            tx(
                "2026-09-03",
                "Old payment",
                &[(VISA, 7000), (VISA_RESERVE, -7000)],
            ),
            tx(
                "2026-09-10",
                "Groceries",
                &[
                    ("expenses:x", 4000),
                    (VISA, -4000),
                    (CHECKING, -4000),
                    (VISA_RESERVE, 4000),
                ],
            ),
            tx(
                "2026-09-11",
                "Insurer",
                &[
                    ("expenses:bills:insurance", 10000),
                    (VISA, -10000),
                    (INSURANCE, -10000),
                    (VISA_RESERVE, 10000),
                ],
            ),
            tx(
                "2026-09-12",
                "Phone",
                &[
                    ("expenses:bills:cellphone", 3000),
                    (VISA, -3000),
                    (cellphone, -3000),
                    (VISA_RESERVE, 3000),
                ],
            ),
            // Partial payment: the oldest posting (groceries) goes first.
            tx(
                "2026-09-20",
                "Payment",
                &[(VISA, 6000), (VISA_RESERVE, -6000)],
            ),
            // A refund goes back to its fund.
            tx(
                "2026-09-21",
                "Refund",
                &[
                    ("expenses:bills:cellphone", -1000),
                    (VISA, 1000),
                    (cellphone, 1000),
                    (VISA_RESERVE, -1000),
                ],
            ),
        ];
        let chart = chart(&txs, &[cellphone]);
        let reserves = card_reserves(&chart, &txs, "USD", None);
        assert_eq!(reserves.len(), 1);
        let visa = &reserves[0];
        assert_eq!(visa.since, NaiveDate::from_ymd_opt(2026, 9, 3));
        assert_eq!(visa.card.reserve_balance.money, usd(10000));
        assert_eq!(visa.card.owed.money, usd(10000));
        assert_eq!(
            visa.sources,
            [
                (
                    ReserveSource::Fund(AccountName::new(cellphone)),
                    Amount::new("USD", usd(2000))
                ),
                (
                    ReserveSource::Fund(AccountName::new(INSURANCE)),
                    Amount::new("USD", usd(8000))
                ),
            ]
        );
        assert!(
            card_reserves(&chart, &txs, "USD", Some("liabilities:credit-card:other")).is_empty()
        );

        // A credit balance on the card mirrors a negative free amount.
        let credit = vec![
            tx(
                "2026-09-10",
                "Shoes",
                &[
                    ("expenses:x", 1000),
                    (VISA, -1000),
                    (CHECKING, -1000),
                    (VISA_RESERVE, 1000),
                ],
            ),
            tx(
                "2026-09-11",
                "Return",
                &[
                    ("expenses:x", -5432),
                    (VISA, 5432),
                    (CHECKING, 5432),
                    (VISA_RESERVE, -5432),
                ],
            ),
        ];
        let chart = chart_for(&credit);
        let visa = &card_reserves(&chart, &credit, "USD", None)[0];
        assert_eq!(
            visa.sources,
            [(ReserveSource::Free, Amount::new("USD", usd(-4432)))]
        );
    }

    fn purchase(date: &str, description: &str, cents: i128) -> Transaction {
        tx(
            date,
            description,
            &[("expenses:household:groceries", cents), (VISA, -cents)],
        )
    }

    fn opened_card() -> Vec<Transaction> {
        vec![tx(
            "2026-01-01",
            "Opening",
            &[(CHECKING, 100000), ("equity:opening", -100000)],
        )]
    }

    /// An unreserved purchase, its reversal and an unreserved replacement
    /// are each judged on their own: all three are flagged, and the flagged
    /// amounts add up to the card's difference.
    #[test]
    fn each_transaction_is_judged_on_its_own() {
        let mut txs = opened_card();
        txs.extend([
            purchase("2026-02-01", "Store", 4000),
            purchase("2026-02-05", "Store reversal", -4000),
            purchase("2026-02-05", "Store replacement", 4500),
        ]);
        let chart = chart(&txs, &[VISA_RESERVE]);

        let flags = unreserved_flags(&chart, &txs);
        assert_eq!(flags.keys().cloned().collect::<Vec<_>>(), vec![1, 2, 3]);

        let report = cash_on_hand(&chart, &txs, "USD", None);
        let card = report
            .cards
            .iter()
            .find(|c| c.card.as_str() == VISA)
            .unwrap();
        assert_eq!(card.unreserved.len(), 3);
        assert_eq!(card.difference.money, usd(4500));
        assert_eq!(card.missing_total(), card.difference.money);
        assert!(!report
            .warnings
            .iter()
            .any(|w| w.code == "unreserved-unaccounted"));
    }

    /// Every transaction reserved: nothing flagged.
    #[test]
    fn a_fully_reserved_card_has_nothing_flagged() {
        let mut txs = opened_card();
        txs.push(tx(
            "2026-02-01",
            "Store",
            &[
                ("expenses:household:groceries", 4000),
                (VISA, -4000),
                (CHECKING, -4000),
                (VISA_RESERVE, 4000),
            ],
        ));
        let chart = chart(&txs, &[VISA_RESERVE]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        let card = report
            .cards
            .iter()
            .find(|c| c.card.as_str() == VISA)
            .unwrap();
        assert_eq!(card.unreserved.len(), 0);
        assert!(card.difference.money.is_zero());
    }

    /// A reversal of a reserved purchase that leaves out the reserve lines
    /// moves the card without its reserve, so it is flagged, and the
    /// flagged amounts still add up to the difference.
    #[test]
    fn a_reversal_without_reserve_lines_is_flagged() {
        let mut txs = opened_card();
        txs.extend([
            tx(
                "2026-02-01",
                "Store",
                &[
                    ("expenses:household:groceries", 4000),
                    (VISA, -4000),
                    (CHECKING, -4000),
                    (VISA_RESERVE, 4000),
                ],
            ),
            purchase("2026-02-05", "Store reversal", -4000),
        ]);
        let chart = chart(&txs, &[VISA_RESERVE]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        let card = report
            .cards
            .iter()
            .find(|c| c.card.as_str() == VISA)
            .unwrap();
        assert_eq!(card.unreserved.len(), 1);
        assert_eq!(card.missing_total(), card.difference.money);
        assert_eq!(card.difference.money, usd(-4000));
        assert!(!report
            .warnings
            .iter()
            .any(|w| w.code == "unreserved-unaccounted"));
    }

    /// Money moved into a reserve without the card is short of nothing a
    /// transaction explains: the invariant check says so.
    #[test]
    fn a_difference_no_flagged_transaction_explains_is_a_warning() {
        let mut txs = opened_card();
        txs.push(tx(
            "2026-02-01",
            "Stray",
            &[(VISA_RESERVE, 1000), (CHECKING, -1000)],
        ));
        let chart = chart(&txs, &[VISA_RESERVE, VISA]);
        let report = cash_on_hand(&chart, &txs, "USD", None);
        let warning = report
            .warnings
            .iter()
            .find(|w| w.code == "unreserved-unaccounted")
            .expect("an unreserved-unaccounted warning");
        assert!(
            warning.message.contains("10.00 is unaccounted"),
            "{}",
            warning.message
        );
    }

    fn chart_for(txs: &[Transaction]) -> Chart {
        chart(txs, &[])
    }
}
