// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading transactions: structured search with paging and
//! projections, single-transaction lookup with its history, and the paged
//! register.

use crate::caller::{Caller, Permission as Role};
use crate::error::Error as ApiError;
use crate::ops;
use crate::service::{
    self, invalid, parse_optional_date, require, AmountInput, TransactionDto, UnreservedIndex,
};
use crate::state::Service;
use base64::Engine;
use kabacus_core::{AccountKind, AccountName, Money, Status, Transaction};

use serde::{Deserialize, Serialize};

/// The default and the most rows a page has.
pub const DEFAULT_LIMIT: usize = 200;
pub const MAX_LIMIT: usize = 1000;

/// One value or several, for a repeatable filter.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T: Clone> OneOrMany<T> {
    pub fn to_vec(&self) -> Vec<T> {
        match self {
            OneOrMany::One(v) => vec![v.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

/// `{key, value?}`: a tag the transaction must have, with that value if
/// given.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema, PartialEq)]
pub struct TagFilter {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
}

/// Text as an opaque URL-safe token: what every cursor is made of.
pub fn encode_token(text: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text)
}

pub fn decode_token(token: &str) -> Option<String> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
}

/// An opaque position in a result list.
pub fn encode_cursor(offset: usize) -> String {
    encode_token(&format!("o:{offset}"))
}

pub fn decode_cursor(cursor: &str) -> Result<usize, ApiError> {
    decode_token(cursor)
        .and_then(|s| s.strip_prefix("o:").and_then(|n| n.parse().ok()))
        .ok_or_else(|| {
            invalid(
                "cursor",
                format!("{cursor:?} isn't a cursor from this server"),
            )
        })
}

/// A page of `items`: the rows from `cursor`, at most `limit`, and the
/// cursor for the next page.
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub total_count: usize,
    pub truncated: bool,
}

pub fn page<T>(
    items: Vec<T>,
    cursor: Option<&str>,
    limit: Option<usize>,
    default_limit: usize,
) -> Result<Page<T>, ApiError> {
    let total_count = items.len();
    let offset = cursor.map(decode_cursor).transpose()?.unwrap_or(0);
    let limit = limit.unwrap_or(default_limit).clamp(1, MAX_LIMIT);
    let items: Vec<T> = items.into_iter().skip(offset).take(limit).collect();
    let end = offset + items.len();
    let truncated = end < total_count;
    Ok(Page {
        items,
        next_cursor: truncated.then(|| encode_cursor(end)),
        total_count,
        truncated,
    })
}

/// Keep only `fields` of each row (all of them if `None`).
pub fn project<T: Serialize>(
    rows: &[T],
    fields: Option<&[String]>,
    extra: impl Fn(usize) -> serde_json::Map<String, serde_json::Value>,
) -> Vec<serde_json::Value> {
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let mut value = serde_json::to_value(row).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.extend(extra(i));
                if let Some(fields) = fields {
                    object.retain(|k, _| fields.iter().any(|f| f == k));
                }
            }
            value
        })
        .collect()
}

