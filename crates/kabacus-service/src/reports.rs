// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The report tools and `reconcile`, over kabacus-core's reports. Every
//! report counts every transaction.

use crate::caller::{Caller, Permission as Role};
use crate::error::Error as ApiError;
use crate::ops;
use crate::search::{decode_cursor, decode_token, encode_cursor, encode_token, OneOrMany};
use crate::service::{self, invalid, parse_date, parse_optional_date, require, AmountInput};
use crate::state::Service;
use chrono::NaiveDate;
use kabacus_core::reports::{self as core_reports, Exclude, GroupBy, Interval, Period};
use kabacus_core::{AccountKind, AccountName, Amount, Money, Status, Transaction};

use serde::Deserialize;
use serde_json::{json, Value};

fn s(m: Money) -> String {
    m.to_string()
}

fn interval(i: Option<&str>) -> Result<Interval, ApiError> {
    match i {
        None => Ok(Interval::None),
        Some(i) => Interval::parse(i).ok_or_else(|| {
            invalid(
                "interval",
                format!("{i:?}: use none, month, quarter or year"),
            )
        }),
    }
}

fn group_by(g: Option<&str>, default: GroupBy) -> Result<GroupBy, ApiError> {
    match g {
        None => Ok(default),
        Some(g) => GroupBy::parse(g)
            .ok_or_else(|| invalid("group_by", format!("{g:?}: use kind or account"))),
    }
}

/// Entries of a list given as an array or comma-separated.
fn list(values: &Option<OneOrMany<String>>) -> Vec<String> {
    values
        .as_ref()
        .map(OneOrMany::to_vec)
        .unwrap_or_default()
        .iter()
        .flat_map(|v| v.split(','))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

fn kinds(values: &[String], field: &str) -> Result<Vec<AccountKind>, ApiError> {
    values
        .iter()
        .map(|k| {
            serde_json::from_value(json!(k))
                .map_err(|_| invalid(field, format!("{k:?} isn't an account kind")))
        })
        .collect()
}

fn range(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let (from, to) = (parse_date(from)?, parse_date(to)?);
    if to < from {
        return Err(invalid("to", "to is before from".to_string()));
    }
    Ok((from, to))
}

struct Books {
    chart: kabacus_core::Chart,
    txs: Vec<Transaction>,
    commodity: String,
}

async fn books(state: &Service, principal: &dyn Caller) -> Result<Books, ApiError> {
    require(principal, Role::Read)?;
    let (chart, txs) = service::books_chart(state).await?;
    Ok(Books {
        chart,
        txs,
        commodity: service::books_commodity(state).await?,
    })
}

fn period_dto(p: &Period, interval: Interval) -> Value {
    json!({
        "label": p.label(interval),
        "start": p.start.format("%Y-%m-%d").to_string(),
        "end": p.end.format("%Y-%m-%d").to_string(),
    })
}

// --- income statement ------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct IncomeStatementParams {
    /// Show this commodity without converting other currencies into it.
    pub commodity: Option<String>,
    /// `YYYY-MM-DD`, inclusive.
    pub from: String,
    pub to: String,
    /// `none` (default), `month`, `quarter` or `year`.
    #[serde(default)]
    pub interval: Option<String>,
    /// Accounts rolled up to this many segments (default 2).
    #[serde(default)]
    pub depth: Option<usize>,
    /// Kinds or account prefixes to leave out.
    #[serde(default)]
    pub exclude: Option<OneOrMany<String>>,
    /// The equity conversion section (default true).
    #[serde(default)]
    pub equity_conversion: Option<bool>,
    /// The kinds counted as equity conversion (default `["term-liability"]`;
    /// add `"non-liquid"` for money moved into investments).
    #[serde(default)]
    pub conversion_kinds: Option<OneOrMany<String>>,
}

