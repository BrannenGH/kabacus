// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bridges kabacus-core's pure logic with the store: every write
//! operation, applied to the books' working copy ([`crate::books`]), then
//! checked (a dry run) or committed. A single write is a batch of one, so
//! the API, the MCP tools and `batch` share one writer.

use crate::auth::roles::{Channel, Principal};
use crate::books::{Change, WorkingCopy};
use crate::config::ClientsConfig;
use crate::state::AppState;
use chrono::NaiveDate;
use kabacus_core::{
    carry_over_with, check_accounts_declared, check_description, resolve_amounts_by_kind,
    AccountKind, AccountName, Amount, AssertionEdit, BalanceAssertion, DraftPosting,
    MissingReserve, Money, Posting, PostingKind, Status, Tag, Transaction, ValidationError,
};
use kabacus_ledger::CheckFailure;
use kabacus_store::{BookStore, ChangeSet, StoreError, Who};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("{0:?}")]
    ValidationMany(Vec<ValidationError>),
    #[error("transaction {0} not found")]
    NotFound(String),
    /// The books moved on from the revision the caller read.
    #[error("the books changed since {expected}; they are at {current}")]
    Conflict { expected: String, current: String },
    /// A rule kabacus checks before the books' own checks.
    #[error("invalid: {message}")]
    Invalid { rule: String, message: String },
    /// The books would fail the strict check.
    #[error("{}", .0.iter().map(CheckFailure::message).collect::<Vec<_>>().join("; "))]
    Check(Vec<CheckFailure>),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    /// The account is still referred to.
    #[error("{account} is still in use: {counts:?}")]
    AccountInUse {
        account: String,
        counts: BTreeMap<String, usize>,
    },
}

impl OpError {
    pub fn invalid(rule: &str, message: impl Into<String>) -> OpError {
        OpError::Invalid {
            rule: rule.to_string(),
            message: message.into(),
        }
    }

    fn from_store(e: StoreError) -> OpError {
        match e {
            StoreError::Conflict { expected, current } => OpError::Conflict {
                expected: expected.to_string(),
                current: current.to_string(),
            },
            other => OpError::Store(other),
        }
    }
}

/// The write's actor: `mcp` for anything that came through `/mcp`, `app`
/// for the configured app client, otherwise the raw client id (in no-auth
/// mode, the client's own hint).
pub fn actor(clients: &ClientsConfig, principal: &Principal) -> String {
    let client = principal.client.as_str();
    if principal.channel == Channel::Mcp {
        "mcp".to_string()
    } else if Some(client) == clients.app.as_deref() {
        "app".to_string()
    } else {
        client.to_string()
    }
}

/// Who made a write, beside the user and actor: what the audit log keeps
/// of the caller's sign-in.
pub fn who_details(principal: &Principal) -> serde_json::Value {
    let mut details = serde_json::json!({
        "user": principal.username,
        "client": principal.client,
        "session": principal.session.clone().unwrap_or_else(|| "none".to_string()),
    });
    if principal.auth_none {
        details["auth"] = serde_json::json!("none");
    } else {
        details["issuer"] = serde_json::json!(principal.issuer);
    }
    details
}

/// The audit log's record of a write by `principal`.
pub async fn who(
    state: &AppState,
    principal: &Principal,
    kind: &str,
    summary: &str,
) -> Result<Who, OpError> {
    Ok(Who {
        user: state.user_of(principal).await?,
        actor: actor(&state.config.auth.clients, principal),
        kind: kind.to_string(),
        summary: summary.to_string(),
        details: who_details(principal),
    })
}

pub async fn read_all_transactions(state: &AppState) -> Result<Vec<Transaction>, OpError> {
    Ok(state.books_snapshot().await?.transactions.clone())
}

/// The books as they were at revision `rev`.
pub async fn transactions_at(state: &AppState, rev: &str) -> Result<Vec<Transaction>, OpError> {
    let revision = crate::books::parse_head(rev)
        .ok_or_else(|| OpError::invalid("revision", format!("{rev:?} isn't a revision")))?;
    match state.books_at(Some(revision)).await {
        Ok(snapshot) => Ok(snapshot.transactions.clone()),
        Err(e) => match e.downcast::<StoreError>() {
            Ok(StoreError::NotFound(what)) => Err(OpError::NotFound(what)),
            Ok(other) => Err(OpError::Store(other)),
            Err(other) => Err(OpError::Other(other)),
        },
    }
}

/// A transaction id as callers write it.
pub fn parse_id(id: &str) -> Result<u64, OpError> {
    id.trim()
        .parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| OpError::NotFound(id.to_string()))
}

// --- requests -----------------------------------------------------------------

pub struct AddRequest {
    pub date: NaiveDate,
    pub description: String,
    pub postings: Vec<DraftPosting>,
    pub tags: Vec<(String, String)>,
}