/// Search parameters, shared by `search_transactions` and
/// `GET /transactions`. Every filter combines with the others by AND.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct SearchQuery {
    /// A plain-text filter matched against payee, memo and posting
    /// accounts (not hledger's query language).
    #[serde(default)]
    pub query: Option<String>,
    /// `YYYY-MM-DD`, inclusive.
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    /// Only card transactions flagged unreserved.
    #[serde(default, alias = "unreserved")]
    pub unreserved_only: Option<bool>,
    /// An account path or prefix; several must each match some posting.
    #[serde(default)]
    pub account: Option<OneOrMany<String>>,
    /// An account kind (`money`, `expense`, ...): some posting to an
    /// account of that kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Decimal strings, compared with the absolute value of the
    /// transaction's largest posting.
    #[serde(default)]
    pub amount_min: Option<AmountInput>,
    #[serde(default)]
    pub amount_max: Option<AmountInput>,
    /// Case-insensitive substring of the payee, or `/regex/`.
    #[serde(default)]
    pub payee: Option<String>,
    /// Case-insensitive substring of the memo, or `/regex/`.
    #[serde(default)]
    pub memo: Option<String>,
    /// `{key, value?}`; several must all match.
    #[serde(default)]
    pub tag: Option<OneOrMany<TagFilter>>,
    /// Tag keys the transaction must have.
    #[serde(default)]
    pub has_tag: Option<OneOrMany<String>>,
    /// Tag keys the transaction must not have.
    #[serde(default)]
    pub lacks_tag: Option<OneOrMany<String>>,
    /// `unmarked`, `pending` or `cleared`, on any posting.
    #[serde(default)]
    pub status: Option<String>,
    /// `date_asc` (default) or `date_desc`.
    #[serde(default)]
    pub sort: Option<String>,
    /// Rows per page: 200 by default, at most 1000.
    #[serde(default)]
    pub limit: Option<usize>,
    /// `next_cursor` from the previous page, with the same filters.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Only these fields of each row, e.g. `["id", "date", "payee",
    /// "postings"]`; `cash_effect` adds the transaction's cash effect.
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Only `{total_count}`, no rows.
    #[serde(default)]
    pub count_only: Option<bool>,
    /// `zero` or `nonzero`: the sum of the postings to money accounts
    /// (free money).
    #[serde(default)]
    pub cash_effect: Option<String>,
}

/// A substring (case-insensitive) or `/regex/` matcher.
enum TextMatch {
    Contains(String),
    Regex(regex::Regex),
}

impl TextMatch {
    fn parse(field: &str, pattern: &str) -> Result<TextMatch, ApiError> {
        match pattern
            .strip_prefix('/')
            .and_then(|p| p.strip_suffix('/'))
            .filter(|p| !p.is_empty())
        {
            Some(re) => regex::RegexBuilder::new(re)
                .case_insensitive(true)
                .build()
                .map(TextMatch::Regex)
                .map_err(|e| invalid(field, format!("{pattern:?} isn't a regex: {e}"))),
            None => Ok(TextMatch::Contains(pattern.to_lowercase())),
        }
    }

    fn matches(&self, text: &str) -> bool {
        match self {
            TextMatch::Contains(s) => text.to_lowercase().contains(s),
            TextMatch::Regex(re) => re.is_match(text),
        }
    }
}

fn effective_status(tx: &Transaction, status: Status) -> Status {
    if tx.status == Status::Cleared || status == Status::Cleared {
        Status::Cleared
    } else if tx.status == Status::Pending || status == Status::Pending {
        Status::Pending
    } else {
        Status::Unmarked
    }
}

fn abs(m: Money) -> Money {
    if m.is_negative() {
        -m
    } else {
        m
    }
}

/// The result of a search: a page of rows, or only the count.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SearchResult {
    Page {
        transactions: Vec<serde_json::Value>,
        next_cursor: Option<String>,
        total_count: usize,
        truncated: bool,
    },
    Count {
        total_count: usize,
    },
}

