// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! What each screen shows: Overview, accounts by kind, the register,
//! funds, cards, people, reports, reconcile and activity, as plain async
//! Rust over [`KabacusClient`]. The numbers are the server's; this module
//! only arranges them, so the app and MCP never disagree.

use crate::model::ModelError;
use crate::{i18n, i18nc, i18np};
use chrono::{Datelike, Months, NaiveDate};
use kabacus_client::types::{
    AccountView, AmountView, CashOnHandView, PostingView, TransactionView,
};
use kabacus_client::KabacusClient;
use kabacus_core::Money;
use num_traits::ToPrimitive;
use serde::Serialize;

/// An amount as shown: the server's formatted text (with a real minus
/// sign) and its sign, so the UI can colour it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Shown {
    pub text: String,
    pub exact: String,
    /// `neg`, `pos` or `zero`.
    pub tone: &'static str,
}

/// `s` with its sign flipped and no sign colour (income and equity).
fn earned(s: Shown) -> Shown {
    let flip = |t: &str, minus: &str| match t.contains(minus) {
        true => t.replacen(minus, "", 1),
        false if t.is_empty() || s.tone == "zero" => t.to_string(),
        false => format!("{minus}{t}"),
    };
    Shown {
        text: flip(&s.text, "−"),
        exact: flip(&s.exact, "-"),
        tone: "zero",
    }
}

/// The books' commodity, for amounts the server sends as bare decimals
/// (reports); empty if it can't say.
async fn commodity(client: &KabacusClient) -> String {
    client
        .commodity()
        .await
        .map(|c| c.symbol)
        .unwrap_or_default()
}

impl Shown {
    /// A bare decimal in `commodity`, formatted like every other amount
    /// (so reports never show `8,747.82` without its `$`).
    pub fn amount(exact: &str, commodity: &str) -> Shown {
        if commodity.is_empty() {
            return Shown::from_exact(exact, None);
        }
        let money = parse_money(exact).unwrap_or_else(Money::zero);
        let text = kabacus_core::format_money(&kabacus_core::Amount::new(commodity, money));
        Shown::from_exact(exact, Some(text.trim()))
    }

    pub fn of(a: &AmountView) -> Shown {
        Shown::from_exact(&a.exact, Some(&a.formatted))
    }

    /// From an exact decimal string, formatted here when the server sent
    /// none (reports carry bare decimals).
    pub fn from_exact(exact: &str, formatted: Option<&str>) -> Shown {
        let money = parse_money(exact).unwrap_or_else(Money::zero);
        let text = match formatted {
            Some(f) => f.to_string(),
            // No commodity: `- 7.65` would keep the space after the sign.
            None => kabacus_core::format_money(&kabacus_core::Amount::new("", money.clone()))
                .trim()
                .replacen("- ", "-", 1),
        };
        Shown {
            text: text.replacen('-', "−", 1),
            exact: exact.to_string(),
            tone: tone(money),
        }
    }

    pub fn money(&self) -> Money {
        parse_money(&self.exact).unwrap_or_else(Money::zero)
    }
}

fn tone(m: Money) -> &'static str {
    if m.is_negative() {
        "neg"
    } else if m.is_positive() {
        "pos"
    } else {
        "zero"
    }
}

/// Parse an exact decimal string (`"-45.60"`) without going through a
/// float.
pub fn parse_money(exact: &str) -> Option<Money> {
    AmountView {
        commodity: String::new(),
        exact: exact.trim().to_string(),
        formatted: String::new(),
    }
    .money()
}

fn first(amounts: &[AmountView]) -> Shown {
    amounts.first().map(Shown::of).unwrap_or(Shown {
        text: "0.00".to_string(),
        exact: "0".to_string(),
        tone: "zero",
    })
}