pub struct EditRequest {
    pub id: u64,
    pub date: NaiveDate,
    pub description: String,
    pub postings: Vec<DraftPosting>,
    /// `None` keeps the transaction's tags.
    pub tags: Option<Vec<Tag>>,
}

/// What the caller wants from `POST /transactions/{id}/corrections`: a
/// correction, which is an edit in place, or a void, which adds the
/// transaction's reversal.
pub enum CorrectionRequest {
    Correct {
        description: String,
        postings: Vec<DraftPosting>,
    },
    Void,
}

/// What `reconcile` writes in `assert` and `adjust` mode.
pub struct ReconcileWrite {
    pub account: AccountName,
    pub date: NaiveDate,
    /// The balance asserted: the ledger's (after any adjustment).
    pub balance: Amount,
    pub inclusive: bool,
    /// `adjust`: the difference posted to `equity:adjustments` first.
    pub adjustment: Option<Money>,
    pub mark_cleared: bool,
}

/// One write operation, as the API, the MCP tools and `batch` name them.
pub enum WriteOp {
    Add(AddRequest),
    Edit(EditRequest),
    Correct {
        id: u64,
        request: CorrectionRequest,
        today: NaiveDate,
    },
    SetStatus {
        id: u64,
        account: AccountName,
        status: Status,
    },
    Allocate {
        from: String,
        to: String,
        amount: Money,
        date: NaiveDate,
        description: Option<String>,
        /// `to` is a new fund, declared by this write.
        new_fund: bool,
    },
    CardPurchase {
        card: String,
        expense_account: String,
        amount: Money,
        date: NaiveDate,
        description: String,
        source: String,
        tags: Vec<Tag>,
    },
    PayCard {
        card: String,
        amount: Money,
        date: NaiveDate,
    },
    Reconcile(ReconcileWrite),
    // Rewrite-class: behind the rewrite gates.
    Rewrite(EditRequest),
    Delete {
        id: u64,
    },
    DeclareAccount {
        account: String,
        tags: Vec<Tag>,
    },
    SetAccountType {
        account: String,
        synthetic: bool,
    },
    RenameAccount {
        from: String,
        to: String,
        merge: bool,
        allow_kind_change: bool,
    },
    RemoveAccount {
        account: String,
    },
    SetBalanceAssertion {
        id: u64,
        account: String,
        balance: Option<Amount>,
    },
    ConvertMoneyAssertions,
}

impl WriteOp {
    /// Account and history rewrites, behind `ledger-rewrite`.
    pub fn is_rewrite(&self) -> bool {
        matches!(
            self,
            WriteOp::Rewrite(_)
                | WriteOp::Delete { .. }
                | WriteOp::DeclareAccount { .. }
                | WriteOp::SetAccountType { .. }
                | WriteOp::RenameAccount { .. }
                | WriteOp::RemoveAccount { .. }
                | WriteOp::SetBalanceAssertion { .. }
                | WriteOp::ConvertMoneyAssertions
        )
    }
}

/// What applying one operation did.
#[derive(Debug, Clone)]
pub struct Applied {
    /// The main transaction's id (none for account operations).
    pub id: Option<u64>,
    /// The kind of change, as the audit log records it.
    pub kind: &'static str,
    pub subject: String,
    pub changes: Vec<Change>,
    /// The transactions as they were, and as they are now.
    pub before: Vec<Transaction>,
    pub after: Vec<Transaction>,
    /// Operation-specific preview (rename counts and samples, ...).
    pub details: serde_json::Value,
    pub warnings: Vec<OpWarning>,
}

impl Applied {
    pub fn new(id: Option<u64>, kind: &'static str, subject: String) -> Applied {
        Applied {
            id,
            kind,
            subject,
            changes: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            details: serde_json::Value::Null,
            warnings: Vec::new(),
        }
    }
}

/// Something about a write the caller should know; never a refusal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OpWarning {
    pub code: String,
    pub message: String,
}

/// Postings from drafts, with amounts resolved and balanced, each keeping
/// what [`kabacus_core::carry_over`] carries over from `original` (empty
/// for a new transaction): comment, tags, kind and balance assertion. A
/// new assertion on a money account is inclusive.
fn build_postings(
    wc: &WorkingCopy,
    drafts: &[DraftPosting],
    original: &[Posting],
) -> Result<Vec<Posting>, OpError> {
    let chart = wc.rules_chart();
    let carried = carry_over_with(original, drafts, |account| {
        chart.kind_of(account) == AccountKind::Money
    })?;
    let kinds: Vec<PostingKind> = carried.iter().map(|c| c.kind).collect();
    let amounts = resolve_amounts_by_kind(drafts, &kinds)?;
    let costs: Vec<Option<kabacus_core::Cost>> = drafts
        .iter()
        .map(|d| {
            original
                .iter()
                .find(|p| p.account == d.account && Some(&p.amount) == d.amount.as_ref())
                .and_then(|p| p.cost.clone())
        })
        .collect();
    Ok(drafts
        .iter()
        .zip(amounts)
        .zip(carried)
        .zip(costs)
        .map(|(((d, amount), carried), cost)| Posting {
            account: d.account.clone(),
            amount,
            status: d.status,
            kind: carried.kind,
            comment: carried.comment,
            tags: carried.tags,
            cost,
            balance_assertion: carried.balance_assertion,
        })
        .collect())
}

