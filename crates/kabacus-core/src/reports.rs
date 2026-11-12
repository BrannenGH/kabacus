// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reports over the books: the income statement with its equity
//! conversion section, the balance sheet, category breakdowns, Cash on Hand
//! history, the cash flow bridge, receivables activity and the suggestions
//! `reconcile` makes on a mismatch.
//!
//! Every report counts every transaction, reversals too, in one commodity,
//! at the books' precision.

use crate::funds::Chart;
use crate::kinds::AccountKind;
use crate::{AccountName, Money, Transaction};
use chrono::{Datelike, Duration, NaiveDate};
use num_bigint::BigInt;
use num_traits::Signed;
use std::collections::{BTreeMap, HashMap};

/// The most points [`cash_on_hand_history`] returns.
pub const MAX_HISTORY_POINTS: usize = 366;

/// How a report's range is split into periods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    None,
    Month,
    Quarter,
    Year,
}

impl Interval {
    pub fn parse(s: &str) -> Option<Interval> {
        match s {
            "none" => Some(Interval::None),
            "month" => Some(Interval::Month),
            "quarter" => Some(Interval::Quarter),
            "year" => Some(Interval::Year),
            _ => None,
        }
    }
}

/// One period of a report, both ends inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Period {
    pub fn contains(&self, date: NaiveDate) -> bool {
        self.start <= date && date <= self.end
    }

    /// `2026-09`, `2026-Q3`, `2026`, or `from..to` for the whole range.
    pub fn label(&self, interval: Interval) -> String {
        match interval {
            Interval::Month => self.start.format("%Y-%m").to_string(),
            Interval::Quarter => format!("{}-Q{}", self.start.year(), self.start.month0() / 3 + 1),
            Interval::Year => self.start.format("%Y").to_string(),
            Interval::None => format!(
                "{}..{}",
                self.start.format("%Y-%m-%d"),
                self.end.format("%Y-%m-%d")
            ),
        }
    }
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).expect("a valid month")
}

fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    let total = date.year() * 12 + date.month0() as i32 + months as i32;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1)
        .expect("a valid month")
}

/// `from..=to` split by `interval`, calendar-aligned: the first and last
/// periods are clipped to the range.
pub fn periods(from: NaiveDate, to: NaiveDate, interval: Interval) -> Vec<Period> {
    if to < from {
        return Vec::new();
    }
    let step = match interval {
        Interval::None => {
            return vec![Period {
                start: from,
                end: to,
            }]
        }
        Interval::Month => 1,
        Interval::Quarter => 3,
        Interval::Year => 12,
    };
    let mut start = match interval {
        Interval::Quarter => {
            NaiveDate::from_ymd_opt(from.year(), from.month0() / 3 * 3 + 1, 1).expect("a quarter")
        }
        Interval::Year => NaiveDate::from_ymd_opt(from.year(), 1, 1).expect("a year"),
        _ => first_of_month(from),
    };
    let mut out = Vec::new();
    while start <= to {
        let next = add_months(start, step);
        out.push(Period {
            start: start.max(from),
            end: (next - Duration::days(1)).min(to),
        });
        start = next;
    }
    out
}

/// The period of equal length just before `from..=to`.
pub fn previous_period(from: NaiveDate, to: NaiveDate) -> Period {
    let end = from - Duration::days(1);
    Period {
        start: end - (to - from),
        end,
    }
}

/// Accounts left out of a report: by kind, or by account prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exclude {
    pub kinds: Vec<AccountKind>,
    pub prefixes: Vec<String>,
}