/// A share of a whole, for drawing bars only (never shown as money).
fn share(part: Money, whole: Money) -> f64 {
    let whole = whole.rescaled(4).mantissa().to_f64().unwrap_or(f64::MAX);
    if whole == 0.0 {
        return 0.0;
    }
    (part.rescaled(4).mantissa().to_f64().unwrap_or(f64::MAX) / whole).clamp(-1.0, 1.0)
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

// --- Overview ----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub cash_on_hand: Shown,
    pub money_on_hand: Shown,
    pub future_allocations: Shown,
    pub receivables: Shown,
    /// The change since `compare_to`, a month ago.
    pub change: Option<Shown>,
    pub compare_to: String,
    pub homes: Vec<HomeRow>,
    pub funds_total: Shown,
    pub funds: Vec<NamedAmount>,
    pub cards: Vec<CardRow>,
    pub warnings: Vec<WarningRow>,
    /// Cash on Hand at each month's end, oldest first.
    pub history: Vec<Point>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomeRow {
    pub account: String,
    pub name: String,
    pub free: Shown,
    pub funds: Shown,
    pub reserves: Shown,
    pub inclusive: Shown,
    /// Shares of the bank balance, for the split bar.
    pub free_share: f64,
    pub funds_share: f64,
    pub reserves_share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedAmount {
    pub account: String,
    /// The bank account a reserve (a fund) is kept in: setting money
    /// aside in it or releasing it moves money from or to there.
    pub home: String,
    pub name: String,
    pub amount: Shown,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardRow {
    pub card: String,
    pub name: String,
    pub owed: Shown,
    pub reserve: Shown,
    /// What the reserve still needs; zero when covered.
    pub short: Shown,
    pub covered: bool,
    pub unreserved_count: usize,
}

/// One cash_on_hand warning and the screen that fixes it.
#[derive(Debug, Clone, Serialize)]
pub struct WarningRow {
    pub code: String,
    pub message: String,
    /// The one action's label.
    pub action: String,
    /// Where it goes: `card`, `register`, `funds`, `settings` or `none`.
    pub target: String,
    /// The account it concerns, if any.
    pub account: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Point {
    pub date: String,
    pub value: Shown,
    /// 0..1 between the lowest and highest point, for drawing.
    pub y: f64,
}

/// What Overview shows. Warnings come from the server and each
/// gets the one action that fixes it.
pub async fn overview(client: &KabacusClient) -> Result<Overview, ModelError> {
    let compare_to = today()
        .checked_sub_months(Months::new(1))
        .unwrap_or(today());
    let since = ymd(compare_to);
    let (coh, history) = tokio::join!(
        client.cash_on_hand(Some(&since), "summary"),
        cash_history(client, 6)
    );
    let (coh, history) = (coh?, history.unwrap_or_default());
    Ok(overview_from(coh, since, history))
}

fn overview_from(coh: CashOnHandView, compare_to: String, history: Vec<Point>) -> Overview {
    let funds_total = coh
        .by_kind
        .iter()
        .find(|g| g.kind == "pre-allocation")
        .map(|g| Shown::of(&g.total))
        .unwrap_or_else(|| Shown::from_exact("0", None));
    Overview {
        cash_on_hand: Shown::of(&coh.cash_on_hand),
        money_on_hand: Shown::of(&coh.money_on_hand),
        future_allocations: Shown::of(&coh.future_allocations),
        receivables: Shown::of(&coh.receivables),
        change: coh.change.as_ref().map(|c| Shown::of(&c.cash_on_hand)),
        compare_to,
        // Only what holds money: a bank account with nothing free or an
        // empty fund is noise on the Overview. The Funds page lists them all.
        homes: coh
            .homes
            .iter()
            .filter(|h| !Shown::of(&h.free).money().is_zero())
            .map(home_row)
            .collect(),
        funds_total,
        funds: coh
            .funds
            .iter()
            .filter(|f| !Shown::of(&f.balance).money().is_zero())
            .map(|f| NamedAmount {
                account: f.account.clone(),
                home: f.home.clone(),
                name: f.display_name.clone(),
                amount: Shown::of(&f.balance),
            })
            .collect(),
        cards: coh.cards.iter().map(card_row).collect(),
        warnings: coh.warnings.iter().map(warning_row).collect(),
        history,
    }
}

fn home_row(h: &kabacus_client::types::HomeView) -> HomeRow {
    let inclusive = Shown::of(&h.inclusive).money();
    HomeRow {
        account: h.account.clone(),
        name: h.display_name.clone(),
        free: Shown::of(&h.free),
        funds: Shown::of(&h.funds),
        reserves: Shown::of(&h.reserves),
        inclusive: Shown::of(&h.inclusive),
        free_share: share(Shown::of(&h.free).money(), inclusive.clone()).max(0.0),
        funds_share: share(Shown::of(&h.funds).money(), inclusive.clone()).max(0.0),
        reserves_share: share(Shown::of(&h.reserves).money(), inclusive).max(0.0),
    }
}

fn card_row(c: &kabacus_client::types::CardView) -> CardRow {
    let short = Shown::of(&c.difference);
    CardRow {
        card: c.card.clone(),
        name: c.display_name.clone(),
        owed: Shown::of(&c.owed),
        reserve: Shown::of(&c.reserve_balance),
        covered: short.money().is_zero(),
        short,
        unreserved_count: c.unreserved_count,
    }
}

fn warning_row(w: &kabacus_client::types::WarningView) -> WarningRow {
    let account = w.accounts.first().cloned().unwrap_or_default();
    let (action, target) = match w.code.as_str() {
        "reserve-mismatch" | "unreserved-unaccounted" => {
            (i18nc!("@action fix a card", "Fix"), "card")
        }
        "uncategorized" => (i18n!("Categorize"), "register"),
        "negative-fund" | "negative-free-money" => (i18n!("Move money"), "funds"),
        "unclassified" | "exclusive-assertion" | "duplicate-display-name" => {
            (i18n!("Details"), "settings")
        }
        _ => (String::new(), "none"),
    };
    WarningRow {
        code: w.code.clone(),
        // The server's words: it doesn't translate them (yet).
        message: w.message.clone(),
        action,
        target: target.to_string(),
        account,
    }
}

/// Cash on Hand at the end of each of the last `months` months.
pub async fn cash_history(client: &KabacusClient, months: u32) -> Result<Vec<Point>, ModelError> {
    let to = today();
    let from = NaiveDate::from_ymd_opt(to.year(), to.month(), 1)
        .and_then(|d| d.checked_sub_months(Months::new(months)))
        .unwrap_or(to);
    let report = client
        .report(
            "cash-on-hand-history",
            &[
                ("from", &ymd(from)),
                ("to", &ymd(to)),
                ("interval", "month"),
            ],
        )
        .await?;
    let points: Vec<(String, Shown)> = report["points"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|p| {
            (
                p["date"].as_str().unwrap_or_default().to_string(),
                Shown::from_exact(p["cash_on_hand"].as_str().unwrap_or("0"), None),
            )
        })
        .collect();
    let values: Vec<f64> = points
        .iter()
        .map(|(_, v)| {
            v.money()
                .rescaled(4)
                .mantissa()
                .to_f64()
                .unwrap_or(f64::MAX)
        })
        .collect();
    let (lo, hi) = (
        values.iter().copied().fold(f64::INFINITY, f64::min),
        values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    Ok(points
        .into_iter()
        .zip(values)
        .map(|((date, value), v)| Point {
            date,
            value,
            y: if hi == lo { 0.5 } else { (v - lo) / (hi - lo) },
        })
        .collect())
}

// --- Accounts by kind -------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AccountGroup {
    pub kind: String,
    pub title: String,
    pub total: Option<Shown>,
    pub rows: Vec<AccountRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountRow {
    pub account: String,
    pub name: String,
    /// A second line: a money account's split, a card's reserve.
    pub detail: String,
    pub balance: Shown,
    /// Something needs attention (a card's reserve is short).
    pub warn: bool,
}

const GROUPS: [&str; 8] = [
    "money",
    "revolving-debt",
    "term-liability",
    "non-liquid",
    "people",
    "expense",
    "income",
    "other",
];

/// A group's heading.
fn group_title(group: &str) -> String {
    match group {
        "money" => i18nc!("accounts group", "Money"),
        "revolving-debt" => i18nc!("accounts group", "Cards"),
        "term-liability" => i18nc!("accounts group", "Loans"),
        "non-liquid" => i18nc!("accounts group", "Investments"),
        "people" => i18nc!("accounts group", "People"),
        "expense" => i18nc!("accounts group", "Spending"),
        "income" => i18nc!("accounts group", "Income"),
        _ => i18nc!("accounts group", "Other"),
    }
}

fn group_of(kind: &str) -> &'static str {
    match kind {
        "money" => "money",
        "revolving-debt" => "revolving-debt",
        "term-liability" => "term-liability",
        "non-liquid" => "non-liquid",
        "receivable" | "payable" => "people",
        "expense" => "expense",
        "income" => "income",
        // Funds and card reserves live under their home (Funds);
        // parents have no postings of their own.
        "pre-allocation" | "debt-reserve" | "parent" => "",
        _ => "other",
    }
}

/// Every account grouped by kind, funds and reserves folded into
/// their home. A money account shows what the bank shows (inclusive) and
/// how it splits.
pub async fn accounts(client: &KabacusClient) -> Result<Vec<AccountGroup>, ModelError> {
    let (accounts, coh) = tokio::join!(
        client.list_accounts(None),
        client.cash_on_hand(None, "summary")
    );
    let (accounts, coh) = (accounts?, coh?);
    Ok(group_accounts(&accounts, &coh))
}

/// The loans (term liabilities), as the Accounts page lists them. A
/// mortgage and an installment loan will get pages of their own; until
/// then each opens its register.
pub async fn loans(client: &KabacusClient) -> Result<Vec<AccountRow>, ModelError> {
    Ok(accounts(client)
        .await?
        .into_iter()
        .find(|g| g.kind == "term-liability")
        .map(|g| g.rows)
        .unwrap_or_default())
}

fn group_accounts(accounts: &[AccountView], coh: &CashOnHandView) -> Vec<AccountGroup> {
    let mut groups: Vec<AccountGroup> = GROUPS
        .iter()
        .map(|kind| AccountGroup {
            kind: kind.to_string(),
            title: group_title(kind),
            total: coh
                .by_kind
                .iter()
                .find(|g| g.kind == *kind && *kind == "money")
                .map(|_| {
                    // A money group's total is what the banks show.
                    let sum = coh.homes.iter().fold(Money::zero(), |acc, h| {
                        acc.checked_add(&Shown::of(&h.inclusive).money())
                            .unwrap_or(acc)
                    });
                    Shown::from_exact(&sum.to_string(), None)
                }),
            rows: vec![],
        })
        .collect();
    for a in accounts {
        // Equity (opening balances) is bookkeeping, not an investment.
        let group = if a.account.starts_with("equity:") {
            "other"
        } else {
            group_of(&a.kind)
        };
        if group.is_empty() {
            continue;
        }
        let name = if a.display_name.is_empty() {
            a.friendly_name.clone().unwrap_or_else(|| a.account.clone())
        } else {
            a.display_name.clone()
        };
        let (balance, detail, warn) = match a.kind.as_str() {
            "money" => {
                let home = coh.homes.iter().find(|h| h.account == a.account);
                let funds = coh.funds.iter().filter(|f| f.home == a.account).count();
                let reserves = coh
                    .cards
                    .iter()
                    .filter(|c| {
                        accounts
                            .iter()
                            .any(|x| x.account == c.reserve && x.home == a.account)
                    })
                    .count();
                let detail = match home {
                    // One sentence for translators, not three joined pieces:
                    // the order and the separator are theirs to choose.
                    Some(h) if funds + reserves > 0 => i18nc!(
                        "%1 is the free money, %2 how many reserves (“2 reserves”), %3 how many card reserves (“1 card reserve”)",
                        "Free %1 · %2 · %3",
                        Shown::of(&h.free).text,
                        i18np!("%1 reserve", "%1 reserves", funds),
                        i18np!("%1 card reserve", "%1 card reserves", reserves)
                    ),
                    _ => String::new(),
                };
                (first(&a.balance_inclusive), detail, false)
            }
            "revolving-debt" => {
                let card = coh.cards.iter().find(|c| c.card == a.account);
                let detail = card
                    .map(|c| {
                        i18nc!(
                            "%1 is the card's reserve",
                            "Reserve %1",
                            Shown::of(&c.reserve_balance).text
                        )
                    })
                    .unwrap_or_default();
                let warn = card.is_some_and(|c| !Shown::of(&c.difference).money().is_zero());
                (first(&a.balance_inclusive), detail, warn)
            }
            "receivable" => (first(&a.balances), i18n!("Owes you"), false),
            "payable" => (first(&a.balances), i18n!("You owe"), false),
            _ => (first(&a.balance_inclusive), String::new(), false),
        };
        // Zero-balance people accounts are noise.
        if group == "people" && balance.tone == "zero" {
            continue;
        }
        // Income and equity balances are negative; shown earned
        // (positive) and neutral, since red reads as "bad".
        let balance = if a.kind == "income" || a.account.starts_with("equity:") {
            earned(balance)
        } else {
            balance
        };
        if let Some(g) = groups.iter_mut().find(|g| g.kind == group) {
            g.rows.push(AccountRow {
                account: a.account.clone(),
                name,
                detail,
                balance,
                warn,
            });
        }
    }
    groups.retain(|g| !g.rows.is_empty());
    groups
}

// --- Register -----------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RegisterRow {
    pub status: String,
    pub id: String,
    pub date: String,
    pub payee: String,
    /// The other side.
    pub account: String,
    pub memo: String,
    pub amount: Shown,
    /// The running balance, coloured by sign.
    pub balance: Shown,
    /// `""` or `move` (between a fund and its
    /// home: the bank balance doesn't change).
    pub kind: String,
    /// A card transaction missing its reserve lines.
    pub unreserved: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RegisterFilter {
    /// Matched against payee, memo and the other account.
    pub text: String,
    pub unreserved_only: bool,
    /// Include money moved between a fund or card reserve and its bank
    /// account; off, the register lists real transactions only.
    pub fund_transfers: bool,
}

/// The register of every real account together (no running balance).
pub const ALL_ACCOUNTS: &str = "";

/// Whether a transaction only shuffles money between funds, card
/// reserves and the bank accounts holding them.
fn is_fund_transfer(tx: &TransactionView, accounts: &[AccountView]) -> bool {
    let kind_of = |name: &str| accounts.iter().find(|a| a.account == name);
    let is_fund = |a: &AccountView| matches!(a.kind.as_str(), "pre-allocation" | "debt-reserve");
    let mut any_fund = false;
    for p in &tx.postings {
        let Some(a) = kind_of(&p.account) else {
            return false;
        };
        if is_fund(a) {
            any_fund = true;
        } else if !accounts.iter().any(|f| is_fund(f) && f.home == a.account) {
            return false;
        }
    }
    any_fund
}

/// Every real transaction, newest first: what happened to the
/// money in bank, card, loan and investment accounts, whichever of them it
/// touched.
async fn all_register(
    client: &KabacusClient,
    filter: &RegisterFilter,
) -> Result<Vec<RegisterRow>, ModelError> {
    let (txs, accounts) = tokio::join!(client.list_transactions(), client.list_accounts(None));
    let (txs, accounts) = (txs?, accounts?);
    let name = |account: &str| {
        accounts
            .iter()
            .find(|a| a.account == account)
            .map(|a| a.display_name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| account.to_string())
    };
    let holds_money = |account: &str| {
        accounts.iter().any(|a| {
            a.account == account
                && matches!(
                    a.kind.as_str(),
                    "money" | "revolving-debt" | "term-liability" | "non-liquid"
                )
        })
    };
    let needle = filter.text.trim().to_lowercase();
    let mut rows: Vec<RegisterRow> = txs
        .iter()
        .filter(|t| !filter.unreserved_only || t.unreserved)
        .filter(|t| filter.fund_transfers || !is_fund_transfer(t, &accounts))
        .map(|t| {
            let real: Vec<&PostingView> = t
                .postings
                .iter()
                .filter(|p| holds_money(&p.account))
                .collect();
            let net = real
                .iter()
                .filter_map(|p| p.amount.money())
                .fold(Money::zero(), |a, m| a.checked_add(&m).unwrap_or(a));
            let shown = if net.is_zero() {
                // Between two real accounts: what moved.
                real.iter()
                    .filter(|p| !p.amount.exact.starts_with('-'))
                    .map(|p| Shown::of(&p.amount))
                    .next()
            } else {
                real.iter()
                    .find(|p| p.amount.exact.starts_with('-') == net.is_negative())
                    .map(|p| Shown::of(&p.amount))
            };
            let amount = shown.unwrap_or_else(|| {
                t.postings
                    .iter()
                    .map(|p| Shown::of(&p.amount))
                    .find(|s| s.tone == "pos")
                    .unwrap_or_else(|| Shown::from_exact("0", None))
            });
            let account = t
                .postings
                .iter()
                .map(|p| name(&p.account))
                .collect::<Vec<_>>()
                .join(&crate::model::list_separator());
            RegisterRow {
                status: t.status.clone(),
                id: t.id.to_string(),
                date: t.date.clone(),
                payee: t.payee.clone(),
                account,
                memo: t.memo.clone(),
                amount,
                balance: Shown::from_exact("0", Some("")),
                kind: if is_fund_transfer(t, &accounts) {
                    "move".to_string()
                } else {
                    String::new()
                },
                unreserved: t.unreserved,
            }
        })
        .filter(|r| {
            needle.is_empty()
                || r.payee.to_lowercase().contains(&needle)
                || r.memo.to_lowercase().contains(&needle)
                || r.account.to_lowercase().contains(&needle)
        })
        .collect();
    rows.sort_by(|a, b| (&b.date, b.id.len(), &b.id).cmp(&(&a.date, a.id.len(), &a.id)));
    Ok(rows)
}

/// An account's register, newest first, filtered; `ALL_ACCOUNTS` is every
/// real account's together.
pub async fn register(
    client: &KabacusClient,
    account: &str,
    filter: &RegisterFilter,
) -> Result<Vec<RegisterRow>, ModelError> {
    if account == ALL_ACCOUNTS {
        return all_register(client, filter).await;
    }
    let (lines, accounts) = tokio::join!(
        client.get_register(account, None, None),
        client.list_accounts(None)
    );
    let (lines, accounts) = (lines?, accounts?);
    let needle = filter.text.trim().to_lowercase();
    let mut rows: Vec<RegisterRow> = lines
        .into_iter()
        .map(|l| {
            let other = accounts.iter().find(|a| a.account == l.other_account);
            let is_move = other.is_some_and(|o| {
                matches!(o.kind.as_str(), "pre-allocation" | "debt-reserve")
                    && (o.home == account || l.other_account.starts_with(&format!("{account}:")))
            });
            let id = l.id.to_string();
            let kind = if is_move { "move" } else { "" };
            RegisterRow {
                account: other
                    .map(|o| o.display_name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or(l.other_account),
                id,
                date: l.date,
                payee: l.payee,
                memo: l.memo,
                status: l.status,
                amount: Shown::of(&l.amount),
                balance: Shown::of(&l.balance),
                kind: kind.to_string(),
                unreserved: l.unreserved,
            }
        })
        .filter(|r| !filter.unreserved_only || r.unreserved)
        .filter(|r| filter.fund_transfers || r.kind != "move")
        .filter(|r| {
            needle.is_empty()
                || r.payee.to_lowercase().contains(&needle)
                || r.memo.to_lowercase().contains(&needle)
                || r.account.to_lowercase().contains(&needle)
        })
        .collect();
    rows.reverse();
    Ok(rows)
}

// --- Funds ------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct FundsHome {
    pub home: HomeRow,
    pub funds: Vec<NamedAmount>,
    pub reserves: Vec<ReserveRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReserveRow {
    pub account: String,
    pub card: String,
    pub name: String,
    pub balance: Shown,
    pub short: Shown,
    pub covered: bool,
}

/// Each bank account with its funds and card reserves.
pub async fn funds(client: &KabacusClient) -> Result<Vec<FundsHome>, ModelError> {
    let (coh, accounts) = tokio::join!(
        client.cash_on_hand(None, "summary"),
        client.list_accounts(None)
    );
    let (coh, accounts) = (coh?, accounts?);
    Ok(coh
        .homes
        .iter()
        .map(|h| FundsHome {
            home: home_row(h),
            funds: coh
                .funds
                .iter()
                .filter(|f| f.home == h.account)
                .map(|f| NamedAmount {
                    account: f.account.clone(),
                    home: f.home.clone(),
                    name: f.display_name.clone(),
                    amount: Shown::of(&f.balance),
                })
                .collect(),
            reserves: coh
                .cards
                .iter()
                .filter(|c| {
                    accounts
                        .iter()
                        .any(|a| a.account == c.reserve && a.home == h.account)
                })
                .map(|c| {
                    let short = Shown::of(&c.difference);
                    ReserveRow {
                        account: c.reserve.clone(),
                        card: c.card.clone(),
                        name: c.display_name.clone(),
                        balance: Shown::of(&c.reserve_balance),
                        covered: short.money().is_zero(),
                        short,
                    }
                })
                .collect(),
        })
        .collect())
}

// --- Cards ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CardDetail {
    pub card: CardRow,
    pub reserve_account: String,
    pub sources: Vec<SourceRow>,
    pub unreserved: Vec<UnreservedRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRow {
    pub name: String,
    pub amount: Shown,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnreservedRow {
    pub id: String,
    pub date: String,
    pub description: String,
    pub missing: Shown,
}

/// Every card with what its reserve holds and the
/// transactions missing their reserve lines.
pub async fn cards(client: &KabacusClient) -> Result<Vec<CardDetail>, ModelError> {
    let (reserves, full) = tokio::join!(
        client.card_reserves(None),
        client.cash_on_hand(None, "full")
    );
    let (reserves, full) = (reserves?, full?);
    Ok(reserves
        .cards
        .iter()
        .map(|c| {
            let total = Shown::of(&c.card.reserve_balance).money();
            let listed = full
                .cards
                .iter()
                .find(|f| f.card == c.card.card)
                .and_then(|f| f.unreserved.clone())
                .unwrap_or_default();
            CardDetail {
                card: card_row(&c.card),
                reserve_account: c.card.reserve.clone(),
                sources: c
                    .sources
                    .iter()
                    .map(|s| SourceRow {
                        name: s.name.clone(),
                        amount: Shown::of(&s.amount),
                        share: share(Shown::of(&s.amount).money(), total.clone()).max(0.0),
                    })
                    .collect(),
                unreserved: listed
                    .iter()
                    .map(|u| UnreservedRow {
                        id: u.id.to_string(),
                        date: u.date.clone(),
                        description: u.description.clone(),
                        missing: Shown::of(&u.missing_reserve),
                    })
                    .collect(),
            }
        })
        .collect())
}

// --- People ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Person {
    /// The counterparty, as the server knows it (statements are by this).
    pub name: String,
    /// As Accounts shows it: `alex` is `Alex`.
    pub display: String,
    /// "Owes you", "You owe" or "Settled".
    pub label: String,
    pub net: Shown,
}

/// One net line per person (list_counterparties).
pub async fn people(client: &KabacusClient) -> Result<Vec<Person>, ModelError> {
    Ok(client
        .counterparties()
        .await?
        .into_iter()
        .map(|c| Person {
            display: kabacus_core::kinds::humanize(&c.name),
            name: c.name,
            label: person_label(c.label),
            net: Shown::of(&c.net),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct StatementRow {
    pub date: String,
    pub description: String,
    pub amount: Shown,
    pub running: Shown,
    pub open: bool,
}

/// The server's label for a person, in the user's language.
fn person_label(label: String) -> String {
    match label.as_str() {
        "Owes you" => i18n!("Owes you"),
        "You owe" => i18n!("You owe"),
        "Settled" => i18nc!("nobody owes anything", "Settled"),
        _ => label,
    }
}

/// A person's statement with its running balance, since the start of last
/// year.
pub async fn statement(
    client: &KabacusClient,
    name: &str,
    open_items: bool,
) -> Result<Vec<StatementRow>, ModelError> {
    let to = today();
    let from = NaiveDate::from_ymd_opt(to.year() - 1, 1, 1).unwrap_or(to);
    let s = client
        .statement(name, &ymd(from), &ymd(to), open_items)
        .await?;
    let mut rows: Vec<StatementRow> = s
        .lines
        .iter()
        .map(|l| StatementRow {
            date: l.date.clone(),
            description: l.description.clone(),
            amount: Shown::of(&l.net_amount),
            running: Shown::of(&l.running_balance),
            open: l.open,
        })
        .collect();
    rows.reverse();
    Ok(rows)
}

// --- Reports -------------------------------------------------------------------------

/// A month: its first and last day.
pub fn month_range(year: i32, month: u32) -> (String, String) {
    let start = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(today());
    let end = start
        .checked_add_months(Months::new(1))
        .and_then(|d| d.pred_opt())
        .unwrap_or(start);
    (ymd(start), ymd(end))
}

#[derive(Debug, Clone, Serialize)]
pub struct CashFlow {
    pub from: String,
    pub to: String,
    pub opening: Shown,
    pub closing: Shown,
    pub change: Shown,
    /// Opening, each line, closing: the bridge's bars.
    pub bars: Vec<Bar>,
    pub lines: Vec<FlowLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bar {
    pub name: String,
    pub amount: Shown,
    /// `base`, `up` or `down`.
    pub kind: &'static str,
    /// Where the bar starts and ends, 0..1 of the tallest point.
    pub lo: f64,
    pub hi: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowLine {
    pub name: String,
    pub cash: Shown,
    pub non_cash: Shown,
}

/// Why Cash on Hand changed between `from` and `to`.
pub async fn cash_flow(
    client: &KabacusClient,
    from: &str,
    to: &str,
) -> Result<CashFlow, ModelError> {
    let sym = commodity(client).await;
    let r = client
        .report("cash-flow", &[("from", from), ("to", to)])
        .await?;
    let get = |v: &serde_json::Value| Shown::amount(v.as_str().unwrap_or("0"), &sym);
    let opening = get(&r["opening_cash"]);
    let closing = get(&r["closing_cash"]);
    let lines: Vec<FlowLine> = r["lines"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|l| FlowLine {
            name: flow_name(
                l["group"].as_str().unwrap_or_default(),
                l["display_name"].as_str().unwrap_or_default(),
            ),
            cash: get(&l["cash"]),
            non_cash: get(&l["non_cash"]),
        })
        .collect();
    // The bridge: each line moves the running total from where the last
    // one left it.
    let mut steps = vec![(
        i18nc!("Cash on Hand at the start", "Opening"),
        opening.clone(),
        "base",
    )];
    for l in &lines {
        if l.cash.money().is_zero() {
            continue;
        }
        let kind = if l.cash.money().is_negative() {
            "down"
        } else {
            "up"
        };
        steps.push((l.name.clone(), l.cash.clone(), kind));
    }
    steps.push((
        i18nc!("Cash on Hand at the end", "Closing"),
        closing.clone(),
        "base",
    ));
    let mut run = Money::zero();
    let mut spans = vec![];
    for (_, amount, kind) in &steps {
        let m = amount.money();
        let (lo, hi) = if *kind == "base" {
            run = m.clone();
            (Money::zero().min(m.clone()), Money::zero().max(m))
        } else {
            let next = run.checked_add(&m).unwrap_or_else(|| run.clone());
            let span = (run.clone().min(next.clone()), run.max(next.clone()));
            run = next;
            span
        };
        spans.push((lo, hi));
    }
    let top = spans
        .iter()
        .map(|(_, hi)| hi.clone())
        .max()
        .unwrap_or(Money::zero());
    let bars = steps
        .into_iter()
        .zip(spans)
        .map(|((name, amount, kind), (lo, hi))| Bar {
            name,
            amount,
            kind,
            lo: share(lo, top.clone()).max(0.0),
            hi: share(hi, top.clone()).max(0.0),
        })
        .collect();
    Ok(CashFlow {
        from: from.to_string(),
        to: to.to_string(),
        change: get(&r["change"]),
        opening,
        closing,
        bars,
        lines,
    })
}

/// A cash-flow line's name: kinds in words (grouped by account, the
/// account's own name).
fn flow_name(group: &str, display_name: &str) -> String {
    match group {
        "income" => i18nc!("accounts group", "Income"),
        "expense" => i18nc!("accounts group", "Spending"),
        "pre-allocation" => i18n!("Set aside in reserves"),
        "debt-reserve" => i18n!("Card reserves"),
        "equity_conversion" | "term-liability" => i18n!("Loan principal"),
        "receivable" => i18n!("Fronted for others"),
        "payable" => i18n!("Owed to others"),
        "revolving-debt" => i18nc!("accounts group", "Cards"),
        "non-liquid" => i18nc!("accounts group", "Investments"),
        "money" => i18nc!("accounts group", "Money"),
        _ => display_name.to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IncomeStatement {
    pub income: Shown,
    pub expenses: Shown,
    /// Loan principal paid down (equity conversion), when counted.
    pub conversion: Option<Shown>,
    pub net: Shown,
}

/// Income, expenses and, if `equity_conversion`, loan
/// principal.
pub async fn income_statement(
    client: &KabacusClient,
    from: &str,
    to: &str,
    equity_conversion: bool,
) -> Result<IncomeStatement, ModelError> {
    let sym = commodity(client).await;
    let eqc = if equity_conversion { "true" } else { "false" };
    let r = client
        .report(
            "income-statement",
            &[("from", from), ("to", to), ("equity_conversion", eqc)],
        )
        .await?;
    let total = |v: &serde_json::Value| {
        Shown::amount(
            v.as_array()
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .unwrap_or("0"),
            &sym,
        )
    };
    Ok(IncomeStatement {
        income: total(&r["totals"]["income"]),
        expenses: total(&r["totals"]["expenses"]),
        conversion: r.get("equity_conversion").map(|c| total(&c["totals"])),
        net: total(&r["net"]),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub account: String,
    pub name: String,
    pub amount: Shown,
    /// The change from the period before.
    pub change: Option<Shown>,
    /// Of the largest category, for the bar.
    pub share: f64,
    pub uncategorized: bool,
}

/// Where the money went, with the change from the period
/// before.
pub async fn categories(
    client: &KabacusClient,
    from: &str,
    to: &str,
) -> Result<Vec<Category>, ModelError> {
    let sym = commodity(client).await;
    let r = client
        .report(
            "categories",
            &[("from", from), ("to", to), ("compare_previous", "true")],
        )
        .await?;
    let rows = r["rows"].as_array().cloned().unwrap_or_default();
    let largest = rows
        .iter()
        .map(|c| Shown::amount(c["amount"].as_str().unwrap_or("0"), &sym).money())
        .max()
        .unwrap_or(Money::zero());
    Ok(rows
        .iter()
        .map(|c| {
            let amount = Shown::amount(c["amount"].as_str().unwrap_or("0"), &sym);
            let account = c["account"].as_str().unwrap_or_default().to_string();
            Category {
                share: share(amount.money(), largest.clone()).max(0.0),
                uncategorized: account.ends_with(":uncategorized"),
                name: c["display_name"].as_str().unwrap_or_default().to_string(),
                change: c["change"].as_str().map(|x| Shown::amount(x, &sym)),
                account,
                amount,
            }
        })
        .collect())
}

// --- Reconcile ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ReconcileCheck {
    pub account: String,
    pub date: String,
    pub ledger: Shown,
    pub statement: Shown,
    pub difference: Shown,
    pub matched: bool,
    /// Transactions that add up to the difference: likely explanations.
    pub suggestions: Vec<Vec<Suggested>>,
    /// Ones dated after the statement that would explain it.
    pub later: Vec<Vec<Suggested>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggested {
    pub id: String,
    pub date: String,
    pub description: String,
    pub amount: Shown,
}

fn suggested(sets: &serde_json::Value) -> Vec<Vec<Suggested>> {
    sets.as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|set| {
            set.as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|t| Suggested {
                    id: t["id"].as_str().unwrap_or_default().to_string(),
                    date: t["date"].as_str().unwrap_or_default().to_string(),
                    description: t["description"].as_str().unwrap_or_default().to_string(),
                    amount: Shown::from_exact(t["amount"].as_str().unwrap_or("0"), None),
                })
                .collect()
        })
        .collect()
}

/// Compare `account` with a statement balance on `date`.
/// Writes nothing.
pub async fn reconcile_check(
    client: &KabacusClient,
    account: &str,
    date: &str,
    balance: &str,
) -> Result<ReconcileCheck, ModelError> {
    let sym = commodity(client).await;
    let balance = balance.trim().replace(',', "");
    let r = client
        .reconcile(&kabacus_client::types::ReconcileBody {
            head: None,
            dry_run: true,
            account: account.to_string(),
            balance,
            date: date.to_string(),
            mode: "check".to_string(),
        })
        .await?;
    let get = |k: &str| Shown::amount(r[k].as_str().unwrap_or("0"), &sym);
    Ok(ReconcileCheck {
        account: account.to_string(),
        date: date.to_string(),
        ledger: get("ledger_balance"),
        statement: get("statement_balance"),
        difference: get("difference"),
        matched: r["status"] == "matched",
        suggestions: suggested(&r["suggestions"]),
        later: suggested(&r["later_transactions"]),
    })
}

// --- Pickers and payee memory -------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Pick {
    pub account: String,
    pub name: String,
    pub kind: String,
    pub home: String,
    pub synthetic: bool,
}

/// The forms' pickers: every account, with its kind and home so a form
/// can offer only the accounts that make sense (funds of the same
/// home), and the books' commodity with its decimal places, so an amount
/// is checked as it's typed.
#[derive(Debug, Clone, Serialize)]
pub struct Pickers {
    pub accounts: Vec<Pick>,
    pub commodity: String,
    pub places: u32,
}

pub async fn picker(client: &KabacusClient) -> Result<Pickers, ModelError> {
    let (accounts, commodity) = tokio::try_join!(client.list_accounts(None), client.commodity())?;
    let accounts = accounts
        .into_iter()
        .filter(|a| a.kind != "parent")
        .map(|a| Pick {
            name: if a.display_name.is_empty() {
                a.account.clone()
            } else {
                a.display_name.clone()
            },
            account: a.account,
            kind: a.kind,
            home: a.home,
            synthetic: a.synthetic,
        })
        .collect();
    Ok(Pickers {
        accounts,
        commodity: commodity.symbol,
        places: commodity.places,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct PayeeMemory {
    pub payee: String,
    /// The category it usually goes to (its last transaction's).
    pub category: String,
    pub category_name: String,
    pub last_amount: Shown,
}

/// Known payees matching `query`, most recent first, with the
/// category and amount to pre-fill.
pub async fn payees(client: &KabacusClient, query: &str) -> Result<Vec<PayeeMemory>, ModelError> {
    let q = query.trim();
    let (accounts, payees) = tokio::join!(
        client.list_accounts(None),
        client.payees((!q.is_empty()).then_some(q))
    );
    let accounts = accounts?;
    let kind = |a: &str| {
        accounts
            .iter()
            .find(|x| x.account == a)
            .map(|x| (x.kind.clone(), x.display_name.clone()))
            .unwrap_or_default()
    };
    Ok(payees?
        .into_iter()
        .take(8)
        .filter_map(|p| {
            let posting = p
                .last
                .postings
                .iter()
                .find(|x| matches!(kind(&x.account).0.as_str(), "expense" | "income"))?;
            Some(PayeeMemory {
                category: posting.account.clone(),
                category_name: kind(&posting.account).1,
                last_amount: Shown::of(&AmountView {
                    exact: posting.amount.exact.trim_start_matches('-').to_string(),
                    formatted: posting.amount.formatted.trim_start_matches('-').to_string(),
                    commodity: posting.amount.commodity.clone(),
                }),
                payee: p.payee,
            })
        })
        .collect())
}

// --- Activity ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub date: String,
    pub summary: String,
    /// `mcp`, `app`, `external` or another client.
    pub actor: String,
    pub actor_label: String,
    pub user: String,
    pub agent: Option<String>,
    /// The transaction it wrote, from the subject's `[#…]`, if any.
    pub transaction_id: String,
}

/// The `[#…]` a write's commit subject names.
fn subject_id(summary: &str) -> String {
    summary
        .split("[#")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// A commit's ISO 8601 time in `zone`, `YYYY-MM-DD HH:MM` (each
/// commit carries its writer's zone, and the server's is UTC).
pub fn local_time<Tz: chrono::TimeZone>(iso: &str, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(t) => t.with_timezone(zone).format("%Y-%m-%d %H:%M").to_string(),
        Err(_) => iso.chars().take(16).collect::<String>().replace('T', " "),
    }
}

/// The latest commits with who made them. `actor` filters
/// (`mcp`, `app`, `external`); empty for all.
pub async fn activity(client: &KabacusClient, actor: &str) -> Result<Vec<Commit>, ModelError> {
    Ok(client
        .history()
        .await?
        .into_iter()
        .map(|c| {
            let who = c.actor.clone().unwrap_or_else(|| {
                if c.summary.starts_with("external") {
                    "external".to_string()
                } else {
                    String::new()
                }
            });
            let actor_label = match who.as_str() {
                "mcp" => i18nc!("who made a change", "MCP"),
                "app" => i18nc!("who made a change", "This app"),
                "external" => i18nc!("who made a change", "External edit"),
                "" => i18nc!("who made a change", "Unknown"),
                other => other.to_string(),
            };
            Commit {
                transaction_id: subject_id(&c.summary),
                short: c.hash.clone(),
                hash: c.hash,
                date: local_time(&c.date, &chrono::Local),
                summary: c.summary,
                actor: who,
                actor_label,
                user: c.user,
                agent: c.agent,
            }
        })
        .filter(|c| actor.is_empty() || c.actor == actor)
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    /// `add`, `del`, `hunk` or `context`.
    pub kind: &'static str,
    pub text: String,
}

/// One commit's diff, line by line.
pub async fn commit_diff(client: &KabacusClient, hash: &str) -> Result<Vec<DiffLine>, ModelError> {
    let diff = client.history_diff(hash).await?;
    Ok(diff_lines(&diff))
}

pub fn diff_lines(diff: &str) -> Vec<DiffLine> {
    diff.lines()
        .filter(|l| {
            !(l.starts_with("diff ")
                || l.starts_with("index ")
                || l.starts_with("--- ")
                || l.starts_with("+++ "))
        })
        .map(|l| DiffLine {
            kind: if l.starts_with("@@") {
                "hunk"
            } else if l.starts_with('+') {
                "add"
            } else if l.starts_with('-') {
                "del"
            } else {
                "context"
            },
            text: l.to_string(),
        })
        .collect()
}

/// One screen's data for `Ledger.load`, by `view` and its `args` (a JSON
/// object, as the page's `Fetch` sends it). An unknown view or report is
/// an error; a missing arg reads as empty.
pub async fn load(
    client: &KabacusClient,
    view: &str,
    args: &str,
) -> Result<serde_json::Value, ModelError> {
    fn value<T: serde::Serialize>(
        r: Result<T, ModelError>,
    ) -> Result<serde_json::Value, ModelError> {
        r.map(|v| serde_json::to_value(v).unwrap_or_default())
    }
    let args: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
    let s = |k: &str| args[k].as_str().unwrap_or_default().to_string();
    let b = |k: &str| args[k].as_bool().unwrap_or_default();
    let n = |k: &str| args[k].as_i64().unwrap_or_default();
    match view {
        "overview" => value(overview(client).await),
        "accounts" => value(accounts(client).await),
        "loans" => value(loans(client).await),
        "register" => {
            let filter = RegisterFilter {
                text: s("text"),
                unreserved_only: b("unreservedOnly"),
                fund_transfers: b("fundTransfers"),
            };
            value(register(client, &s("account"), &filter).await)
        }
        "funds" => value(funds(client).await),
        "cards" => value(cards(client).await),
        "people" => value(people(client).await),
        "statement" => value(statement(client, &s("person"), b("openItems")).await),
        "report" => {
            let (from, to) = month_range(n("year") as i32, n("month").clamp(1, 12) as u32);
            match s("kind").as_str() {
                "cash_flow" => value(cash_flow(client, &from, &to).await),
                "income" => value(income_statement(client, &from, &to, b("principal")).await),
                "categories" => value(categories(client, &from, &to).await),
                other => Err(ModelError::Invalid(format!("no report {other}"))),
            }
        }
        "reconcile" => {
            value(reconcile_check(client, &s("account"), &s("date"), &s("balance")).await)
        }
        "activity" => value(activity(client, &s("actor")).await),
        "diff" => value(commit_diff(client, &s("hash")).await),
        "picker" => value(picker(client).await),
        "payees" => value(payees(client, &s("query")).await),
        other => Err(ModelError::Invalid(format!("no view {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_uses_a_real_minus_and_a_tone() {
        let s = Shown::of(&AmountView {
            commodity: "USD".into(),
            exact: "-46.00".into(),
            formatted: "-$46.00".into(),
        });
        assert_eq!(s.text, "−$46.00");
        assert_eq!(s.tone, "neg");
        assert_eq!(Shown::from_exact("1234.5", None).text, "1,234.5");
        assert_eq!(Shown::from_exact("0.00", None).tone, "zero");
    }

    /// Report amounts carry the commodity like every other
    /// amount, and a minus never has a space after it.
    #[test]
    fn report_amounts_are_formatted_like_the_rest() {
        assert_eq!(Shown::amount("8747.82", "USD").text, "$8,747.82");
        assert_eq!(Shown::amount("-7.65", "USD").text, "−$7.65");
        assert_eq!(Shown::amount("-7.65", "").text, "−7.65");
        assert_eq!(Shown::from_exact("-7.655", None).text, "−7.655");
    }

    #[test]
    fn month_range_ends_on_the_last_day() {
        assert_eq!(
            month_range(2026, 2),
            ("2026-02-01".to_string(), "2026-02-28".to_string())
        );
        assert_eq!(month_range(2026, 12).1, "2026-12-31");
    }

    #[test]
    fn a_commit_subject_names_its_transaction() {
        assert_eq!(subject_id("add: 2026-02-01 Groceries [#12]"), "12");
        assert_eq!(subject_id("import: main.journal"), "");
    }

    #[test]
    fn diff_lines_keep_changes_and_drop_headers() {
        let lines =
            diff_lines("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same");
        let kinds: Vec<_> = lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, ["hunk", "del", "add", "context"]);
    }
}

#[cfg(test)]
mod local_time_tests {
    use super::local_time;

    /// Two writes five minutes apart, one by the server in UTC and one
    /// by hand at UTC−5, show five minutes apart.
    #[test]
    fn commit_times_show_in_the_viewers_zone() {
        let chicago = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(
            local_time("2026-10-03T20:06:00+00:00", &chicago),
            "2026-10-03 15:06"
        );
        assert_eq!(
            local_time("2026-10-03T15:01:00-05:00", &chicago),
            "2026-10-03 15:01"
        );
        assert_eq!(
            local_time("2026-10-03T20:06:00Z", &chrono::Utc),
            "2026-10-03 20:06"
        );
        assert_eq!(local_time("not a date", &chrono::Utc), "not a date");
    }
}