fn check_declared(wc: &WorkingCopy, drafts: &[DraftPosting]) -> Result<(), OpError> {
    let declared: std::collections::HashSet<AccountName> = wc.declared().into_iter().collect();
    let accounts: Vec<AccountName> = drafts.iter().map(|p| p.account.clone()).collect();
    check_accounts_declared(accounts.iter(), &declared).map_err(OpError::ValidationMany)
}

fn new_transaction(
    date: NaiveDate,
    description: String,
    postings: Vec<Posting>,
    tags: Vec<Tag>,
) -> Transaction {
    Transaction {
        id: 0,
        date,
        status: Status::Unmarked,
        code: String::new(),
        description,
        comment: String::new(),
        tags,
        postings,
    }
}

/// `edit: 2022-10-24 McDonald's [#42]`.
fn subject(verb: &str, tx: &Transaction, id: u64) -> String {
    format!(
        "{verb}: {} {} [#{id}]",
        tx.date.format("%Y-%m-%d"),
        tx.payee()
    )
}

fn added(wc: &mut WorkingCopy, tx: Transaction, applied: &mut Applied) -> Result<u64, OpError> {
    let change = wc.add(tx)?;
    let id = change.id.expect("a transaction");
    applied.changes.push(change);
    applied
        .after
        .push(wc.tx(wc.find(id).expect("just added")).clone());
    Ok(id)
}

fn add_one(wc: &mut WorkingCopy, req: AddRequest, kind: &'static str) -> Result<Applied, OpError> {
    let postings = build_postings(wc, &req.postings, &[])?;
    check_description(&req.description)?;
    check_declared(wc, &req.postings)?;
    let tx = new_transaction(req.date, req.description, postings, req.tags);
    let mut applied = Applied::new(None, kind, String::new());
    let id = added(wc, tx, &mut applied)?;
    applied.id = Some(id);
    applied.subject = subject("add", &applied.after[0], id);
    Ok(applied)
}

fn add_planned(
    wc: &mut WorkingCopy,
    plan: kabacus_core::PlannedTransaction,
    date: NaiveDate,
    tags: Vec<Tag>,
) -> Result<Applied, OpError> {
    let commodity = wc.commodity();
    add_one(
        wc,
        AddRequest {
            date,
            description: plan.description,
            postings: plan
                .postings
                .into_iter()
                .map(|(account, money)| DraftPosting {
                    account,
                    amount: Some(Amount::new(&commodity, money)),
                    status: Status::Unmarked,
                    balance_assertion: AssertionEdit::Keep,
                })
                .collect(),
            tags,
        },
        "add",
    )
}

fn find(wc: &WorkingCopy, id: u64) -> Result<usize, OpError> {
    wc.find(id).ok_or_else(|| OpError::NotFound(id.to_string()))
}

/// An edit (or a rewrite): the transaction replaced in place. What the
/// caller doesn't give is kept from the original.
fn edit(wc: &mut WorkingCopy, req: EditRequest, verb: &'static str) -> Result<Applied, OpError> {
    let i = find(wc, req.id)?;
    let original = wc.tx(i).clone();
    let new_postings = build_postings(wc, &req.postings, &original.postings)?;
    check_description(&req.description)?;
    check_declared(wc, &req.postings)?;
    let tags = req.tags.unwrap_or_else(|| original.tags.clone());
    let tx = Transaction {
        date: req.date,
        description: req.description,
        tags,
        postings: new_postings,
        ..original.clone()
    };
    let mut applied = Applied::new(Some(original.id), verb, subject(verb, &tx, original.id));
    let before = wc.book.transactions.clone();
    let change = wc.replace(i, tx)?;
    check_edited_allocation(wc, &before, &original, wc.tx(i))?;
    applied.changes.push(change);
    applied.before.push(original);
    applied.after.push(wc.tx(i).clone());
    Ok(applied)
}

/// A correction is an edit in place; a void adds the transaction's
/// reversal, dated `today`, whose comment says what it reverses.
fn correct(
    wc: &mut WorkingCopy,
    id: u64,
    request: CorrectionRequest,
    today: NaiveDate,
) -> Result<Applied, OpError> {
    let i = find(wc, id)?;
    let original = wc.tx(i).clone();
    match request {
        CorrectionRequest::Correct {
            description,
            postings,
        } => {
            let mut applied = edit(
                wc,
                EditRequest {
                    id,
                    date: original.date,
                    description,
                    postings,
                    tags: None,
                },
                "correct",
            )?;
            applied.subject = subject("correct", &applied.after[0], id);
            Ok(applied)
        }
        CorrectionRequest::Void => {
            let reversal = kabacus_core::build_reversal(&original, today);
            let mut applied = Applied::new(None, "void", String::new());
            let new_id = added(wc, reversal, &mut applied)?;
            applied.id = Some(new_id);
            applied.subject = format!(
                "void: {} {} [#{id}]",
                today.format("%Y-%m-%d"),
                original.payee()
            );
            Ok(applied)
        }
    }
}