/// Income and expenses by period with the equity conversion
/// section.
pub async fn income_statement(
    state: &Service,
    principal: &dyn Caller,
    p: &IncomeStatementParams,
) -> Result<Value, ApiError> {
    let mut b = books(state, principal).await?;
    if let Some(commodity) = &p.commodity {
        b.commodity = commodity.clone();
    }
    let (from, to) = range(&p.from, &p.to)?;
    let interval = interval(p.interval.as_deref())?;
    let periods = core_reports::periods(from, to, interval);
    let conversion_kinds = match &p.conversion_kinds {
        Some(_) => kinds(&list(&p.conversion_kinds), "conversion_kinds")?,
        None => vec![AccountKind::TermLiability],
    };
    let options = core_reports::IncomeStatementOptions {
        depth: p.depth.unwrap_or(2),
        exclude: Exclude::parse(&list(&p.exclude)),
        equity_conversion: p.equity_conversion.unwrap_or(true),
        conversion_kinds,
    };
    let r = core_reports::income_statement(&b.chart, &b.txs, &b.commodity, &periods, &options);
    let rows = |rows: &[core_reports::ReportRow]| -> Vec<Value> {
        rows.iter()
            .map(|r| {
                json!({
                    "account": r.account.to_string(),
                    "display_name": r.display_name,
                    "amounts": r.amounts.iter().cloned().map(s).collect::<Vec<_>>(),
                    "total": s(r.total.clone()),
                })
            })
            .collect()
    };
    let strings = |v: &[Money]| v.iter().cloned().map(s).collect::<Vec<_>>();
    let mut out = json!({
        "commodity": b.commodity,
        "periods": periods.iter().map(|p| period_dto(p, interval)).collect::<Vec<_>>(),
        "income": rows(&r.income),
        "expenses": rows(&r.expenses),
        "totals": {
            "income": strings(&r.income_totals),
            "expenses": strings(&r.expense_totals),
        },
        "net_before_conversion": strings(&r.net_before_conversion),
        "net": strings(&r.net),
    });
    if let (Some(conversion), Some(totals)) = (&r.conversion, &r.conversion_totals) {
        out["equity_conversion"] = json!({
            "rows": conversion.iter().map(|c| json!({
                "account": c.account.to_string(),
                "display_name": c.display_name,
                "amounts": strings(&c.amounts),
                "cash": strings(&c.cash),
                "non_cash": strings(&c.non_cash),
                "total": s(c.total.clone()),
            })).collect::<Vec<_>>(),
            "totals": strings(totals),
        });
    }
    Ok(out)
}

// --- balance sheet ----------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct BalanceSheetParams {
    /// `YYYY-MM-DD`; every transaction if omitted.
    #[serde(default)]
    pub as_of: Option<String>,
    /// With group_by account: rolled up to this many segments (default 2).
    #[serde(default)]
    pub depth: Option<usize>,
    /// `kind` (default) or `account`.
    #[serde(default)]
    pub group_by: Option<String>,
}

/// Assets, liabilities, per-kind subtotals, liquid money and net
/// worth.
pub async fn balance_sheet(
    state: &Service,
    principal: &dyn Caller,
    p: &BalanceSheetParams,
) -> Result<Value, ApiError> {
    let b = books(state, principal).await?;
    let as_of = parse_optional_date(p.as_of.as_deref())?;
    let sheet = core_reports::balance_sheet(
        &b.chart,
        &b.txs,
        &b.commodity,
        as_of,
        p.depth.unwrap_or(2),
        group_by(p.group_by.as_deref(), GroupBy::Kind)?,
    );
    let rows = |rows: &[core_reports::SheetRow]| -> Vec<Value> {
        rows.iter()
            .map(|r| json!({"account": r.group, "display_name": r.display_name, "balance": s(r.balance.clone())}))
            .collect()
    };
    Ok(json!({
        "commodity": b.commodity,
        "as_of": sheet.as_of.map(|d| d.format("%Y-%m-%d").to_string()),
        "assets": rows(&sheet.assets),
        "liabilities": rows(&sheet.liabilities),
        "total_assets": s(sheet.total_assets),
        "total_liabilities": s(sheet.total_liabilities),
        "by_kind": sheet.by_kind.iter().map(|(k, m)| json!({"kind": k, "total": s(m.clone())})).collect::<Vec<_>>(),
        "liquid": s(sheet.liquid),
        "net_worth": s(sheet.net_worth),
    }))
}