/// The books' transactions matching `q`, as DTOs in the requested order
/// (unpaged), with each one's cash effect.
pub async fn matching(
    state: &Service,
    q: &SearchQuery,
) -> Result<Vec<(TransactionDto, Money)>, ApiError> {
    let from = parse_optional_date(q.from.as_deref())?;
    let to = parse_optional_date(q.to.as_deref())?;
    let all = ops::read_all_transactions(state).await?;
    let chart = service::books_chart_from(state, &all).await?;
    let commodity = service::books_commodity(state).await?;
    let flags = UnreservedIndex::new(&chart, &all);
    let visible: Vec<&Transaction> = all.iter().collect();

    let kind: Option<AccountKind> = q
        .kind
        .as_deref()
        .map(|k| {
            serde_json::from_value(serde_json::json!(k))
                .map_err(|_| invalid("kind", format!("{k:?} isn't an account kind")))
        })
        .transpose()?;
    let amount = |a: &Option<AmountInput>, field: &str| {
        a.as_ref()
            .map(|a| a.to_money(&commodity, field))
            .transpose()
    };
    let amount_min = amount(&q.amount_min, "amount_min")?;
    let amount_max = amount(&q.amount_max, "amount_max")?;
    let payee = q
        .payee
        .as_deref()
        .map(|p| TextMatch::parse("payee", p))
        .transpose()?;
    let memo = q
        .memo
        .as_deref()
        .map(|p| TextMatch::parse("memo", p))
        .transpose()?;
    let status: Option<Status> = match q.status.as_deref() {
        None => None,
        Some(s @ ("unmarked" | "pending" | "cleared")) => Some(service::parse_status(s)),
        Some(other) => {
            return Err(invalid(
                "status",
                format!("{other:?} isn't a status; use unmarked, pending or cleared"),
            ))
        }
    };
    let cash_filter = match q.cash_effect.as_deref() {
        None => None,
        Some("zero") => Some(true),
        Some("nonzero") => Some(false),
        Some(other) => {
            return Err(invalid(
                "cash_effect",
                format!("{other:?}: use zero or nonzero"),
            ))
        }
    };
    let accounts = q
        .account
        .as_ref()
        .map(OneOrMany::to_vec)
        .unwrap_or_default();
    let tags = q.tag.as_ref().map(OneOrMany::to_vec).unwrap_or_default();
    let has_tag = q
        .has_tag
        .as_ref()
        .map(OneOrMany::to_vec)
        .unwrap_or_default();
    let lacks_tag = q
        .lacks_tag
        .as_ref()
        .map(OneOrMany::to_vec)
        .unwrap_or_default();
    let unreserved_only = q.unreserved_only.unwrap_or(false);
    let query = q.query.as_deref().map(str::to_lowercase);

    let mut rows: Vec<(&Transaction, Money)> = Vec::new();
    for tx in visible {
        if from.is_some_and(|d| tx.date < d) || to.is_some_and(|d| tx.date > d) {
            continue;
        }
        if let Some(q) = &query {
            let haystack = tx.description.to_lowercase()
                + " "
                + &tx
                    .postings
                    .iter()
                    .map(|p| p.account.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
            if !haystack.contains(q) {
                continue;
            }
        }
        if !accounts
            .iter()
            .all(|a| tx.postings.iter().any(|p| p.account.is_under(a)))
        {
            continue;
        }
        if kind.is_some_and(|k| !tx.postings.iter().any(|p| chart.kind_of(&p.account) == k)) {
            continue;
        }
        if amount_min.is_some() || amount_max.is_some() {
            let largest = tx
                .postings
                .iter()
                .map(|p| abs(p.amount.money.clone()))
                .max()
                .unwrap_or_else(Money::zero);
            if amount_min.clone().is_some_and(|m| largest < m)
                || amount_max.clone().is_some_and(|m| largest > m)
            {
                continue;
            }
        }
        if payee.as_ref().is_some_and(|m| !m.matches(tx.payee()))
            || memo.as_ref().is_some_and(|m| !m.matches(tx.memo()))
        {
            continue;
        }
        let has = |key: &str| tx.tags.iter().any(|(k, _)| k == key);
        if !tags.iter().all(|t| {
            tx.tags
                .iter()
                .any(|(k, v)| *k == t.key && t.value.as_ref().is_none_or(|want| want == v))
        }) || !has_tag.iter().all(|k| has(k))
            || lacks_tag.iter().any(|k| has(k))
        {
            continue;
        }
        if status.is_some_and(|s| {
            !tx.postings
                .iter()
                .any(|p| effective_status(tx, p.status) == s)
        }) {
            continue;
        }
        let cash = kabacus_core::reports::cash_effect(&chart, tx, &commodity);
        if cash_filter.is_some_and(|zero| cash.is_zero() != zero) {
            continue;
        }
        if unreserved_only && flags.get(tx).is_empty() {
            continue;
        }
        rows.push((tx, cash));
    }
    rows.sort_by_key(|(tx, _)| (tx.date, tx.id));
    if q.sort.as_deref() == Some("date_desc") {
        rows.reverse();
    } else if let Some(other) = q.sort.as_deref().filter(|s| *s != "date_asc") {
        return Err(invalid(
            "sort",
            format!("{other:?}: use date_asc or date_desc"),
        ));
    }
    Ok(rows
        .into_iter()
        .map(|(tx, cash)| (service::to_dto(tx, flags.get(tx)), cash))
        .collect())
}

/// `search_transactions`: a page of matching transactions.
pub async fn search(
    state: &Service,
    principal: &dyn Caller,
    q: &SearchQuery,
) -> Result<SearchResult, ApiError> {
    require(principal, Role::Read)?;
    let rows = matching(state, q).await?;
    if q.count_only.unwrap_or(false) {
        return Ok(SearchResult::Count {
            total_count: rows.len(),
        });
    }
    let page = page(rows, q.cursor.as_deref(), q.limit, DEFAULT_LIMIT)?;
    let (dtos, cash): (Vec<TransactionDto>, Vec<Money>) = page.items.into_iter().unzip();
    let wants_cash = q
        .fields
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f == "cash_effect"));
    let commodity = service::books_commodity(state).await?;
    let transactions = project(&dtos, q.fields.as_deref(), |i| {
        let mut extra = serde_json::Map::new();
        if wants_cash {
            extra.insert(
                "cash_effect".to_string(),
                serde_json::to_value(service::AmountDto::from(&kabacus_core::Amount::new(
                    &commodity,
                    cash[i].clone(),
                )))
                .unwrap_or_default(),
            );
        }
        extra
    });
    Ok(SearchResult::Page {
        transactions,
        next_cursor: page.next_cursor,
        total_count: page.total_count,
        truncated: page.truncated,
    })
}