fn set_status(
    wc: &mut WorkingCopy,
    id: u64,
    account: &AccountName,
    status: Status,
) -> Result<Applied, OpError> {
    let i = find(wc, id)?;
    let before = wc.tx(i).clone();
    let change = wc.set_status(i, account, status)?;
    let mut applied = Applied::new(
        Some(id),
        "status",
        format!(
            "status: {} {} {account} -> {} [#{id}]",
            before.date.format("%Y-%m-%d"),
            before.payee(),
            crate::service::status_str(status)
        ),
    );
    applied.changes.push(change);
    applied.before.push(before);
    applied.after.push(wc.tx(i).clone());
    Ok(applied)
}

/// Declare the new fund an allocation sets money aside into, in the same
/// write. Not a rewrite, unlike declaring any other account: core only
/// allows a new fund directly in the money account the money comes from,
/// which changes nothing already recorded.
fn declare_new_fund(wc: &mut WorkingCopy, from: &str, fund: &str) -> Result<(), OpError> {
    kabacus_core::check_new_fund(&wc.chart(), from, fund)
        .map_err(|e| funds_error("allocate", e))?;
    kabacus_core::render_account_declaration(fund, &[])
        .map_err(|message| OpError::invalid("account", message))?;
    wc.declare(fund, "");
    Ok(())
}

fn funds_error(rule: &str, e: kabacus_core::FundsError) -> OpError {
    OpError::invalid(rule, e.to_string())
}

/// `account`'s lowest balance at the end of `from` or any later day, and
/// the day it's reached.
fn lowest_from(
    txs: &[Transaction],
    account: &AccountName,
    commodity: &str,
    from: NaiveDate,
) -> (NaiveDate, Money) {
    let mut by_day: BTreeMap<NaiveDate, Money> = BTreeMap::new();
    for tx in txs {
        for p in tx
            .postings
            .iter()
            .filter(|p| p.account == *account && p.amount.commodity == commodity)
        {
            let day = by_day.entry(tx.date.max(from)).or_insert_with(Money::zero);
            *day = day.clone() + p.amount.money.clone();
        }
    }
    let mut running = Money::zero();
    let mut lowest = (from, Money::zero());
    let mut first = true;
    for (day, change) in by_day {
        running = running + change;
        if first || running < lowest.1 {
            lowest = (day, running.clone());
        }
        first = false;
    }
    lowest
}

/// "Checking has only $40.00 on 2026-02-17, $10.00 short of $50.00".
fn shortfall(
    wc: &WorkingCopy,
    account: &AccountName,
    available: Money,
    day: NaiveDate,
    amount: &Money,
) -> OpError {
    let commodity = wc.commodity();
    let places = wc.places(&commodity);
    let shown = |m: Money| kabacus_core::format_money(&Amount::new(&commodity, m.rescaled(places)));
    OpError::invalid(
        "amount",
        format!(
            "{} has only {} on {}, {} short of {}",
            wc.chart().display_name(account),
            shown(available.clone()),
            day.format("%Y-%m-%d"),
            shown(amount.clone() - available),
            shown(amount.clone()),
        ),
    )
}

/// An allocation may take only what its source holds from its date on:
/// the source's lowest balance at the end of that day or any later one
/// must cover it, so a later spend isn't left overdrawn.
fn check_allocation_covered(
    wc: &WorkingCopy,
    from: &str,
    amount: &Money,
    date: NaiveDate,
) -> Result<(), OpError> {
    let from = AccountName::new(from);
    let (day, low) = lowest_from(&wc.book.transactions, &from, &wc.commodity(), date);
    if low >= *amount {
        return Ok(());
    }
    Err(shortfall(wc, &from, low, day, amount))
}

/// An edit that leaves a transaction moving money only between a money
/// account and its funds or card reserves is held to the same rule as a
/// new allocation: it mustn't take a source lower than it was, below zero.
fn check_edited_allocation(
    wc: &WorkingCopy,
    before: &[Transaction],
    original: &Transaction,
    edited: &Transaction,
) -> Result<(), OpError> {
    let chart = wc.chart();
    let commodity = wc.commodity();
    let kinds: Vec<AccountKind> = edited
        .postings
        .iter()
        .map(|p| chart.kind_of(&p.account))
        .collect();
    let moves_funds = kinds
        .iter()
        .all(|k| *k == AccountKind::Money || k.is_envelope())
        && kinds.iter().any(|k| k.is_envelope());
    if !moves_funds {
        return Ok(());
    }
    let from = original.date.min(edited.date);
    for p in edited
        .postings
        .iter()
        .filter(|p| p.amount.commodity == commodity && p.amount.money.is_negative())
    {
        let (_, was) = lowest_from(before, &p.account, &commodity, from);
        let (day, now) = lowest_from(&wc.book.transactions, &p.account, &commodity, from);
        if now.is_negative() && now < was {
            let taken = -p.amount.money.clone();
            return Err(shortfall(wc, &p.account, now + taken.clone(), day, &taken));
        }
    }
    Ok(())
}