// --- category breakdown -------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CategoryBreakdownParams {
    /// Show this commodity without converting other currencies into it.
    pub commodity: Option<String>,
    pub from: String,
    pub to: String,
    /// Categories rolled up to this many segments (default 2).
    #[serde(default)]
    pub depth: Option<usize>,
    /// The top categories listed (default 20).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Add each category's change from the equal period before.
    #[serde(default)]
    pub compare_previous: Option<bool>,
}

/// The top expense categories with their share and change.
pub async fn category_breakdown(
    state: &Service,
    principal: &dyn Caller,
    p: &CategoryBreakdownParams,
) -> Result<Value, ApiError> {
    let mut b = books(state, principal).await?;
    if let Some(commodity) = &p.commodity {
        b.commodity = commodity.clone();
    }
    let (from, to) = range(&p.from, &p.to)?;
    let r = core_reports::category_breakdown(
        &b.chart,
        &b.txs,
        &b.commodity,
        Period {
            start: from,
            end: to,
        },
        p.depth.unwrap_or(2),
        p.limit.unwrap_or(20).max(1),
        p.compare_previous.unwrap_or(false),
    );
    Ok(json!({
        "commodity": b.commodity,
        "total": s(r.total),
        "rows": r.rows.iter().map(|c| {
            let mut row = json!({
                "account": c.account.to_string(),
                "display_name": c.display_name,
                "amount": s(c.amount.clone()),
                "share_percent": s(c.share.clone()),
            });
            if let (Some(prev), Some(change)) = (c.previous.clone(), c.change.clone()) {
                row["previous"] = json!(s(prev));
                row["change"] = json!(s(change));
            }
            row
        }).collect::<Vec<_>>(),
        "other": s(r.other),
        "previous_period": r.previous_period.map(|p| period_dto(&p, Interval::None)),
        "previous_total": r.previous_total.map(s),
    }))
}

// --- Cash on Hand history ---------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CashHistoryParams {
    pub from: String,
    pub to: String,
    /// `month` (default), `quarter`, `year`, or `none` for every day. At
    /// most 366 points.
    #[serde(default)]
    pub interval: Option<String>,
}

/// Cash on Hand at each period's end.
pub async fn cash_on_hand_history(
    state: &Service,
    principal: &dyn Caller,
    p: &CashHistoryParams,
) -> Result<Value, ApiError> {
    let b = books(state, principal).await?;
    let (from, to) = range(&p.from, &p.to)?;
    let interval = match p.interval.as_deref() {
        None => Interval::Month,
        Some(i) => interval(Some(i))?,
    };
    let points =
        core_reports::cash_on_hand_history(&b.chart, &b.txs, &b.commodity, from, to, interval);
    Ok(json!({
        "commodity": b.commodity,
        "points": points.iter().map(|t| json!({
            "date": t.as_of.map(|d| d.format("%Y-%m-%d").to_string()),
            "cash_on_hand": s(t.cash_on_hand.clone()),
            "money_on_hand": s(t.money_on_hand.clone()),
            "money": s(t.money.clone()),
            "funds": s(t.funds.clone()),
            "reserves": s(t.reserves.clone()),
            "receivables": s(t.receivables.clone()),
        })).collect::<Vec<_>>(),
    }))
}

// --- cash flow -------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CashFlowParams {
    /// Show this commodity without converting other currencies into it.
    pub commodity: Option<String>,
    pub from: String,
    pub to: String,
    /// `kind` (default) or `account`.
    #[serde(default)]
    pub group_by: Option<String>,
    /// With group_by account: rolled up to this many segments (default 2).
    #[serde(default)]
    pub depth: Option<usize>,
    /// Kinds or account prefixes whose cash is left out of
    /// `change_excluding`.
    #[serde(default)]
    pub exclude: Option<OneOrMany<String>>,
    /// List each line's transactions (paged).
    #[serde(default)]
    pub include_transactions: Option<bool>,
    /// Transactions per line (default 50).
    #[serde(default)]
    pub limit: Option<usize>,
    /// A line's `next_cursor`: returns only that line's next page.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// The bridge from opening to closing Cash on Hand.