impl Exclude {
    /// Each entry is a kind (`"receivable"`) or an account prefix.
    pub fn parse(entries: &[String]) -> Exclude {
        let mut out = Exclude::default();
        for entry in entries {
            let kind: Option<AccountKind> =
                serde_json::from_value(serde_json::Value::String(entry.clone())).ok();
            match kind {
                Some(kind) => out.kinds.push(kind),
                None => out.prefixes.push(entry.clone()),
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty() && self.prefixes.is_empty()
    }

    pub fn excludes(&self, chart: &Chart, account: &AccountName) -> bool {
        self.excludes_kind(chart.kind_of(account))
            || self.prefixes.iter().any(|p| account.is_under(p))
    }

    pub fn excludes_kind(&self, kind: AccountKind) -> bool {
        self.kinds.contains(&kind)
    }
}

/// `account` cut to its first `depth` segments (all of them for 0).
pub fn at_depth(account: &AccountName, depth: usize) -> AccountName {
    if depth == 0 {
        return account.clone();
    }
    AccountName::new(
        account
            .as_str()
            .split(':')
            .take(depth)
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// The books' precision in `commodity`: the most decimal places any of its
/// amounts has.
pub fn precision(transactions: &[Transaction], commodity: &str) -> u32 {
    transactions
        .iter()
        .flat_map(|tx| &tx.postings)
        .filter(|p| p.amount.commodity == commodity)
        .map(|p| p.amount.money.places())
        .max()
        .unwrap_or(2)
}

/// The amounts a report shows, all at one precision.
#[derive(Debug, Clone, Copy)]
pub struct Scale(pub u32);

impl Scale {
    pub fn of(transactions: &[Transaction], commodity: &str) -> Scale {
        Scale(precision(transactions, commodity))
    }

    pub fn money(self, m: Money) -> Money {
        m.rescaled(self.0.max(m.places()))
    }

    pub fn zero(self) -> Money {
        Money::from_decimal(0, self.0)
    }
}

/// A transaction's cash effect: the sum of its postings to accounts
/// of kind `money` (free money). Zero for a non-cash transaction.
pub fn cash_effect(chart: &Chart, tx: &Transaction, commodity: &str) -> Money {
    tx.postings
        .iter()
        .filter(|p| p.amount.commodity == commodity)
        .filter(|p| chart.kind_of(&p.account) == AccountKind::Money)
        .map(|p| p.amount.money.clone())
        .sum()
}

// --- Cash on Hand totals ---------------------------------------------------

/// The Cash on Hand components as of a day, without the account
/// detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashTotals {
    pub as_of: Option<NaiveDate>,
    /// Free money: the money accounts' own balances, less what overdrawn
    /// funds and card reserves have taken from them.
    pub money: Money,
    /// What the funds set aside; an overdrawn fund counts as zero.
    pub funds: Money,
    /// What the card reserves set aside; a negative reserve counts as zero.
    pub reserves: Money,
    pub receivables: Money,
    pub money_on_hand: Money,
    pub future_allocations: Money,
    pub cash_on_hand: Money,
}

impl CashTotals {
    /// `self - earlier`, component by component.
    pub fn change_since(&self, earlier: &CashTotals) -> CashTotals {
        CashTotals {
            as_of: self.as_of,
            money: self.money.clone() - earlier.money.clone(),
            funds: self.funds.clone() - earlier.funds.clone(),
            reserves: self.reserves.clone() - earlier.reserves.clone(),
            receivables: self.receivables.clone() - earlier.receivables.clone(),
            money_on_hand: self.money_on_hand.clone() - earlier.money_on_hand.clone(),
            future_allocations: self.future_allocations.clone()
                - earlier.future_allocations.clone(),
            cash_on_hand: self.cash_on_hand.clone() - earlier.cash_on_hand.clone(),
        }
    }
}

/// Cash on Hand's totals as of the end of `as_of`, over every transaction.
pub fn cash_totals(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    as_of: Option<NaiveDate>,
) -> CashTotals {
    let scale = Scale::of(transactions, commodity);
    let mut by_kind: BTreeMap<AccountKind, Money> = BTreeMap::new();
    // Each fund's and reserve's own balance, to find the overdrawn ones.
    let mut funds_by_account: HashMap<&AccountName, (AccountKind, Money)> = HashMap::new();
    for tx in transactions
        .iter()
        .filter(|tx| as_of.is_none_or(|d| tx.date <= d))
    {
        for p in tx
            .postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
        {
            let kind = chart.kind_of(&p.account);
            let total = by_kind.entry(kind).or_insert_with(Money::zero);
            *total = &*total + &p.amount.money;
            if matches!(kind, AccountKind::PreAllocation | AccountKind::DebtReserve) {
                let (_, fund) = funds_by_account
                    .entry(&p.account)
                    .or_insert_with(|| (kind, Money::zero()));
                *fund = fund.clone() + p.amount.money.clone();
            }
        }
    }
    // An overdrawn fund or reserve sets nothing aside: its overdraft comes
    // off free money, as in `funds::cash_on_hand`.
    let overdrafts = |of: AccountKind| -> Money {
        funds_by_account
            .values()
            .filter(|(kind, m)| *kind == of && m.is_negative())
            .map(|(_, m)| -m.clone())
            .sum()
    };
    let fund_overdrafts = overdrafts(AccountKind::PreAllocation);
    let reserve_overdrafts = overdrafts(AccountKind::DebtReserve);
    let get = |k| scale.money(by_kind.get(&k).cloned().unwrap_or_else(Money::zero));
    let money =
        scale.money(get(AccountKind::Money) - fund_overdrafts.clone() - reserve_overdrafts.clone());
    let funds = scale.money(get(AccountKind::PreAllocation) + fund_overdrafts);
    let reserves = scale.money(get(AccountKind::DebtReserve) + reserve_overdrafts);
    let receivables = get(AccountKind::Receivable);
    let money_on_hand = money.clone() + funds.clone() + reserves.clone() + receivables.clone();
    let future_allocations = funds.clone() + reserves.clone();
    CashTotals {
        as_of,
        money,
        funds,
        reserves,
        receivables: receivables.clone(),
        money_on_hand: scale.money(money_on_hand.clone()),
        future_allocations: scale.money(future_allocations.clone()),
        cash_on_hand: scale.money(money_on_hand - future_allocations - receivables),
    }
}

/// Cash on Hand at the end of each period of `from..=to`, at most
/// [`MAX_HISTORY_POINTS`] points (the latest ones).
pub fn cash_on_hand_history(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    from: NaiveDate,
    to: NaiveDate,
    interval: Interval,
) -> Vec<CashTotals> {
    let ends: Vec<NaiveDate> = match interval {
        // Without an interval, one point a day.
        Interval::None => {
            let mut days = Vec::new();
            let mut d = from;
            while d <= to {
                days.push(d);
                d += Duration::days(1);
            }
            days
        }
        _ => periods(from, to, interval).iter().map(|p| p.end).collect(),
    };
    let skip = ends.len().saturating_sub(MAX_HISTORY_POINTS);
    ends.into_iter()
        .skip(skip)
        .map(|end| cash_totals(chart, transactions, commodity, Some(end)))
        .collect()
}

// --- income statement -------------------------------------------------------

/// One account's (or group's) amounts, one per period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    pub account: AccountName,
    pub display_name: String,
    pub amounts: Vec<Money>,
    pub total: Money,
}

/// One equity conversion row: the decrease in what is owed on a loan (or,
/// for `non-liquid`, money moved into it), split into cash and non-cash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionRow {
    pub account: AccountName,
    pub display_name: String,
    pub amounts: Vec<Money>,
    pub cash: Vec<Money>,
    pub non_cash: Vec<Money>,
    pub total: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomeStatementOptions {
    pub depth: usize,
    pub exclude: Exclude,
    /// Add the equity conversion section and subtract it from `net`.
    pub equity_conversion: bool,
    pub conversion_kinds: Vec<AccountKind>,
}

impl Default for IncomeStatementOptions {
    fn default() -> Self {
        IncomeStatementOptions {
            depth: 2,
            exclude: Exclude::default(),
            equity_conversion: true,
            conversion_kinds: vec![AccountKind::TermLiability],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomeStatement {
    pub periods: Vec<Period>,
    /// Income as positive amounts.
    pub income: Vec<ReportRow>,
    pub expenses: Vec<ReportRow>,
    pub income_totals: Vec<Money>,
    pub expense_totals: Vec<Money>,
    /// Loan payments that turn cash into equity; `None` when `equity_conversion` is off or excluded.
    pub conversion: Option<Vec<ConversionRow>>,
    pub conversion_totals: Option<Vec<Money>>,
    /// Income minus expenses.
    pub net_before_conversion: Vec<Money>,
    /// Income minus expenses minus equity conversion.
    pub net: Vec<Money>,
}

fn sum_columns(rows: &[Vec<Money>], width: usize, scale: Scale) -> Vec<Money> {
    (0..width)
        .map(|i| scale.money(rows.iter().map(|r| r[i].clone()).sum()))
        .collect()
}

fn rows_from(
    chart: &Chart,
    grouped: BTreeMap<AccountName, Vec<Money>>,
    scale: Scale,
) -> Vec<ReportRow> {
    grouped
        .into_iter()
        .map(|(account, amounts)| {
            let amounts: Vec<Money> = amounts.into_iter().map(|m| scale.money(m)).collect();
            ReportRow {
                display_name: chart.display_name(&account),
                total: scale.money(amounts.iter().cloned().sum()),
                account,
                amounts,
            }
        })
        .collect()
}

/// Income and expenses by period, with the equity conversion section.
pub fn income_statement(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    periods: &[Period],
    options: &IncomeStatementOptions,
) -> IncomeStatement {
    let scale = Scale::of(transactions, commodity);
    let width = periods.len();
    let mut income: BTreeMap<AccountName, Vec<Money>> = BTreeMap::new();
    let mut expenses: BTreeMap<AccountName, Vec<Money>> = BTreeMap::new();
    // account -> (cash, non_cash) per period
    let mut conversion: BTreeMap<AccountName, (Vec<Money>, Vec<Money>)> = BTreeMap::new();
    let converting = options.equity_conversion
        && !options
            .conversion_kinds
            .iter()
            .all(|k| options.exclude.excludes_kind(*k));

    for tx in transactions {
        let Some(column) = periods.iter().position(|p| p.contains(tx.date)) else {
            continue;
        };
        let is_cash = !cash_effect(chart, tx, commodity).is_zero();
        for p in tx
            .postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
        {
            if options.exclude.excludes(chart, &p.account) {
                continue;
            }
            let kind = chart.kind_of(&p.account);
            let money = p.amount.money.clone();
            let add = |map: &mut BTreeMap<AccountName, Vec<Money>>, amount: Money| {
                let entry = map
                    .entry(at_depth(&p.account, options.depth))
                    .or_insert_with(|| vec![Money::zero(); width]);
                entry[column] = entry[column].clone() + amount;
            };
            match kind {
                AccountKind::Income => add(&mut income, -money),
                AccountKind::Expense => add(&mut expenses, money),
                kind if converting && options.conversion_kinds.contains(&kind) => {
                    let entry = conversion.entry(p.account.clone()).or_insert_with(|| {
                        (vec![Money::zero(); width], vec![Money::zero(); width])
                    });
                    let column_of = if is_cash { &mut entry.0 } else { &mut entry.1 };
                    column_of[column] = column_of[column].clone() + money;
                }
                _ => {}
            }
        }
    }

    let income = rows_from(chart, income, scale);
    let expenses = rows_from(chart, expenses, scale);
    let income_totals = sum_columns(
        &income.iter().map(|r| r.amounts.clone()).collect::<Vec<_>>(),
        width,
        scale,
    );
    let expense_totals = sum_columns(
        &expenses
            .iter()
            .map(|r| r.amounts.clone())
            .collect::<Vec<_>>(),
        width,
        scale,
    );
    let net_before_conversion: Vec<Money> = (0..width)
        .map(|i| scale.money(income_totals[i].clone() - expense_totals[i].clone()))
        .collect();

    let (conversion, conversion_totals, net) = if converting {
        let rows: Vec<ConversionRow> = conversion
            .into_iter()
            .map(|(account, (cash, non_cash))| {
                let cash: Vec<Money> = cash.into_iter().map(|m| scale.money(m)).collect();
                let non_cash: Vec<Money> = non_cash.into_iter().map(|m| scale.money(m)).collect();
                let amounts: Vec<Money> = (0..width)
                    .map(|i| scale.money(cash[i].clone() + non_cash[i].clone()))
                    .collect();
                ConversionRow {
                    display_name: chart.display_name(&account),
                    total: scale.money(amounts.iter().cloned().sum()),
                    account,
                    amounts,
                    cash,
                    non_cash,
                }
            })
            .collect();
        let totals = sum_columns(
            &rows.iter().map(|r| r.amounts.clone()).collect::<Vec<_>>(),
            width,
            scale,
        );
        let net = (0..width)
            .map(|i| scale.money(net_before_conversion[i].clone() - totals[i].clone()))
            .collect();
        (Some(rows), Some(totals), net)
    } else {
        (None, None, net_before_conversion.clone())
    };

    IncomeStatement {
        periods: periods.to_vec(),
        income,
        expenses,
        income_totals,
        expense_totals,
        conversion,
        conversion_totals,
        net_before_conversion,
        net,
    }
}

// --- balance sheet ------------------------------------------------------------

/// Which side of the balance sheet an account is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Asset,
    Liability,
}

/// Inference from the top-level name (`assets`,
/// `liabilities`, `debts`), else from the account's kind.
pub fn side_of(chart: &Chart, account: &AccountName) -> Option<Side> {
    let top = account
        .as_str()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    match top.as_str() {
        "asset" | "assets" => return Some(Side::Asset),
        "liability" | "liabilities" | "debt" | "debts" => return Some(Side::Liability),
        _ => {}
    }
    match chart.kind_of(account) {
        AccountKind::Money
        | AccountKind::PreAllocation
        | AccountKind::DebtReserve
        | AccountKind::Receivable => Some(Side::Asset),
        AccountKind::RevolvingDebt | AccountKind::TermLiability | AccountKind::Payable => {
            Some(Side::Liability)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Kind,
    Account,
}

impl GroupBy {
    pub fn parse(s: &str) -> Option<GroupBy> {
        match s {
            "kind" => Some(GroupBy::Kind),
            "account" => Some(GroupBy::Account),
            _ => None,
        }
    }
}

/// One balance sheet row: a kind or an account (cut to the depth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetRow {
    /// The account, or the kind's name.
    pub group: String,
    pub display_name: String,
    pub balance: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceSheet {
    pub as_of: Option<NaiveDate>,
    pub assets: Vec<SheetRow>,
    pub liabilities: Vec<SheetRow>,
    pub total_assets: Money,
    /// As booked: negative while money is owed.
    pub total_liabilities: Money,
    pub by_kind: Vec<(AccountKind, Money)>,
    /// Money, funds and card reserves.
    pub liquid: Money,
    /// Assets plus liabilities (which are negative).
    pub net_worth: Money,
}

/// Every asset and liability account's balance as of `as_of`.
pub fn balance_sheet(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    as_of: Option<NaiveDate>,
    depth: usize,
    group_by: GroupBy,
) -> BalanceSheet {
    let scale = Scale::of(transactions, commodity);
    let mut balances: BTreeMap<AccountName, Money> = BTreeMap::new();
    for tx in transactions
        .iter()
        .filter(|tx| as_of.is_none_or(|d| tx.date <= d))
    {
        for p in tx
            .postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
        {
            let total = balances
                .entry(p.account.clone())
                .or_insert_with(Money::zero);
            *total = &*total + &p.amount.money;
        }
    }
    let mut sides: BTreeMap<(Side, String), (String, Money)> = BTreeMap::new();
    let mut by_kind: BTreeMap<AccountKind, Money> = BTreeMap::new();
    for (account, money) in &balances {
        let Some(side) = side_of(chart, account) else {
            continue;
        };
        let kind = chart.kind_of(account);
        let total = by_kind.entry(kind).or_insert_with(Money::zero);
        *total = total.clone() + money.clone();
        let (group, display_name) = match group_by {
            GroupBy::Kind => (kind.as_str().to_string(), kind.as_str().to_string()),
            GroupBy::Account => {
                let cut = at_depth(account, depth);
                (cut.to_string(), chart.display_name(&cut))
            }
        };
        let row = sides
            .entry((side, group))
            .or_insert_with(|| (display_name, Money::zero()));
        row.1 = row.1.clone() + money.clone();
    }
    let rows = |side: Side| -> Vec<SheetRow> {
        sides
            .iter()
            .filter(|((s, _), _)| *s == side)
            .map(|((_, group), (display_name, money))| SheetRow {
                group: group.clone(),
                display_name: display_name.clone(),
                balance: scale.money(money.clone()),
            })
            .collect()
    };
    let assets = rows(Side::Asset);
    let liabilities = rows(Side::Liability);
    let total_assets = scale.money(assets.iter().map(|r| r.balance.clone()).sum());
    let total_liabilities = scale.money(liabilities.iter().map(|r| r.balance.clone()).sum());
    let liquid = scale.money(
        [
            AccountKind::Money,
            AccountKind::PreAllocation,
            AccountKind::DebtReserve,
        ]
        .iter()
        .filter_map(|k| by_kind.get(k))
        .cloned()
        .sum(),
    );
    BalanceSheet {
        as_of,
        assets,
        liabilities,
        total_assets: total_assets.clone(),
        total_liabilities: total_liabilities.clone(),
        by_kind: by_kind
            .into_iter()
            .map(|(k, m)| (k, scale.money(m)))
            .collect(),
        liquid,
        net_worth: scale.money(total_assets + total_liabilities),
    }
}

// --- category breakdown -------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRow {
    pub account: AccountName,
    pub display_name: String,
    pub amount: Money,
    /// Percent of all spending in the range, to two places.
    pub share: Money,
    /// The same category over the equal period just before.
    pub previous: Option<Money>,
    pub change: Option<Money>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryBreakdown {
    pub total: Money,
    pub rows: Vec<CategoryRow>,
    /// Spending in the categories past `limit`.
    pub other: Money,
    pub previous_period: Option<Period>,
    pub previous_total: Option<Money>,
}

fn expenses_by_category(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    period: Period,
    depth: usize,
) -> BTreeMap<AccountName, Money> {
    let mut out: BTreeMap<AccountName, Money> = BTreeMap::new();
    for tx in transactions.iter().filter(|tx| period.contains(tx.date)) {
        for p in tx
            .postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
        {
            if chart.kind_of(&p.account) == AccountKind::Expense {
                let total = out
                    .entry(at_depth(&p.account, depth))
                    .or_insert_with(Money::zero);
                *total = &*total + &p.amount.money;
            }
        }
    }
    out
}

/// `part / whole` as a percentage with two decimal places.
pub fn percent(part: Money, whole: Money) -> Money {
    if whole.is_zero() {
        return Money::from_decimal(0, 2);
    }
    let places = part.places().max(whole.places());
    let (p, w) = (
        part.rescaled(places).mantissa().clone(),
        whole.rescaled(places).mantissa().clone(),
    );
    // Rounded half away from zero.
    let scaled: BigInt = p * 10000;
    let q = &scaled / &w;
    let r: BigInt = &scaled % &w;
    let q = if (r * BigInt::from(2)).abs() >= w.abs() {
        q + (scaled.signum() * w.signum())
    } else {
        q
    };
    Money::from_big(q, 2)
}

/// The top `limit` expense categories over `period`, with their
/// share of all spending and, if asked, the change from the equal period
/// before.
pub fn category_breakdown(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    period: Period,
    depth: usize,
    limit: usize,
    compare_previous: bool,
) -> CategoryBreakdown {
    let scale = Scale::of(transactions, commodity);
    let current = expenses_by_category(chart, transactions, commodity, period, depth);
    let total = scale.money(current.values().cloned().sum());
    let previous_period = compare_previous.then(|| previous_period(period.start, period.end));
    let previous =
        previous_period.map(|p| expenses_by_category(chart, transactions, commodity, p, depth));
    let mut ranked: Vec<(AccountName, Money)> = current.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let other = scale.money(ranked.iter().skip(limit).map(|(_, m)| m.clone()).sum());
    let rows = ranked
        .into_iter()
        .take(limit)
        .map(|(account, amount)| {
            let before = previous
                .as_ref()
                .map(|p| scale.money(p.get(&account).cloned().unwrap_or_else(Money::zero)));
            CategoryRow {
                display_name: chart.display_name(&account),
                share: percent(amount.clone(), total.clone()),
                change: before.clone().map(|b| scale.money(amount.clone() - b)),
                previous: before,
                amount: scale.money(amount),
                account,
            }
        })
        .collect();
    CategoryBreakdown {
        total,
        rows,
        other,
        previous_total: previous.map(|p| scale.money(p.values().cloned().sum())),
        previous_period,
    }
}

// --- cash flow ------------------------------------------------------------------

/// The group the term-liability kind is shown as in `cash_flow`.
pub const EQUITY_CONVERSION_GROUP: &str = "equity_conversion";

/// What one transaction contributed to one cash flow line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineTransaction {
    /// Its position in the slice given to [`cash_flow`].
    pub tx: usize,
    pub cash: Money,
    pub non_cash: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashFlowLine {
    /// The kind (`income`, `expense`, ..., `equity_conversion`) or the
    /// account cut to the depth.
    pub group: String,
    pub display_name: String,
    /// From transactions that move free money.
    pub cash: Money,
    /// From transactions that don't.
    pub non_cash: Money,
    pub total: Money,
    pub transactions: Vec<LineTransaction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashFlowOptions {
    pub group_by: GroupBy,
    pub depth: usize,
    pub exclude: Exclude,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashFlow {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Cash on Hand at the end of the day before `from`.
    pub opening_cash: Money,
    pub closing_cash: Money,
    pub change: Money,
    /// `change` without the excluded kinds' and prefixes' cash.
    pub change_excluding: Money,
    pub lines: Vec<CashFlowLine>,
}

/// The bridge from opening to closing Cash on Hand. Each
/// transaction's non-money postings are attributed to lines by kind or
/// account, with the sign of their effect on cash; card and card reserve
/// postings that cancel within a transaction are left out. A transaction
/// that moves free money counts in the `cash` column, any other in
/// `non_cash`. So the cash column sums to `change` and the non-cash column
/// to zero.
pub fn cash_flow(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    from: NaiveDate,
    to: NaiveDate,
    options: &CashFlowOptions,
) -> CashFlow {
    let scale = Scale::of(transactions, commodity);
    let opening = cash_totals(
        chart,
        transactions,
        commodity,
        Some(from - Duration::days(1)),
    );
    let closing = cash_totals(chart, transactions, commodity, Some(to));
    let period = Period {
        start: from,
        end: to,
    };

    let group_of = |account: &AccountName, kind: AccountKind| -> (String, String) {
        match options.group_by {
            GroupBy::Kind => {
                let name = if kind == AccountKind::TermLiability {
                    EQUITY_CONVERSION_GROUP.to_string()
                } else {
                    kind.as_str().to_string()
                };
                (name.clone(), name)
            }
            GroupBy::Account => {
                let cut = at_depth(account, options.depth);
                (cut.to_string(), chart.display_name(&cut))
            }
        }
    };

    let overdrafts = overdraft_changes(chart, transactions, commodity, from, &period);
    let mut lines: BTreeMap<String, CashFlowLine> = BTreeMap::new();
    let mut excluded_cash = Money::zero();
    for (index, tx) in transactions.iter().enumerate() {
        if !period.contains(tx.date) {
            continue;
        }
        let cash = cash_effect(chart, tx, commodity);
        let is_cash = !cash.is_zero();
        // (account, kind, contribution to cash)
        let mut parts: Vec<(AccountName, AccountKind, Money)> = Vec::new();
        let mut card_net = Money::zero();
        let mut card_account: Option<AccountName> = None;
        for p in tx
            .postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
        {
            match chart.kind_of(&p.account) {
                AccountKind::Money => {}
                AccountKind::RevolvingDebt | AccountKind::DebtReserve => {
                    card_net = card_net + p.amount.money.clone();
                    if card_account.is_none()
                        || chart.kind_of(&p.account) == AccountKind::RevolvingDebt
                    {
                        card_account = Some(p.account.clone());
                    }
                }
                kind => parts.push((p.account.clone(), kind, -p.amount.money.clone())),
            }
        }
        if !card_net.is_zero() {
            let account = card_account.expect("a card or reserve posting");
            parts.push((account, AccountKind::RevolvingDebt, -card_net));
        }
        // An overdraft a transaction opens or closes moves Cash on Hand
        // whether or not it moves free money.
        let overdrawn = overdrafts.get(&index).into_iter().flatten();
        let mut by_line: BTreeMap<(String, String), (Money, Money)> = BTreeMap::new();
        for ((account, kind, contribution), in_cash) in parts
            .into_iter()
            .map(|part| (part, is_cash))
            .chain(overdrawn.map(|(account, change)| {
                // A reserve's overdraft goes on its card's line.
                let (account, kind) = match chart.kind_of(account) {
                    AccountKind::DebtReserve => (
                        chart.card_for_reserve(account).unwrap_or(account).clone(),
                        AccountKind::RevolvingDebt,
                    ),
                    kind => (account.clone(), kind),
                };
                ((account, kind, change.clone()), true)
            }))
        {
            if in_cash
                && (options.exclude.excludes(chart, &account)
                    || options.exclude.excludes_kind(kind))
            {
                excluded_cash = excluded_cash + contribution.clone();
            }
            let (cash, non_cash) = by_line
                .entry(group_of(&account, kind))
                .or_insert_with(|| (Money::zero(), Money::zero()));
            if in_cash {
                *cash = cash.clone() + contribution;
            } else {
                *non_cash = non_cash.clone() + contribution;
            }
        }
        for ((group, display_name), (c, n)) in by_line {
            let line = lines.entry(group.clone()).or_insert_with(|| CashFlowLine {
                group,
                display_name,
                cash: Money::zero(),
                non_cash: Money::zero(),
                total: Money::zero(),
                transactions: Vec::new(),
            });
            line.cash = line.cash.clone() + c.clone();
            line.non_cash = line.non_cash.clone() + n.clone();
            line.transactions.push(LineTransaction {
                tx: index,
                cash: scale.money(c),
                non_cash: scale.money(n),
            });
        }
    }
    let lines: Vec<CashFlowLine> = lines
        .into_values()
        .map(|mut l| {
            l.cash = scale.money(l.cash);
            l.non_cash = scale.money(l.non_cash);
            l.total = scale.money(&l.cash + &l.non_cash);
            l
        })
        .collect();
    let change = scale.money(&closing.cash_on_hand - &opening.cash_on_hand);
    CashFlow {
        from,
        to,
        opening_cash: opening.cash_on_hand,
        closing_cash: closing.cash_on_hand,
        change: change.clone(),
        change_excluding: scale.money(change - excluded_cash),
        lines,
    }
}

/// Per transaction in `period`, each fund or card reserve whose overdraft
/// it changes and that change's effect on Cash on Hand. They are followed
/// in date order from their balances before `from`, so the changes add up
/// to the difference between the opening and closing overdrafts.
fn overdraft_changes(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    from: NaiveDate,
    period: &Period,
) -> HashMap<usize, Vec<(AccountName, Money)>> {
    let fund_postings = |tx: &'_ Transaction| {
        tx.postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
            .filter(|p| {
                matches!(
                    chart.kind_of(&p.account),
                    AccountKind::PreAllocation | AccountKind::DebtReserve
                )
            })
            .map(|p| (p.account.clone(), p.amount.money.clone()))
            .collect::<Vec<_>>()
    };
    let overdraft = |m: &Money| {
        if m.is_negative() {
            -m.clone()
        } else {
            Money::zero()
        }
    };
    let mut balances: HashMap<AccountName, Money> = HashMap::new();
    for tx in transactions.iter().filter(|tx| tx.date < from) {
        for (account, amount) in fund_postings(tx) {
            let balance = balances.entry(account).or_insert_with(Money::zero);
            *balance = balance.clone() + amount;
        }
    }
    let mut in_period: Vec<usize> = (0..transactions.len())
        .filter(|&i| period.contains(transactions[i].date))
        .collect();
    in_period.sort_by_key(|&i| transactions[i].date);
    let mut changes: HashMap<usize, Vec<(AccountName, Money)>> = HashMap::new();
    for index in in_period {
        for (account, amount) in fund_postings(&transactions[index]) {
            let balance = balances.entry(account.clone()).or_insert_with(Money::zero);
            let before = overdraft(balance);
            *balance = balance.clone() + amount;
            let change = before - overdraft(balance);
            if !change.is_zero() {
                changes.entry(index).or_default().push((account, change));
            }
        }
    }
    changes
}

// --- receivables activity ---------------------------------------------------------

/// Part of what was filed on a receivable, by the account on the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterShare {
    pub account: AccountName,
    pub amount: Money,
    /// The other side is a credit to an expense: the reimbursement was
    /// netted against the expense.
    pub netted_against_expense: bool,
}

/// One posting to a receivable in the period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivableEntry {
    pub tx: usize,
    pub amount: Money,
    /// The transaction's other postings.
    pub counter: Vec<(AccountName, Money)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub opening: Money,
    /// Debits: what was fronted or filed.
    pub filed: Money,
    /// Credits: what came back.
    pub collected: Money,
    pub closing: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterpartyActivity {
    pub account: AccountName,
    pub display_name: String,
    pub activity: Activity,
    pub filed_by_counter_account: Vec<CounterShare>,
    pub entries: Vec<ReceivableEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivablesActivity {
    pub counterparties: Vec<CounterpartyActivity>,
    pub total: Activity,
}

/// What was filed and collected on each receivable (or only those
/// under `counterparty`) over `from..=to`.
pub fn receivables_activity(
    chart: &Chart,
    transactions: &[Transaction],
    commodity: &str,
    from: NaiveDate,
    to: NaiveDate,
    counterparty: Option<&str>,
) -> ReceivablesActivity {
    let scale = Scale::of(transactions, commodity);
    let wanted = |account: &AccountName| {
        chart.kind_of(account) == AccountKind::Receivable
            && counterparty.is_none_or(|c| account.is_under(c))
    };
    struct Acc {
        opening: Money,
        filed: Money,
        collected: Money,
        shares: BTreeMap<AccountName, Money>,
        entries: Vec<ReceivableEntry>,
    }
    let mut accounts: BTreeMap<AccountName, Acc> = BTreeMap::new();
    for (index, tx) in transactions.iter().enumerate() {
        if tx.date > to {
            continue;
        }
        let postings: Vec<&crate::Posting> = tx
            .postings
            .iter()
            .filter(|p| p.amount.commodity == commodity)
            .collect();
        for p in postings.iter().filter(|p| wanted(&p.account)) {
            let acc = accounts.entry(p.account.clone()).or_insert_with(|| Acc {
                opening: Money::zero(),
                filed: Money::zero(),
                collected: Money::zero(),
                shares: BTreeMap::new(),
                entries: Vec::new(),
            });
            let money = p.amount.money.clone();
            if tx.date < from {
                acc.opening = acc.opening.clone() + money;
                continue;
            }
            if money.is_negative() {
                acc.collected = acc.collected.clone() - money.clone();
            } else {
                acc.filed = acc.filed.clone() + money.clone();
                // Attribute the debit to the credits on the other side.
                let mut left = money.clone();
                for q in postings
                    .iter()
                    .filter(|q| q.account != p.account && q.amount.money.is_negative())
                {
                    if left.is_zero() {
                        break;
                    }
                    let take = left.clone().min(-q.amount.money.clone());
                    let share = acc
                        .shares
                        .entry(q.account.clone())
                        .or_insert_with(Money::zero);
                    *share = share.clone() + take.clone();
                    left = left - take;
                }
                if !left.is_zero() {
                    let share = acc
                        .shares
                        .entry(AccountName::new("(unattributed)"))
                        .or_insert_with(Money::zero);
                    *share = share.clone() + left;
                }
            }
            acc.entries.push(ReceivableEntry {
                tx: index,
                amount: scale.money(money),
                counter: postings
                    .iter()
                    .filter(|q| q.account != p.account)
                    .map(|q| (q.account.clone(), scale.money(q.amount.money.clone())))
                    .collect(),
            });
        }
    }
    let mut total = Activity {
        opening: scale.zero(),
        filed: scale.zero(),
        collected: scale.zero(),
        closing: scale.zero(),
    };
    let counterparties = accounts
        .into_iter()
        .map(|(account, acc)| {
            let activity = Activity {
                closing: scale.money(&acc.opening + &acc.filed - &acc.collected),
                opening: scale.money(acc.opening),
                filed: scale.money(acc.filed),
                collected: scale.money(acc.collected),
            };
            total = Activity {
                opening: scale.money(&total.opening + &activity.opening),
                filed: scale.money(&total.filed + &activity.filed),
                collected: scale.money(&total.collected + &activity.collected),
                closing: scale.money(&total.closing + &activity.closing),
            };
            CounterpartyActivity {
                display_name: chart.display_name(&account),
                filed_by_counter_account: acc
                    .shares
                    .into_iter()
                    .map(|(counter, amount)| CounterShare {
                        netted_against_expense: chart.kind_of(&counter) == AccountKind::Expense,
                        account: counter,
                        amount: scale.money(amount),
                    })
                    .collect(),
                entries: acc.entries,
                account,
                activity,
            }
        })
        .collect();
    ReceivablesActivity {
        counterparties,
        total,
    }
}

// --- reconcile ------------------------------------------------------------------

/// Sets of up to `max_size` candidates (each `(id, amount)`) whose
/// amounts sum to `target`, smallest sets first, at most `limit` of them.
pub fn subsets_summing_to(
    candidates: &[(usize, Money)],
    target: Money,
    max_size: usize,
    limit: usize,
) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let n = candidates.len();
    if target.is_zero() {
        return out;
    }
    for size in 1..=max_size.min(3) {
        match size {
            1 => {
                for (id, amount) in candidates {
                    if *amount == target {
                        out.push(vec![*id]);
                    }
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
            2 => {
                for a in 0..n {
                    for b in a + 1..n {
                        if candidates[a].1.clone() + candidates[b].1.clone() == target {
                            out.push(vec![candidates[a].0, candidates[b].0]);
                            if out.len() >= limit {
                                return out;
                            }
                        }
                    }
                }
            }
            _ => {
                for a in 0..n {
                    for b in a + 1..n {
                        let ab = candidates[a].1.clone() + candidates[b].1.clone();
                        for c in b + 1..n {
                            if ab.clone() + candidates[c].1.clone() == target {
                                out.push(vec![candidates[a].0, candidates[b].0, candidates[c].0]);
                                if out.len() >= limit {
                                    return out;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::funds::DEFAULT_RESERVE_TEMPLATE;
    use crate::kinds::default_kind_rules;
    use crate::{Amount, Posting, PostingKind, Status};
    use std::collections::HashMap;

    fn usd(m: i128) -> Money {
        Money::from_decimal(m, 2)
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn tx(d: &str, description: &str, postings: &[(&str, i128)]) -> Transaction {
        Transaction {
            date: date(d),
            status: Status::Unmarked,
            code: String::new(),
            description: description.into(),
            comment: String::new(),
            tags: vec![("id".into(), format!("{d}-{description}"))],
            postings: postings
                .iter()
                .map(|(account, cents)| Posting {
                    account: AccountName::new(*account),
                    amount: Amount::new("USD", usd(*cents)),
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

    fn chart(txs: &[Transaction]) -> Chart {
        Chart::from_books(
            default_kind_rules(),
            DEFAULT_RESERVE_TEMPLATE.to_string(),
            Vec::new(),
            txs,
            &HashMap::new(),
        )
    }

    const CHECKING: &str = "assets:bank:checking";
    const LOAN: &str = "liabilities:loans:car";
    const JORDAN: &str = "assets:receivable:jordan";

    /// A month of synthetic books: pay, groceries, a loan payment with a
    /// co-owner's share booked to a receivable, the
    /// share repaid, an interest accrual, a card purchase with its reserve
    /// lines, and a fund allocation.
    fn september() -> Vec<Transaction> {
        vec![
            tx(
                "2026-08-31",
                "Opening",
                &[(CHECKING, 500000), ("equity:opening", -500000)],
            ),
            tx(
                "2026-09-01",
                "Pay",
                &[(CHECKING, 300000), ("income:salary", -300000)],
            ),
            tx(
                "2026-09-03",
                "Groceries",
                &[("expenses:food:groceries", 12345), (CHECKING, -12345)],
            ),
            tx(
                "2026-09-05",
                "Loan payment",
                &[(LOAN, 38000), (CHECKING, -38000)],
            ),
            tx(
                "2026-09-05",
                "Co-owner's share",
                &[(JORDAN, 19000), (LOAN, -19000)],
            ),
            tx(
                "2026-09-20",
                "Jordan repays",
                &[(CHECKING, 19000), (JORDAN, -19000)],
            ),
            tx(
                "2026-09-30",
                "Interest accrual",
                &[("expenses:interest:car", 1500), (LOAN, -1500)],
            ),
            tx(
                "2026-09-10",
                "Card groceries",
                &[
                    ("expenses:food:groceries", 4000),
                    ("liabilities:credit-card:visa", -4000),
                    (CHECKING, -4000),
                    ("assets:bank:checking:card-visa", 4000),
                ],
            ),
            tx(
                "2026-09-11",
                "Allocate",
                &[
                    ("assets:bank:checking:insurance", 10000),
                    (CHECKING, -10000),
                ],
            ),
            tx(
                "2026-10-02",
                "Pay",
                &[(CHECKING, 300000), ("income:salary", -300000)],
            ),
        ]
    }
    #[test]
    fn periods_are_calendar_aligned_and_clipped() {
        let p = periods(date("2025-10-15"), date("2026-01-10"), Interval::Month);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].start, date("2025-10-15"));
        assert_eq!(p[0].end, date("2025-10-31"));
        assert_eq!(p[3].start, date("2026-01-01"));
        assert_eq!(p[3].end, date("2026-01-10"));
        let q = periods(date("2025-10-01"), date("2026-09-30"), Interval::Quarter);
        assert_eq!(q.len(), 4);
        assert_eq!(q[0].label(Interval::Quarter), "2025-Q4");
        let prev = previous_period(date("2026-09-01"), date("2026-09-30"));
        assert_eq!(prev.end, date("2026-08-31"));
        assert_eq!(prev.start, date("2026-08-02"));
    }

    /// Income positive, expenses positive, the loan's equity conversion its
    /// own section: the payment less the co-owner's share less the accrued
    /// interest, and net = income - expenses - conversion.
    #[test]
    fn income_statement_with_equity_conversion() {
        let txs = september();
        let chart = chart(&txs);
        let periods = vec![Period {
            start: date("2026-09-01"),
            end: date("2026-09-30"),
        }];
        let report = income_statement(
            &chart,
            &txs,
            "USD",
            &periods,
            &IncomeStatementOptions::default(),
        );
        assert_eq!(report.income_totals, vec![usd(300000)]);
        assert_eq!(report.income[0].account.as_str(), "income:salary");
        assert_eq!(report.expense_totals, vec![usd(12345 + 4000 + 1500)]);
        // Depth 2: groceries roll up into expenses:food.
        assert!(report
            .expenses
            .iter()
            .any(|r| r.account.as_str() == "expenses:food" && r.total == usd(16345)));
        let conversion = report.conversion.as_ref().unwrap();
        assert_eq!(conversion.len(), 1);
        assert_eq!(conversion[0].total, usd(38000 - 19000 - 1500));
        assert_eq!(conversion[0].cash, vec![usd(38000)]);
        assert_eq!(conversion[0].non_cash, vec![usd(-19000 - 1500)]);
        let total = report.conversion_totals.as_ref().unwrap()[0].clone();
        assert_eq!(
            report.net_before_conversion[0].clone() - total,
            report.net[0]
        );

        // Off: the same figures as without equity conversion.
        let plain = income_statement(
            &chart,
            &txs,
            "USD",
            &periods,
            &IncomeStatementOptions {
                equity_conversion: false,
                ..Default::default()
            },
        );
        assert!(plain.conversion.is_none());
        assert_eq!(plain.net, plain.net_before_conversion);
        assert_eq!(plain.expense_totals, report.expense_totals);
    }

    /// A 380.00 payment with a 190.00 share
    /// booked to a receivable leaves 190.00 of conversion and no expense.
    /// With non-liquid counted, money moved into investments adds to it.
    #[test]
    fn a_shared_payment_converts_only_the_own_share() {
        let txs = vec![
            tx(
                "2026-09-05",
                "Payment",
                &[(LOAN, 38000), (CHECKING, -38000)],
            ),
            tx("2026-09-05", "Share", &[(JORDAN, 19000), (LOAN, -19000)]),
            tx(
                "2026-09-06",
                "Invest",
                &[("assets:investments:brokerage", 10000), (CHECKING, -10000)],
            ),
        ];
        let chart = chart(&txs);
        let periods = vec![Period {
            start: date("2026-09-01"),
            end: date("2026-09-30"),
        }];
        let report = income_statement(
            &chart,
            &txs,
            "USD",
            &periods,
            &IncomeStatementOptions::default(),
        );
        assert_eq!(report.conversion_totals.unwrap(), vec![usd(19000)]);
        assert!(report.expenses.is_empty());
        let with_investments = income_statement(
            &chart,
            &txs,
            "USD",
            &periods,
            &IncomeStatementOptions {
                conversion_kinds: vec![AccountKind::TermLiability, AccountKind::NonLiquid],
                ..Default::default()
            },
        );
        assert_eq!(
            with_investments.conversion_totals.unwrap(),
            vec![usd(19000 + 10000)]
        );
    }
    #[test]
    fn balance_sheet_net_worth_is_assets_plus_liabilities() {
        let txs = september();
        let chart = chart(&txs);
        let sheet = balance_sheet(
            &chart,
            &txs,
            "USD",
            Some(date("2026-09-30")),
            2,
            GroupBy::Kind,
        );
        assert_eq!(
            sheet.net_worth,
            sheet.total_assets.clone() + sheet.total_liabilities.clone()
        );
        let loan = 38000 - 19000 - 1500;
        let card = -4000;
        assert_eq!(sheet.total_liabilities, usd(loan + card));
        assert!(sheet.liquid.is_positive());
        let by_account = balance_sheet(
            &chart,
            &txs,
            "USD",
            Some(date("2026-09-30")),
            2,
            GroupBy::Account,
        );
        assert_eq!(by_account.net_worth, sheet.net_worth);
        assert!(by_account.assets.iter().any(|r| r.group == "assets:bank"));
    }
    #[test]
    fn category_breakdown_ranks_and_compares() {
        let mut txs = september();
        txs.push(tx(
            "2026-08-15",
            "August groceries",
            &[("expenses:food:groceries", 10000), (CHECKING, -10000)],
        ));
        let chart = chart(&txs);
        let report = category_breakdown(
            &chart,
            &txs,
            "USD",
            Period {
                start: date("2026-09-01"),
                end: date("2026-09-30"),
            },
            2,
            1,
            true,
        );
        assert_eq!(report.total, usd(17845));
        assert_eq!(report.rows.len(), 1);
        let food = &report.rows[0];
        assert_eq!(food.account.as_str(), "expenses:food");
        assert_eq!(food.amount, usd(16345));
        assert_eq!(food.share.to_string(), "91.59");
        assert_eq!(food.previous, Some(usd(10000)));
        assert_eq!(food.change, Some(usd(6345)));
        assert_eq!(report.other, usd(1500));
    }

    /// The last point of the history is Cash on Hand on that day.
    #[test]
    fn cash_on_hand_history_ends_at_cash_on_hand() {
        let txs = september();
        let chart = chart(&txs);
        let points = cash_on_hand_history(
            &chart,
            &txs,
            "USD",
            date("2026-07-01"),
            date("2026-09-30"),
            Interval::Month,
        );
        assert_eq!(points.len(), 3);
        let last = points.last().unwrap();
        let report = crate::funds::cash_on_hand(&chart, &txs, "USD", Some(date("2026-09-30")));
        assert_eq!(last.cash_on_hand, report.cash_on_hand.money);
        assert_eq!(last.money_on_hand, report.money_on_hand.money);
        assert_eq!(points[0].cash_on_hand, usd(0));
        let daily = cash_on_hand_history(
            &chart,
            &txs,
            "USD",
            date("2025-01-01"),
            date("2026-09-30"),
            Interval::None,
        );
        assert_eq!(daily.len(), MAX_HISTORY_POINTS);
    }

    /// The bridge's invariants: cash sums to the change in Cash on Hand,
    /// non-cash to zero; accruals and shares only in non-cash; card and
    /// reserve postings cancel out; exclusions come out of
    /// change_excluding only.
    #[test]
    fn cash_flow_bridges_cash_on_hand() {
        let txs = september();
        let chart = chart(&txs);
        let options = CashFlowOptions {
            group_by: GroupBy::Kind,
            depth: 2,
            exclude: Exclude::default(),
        };
        let flow = cash_flow(
            &chart,
            &txs,
            "USD",
            date("2026-09-01"),
            date("2026-09-30"),
            &options,
        );
        let opening = cash_totals(&chart, &txs, "USD", Some(date("2026-08-31")));
        let closing = cash_totals(&chart, &txs, "USD", Some(date("2026-09-30")));
        assert_eq!(flow.opening_cash, opening.cash_on_hand);
        assert_eq!(flow.change, closing.cash_on_hand - opening.cash_on_hand);
        let cash: Money = flow.lines.iter().map(|l| l.cash.clone()).sum();
        let non_cash: Money = flow.lines.iter().map(|l| l.non_cash.clone()).sum();
        let total: Money = flow.lines.iter().map(|l| l.total.clone()).sum();
        assert_eq!(cash, flow.change);
        assert!(non_cash.is_zero());
        assert_eq!(total, flow.change);
        assert!(!flow
            .lines
            .iter()
            .any(|l| l.group == "revolving-debt" || l.group == "debt-reserve"));
        let line = |g: &str| flow.lines.iter().find(|l| l.group == g).unwrap();
        assert_eq!(line("income").cash, usd(300000));
        assert_eq!(line("expense").cash, usd(-12345 - 4000));
        assert_eq!(line("expense").non_cash, usd(-1500));
        assert_eq!(line(EQUITY_CONVERSION_GROUP).cash, usd(-38000));
        assert_eq!(line(EQUITY_CONVERSION_GROUP).non_cash, usd(19000 + 1500));
        assert_eq!(line("receivable").cash, usd(19000));
        assert_eq!(line("pre-allocation").cash, usd(-10000));

        let excluding = cash_flow(
            &chart,
            &txs,
            "USD",
            date("2026-09-01"),
            date("2026-09-30"),
            &CashFlowOptions {
                exclude: Exclude::parse(&["receivable".to_string()]),
                ..options
            },
        );
        assert_eq!(excluding.change, flow.change);
        assert_eq!(excluding.change_excluding, flow.change - usd(19000));
    }

    /// An overdrawn fund's overdraft comes off Cash on Hand, so the bridge
    /// shows it in the cash column: spending a fund past zero, and voiding
    /// a set-aside the fund had already spent.
    #[test]
    fn cash_flow_bridges_overdrawn_funds() {
        const EMERGENCY: &str = "assets:bank:checking:emergency";
        let txs = vec![
            tx(
                "2026-01-01",
                "Opening",
                &[(CHECKING, 321846), ("equity:opening", -321846)],
            ),
            tx(
                "2026-01-15",
                "Allocate",
                &[(EMERGENCY, 25000), (CHECKING, -25000)],
            ),
            tx(
                "2026-02-17",
                "Northside Dental Group",
                &[("expenses:dental", 18600), (EMERGENCY, -18600)],
            ),
            tx(
                "2026-02-20",
                "Allocate | reversal",
                &[(EMERGENCY, -25000), (CHECKING, 25000)],
            ),
            tx(
                "2026-02-24",
                "Northside Dental Group",
                &[("expenses:dental", 4217), (EMERGENCY, -4217)],
            ),
        ];
        let chart = chart(&txs);
        let options = CashFlowOptions {
            group_by: GroupBy::Kind,
            depth: 2,
            exclude: Exclude::default(),
        };
        for (from, to) in [("2026-01-01", "2026-03-31"), ("2026-02-21", "2026-02-28")] {
            let flow = cash_flow(&chart, &txs, "USD", date(from), date(to), &options);
            let cash: Money = flow.lines.iter().map(|l| l.cash.clone()).sum();
            let non_cash: Money = flow.lines.iter().map(|l| l.non_cash.clone()).sum();
            let line_tx_cash: Money = flow
                .lines
                .iter()
                .flat_map(|l| &l.transactions)
                .map(|t| t.cash.clone())
                .sum();
            assert_eq!(cash, flow.change, "{from}..{to}");
            assert_eq!(line_tx_cash, flow.change, "{from}..{to}");
            assert!(non_cash.is_zero(), "{from}..{to}");
        }
        let late = cash_flow(
            &chart,
            &txs,
            "USD",
            date("2026-02-21"),
            date("2026-02-28"),
            &options,
        );
        assert_eq!(late.change, usd(-4217));
    }

    /// A card payment from a reserve that has nothing in it comes off Cash
    /// on Hand, so the bridge shows the shortfall in the cash column on the
    /// card's line.
    #[test]
    fn cash_flow_bridges_negative_card_reserves() {
        const AMEX: &str = "liabilities:credit-card:macys-amex";
        const AMEX_RESERVE: &str = "assets:bank:checking:card-macys-amex";
        let txs = vec![
            tx(
                "2026-05-01",
                "Opening",
                &[(CHECKING, 40578), ("equity:opening", -40578)],
            ),
            tx(
                "2026-05-22",
                "Payment to Macy's AMEX",
                &[(AMEX, 1883), (AMEX_RESERVE, -1883)],
            ),
            tx(
                "2026-09-09",
                "Payment to Macy's AMEX",
                &[(AMEX, 2549), (AMEX_RESERVE, -2549)],
            ),
        ];
        let chart = chart(&txs);
        let options = CashFlowOptions {
            group_by: GroupBy::Account,
            depth: 3,
            exclude: Exclude::default(),
        };
        let flow = cash_flow(
            &chart,
            &txs,
            "USD",
            date("2026-05-02"),
            date("2026-09-30"),
            &options,
        );
        assert_eq!(flow.change, usd(-4432));
        let cash: Money = flow.lines.iter().map(|l| l.cash.clone()).sum();
        assert_eq!(cash, flow.change);
        let card = flow
            .lines
            .iter()
            .find(|l| l.group == AMEX)
            .expect("the card's line");
        assert_eq!(card.cash, usd(-4432));
    }
    #[test]
    fn receivables_activity_balances() {
        let mut txs = september();
        txs.push(tx(
            "2026-09-12",
            "Dinner, Jordan's half",
            &[
                ("expenses:food:dining", 3000),
                (JORDAN, 3000),
                (CHECKING, -6000),
            ],
        ));
        txs.push(tx(
            "2026-09-13",
            "Netted",
            &[(JORDAN, 2000), ("expenses:food:dining", -2000)],
        ));
        let chart = chart(&txs);
        let activity = receivables_activity(
            &chart,
            &txs,
            "USD",
            date("2026-09-01"),
            date("2026-09-30"),
            None,
        );
        let jordan = &activity.counterparties[0];
        let a = &jordan.activity;
        assert_eq!(
            a.opening.clone() + a.filed.clone() - a.collected.clone(),
            a.closing
        );
        assert_eq!(a.filed, usd(19000 + 3000 + 2000));
        assert_eq!(a.collected, usd(19000));
        let netted: Vec<&CounterShare> = jordan
            .filed_by_counter_account
            .iter()
            .filter(|s| s.netted_against_expense)
            .collect();
        assert_eq!(netted.len(), 1);
        assert_eq!(netted[0].amount, usd(2000));
        assert_eq!(activity.total.closing, a.closing);
    }
    #[test]
    fn subsets_sum_to_the_target() {
        let c = vec![
            (0, usd(1000)),
            (1, usd(9744)),
            (2, usd(500)),
            (3, usd(8744)),
        ];
        let sets = subsets_summing_to(&c, usd(9744), 3, 10);
        assert_eq!(sets[0], vec![1]);
        assert!(sets.contains(&vec![0, 3]));
    }
}