/// The comment of a declaration with `tags`, as stored.
fn declaration_tags(tags: &[Tag]) -> String {
    tags.iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn declare_account(wc: &mut WorkingCopy, account: &str, tags: &[Tag]) -> Result<Applied, OpError> {
    kabacus_core::AccountDeclaration {
        name: AccountName::new(account),
        tags: tags.to_vec(),
    }
    .synthetic_override()
    .map_err(|message| OpError::invalid("synthetic", message))?;
    let line = kabacus_core::render_account_declaration(account, tags)
        .map_err(|message| OpError::invalid("account", message))?;
    if wc.is_declared(account) {
        return Err(OpError::invalid(
            "account",
            format!("{account} is already declared"),
        ));
    }
    let change = wc.declare(account, &declaration_tags(tags));
    let mut applied = Applied::new(None, "rewrite", format!("declare account: {account}"));
    applied.changes.push(change);
    applied.details = serde_json::json!({ "account": account, "declaration": line });
    Ok(applied)
}

fn set_account_type(
    wc: &mut WorkingCopy,
    account: &str,
    synthetic: bool,
) -> Result<Applied, OpError> {
    let declaration = wc
        .book
        .accounts
        .get(account)
        .ok_or_else(|| OpError::NotFound(account.to_string()))?;
    let chart = wc.chart();
    let old = chart.get(account).is_some_and(|c| c.synthetic);
    let mut tags = kabacus_core::declared_tags(&declaration.tags);
    tags.retain(|(key, _)| key != "synthetic");
    tags.push(("synthetic".to_string(), synthetic.to_string()));
    let (mut comment, missing) = kabacus_core::reconcile_tags(&declaration.tags, &tags);
    if !missing.is_empty() {
        if !comment.is_empty() {
            comment.push('\n');
        }
        comment.push_str(&declaration_tags(&missing));
    }
    let change = wc.declare(account, &comment);
    let mut applied = Applied::new(None, "rewrite", format!("set account type: {account}"));
    applied.changes.push(change);
    applied.details = serde_json::json!({
        "account": account,
        "before_synthetic": old,
        "synthetic": synthetic,
    });
    Ok(applied)
}

fn delete(wc: &mut WorkingCopy, id: u64) -> Result<Applied, OpError> {
    let i = find(wc, id)?;
    let tx = wc.tx(i).clone();
    let change = wc.delete(i)?;
    let mut applied = Applied::new(Some(id), "delete", subject("delete", &tx, id));
    applied.changes.push(change);
    applied.before.push(tx);
    Ok(applied)
}

/// A write blocked by an out-of-date balance assertion goes through once
/// that assertion is removed or updated (the failure names its
/// transaction and account).
fn set_balance_assertion(
    wc: &mut WorkingCopy,
    id: u64,
    account: &str,
    balance: Option<Amount>,
) -> Result<Applied, OpError> {
    let i = wc
        .find(id)
        .ok_or_else(|| OpError::invalid("assertion", format!("there is no transaction #{id}")))?;
    let before = wc.tx(i).clone();
    let mut tx = before.clone();
    let account_name = AccountName::new(account);
    let posting = tx
        .postings
        .iter()
        .position(|p| p.account == account_name && p.balance_assertion.is_some())
        .or_else(|| tx.postings.iter().position(|p| p.account == account_name))
        .ok_or_else(|| {
            OpError::invalid(
                "assertion",
                format!("transaction #{id} has no posting to {account}"),
            )
        })?;
    // A new assertion on a money account is inclusive.
    let inclusive = wc.rules_chart().kind_of(&account_name) == AccountKind::Money;
    let p = &mut tx.postings[posting];
    p.balance_assertion = balance.clone().map(|amount| match &p.balance_assertion {
        Some(old) => BalanceAssertion {
            amount,
            ..old.clone()
        },
        None => BalanceAssertion {
            inclusive,
            ..BalanceAssertion::new(amount)
        },
    });
    let subject = match &balance {
        Some(amount) => format!(
            "assertion: {account} = {} [#{id}]",
            kabacus_ledger::render_money(amount, &wc.styles())
        ),
        None => format!("assertion: remove {account} [#{id}]"),
    };
    let mut applied = Applied::new(Some(id), "rewrite", subject);
    applied.changes.push(wc.replace(i, tx)?);
    applied.before.push(before);
    applied.after.push(wc.tx(i).clone());
    Ok(applied)
}

/// Every exclusive balance assertion on a money account made inclusive
/// (`=` to `=*`, `==` to `==*`).
fn convert_money_assertions(wc: &mut WorkingCopy) -> Result<Applied, OpError> {
    let chart = wc.rules_chart();
    let targets: Vec<usize> = wc
        .book
        .transactions
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.postings.iter().any(|p| {
                p.balance_assertion.as_ref().is_some_and(|a| !a.inclusive)
                    && chart.kind_of(&p.account) == AccountKind::Money
            })
        })
        .map(|(i, _)| i)
        .collect();
    let mut converted = 0;
    let mut applied = Applied::new(None, "rewrite", String::new());
    let mut transactions = Vec::new();
    for i in targets {
        let before = wc.tx(i).clone();
        let mut tx = before.clone();
        for p in &mut tx.postings {
            if let Some(a) = &mut p.balance_assertion {
                if !a.inclusive && chart.kind_of(&p.account) == AccountKind::Money {
                    a.inclusive = true;
                    converted += 1;
                }
            }
        }
        transactions.push(tx.id);
        applied.changes.push(wc.replace(i, tx)?);
        applied.before.push(before);
        applied.after.push(wc.tx(i).clone());
    }
    if converted == 0 {
        return Err(OpError::invalid(
            "assertion",
            "every balance assertion on a money account is already inclusive (=*)",
        ));
    }
    applied.subject = format!("assertions: make {converted} money account assertion(s) inclusive");
    applied.details = serde_json::json!({
        "converted": converted,
        "transactions": transactions,
    });
    Ok(applied)
}