pub async fn cash_flow(
    state: &Service,
    principal: &dyn Caller,
    p: &CashFlowParams,
) -> Result<Value, ApiError> {
    let mut b = books(state, principal).await?;
    if let Some(commodity) = &p.commodity {
        b.commodity = commodity.clone();
    }
    let (from, to) = range(&p.from, &p.to)?;
    let options = core_reports::CashFlowOptions {
        group_by: group_by(p.group_by.as_deref(), GroupBy::Kind)?,
        depth: p.depth.unwrap_or(2),
        exclude: Exclude::parse(&list(&p.exclude)),
    };
    let flow = core_reports::cash_flow(&b.chart, &b.txs, &b.commodity, from, to, &options);
    // A line cursor is "<group>\u{1f}<offset>", encoded.
    let page_of = match &p.cursor {
        Some(cursor) => {
            let raw = decode_token(cursor).ok_or_else(|| {
                invalid(
                    "cursor",
                    format!("{cursor:?} isn't a cursor from this server"),
                )
            })?;
            let (group, offset) = raw
                .split_once('\u{1f}')
                .ok_or_else(|| invalid("cursor", "not a cash_flow cursor".to_string()))?;
            Some((group.to_string(), decode_cursor(offset)?))
        }
        None => None,
    };
    let include = p.include_transactions.unwrap_or(false) || page_of.is_some();
    let limit = p.limit.unwrap_or(50).clamp(1, crate::search::MAX_LIMIT);
    let lines: Vec<Value> = flow
        .lines
        .iter()
        .filter(|l| page_of.as_ref().is_none_or(|(g, _)| *g == l.group))
        .map(|l| {
            let mut line = json!({
                "group": l.group,
                "display_name": l.display_name,
                "cash": s(l.cash.clone()),
                "non_cash": s(l.non_cash.clone()),
                "total": s(l.total.clone()),
            });
            if include {
                let offset = page_of.as_ref().map_or(0, |(_, o)| *o);
                let items: Vec<Value> = l
                    .transactions
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|t| {
                        let tx = &b.txs[t.tx];
                        json!({
                            "id": tx.id,
                            "date": tx.date.format("%Y-%m-%d").to_string(),
                            "description": tx.description,
                            "cash": s(t.cash.clone()),
                            "non_cash": s(t.non_cash.clone()),
                        })
                    })
                    .collect();
                let end = offset + items.len();
                let truncated = end < l.transactions.len();
                line["transactions"] = json!(items);
                line["total_count"] = json!(l.transactions.len());
                line["truncated"] = json!(truncated);
                line["next_cursor"] = json!(truncated.then(|| encode_token(&format!(
                    "{}\u{1f}{}",
                    l.group,
                    encode_cursor(end)
                ))));
            }
            line
        })
        .collect();
    Ok(json!({
        "commodity": b.commodity,
        "from": from.format("%Y-%m-%d").to_string(),
        "to": to.format("%Y-%m-%d").to_string(),
        "opening_cash": s(flow.opening_cash),
        "closing_cash": s(flow.closing_cash),
        "change": s(flow.change),
        "change_excluding": s(flow.change_excluding),
        "lines": lines,
    }))
}

// --- receivables activity -----------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ReceivablesParams {
    pub from: String,
    pub to: String,
    /// One receivable account (or prefix); every receivable if omitted.
    #[serde(default)]
    pub counterparty: Option<String>,
    /// Each entry with its id, date, description, amount and counter
    /// postings.
    #[serde(default)]
    pub include_entries: Option<bool>,
}