/// One write that changed a transaction.
#[derive(Debug, Clone, Serialize)]
pub struct TransactionCommitDto {
    /// The revision it made.
    pub hash: String,
    pub date: String,
    pub actor: Option<String>,
    pub kind: Option<String>,
    pub summary: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetTransactionDto {
    #[serde(flatten)]
    pub transaction: TransactionDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<TransactionCommitDto>>,
}

/// One transaction by its id, with the writes that changed it.
pub async fn get_transaction(
    state: &Service,
    principal: &dyn Caller,
    id: &str,
    include_history: bool,
) -> Result<GetTransactionDto, ApiError> {
    use kabacus_store::BookStore;
    require(principal, Role::Read)?;
    let all = ops::read_all_transactions(state).await?;
    let wanted = ops::parse_id(id)?;
    let tx = all
        .iter()
        .find(|t| t.id == wanted)
        .ok_or_else(|| ApiError::NotFound(id.to_string()))?;
    let chart = service::books_chart_from(state, &all).await?;
    let flags = UnreservedIndex::new(&chart, &all);
    let history = if include_history {
        Some(
            state
                .store
                .transaction_history(state.ledger, wanted)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?
                .into_iter()
                .map(|c| TransactionCommitDto {
                    hash: c.revision.to_string(),
                    date: c.at,
                    actor: Some(c.actor),
                    kind: Some(c.kind),
                    summary: c.summary,
                    user: c.user,
                })
                .collect(),
        )
    } else {
        None
    };
    Ok(GetTransactionDto {
        transaction: service::to_dto(tx, flags.get(tx)),
        history,
    })
}

/// A page of one account's register.
#[derive(Debug, Clone, Serialize)]
pub struct RegisterPageDto {
    pub lines: Vec<service::RegisterLineDto>,
    pub next_cursor: Option<String>,
    pub total_count: usize,
    pub truncated: bool,
}

pub async fn register_page(
    state: &Service,
    principal: &dyn Caller,
    account: &str,
    from: Option<&str>,
    to: Option<&str>,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<RegisterPageDto, ApiError> {
    let lines = service::register(state, principal, account, from, to).await?;
    let page = page(lines, cursor, limit, DEFAULT_LIMIT)?;
    Ok(RegisterPageDto {
        lines: page.items,
        next_cursor: page.next_cursor,
        total_count: page.total_count,
        truncated: page.truncated,
    })
}

/// Whether `account` is shown at `depth` (0: every depth).
pub fn within_depth(account: &AccountName, depth: Option<usize>) -> bool {
    depth.is_none_or(|d| d == 0 || account.segments().count() <= d)
}