fn reconcile(wc: &mut WorkingCopy, r: ReconcileWrite) -> Result<Applied, OpError> {
    let commodity = r.balance.commodity.clone();
    let places = r.balance.money.places();
    let mut applied = Applied::new(
        None,
        "reconcile",
        format!(
            "reconcile: {} {} = {}",
            r.date.format("%Y-%m-%d"),
            r.account,
            kabacus_ledger::render_money(&r.balance, &wc.styles())
        ),
    );
    if let Some(adjustment) = r.adjustment {
        let adjustments = AccountName::new("equity:adjustments");
        if !wc.is_declared(adjustments.as_str()) {
            return Err(OpError::invalid(
                "reconcile",
                "equity:adjustments isn't declared; declare it (declare_account) before adjusting",
            ));
        }
        let tx = new_transaction(
            r.date,
            format!("Reconciliation adjustment | {}", r.account),
            vec![
                plain_posting(&r.account, Amount::new(&commodity, adjustment.clone())),
                plain_posting(&adjustments, Amount::new(&commodity, -adjustment)),
            ],
            Vec::new(),
        );
        added(wc, tx, &mut applied)?;
    }
    let mut posting = plain_posting(
        &r.account,
        Amount::new(&commodity, Money::from_decimal(0, places)),
    );
    posting.balance_assertion = Some(BalanceAssertion {
        inclusive: r.inclusive,
        ..BalanceAssertion::new(r.balance.clone())
    });
    let tx = new_transaction(
        r.date,
        format!("Reconciliation | {}", r.account),
        vec![posting],
        vec![(
            "reconciled".to_string(),
            r.date.format("%Y-%m-%d").to_string(),
        )],
    );
    let id = added(wc, tx, &mut applied)?;
    applied.id = Some(id);
    if r.mark_cleared {
        let targets: Vec<usize> = wc
            .book
            .transactions
            .iter()
            .enumerate()
            .filter(|(_, t)| t.date <= r.date && t.status != Status::Cleared)
            .filter(|(_, t)| {
                t.postings
                    .iter()
                    .find(|p| p.account == r.account)
                    .is_some_and(|p| p.status != Status::Cleared)
            })
            .map(|(i, _)| i)
            .collect();
        for &i in &targets {
            wc.set_status(i, &r.account, Status::Cleared)?;
        }
        applied.details = serde_json::json!({ "marked_cleared": targets.len() });
    }
    Ok(applied)
}

fn plain_posting(account: &AccountName, amount: Amount) -> Posting {
    Posting {
        account: account.clone(),
        amount,
        status: Status::Unmarked,
        kind: PostingKind::Real,
        comment: String::new(),
        tags: Vec::new(),
        cost: None,
        balance_assertion: None,
    }
}

/// Apply one operation to the working copy. Each operation checks
/// everything it can before changing anything, so a refused operation
/// leaves the working copy as it was.
pub fn apply(wc: &mut WorkingCopy, op: WriteOp) -> Result<Applied, OpError> {
    // Some checks run on what the operation wrote, so a refusal puts the
    // books back.
    let (book, next_id) = (wc.book.clone(), wc.next_id);
    let applied = (|| {
        let applied = apply_unchecked(wc, op)?;
        check_places(wc, &applied)?;
        Ok(applied)
    })();
    if applied.is_err() {
        (wc.book, wc.next_id) = (book, next_id);
    }
    applied
}