/// Filed and collected per counterparty.
pub async fn receivables_activity(
    state: &Service,
    principal: &dyn Caller,
    p: &ReceivablesParams,
) -> Result<Value, ApiError> {
    let b = books(state, principal).await?;
    let (from, to) = range(&p.from, &p.to)?;
    let r = core_reports::receivables_activity(
        &b.chart,
        &b.txs,
        &b.commodity,
        from,
        to,
        p.counterparty.as_deref(),
    );
    let activity = |a: &core_reports::Activity| {
        json!({
            "opening": s(a.opening.clone()),
            "filed": s(a.filed.clone()),
            "collected": s(a.collected.clone()),
            "closing": s(a.closing.clone()),
        })
    };
    let with_entries = p.include_entries.unwrap_or(false);
    Ok(json!({
        "commodity": b.commodity,
        "counterparties": r.counterparties.iter().map(|c| {
            let mut v = json!({
                "account": c.account.to_string(),
                "display_name": c.display_name,
                "activity": activity(&c.activity),
                "filed_by_counter_account": c.filed_by_counter_account.iter().map(|f| json!({
                    "account": f.account.to_string(),
                    "amount": s(f.amount.clone()),
                    "netted_against_expense": f.netted_against_expense,
                })).collect::<Vec<_>>(),
            });
            if with_entries {
                v["entries"] = json!(c.entries.iter().filter(|e| b.txs[e.tx].date >= from).map(|e| {
                    let tx = &b.txs[e.tx];
                    json!({
                        "id": tx.id,
                        "date": tx.date.format("%Y-%m-%d").to_string(),
                        "description": tx.description,
                        "amount": s(e.amount.clone()),
                        "counter_postings": e.counter.iter().map(|(a, m)| json!({"account": a.to_string(), "amount": s(m.clone())})).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>());
            }
            v
        }).collect::<Vec<_>>(),
        "total": activity(&r.total),
    }))
}

// --- reconcile --------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ReconcileParams {
    /// Required for `assert` and `adjust`.
    #[serde(default)]
    pub head: Option<String>,
    /// Defaults to true: `assert` and `adjust` preview without writing.
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// A money, revolving-debt, non-liquid or term-liability account.
    pub account: String,
    /// The statement balance as booked (negative for what is owed on a
    /// card or loan), a decimal string. A money account's is compared with
    /// its inclusive balance.
    pub balance: AmountInput,
    /// The statement's day: balances as of its end.
    pub date: String,
    /// Default `"0.00"`.
    #[serde(default)]
    pub tolerance: Option<AmountInput>,
    /// `check` (default) writes nothing; `assert` writes a balance
    /// assertion tagged `reconciled`; `adjust` posts the difference to
    /// `equity:adjustments` first (needs confirm_adjust).
    #[serde(default)]
    pub mode: Option<String>,
    /// When matched, mark the account's postings up to `date` cleared.
    #[serde(default)]
    pub mark_cleared: Option<bool>,
    /// Required for `adjust`.
    #[serde(default)]
    pub confirm_adjust: Option<bool>,
    #[serde(default)]
    pub include_diff: Option<bool>,
}

fn reconciliation_commodity(
    p: &ReconcileParams,
    default: &str,
    txs: &[Transaction],
) -> Result<String, ApiError> {
    if let Some(commodity) = p.balance.0.get("commodity").and_then(Value::as_str) {
        return Ok(commodity.to_string());
    }
    let currencies: std::collections::BTreeSet<&str> = txs
        .iter()
        .flat_map(|t| &t.postings)
        .filter(|q| q.account.as_str() == p.account || q.account.is_under(&p.account))
        .map(|q| q.amount.commodity.as_str())
        .collect();
    match currencies.len() {
        0 => Ok(default.to_string()),
        1 => Ok(currencies.into_iter().next().unwrap().to_string()),
        _ => Err(invalid(
            "balance",
            "Choose a currency for this account's statement balance".into(),
        )),
    }
}

fn effective(tx: &Transaction, p: &kabacus_core::Posting) -> Status {
    if tx.status == Status::Cleared || p.status == Status::Cleared {
        Status::Cleared
    } else {
        p.status
    }
}

/// Compare an account with a statement; on a match, optionally
/// assert it (or, with `adjust`, make it match first).
pub async fn reconcile(
    state: &Service,
    principal: &dyn Caller,
    p: &ReconcileParams,
) -> Result<Value, ApiError> {
    let mut b = books(state, principal).await?;
    b.commodity = reconciliation_commodity(p, &b.commodity, &b.txs)?;
    let account = AccountName::new(&p.account);
    let kind = b.chart.kind_of(&account);
    if !matches!(
        kind,
        AccountKind::Money
            | AccountKind::RevolvingDebt
            | AccountKind::NonLiquid
            | AccountKind::TermLiability
    ) {
        return Err(invalid(
            "account",
            format!("{} is {kind}; reconcile a money, revolving-debt, non-liquid or term-liability account", p.account),
        ));
    }
    let date = parse_date(&p.date)?;
    let statement = p.balance.to_money(&b.commodity, "balance")?;
    let tolerance = match &p.tolerance {
        Some(t) => t.to_money(&b.commodity, "tolerance")?,
        None => Money::from_decimal(0, 2),
    };
    let mode = p.mode.as_deref().unwrap_or("check");
    if !matches!(mode, "check" | "assert" | "adjust") {
        return Err(invalid(
            "mode",
            format!("{mode:?}: use check, assert or adjust"),
        ));
    }
    let inclusive = kind == AccountKind::Money;
    let balances = kabacus_core::Balances::compute(&b.txs, &b.commodity, Some(date));
    let ledger = if inclusive {
        balances.inclusive(&account)
    } else {
        balances.exclusive(&account)
    }
    .money;
    let places = ledger.places().max(statement.places());
    let difference = (ledger.clone() - statement.clone()).rescaled(places);
    let abs = |m: Money| if m.is_negative() { -m } else { m };
    let matched = abs(difference.clone()) <= tolerance;
    let mut result = json!({
        "account": p.account,
        "date": date.format("%Y-%m-%d").to_string(),
        "commodity": b.commodity,
        "ledger_balance": s(ledger.rescaled(places)),
        "statement_balance": s(statement.rescaled(places)),
        "difference": s(difference.clone()),
        "status": if matched { "matched" } else { "mismatch" },
        "mode": mode,
    });

    if !matched {
        let in_account = |tx: &Transaction| -> Money {
            tx.postings
                .iter()
                .filter(|q| q.amount.commodity == b.commodity)
                .filter(|q| {
                    if inclusive {
                        q.account.is_under(&p.account)
                    } else {
                        q.account == account
                    }
                })
                .map(|q| q.amount.money.clone())
                .sum()
        };
        let describe = |i: usize, amount: Money| {
            let tx = &b.txs[i];
            json!({
                "id": tx.id,
                "date": tx.date.format("%Y-%m-%d").to_string(),
                "description": tx.description,
                "amount": s(amount),
            })
        };
        let uncleared: Vec<(usize, Money)> = b
            .txs
            .iter()
            .enumerate()
            .filter(|(_, tx)| tx.date <= date)
            .filter(|(_, tx)| {
                tx.postings.iter().any(|q| {
                    (if inclusive {
                        q.account.is_under(&p.account)
                    } else {
                        q.account == account
                    }) && effective(tx, q) != Status::Cleared
                })
            })
            .map(|(i, tx)| (i, in_account(tx)))
            .filter(|(_, m)| !m.is_zero())
            .collect();
        let mut later: Vec<(usize, Money)> = b
            .txs
            .iter()
            .enumerate()
            .filter(|(_, tx)| tx.date > date)
            .map(|(i, tx)| (i, in_account(tx)))
            .filter(|(_, m)| !m.is_zero())
            .collect();
        later.sort_by_key(|(i, _)| (b.txs[*i].date, b.txs[*i].id));
        later.truncate(60);
        // Uncleared ones the bank hasn't seen yet: without them, the ledger
        // would match.
        let sets = core_reports::subsets_summing_to(&uncleared, difference.clone(), 3, 10);
        result["suggestions"] = json!(sets
            .iter()
            .map(|set| set
                .iter()
                .map(|i| describe(
                    *i,
                    uncleared
                        .iter()
                        .find(|(j, _)| j == i)
                        .map_or(Money::zero(), |(_, m)| m.clone())
                ))
                .collect::<Vec<_>>())
            .collect::<Vec<_>>());
        // Later ones that would match if they were dated by the statement.
        let sets = core_reports::subsets_summing_to(&later, -difference.clone(), 3, 10);
        result["later_transactions"] = json!(sets
            .iter()
            .map(|set| set
                .iter()
                .map(|i| describe(
                    *i,
                    later
                        .iter()
                        .find(|(j, _)| j == i)
                        .map_or(Money::zero(), |(_, m)| m.clone())
                ))
                .collect::<Vec<_>>())
            .collect::<Vec<_>>());
    }

    if mode == "check" {
        return Ok(result);
    }
    require(principal, Role::Write)?;
    let head = p
        .head
        .as_deref()
        .ok_or_else(|| invalid("head", format!("head is required to {mode}")))?;
    let op = reconcile_op(p, &b.commodity, &b.chart, &b.txs)?;
    let written = service::run_write(
        state,
        principal,
        op,
        head,
        p.dry_run.unwrap_or(true),
        p.include_diff.unwrap_or(false),
    )
    .await?;
    result["written"] = serde_json::to_value(&written).unwrap_or_default();
    result["head"] = json!(written.head);
    result["dry_run"] = json!(written.dry_run);
    Ok(result)
}

/// Build a reconciliation against the current working copy, including earlier batch steps.
pub fn reconcile_op(
    p: &ReconcileParams,
    commodity: &str,
    chart: &kabacus_core::Chart,
    txs: &[Transaction],
) -> Result<ops::WriteOp, ApiError> {
    let selected = reconciliation_commodity(p, commodity, txs)?;
    let commodity = selected.as_str();
    let account = AccountName::new(&p.account);
    let kind = chart.kind_of(&account);
    if !matches!(
        kind,
        AccountKind::Money
            | AccountKind::RevolvingDebt
            | AccountKind::NonLiquid
            | AccountKind::TermLiability
    ) {
        return Err(invalid("account", "Choose a reconcilable account".into()));
    }
    let date = parse_date(&p.date)?;
    let statement = p.balance.to_money(commodity, "balance")?;
    let tolerance = p
        .tolerance
        .as_ref()
        .map(|v| v.to_money(commodity, "tolerance"))
        .transpose()?
        .unwrap_or(Money::zero());
    if tolerance.is_negative() {
        return Err(invalid("tolerance", "Tolerance cannot be negative".into()));
    }
    let inclusive = kind == AccountKind::Money;
    let balances = kabacus_core::Balances::compute(txs, commodity, Some(date));
    let ledger = if inclusive {
        balances.inclusive(&account)
    } else {
        balances.exclusive(&account)
    }
    .money;
    let places = ledger.places().max(statement.places());
    let difference = (ledger.clone() - statement.clone()).rescaled(places);
    let magnitude = if difference.is_negative() {
        -difference.clone()
    } else {
        difference.clone()
    };
    let matched = magnitude <= tolerance;
    let mode = p.mode.as_deref().unwrap_or("assert");
    if !matches!(mode, "assert" | "adjust") {
        return Err(invalid(
            "mode",
            "A script reconciliation must assert or adjust".into(),
        ));
    }
    let result =
        json!({"account":p.account,"status":"mismatch","difference":difference.to_string()});
    let adjustment = match (mode, matched) {
        ("assert", false) => return Err(ApiError::ReconcileMismatch(result)),
        ("adjust", false) => {
            if !p.confirm_adjust.unwrap_or(false) {
                return Err(invalid(
                    "confirm_adjust",
                    "adjust posts the difference to equity:adjustments; pass confirm_adjust: true"
                        .to_string(),
                ));
            }
            Some(-difference)
        }
        _ => None,
    };
    let asserted = match adjustment {
        Some(_) => statement.rescaled(places),
        None => ledger.rescaled(places),
    };
    let op = ops::WriteOp::Reconcile(ops::ReconcileWrite {
        account,
        date,
        balance: Amount::new(commodity, asserted),
        inclusive,
        adjustment,
        mark_cleared: matched && p.mark_cleared.unwrap_or(false),
    });
    Ok(op)
}