fn apply_unchecked(wc: &mut WorkingCopy, op: WriteOp) -> Result<Applied, OpError> {
    match op {
        WriteOp::Add(req) => add_one(wc, req, "add"),
        WriteOp::Edit(req) => edit(wc, req, "edit"),
        WriteOp::Rewrite(req) => edit(wc, req, "rewrite"),
        WriteOp::Correct { id, request, today } => correct(wc, id, request, today),
        WriteOp::SetStatus {
            id,
            account,
            status,
        } => set_status(wc, id, &account, status),
        WriteOp::Allocate {
            from,
            to,
            amount,
            date,
            description,
            new_fund,
        } => {
            if new_fund {
                declare_new_fund(wc, &from, &to)?;
            }
            let plan = kabacus_core::plan_allocation(
                &wc.chart(),
                &from,
                &to,
                amount.clone(),
                description.as_deref(),
            )
            .map_err(|e| funds_error("allocate", e))?;
            check_allocation_covered(wc, &from, &amount, date)?;
            add_planned(wc, plan, date, vec![])
        }
        WriteOp::CardPurchase {
            card,
            expense_account,
            amount,
            date,
            description,
            source,
            tags,
        } => {
            let plan = kabacus_core::plan_card_purchase(
                &wc.chart(),
                &card,
                &expense_account,
                amount,
                &kabacus_core::PurchaseSource::parse(&source),
                &description,
            )
            .map_err(|e| funds_error("card", e))?;
            add_planned(wc, plan, date, tags)
        }
        WriteOp::PayCard { card, amount, date } => {
            let plan = kabacus_core::plan_card_payment(&wc.chart(), &card, amount)
                .map_err(|e| funds_error("card", e))?;
            add_planned(wc, plan, date, vec![])
        }
        WriteOp::Reconcile(r) => reconcile(wc, r),
        WriteOp::Delete { id } => delete(wc, id),
        WriteOp::DeclareAccount { account, tags } => declare_account(wc, &account, &tags),
        WriteOp::SetAccountType { account, synthetic } => set_account_type(wc, &account, synthetic),
        WriteOp::RenameAccount {
            from,
            to,
            merge,
            allow_kind_change,
        } => crate::account_edit::rename(wc, &from, &to, merge, allow_kind_change),
        WriteOp::RemoveAccount { account } => crate::account_edit::remove(wc, &account),
        WriteOp::SetBalanceAssertion {
            id,
            account,
            balance,
        } => set_balance_assertion(wc, id, &account, balance),
        WriteOp::ConvertMoneyAssertions => convert_money_assertions(wc),
    }
}

/// Refuse an amount with more decimal places than its commodity takes.
/// Only an import raises a commodity's places.
fn check_places(wc: &WorkingCopy, applied: &Applied) -> Result<(), OpError> {
    let needed = kabacus_ledger::places_needed(&wc.book, &applied.after);
    let Some((symbol, need)) = needed.into_iter().next() else {
        return Ok(());
    };
    let takes = wc
        .book
        .commodities
        .get(&symbol)
        .and_then(|c| c.style.places)
        .unwrap_or_default();
    Err(OpError::invalid(
        "amount",
        format!(
            "{} on {} has more decimal places than {symbol} takes ({takes})",
            kabacus_ledger::render_money(&need.amount, &wc.styles()),
            need.account
        ),
    ))
}

// --- results ----------------------------------------------------------------------

/// The cap on `balance_changes`.
pub const MAX_BALANCE_CHANGES: usize = 200;

/// Touched accounts' balances before and after a set of changes.
pub fn balance_changes(
    wc: &WorkingCopy,
    before: &[Transaction],
    after: &[Transaction],
) -> (Vec<kabacus_core::BalanceChange>, bool) {
    let mut delta: BTreeMap<(String, String), Money> = BTreeMap::new();
    for (txs, add) in [(after, true), (before, false)] {
        for p in txs.iter().flat_map(|t| &t.postings) {
            let d = delta
                .entry((p.account.as_str().to_string(), p.amount.commodity.clone()))
                .or_insert_with(Money::zero);
            *d = if add {
                d.clone() + p.amount.money.clone()
            } else {
                d.clone() - p.amount.money.clone()
            };
        }
    }
    let keys: BTreeSet<(String, String)> = delta.keys().cloned().collect();
    let now = wc.balances(&keys);
    let truncated = delta.len() > MAX_BALANCE_CHANGES;
    let changes = delta
        .into_iter()
        .take(MAX_BALANCE_CHANGES)
        .map(|((account, commodity), d)| {
            let after = now[&(account.clone(), commodity.clone())].clone();
            let places = after.places().max(d.places());
            kabacus_core::BalanceChange {
                account,
                before: Amount::new(&commodity, (after.clone() - d).rescaled(places)),
                after: Amount::new(&commodity, after.rescaled(places)),
            }
        })
        .collect();
    (changes, truncated)
}

/// What the written transactions still owe their card reserves.
pub fn missing_reserves_of(wc: &WorkingCopy, written: &[Transaction]) -> Vec<MissingReserve> {
    let chart = wc.rules_chart();
    written
        .iter()
        .flat_map(|t| chart.missing_reserves(t))
        .collect()
}

/// A finished write: committed, or checked in a dry run.
#[derive(Debug, Clone)]
pub struct WriteResult {
    pub id: Option<u64>,
    pub head: String,
    pub dry_run: bool,
    pub changes: Vec<Change>,
    /// Each touched account's balance before and after.
    pub balances: Vec<kabacus_core::BalanceChange>,
    pub balances_truncated: bool,
    pub cash_on_hand: (Amount, Amount),
    pub missing_reserve: Vec<MissingReserve>,
    pub diff: String,
    pub details: serde_json::Value,
    pub warnings: Vec<OpWarning>,
}

/// The strict check of the working copy's changes, as the commit makes
/// it: only what they can affect is checked.
pub fn check(wc: &WorkingCopy) -> Result<(), OpError> {
    let changes = wc.changes();
    let (_, failures) = kabacus_store::checked(&wc.base.book, &changes)?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(OpError::Check(failures))
    }
}

/// Cash on Hand on the same day and commodity on both sides of this write.
pub fn cash_change(wc: &WorkingCopy) -> (Amount, Amount) {
    let commodity = wc.commodity();
    let today = Some(chrono::Local::now().date_naive());
    let totals = |book: &kabacus_ledger::Book| {
        let chart = kabacus_core::Chart::from_books(
            wc.kinds.rules.clone(),
            wc.kinds.reserve_template.clone(),
            book.declared_accounts().cloned(),
            &book.transactions,
            &crate::state::friendly_names(book),
        )
        .with_account_types(&crate::state::account_types(book));
        Amount::new(
            &commodity,
            kabacus_core::reports::cash_totals(&chart, &book.transactions, &commodity, today)
                .cash_on_hand,
        )
    };
    (totals(&wc.base.book), totals(&wc.book))
}

fn result(wc: &WorkingCopy, applied: Applied, head: String, dry_run: bool) -> WriteResult {
    let (balances, balances_truncated) = balance_changes(wc, &applied.before, &applied.after);
    WriteResult {
        missing_reserve: missing_reserves_of(wc, &applied.after),
        id: applied.id,
        head,
        dry_run,
        changes: applied.changes,
        balances,
        balances_truncated,
        cash_on_hand: cash_change(wc),
        diff: wc.diff(),
        details: applied.details,
        warnings: applied.warnings,
    }
}

/// Commit `changes` to the books at `wc`'s revision as one write, and keep
/// the books it leaves in memory for the reads that follow.
pub async fn commit(
    state: &AppState,
    changes: ChangeSet,
    who: &Who,
    expected: kabacus_store::Revision,
) -> Result<kabacus_store::Revision, OpError> {
    let revision = state
        .store
        .commit(state.ledger, expected, changes, who)
        .await
        .map_err(OpError::from_store)?;
    Ok(revision)
}

/// Keep the books `wc` holds as revision `revision`.
pub async fn remember(state: &AppState, wc: &WorkingCopy, revision: kabacus_store::Revision) {
    let snapshot = crate::cache::Snapshot::new(revision, wc.book.clone(), wc.next_id);
    state
        .books_cache
        .put(state.ledger, std::sync::Arc::new(snapshot))
        .await;
}

/// One write, end to end: load the working copy at `if_match`, apply `op`,
/// then check it (a dry run) or commit it.
pub async fn write(
    state: &AppState,
    principal: &Principal,
    op: WriteOp,
    if_match: &str,
    dry_run: bool,
) -> Result<WriteResult, OpError> {
    let rewrite = op.is_rewrite();
    let mut wc = WorkingCopy::load(state, if_match).await?;
    let mut applied = apply(&mut wc, op)?;
    let changes = wc.changes();
    // Saving something exactly as the books already have it (an editor
    // saved without changes) writes nothing. Said the same way for a dry
    // run, so a clean preview still means a clean save.
    if changes.is_empty() {
        applied.warnings.push(OpWarning {
            code: "unchanged".to_string(),
            message: "Nothing changed: the books already say this, so nothing was written"
                .to_string(),
        });
        return Ok(result(&wc, applied, wc.head.clone(), dry_run));
    }
    check(&wc)?;
    if dry_run {
        return Ok(result(&wc, applied, wc.head.clone(), true));
    }
    // A rewrite is recorded as such, or as a delete.
    let kind = match (rewrite, applied.kind) {
        (true, "delete") => "delete",
        (true, _) => "rewrite",
        (false, kind) => kind,
    };
    let who = who(state, principal, kind, &applied.subject).await?;
    let revision = commit(state, changes, &who, wc.revision).await?;
    remember(state, &wc, revision).await;
    Ok(result(&wc, applied, revision.to_string(), false))
}

#[cfg(test)]
mod tests;
