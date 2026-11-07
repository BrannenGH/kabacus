// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every change the app makes: a [`Draft`] is previewed with the
//! server's dry run, shown, then committed with the same call and the same
//! head. The app has no write path of its own; MCP's tools go through the
//! same service functions.

use crate::views::{diff_lines, DiffLine, Shown};
use crate::{i18n, i18nc, i18ncp, i18np};
use kabacus_client::types::{
    AccountView, AddTransactionBody, AllocateBody, BatchOpBody, CardPaymentBody, CardPurchaseBody,
    DraftAmount, DraftPostingBody, TransactionView, WriteResultView,
};
use kabacus_client::{ClientError, KabacusClient};
use kabacus_core::Money;
use serde::{Deserialize, Serialize};

/// What a form wants written. QML sends it as JSON, tagged by `kind`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Draft {
    AccountType {
        account: String,
        synthetic: bool,
    },
    /// Money left an account ("Spent"), came in ("Received"), moved
    /// between banks ("Transfer"), or any two accounts ("Postings").
    Simple {
        date: String,
        payee: String,
        #[serde(default)]
        memo: String,
        from: String,
        to: String,
        amount: String,
    },
    /// With its reserve lines. `source` is `free` or a fund.
    CardPurchase {
        date: String,
        payee: String,
        #[serde(default)]
        memo: String,
        card: String,
        category: String,
        source: String,
        amount: String,
    },
    /// Set aside, release, or move between funds.
    SetAside {
        date: String,
        from: String,
        to: String,
        amount: String,
        #[serde(default)]
        description: String,
    },
    /// Pay a card from its reserve.
    PayCard {
        date: String,
        card: String,
        amount: String,
    },
    /// An expense shared with a person, netted against what
    /// you already owe them.
    Split {
        date: String,
        payee: String,
        #[serde(default)]
        memo: String,
        /// The money account or card it was paid with.
        paid_with: String,
        category: String,
        person: String,
        total: String,
        their_share: String,
    },
    /// Edit a simple transaction in place.
    Edit {
        id: String,
        date: String,
        payee: String,
        #[serde(default)]
        memo: String,
        from: String,
        to: String,
        amount: String,
    },
    EditPostings {
        id: String,
        date: String,
        payee: String,
        #[serde(default)]
        memo: String,
        postings: Vec<EditPosting>,
    },
    /// Only the reversal.
    Void {
        id: String,
    },
    /// Write the reconciled balance assertion once the
    /// statement matches.
    Reconcile {
        account: String,
        date: String,
        balance: String,
    },
    /// Add the missing reserve lines to `ids`, from `source`
    /// (`free` or a fund), as one batch.
    FixReserves {
        card: String,
        ids: Vec<String>,
        source: String,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct EditPosting {
    pub account: String,
    pub amount: String,
    pub commodity: String,
    pub original: Option<usize>,
}

/// Why a draft can't be written, in terms the form can show.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteError {
    /// One field is wrong; `suggestion` is an existing account to use.
    #[error("{message}")]
    Field {
        field: String,
        message: String,
        suggestion: String,
    },
    /// A balance assertion would break.
    #[error("{}", assertion_message(.account))]
    Assertion {
        account: String,
        asserted: String,
        calculated: String,
        transaction: String,
        posting: String,
    },
    /// The books changed since the preview; who changed what.
    #[error("{}", conflict_message(.what))]
    Conflict { who: String, what: String },
    /// The server can't be reached, or anything else.
    #[error("{message}")]
    Other { message: String },
}

fn assertion_message(account: &str) -> String {
    i18nc!(
        "%1 is an account",
        "This would break a balance check on %1",
        account
    )
}

fn conflict_message(what: &str) -> String {
    i18nc!(
        "%1 is the change made meanwhile",
        "The books changed: %1",
        what
    )
}

impl From<ClientError> for WriteError {
    fn from(e: ClientError) -> Self {
        WriteError::Other {
            message: crate::model::ModelError::from(e).user_message(""),
        }
    }
}

impl From<crate::model::ModelError> for WriteError {
    fn from(e: crate::model::ModelError) -> Self {
        WriteError::Other {
            message: e.user_message(""),
        }
    }
}

/// What a draft will change, before anything is written.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    /// The head the dry run was checked against: commit with it.
    pub head: String,
    pub summary: String,
    pub changes: Vec<BalanceChange>,
    pub cash_on_hand: Option<CashChange>,
    pub journal: Vec<DiffLine>,
    /// Checks that passed: shown ticked.
    pub notes: Vec<String>,
    /// What looks wrong but doesn't stop the save (an account the change
    /// would overdraw, a card left unreserved): shown as warnings, never
    /// among the ticked notes, where they read as passed checks.
    pub warnings: Vec<String>,
    /// A card transaction still missing its reserve lines.
    pub unreserved: bool,
    /// The books already say this (an editor opened and not changed):
    /// there is nothing to save, so the form turns Save off.
    pub unchanged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BalanceChange {
    pub account: String,
    pub name: String,
    pub before: Shown,
    pub after: Shown,
}

#[derive(Debug, Clone, Serialize)]
pub struct CashChange {
    pub before: Shown,
    pub after: Shown,
    pub delta: Shown,
}

/// What was written.
#[derive(Debug, Clone, Serialize)]
pub struct Saved {
    pub id: String,
    pub head: String,
    pub short: String,
}

/// A new fund's name as typed, or its account's last part.
fn fund_name(typed: &str, fund: &str) -> String {
    if typed.contains(':') {
        fund.rsplit(':').next().unwrap_or(fund).to_string()
    } else {
        typed.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

fn field(field: &str, message: impl Into<String>) -> WriteError {
    WriteError::Field {
        field: field.to_string(),
        message: message.into(),
        suggestion: String::new(),
    }
}

fn check_date(date: &str) -> Result<(), WriteError> {
    kabacus_core::validate::parse_date(date.trim())
        .map(|_| ())
        .map_err(|_| field("date", i18n!("Use a date like 2026-10-03")))
}

/// What's wrong with an amount typed in a form, as one plain
/// sentence; empty when it's fine. `positive`: it must be more than zero
/// (every form but Reconcile).
pub fn amount_problem(text: &str, places: u32, positive: bool) -> String {
    match check_amount("amount", text, places, positive) {
        Ok(_) => String::new(),
        Err(WriteError::Field { message, .. }) => message,
        Err(e) => e.to_string(),
    }
}

/// An amount typed in a form, parsed by core's one grammar.
fn check_amount(name: &str, text: &str, places: u32, positive: bool) -> Result<Money, WriteError> {
    match kabacus_core::parse_amount(text, places) {
        Ok(m) if positive && !m.is_positive() => Err(field(name, i18n!("Must be more than zero"))),
        Ok(m) => Ok(m),
        Err(e) => Err(field(name, amount_error(&e))),
    }
}

/// Core's words for a refused amount, in the user's language.
fn amount_error(e: &kabacus_core::money::AmountError) -> String {
    use kabacus_core::money::AmountError;
    match e {
        AmountError::Empty => i18n!("Enter an amount"),
        AmountError::NotANumber => {
            i18n!("Not an amount: use digits and a decimal point, like 12.34")
        }
        AmountError::DecimalComma { suggest } => {
            i18nc!(
                "%1 is the amount as it should be typed",
                "Use a decimal point: %1",
                suggest
            )
        }
        AmountError::TooManyPlaces { places: 0 } => {
            i18n!("Use a whole number, without decimal places")
        }
        AmountError::TooManyPlaces { places } => i18np!(
            "Use at most %1 decimal place",
            "Use at most %1 decimal places",
            *places
        ),
    }
}

/// `what` is the field's name, already translated.
fn required(name: &str, value: &str, what: &str) -> Result<(), WriteError> {
    if value.trim().is_empty() {
        Err(field(
            name,
            i18nc!("%1 is a form field", "%1 is required", what),
        ))
    } else {
        Ok(())
    }
}

fn description(payee: &str, memo: &str) -> String {
    if memo.trim().is_empty() {
        payee.trim().to_string()
    } else {
        format!("{} | {}", payee.trim(), memo.trim())
    }
}

fn posting(account: &str, commodity: &str, money: Money) -> DraftPostingBody {
    DraftPostingBody {
        account: account.to_string(),
        amount: Some(DraftAmount::from_money(commodity, money)),
        status: String::new(),
        balance_assertion: None,
        clear_balance_assertion: false,
    }
}

/// What the app knows about the books when previewing: accounts (for
/// names, kinds and homes) and the commodity.
struct Books {
    accounts: Vec<AccountView>,
    commodity: String,
    /// The decimal places the commodity takes.
    places: u32,
}

impl Books {
    async fn load(client: &KabacusClient) -> Result<Books, WriteError> {
        let (accounts, commodity) =
            tokio::try_join!(client.list_accounts(None), client.commodity())?;
        Ok(Books {
            accounts,
            commodity: commodity.symbol,
            places: commodity.places,
        })
    }

    fn get(&self, account: &str) -> Option<&AccountView> {
        self.accounts.iter().find(|a| a.account == account)
    }

    fn name(&self, account: &str) -> String {
        self.get(account)
            .map(|a| a.display_name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| account.to_string())
    }

    fn kind(&self, account: &str) -> String {
        self.get(account)
            .map(|a| a.kind.clone())
            .unwrap_or_default()
    }

    /// Where Set aside puts the money: `to` itself, or, when it names no
    /// account and the money comes from a money account's free money, a new
    /// fund there called what was typed (`true`: the save makes it).
    fn set_aside_target(&self, from: &str, to: &str) -> Result<(String, bool), WriteError> {
        if self.get(to).is_some() || self.kind(from) != "money" {
            return Ok((to.to_string(), false));
        }
        if to.contains(':') {
            // A path typed out: new only directly in `from`.
            return match to.strip_prefix(from).and_then(|r| r.strip_prefix(':')) {
                Some(rest) if !rest.contains(':') => Ok((to.to_string(), true)),
                _ => self.known("to", to).map(|()| (to.to_string(), false)),
            };
        }
        // Core refuses an empty name, and one with `;` (`:` is a path, above).
        let fund = kabacus_core::new_fund_account(from, to).map_err(|_| {
            field(
                "to",
                if to.trim().is_empty() {
                    i18n!("Name the new reserve")
                } else {
                    i18nc!(
                        "%1 is what was typed",
                        "“%1” can't name a reserve: no “:” or “;”",
                        to.trim()
                    )
                },
            )
        })?;
        Ok((fund.clone(), self.get(&fund).is_none()))
    }

    /// A field naming an account the books don't declare.
    fn known(&self, name: &str, account: &str) -> Result<(), WriteError> {
        if self.get(account).is_some() {
            return Ok(());
        }
        let leaf = account.rsplit(':').next().unwrap_or(account).to_lowercase();
        let suggestion = self
            .accounts
            .iter()
            .find(|a| {
                a.account.to_lowercase().ends_with(&leaf)
                    || a.display_name.to_lowercase().contains(&leaf)
            })
            .map(|a| a.account.clone())
            .unwrap_or_default();
        Err(WriteError::Field {
            field: name.to_string(),
            message: i18nc!(
                "%1 is what was typed",
                "There's no account called %1",
                account
            ),
            suggestion,
        })
    }

    /// A card's reserve and the money account holding it.
    fn reserve_of(&self, card: &str) -> Option<(String, String)> {
        let leaf = card.rsplit(':').next()?;
        self.accounts
            .iter()
            .find(|a| a.kind == "debt-reserve" && a.account.ends_with(&format!("card-{leaf}")))
            .map(|r| (r.account.clone(), r.home.clone()))
    }
}

/// The current head; every preview is checked against it.
async fn head(client: &KabacusClient) -> Result<String, WriteError> {
    client
        .version()
        .await?
        .head
        .ok_or_else(|| WriteError::Other {
            message: i18n!("The books have no revisions yet"),
        })
}

fn summary(draft: &Draft, books: &Books) -> String {
    let n = |a: &str| books.name(a);
    match draft {
        Draft::AccountType { account, synthetic } => format!(
            "{} — {}",
            n(account),
            if *synthetic {
                i18n!("Synthetic account")
            } else {
                i18n!("Real account")
            }
        ),
        Draft::Simple {
            payee,
            amount,
            from,
            to,
            date,
            ..
        } => i18nc!(
            "%1 is a payee, %2 an amount, %3 and %4 accounts, %5 a date",
            "%1: %2 from %3 to %4, %5.",
            payee,
            amount,
            n(from),
            n(to),
            date
        ),
        Draft::CardPurchase {
            payee,
            amount,
            card,
            source,
            date,
            ..
        } => {
            if source == "free" {
                i18nc!(
                    "%1 is a payee, %2 an amount, %3 a card, %4 a date",
                    "%1: %2 on %3, paid from free money, %4.",
                    payee,
                    amount,
                    n(card),
                    date
                )
            } else {
                i18nc!(
                    "%1 is a payee, %2 an amount, %3 a card, %4 a reserve, %5 a date",
                    "%1: %2 on %3, paid from the %4 reserve, %5.",
                    payee,
                    amount,
                    n(card),
                    n(source),
                    date
                )
            }
        }
        Draft::SetAside {
            from, to, amount, ..
        } => match books.set_aside_target(from, to) {
            Ok((fund, true)) => i18nc!(
                "%1 is an amount, %2 an account, %3 the new reserve's name",
                "Set aside %1 from %2 into a new reserve, %3.",
                amount,
                n(from),
                fund_name(to, &fund)
            ),
            _ => i18nc!(
                "%1 is an amount, %2 and %3 accounts",
                "Move %1 from %2 to %3.",
                amount,
                n(from),
                n(to)
            ),
        },
        Draft::PayCard { card, amount, .. } => {
            i18nc!(
                "%1 is an amount, %2 a card",
                "Pay %1 to %2 from its reserve.",
                amount,
                n(card)
            )
        }
        Draft::Split {
            payee,
            total,
            paid_with,
            person,
            their_share,
            ..
        } => i18nc!(
            "%1 is a payee, %2 the total, %3 an account, %4 a person, %5 their share",
            "%1: %2 paid with %3; %4 owes %5 of it.",
            payee,
            total,
            n(paid_with),
            person,
            their_share
        ),
        Draft::Edit { payee, .. } | Draft::EditPostings { payee, .. } => {
            i18nc!(
                "%1 is a payee",
                "Edit %1 in place. History keeps the old version.",
                payee
            )
        }
        Draft::Void { .. } => i18n!("Void it: add a reversal, dated today."),
        Draft::Reconcile {
            account,
            date,
            balance,
        } => i18nc!(
            "%1 is an account, %2 a date, %3 a balance",
            "Record that %1 matched the statement on %2: %3.",
            n(account),
            date,
            balance
        ),
        Draft::FixReserves { card, ids, .. } => i18ncp!(
            "%2 is a card",
            "Add the missing reserve lines to %1 transaction on %2, as one change.",
            "Add the missing reserve lines to %1 transactions on %2, as one change.",
            ids.len(),
            n(card)
        ),
    }
}

/// Check a draft locally, in the order a form shows it.
fn validate(draft: &Draft, books: &Books) -> Result<(), WriteError> {
    match draft {
        Draft::AccountType { account, .. } => books.known("account", account)?,
        Draft::Simple {
            date,
            payee,
            from,
            to,
            amount,
            ..
        }
        | Draft::Edit {
            date,
            payee,
            from,
            to,
            amount,
            ..
        } => {
            check_date(date)?;
            required("payee", payee, &i18n!("Payee"))?;
            required(
                "from",
                from,
                &i18nc!("the account money comes from", "From"),
            )?;
            required("to", to, &i18nc!("the account money goes to", "To"))?;
            books.known("from", from)?;
            books.known("to", to)?;
            if from == to {
                return Err(field("to", i18n!("From and To must differ")));
            }
            check_amount("amount", amount, books.places, true)?;
        }
        Draft::EditPostings {
            date,
            payee,
            postings,
            ..
        } => {
            check_date(date)?;
            required("payee", payee, &i18n!("Payee"))?;
            for p in postings {
                books.known("postings", &p.account)?;
                check_amount("postings", &p.amount, books.places, false)?;
            }
        }
        Draft::CardPurchase {
            date,
            payee,
            card,
            category,
            source,
            amount,
            ..
        } => {
            check_date(date)?;
            required("card", card, &i18nc!("a credit card", "Card"))?;
            books.known("card", card)?;
            required("payee", payee, &i18n!("Payee"))?;
            required("category", category, &i18n!("Category"))?;
            books.known("category", category)?;
            if source != "free" {
                books.known("source", source)?;
            }
            check_amount("amount", amount, books.places, true)?;
        }
        Draft::SetAside {
            date,
            from,
            to,
            amount,
            ..
        } => {
            check_date(date)?;
            books.known("from", from)?;
            required("to", to, &i18nc!("the account money goes to", "To"))?;
            // A new fund isn't in the books yet; the server checks it.
            let (to, new_fund) = books.set_aside_target(from, to)?;
            if !new_fund {
                books.known("to", &to)?;
            }
            check_amount("amount", amount, books.places, true)?;
        }
        Draft::PayCard { date, card, amount } => {
            check_date(date)?;
            books.known("card", card)?;
            check_amount("amount", amount, books.places, true)?;
        }
        Draft::Split {
            date,
            payee,
            paid_with,
            category,
            person,
            total,
            their_share,
            ..
        } => {
            check_date(date)?;
            required("payee", payee, &i18n!("Payee"))?;
            books.known("paid_with", paid_with)?;
            books.known("category", category)?;
            required(
                "person",
                person,
                &i18nc!("the person a cost is split with", "Who"),
            )?;
            let total = check_amount("total", total, books.places, true)?;
            let theirs = check_amount("their_share", their_share, books.places, true)?;
            if theirs > total {
                return Err(field(
                    "their_share",
                    i18n!("Their share can't be more than the total"),
                ));
            }
        }
        Draft::Void { id } => required("id", id, &i18n!("The transaction"))?,
        Draft::Reconcile {
            account,
            date,
            balance,
        } => {
            check_date(date)?;
            books.known("account", account)?;
            check_amount("balance", balance, books.places, false)?;
        }
        Draft::FixReserves { card, ids, source } => {
            books.known("card", card)?;
            if ids.is_empty() {
                return Err(field("ids", i18n!("Choose at least one transaction")));
            }
            if source != "free" {
                books.known("source", source)?;
            }
        }
    }
    Ok(())
}

fn amount_str(text: &str) -> String {
    text.trim().replace([',', '$'], "")
}

async fn transaction(client: &KabacusClient, id: &str) -> Result<TransactionView, WriteError> {
    client
        .list_transactions()
        .await?
        .into_iter()
        .find(|t| crate::model::is_transaction(t, id))
        .ok_or_else(|| WriteError::Other {
            message: i18nc!("%1 is a transaction id", "Transaction %1 wasn't found", id),
        })
}

/// The two-posting body for Simple/Edit.
fn simple_postings(
    from: &str,
    to: &str,
    amount: &str,
    books: &Books,
) -> Result<Vec<DraftPostingBody>, WriteError> {
    let m = check_amount("amount", amount, books.places, true)?;
    let c = &books.commodity;
    Ok(vec![posting(to, c, m.clone()), posting(from, c, -m)])
}

/// The Split's postings: your share to the category, theirs netted
/// against what you owe them, and, on a card, its reserve lines.
async fn split_postings(
    client: &KabacusClient,
    books: &Books,
    paid_with: &str,
    category: &str,
    person: &str,
    total: &str,
    their_share: &str,
) -> Result<Vec<DraftPostingBody>, WriteError> {
    let c = &books.commodity;
    let total = check_amount("total", total, books.places, true)?;
    let theirs = check_amount("their_share", their_share, books.places, true)?;
    let person = person.trim().to_lowercase();
    let receivable = format!("assets:receivable:{person}");
    let payable = format!("liabilities:payable:{person}");
    books.known("person", &receivable)?;
    let balance = |account: &str| {
        books
            .get(account)
            .and_then(|a| a.balances.first())
            .and_then(|b| b.money())
            .unwrap_or_else(Money::zero)
    };
    let sides = kabacus_core::select_counterparty_sides(
        balance(&receivable).max(Money::zero()),
        balance(&payable).min(Money::zero()),
        theirs.clone(),
        kabacus_core::FlowDirection::ToPerson,
    );
    let mut postings = vec![];
    let mine = total.clone() - theirs;
    if !mine.is_zero() {
        postings.push(posting(category, c, mine));
    }
    if !sides.receivable_delta.is_zero() {
        postings.push(posting(&receivable, c, sides.receivable_delta));
    }
    if !sides.payable_delta.is_zero() {
        books.known("person", &payable)?;
        postings.push(posting(&payable, c, sides.payable_delta));
    }
    postings.push(posting(paid_with, c, -total.clone()));
    if books.kind(paid_with) == "revolving-debt" {
        let (reserve, home) = books
            .reserve_of(paid_with)
            .ok_or_else(|| WriteError::Other {
                message: i18nc!(
                    "%1 is a card",
                    "%1 has no reserve account",
                    books.name(paid_with)
                ),
            })?;
        postings.push(posting(&home, c, -total.clone()));
        postings.push(posting(&reserve, c, total));
    }
    let _ = client;
    Ok(postings)
}

/// The edit_transaction batch ops that add each transaction's missing
/// reserve lines.
async fn reserve_fixes(
    client: &KabacusClient,
    books: &Books,
    card: &str,
    ids: &[String],
    source: &str,
) -> Result<Vec<BatchOpBody>, WriteError> {
    let (reserve, home) = books.reserve_of(card).ok_or_else(|| WriteError::Other {
        message: i18nc!(
            "%1 is a card",
            "%1 has no reserve account",
            books.name(card)
        ),
    })?;
    let from = if source == "free" {
        home
    } else {
        source.to_string()
    };
    let all = client.list_transactions().await?;
    let mut ops = vec![];
    for id in ids {
        let tx = all
            .iter()
            .find(|t| t.id.to_string() == *id)
            .ok_or_else(|| WriteError::Other {
                message: i18nc!("%1 is a transaction id", "Transaction %1 wasn't found", id),
            })?;
        let missing = tx
            .missing_reserve
            .iter()
            .find(|m| m.card == card)
            .and_then(|m| m.amount.money())
            .ok_or_else(|| WriteError::Other {
                message: i18nc!(
                    "%1 is a transaction's payee",
                    "%1 isn't missing reserve lines",
                    tx.payee
                ),
            })?;
        let mut postings: Vec<serde_json::Value> = tx
            .postings
            .iter()
            .map(|p| {
                serde_json::json!({
                    "account": p.account,
                    "amount": p.amount.exact,
                    "status": p.status,
                })
            })
            .collect();
        postings
            .push(serde_json::json!({ "account": from, "amount": (-missing.clone()).to_string() }));
        postings.push(serde_json::json!({ "account": reserve, "amount": missing.to_string() }));
        ops.push(BatchOpBody {
            tool: "edit_transaction".to_string(),
            args: serde_json::json!({
                "id": tx.id,
                "date": tx.date,
                "description": description(&tx.payee, &tx.memo),
                "postings": postings,
            }),
        });
    }
    Ok(ops)
}

/// Send `draft` to the server with `dry_run`, against `head`.
async fn send(
    client: &KabacusClient,
    books: &Books,
    draft: &Draft,
    head: &str,
    dry_run: bool,
) -> Result<WriteResultView, WriteError> {
    let result = match draft {
        Draft::AccountType { account, synthetic } => {
            client
                .set_account_type(
                    account,
                    &kabacus_client::types::SetAccountTypeBody {
                        synthetic: *synthetic,
                        dry_run,
                    },
                    head,
                )
                .await
        }
        Draft::Simple {
            date,
            payee,
            memo,
            from,
            to,
            amount,
        } => {
            client
                .add_transaction(
                    head,
                    AddTransactionBody {
                        date: date.trim().to_string(),
                        description: description(payee, memo),
                        postings: simple_postings(from, to, amount, books)?,
                        tags: vec![],
                        dry_run,
                    },
                )
                .await
        }
        Draft::CardPurchase {
            date,
            payee,
            memo,
            card,
            category,
            source,
            amount,
        } => {
            client
                .record_card_purchase(
                    head,
                    &CardPurchaseBody {
                        card: card.clone(),
                        expense_account: category.clone(),
                        amount: amount_str(amount),
                        date: date.trim().to_string(),
                        description: description(payee, memo),
                        source: source.clone(),
                        dry_run,
                    },
                )
                .await
        }
        Draft::SetAside {
            date,
            from,
            to,
            amount,
            description,
        } => {
            let (to, new_fund) = books.set_aside_target(from, to)?;
            client
                .allocate(
                    head,
                    &AllocateBody {
                        from: from.clone(),
                        to,
                        new_fund,
                        amount: amount_str(amount),
                        date: date.trim().to_string(),
                        description: (!description.trim().is_empty())
                            .then(|| description.trim().to_string()),
                        dry_run,
                    },
                )
                .await
        }
        Draft::PayCard { date, card, amount } => {
            client
                .pay_card(
                    head,
                    &CardPaymentBody {
                        card: card.clone(),
                        amount: amount_str(amount),
                        date: date.trim().to_string(),
                        dry_run,
                    },
                )
                .await
        }
        Draft::Split {
            date,
            payee,
            memo,
            paid_with,
            category,
            person,
            total,
            their_share,
        } => {
            let postings = split_postings(
                client,
                books,
                paid_with,
                category,
                person,
                total,
                their_share,
            )
            .await?;
            client
                .add_transaction(
                    head,
                    AddTransactionBody {
                        date: date.trim().to_string(),
                        description: description(payee, memo),
                        postings,
                        tags: vec![],
                        dry_run,
                    },
                )
                .await
        }
        Draft::Edit {
            id,
            date,
            payee,
            memo,
            from,
            to,
            amount,
        } => {
            let tx = transaction(client, id).await?;
            let mut postings = simple_postings(from, to, amount, books)?;
            // Keep each kept account's status, and the journal's order.
            for p in &mut postings {
                if let Some(old) = tx.postings.iter().find(|o| o.account == p.account) {
                    p.status = old.status.clone();
                }
            }
            if tx
                .postings
                .first()
                .is_some_and(|p| p.amount.exact.starts_with('-'))
            {
                postings.reverse();
            }
            client
                .edit_transaction(
                    crate::model::transaction_id(id)?,
                    head,
                    AddTransactionBody {
                        date: date.trim().to_string(),
                        description: description(payee, memo),
                        postings,
                        tags: tx.tags.clone(),
                        dry_run,
                    },
                )
                .await
        }
        Draft::EditPostings {
            id,
            date,
            payee,
            memo,
            postings,
        } => {
            let tx = transaction(client, id).await?;
            let postings = postings
                .iter()
                .map(|p| {
                    let money = p
                        .amount
                        .parse::<Money>()
                        .map_err(|e| field("postings", e.to_string()))?;
                    let mut body = posting(&p.account, &p.commodity, money);
                    if let Some(old) = p
                        .original
                        .and_then(|i| tx.postings.get(i))
                        .filter(|old| old.account == p.account)
                    {
                        body.status = old.status.clone();
                    }
                    Ok(body)
                })
                .collect::<Result<Vec<_>, WriteError>>()?;
            client
                .edit_transaction(
                    crate::model::transaction_id(id)?,
                    head,
                    AddTransactionBody {
                        date: date.trim().into(),
                        description: description(payee, memo),
                        postings,
                        tags: tx.tags,
                        dry_run,
                    },
                )
                .await
        }
        Draft::Void { id } => {
            client
                .void_transaction(crate::model::transaction_id(id)?, head, dry_run)
                .await
        }
        Draft::Reconcile {
            account,
            date,
            balance,
        } => {
            let result = client
                .reconcile(&kabacus_client::types::ReconcileBody {
                    head: Some(head.to_string()),
                    dry_run,
                    account: account.clone(),
                    balance: amount_str(balance),
                    date: date.trim().to_string(),
                    mode: "assert".to_string(),
                })
                .await
                .map_err(|e| explain_sync(e, draft, books))?;
            return serde_json::from_value(result["written"].clone()).map_err(|e| {
                WriteError::Other {
                    message: e.to_string(),
                }
            });
        }
        Draft::FixReserves { card, ids, source } => {
            let ops = reserve_fixes(client, books, card, ids, source).await?;
            let result = client
                .batch(head, ops, dry_run)
                .await
                .map_err(|e| explain_sync(e, draft, books))?;
            return Ok(batch_result(result, head));
        }
    };
    result.map_err(|e| explain_sync(e, draft, books))
}

/// A batch's result as a write result: its balance changes and head.
fn batch_result(v: serde_json::Value, head: &str) -> WriteResultView {
    let balances = v["balance_changes"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|b| serde_json::from_value(b).ok())
        .collect();
    WriteResultView {
        id: v["ops"][0]["id"].as_u64(),
        head: v["head"].as_str().unwrap_or(head).to_string(),
        dry_run: v["dry_run"].as_bool().unwrap_or(true),
        changed: None,
        before: None,
        after: None,
        transactions: vec![],
        preview: String::new(),
        balances,
        unreserved: false,
        missing_reserve: vec![],
        warnings: vec![],
    }
}

/// Turn a server error into one the form can show.
fn explain_sync(e: ClientError, draft: &Draft, books: &Books) -> WriteError {
    match e {
        ClientError::Conflict { .. } => WriteError::Conflict {
            who: String::new(),
            what: String::new(),
        },
        ClientError::Invalid {
            rule,
            message,
            details,
            ..
        } => {
            let s = |k: &str| match &details[k] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            if rule == "assertion" {
                // The failure's details read "= <asserted>; calculated <amount>".
                let failure = &details["failures"][0];
                let text = failure["details"].as_str().unwrap_or_default();
                let (asserted, calculated) = text
                    .trim_start_matches("= ")
                    .split_once("; calculated ")
                    .unwrap_or((text, ""));
                // "At the end of <date>, <account> holds …", or "<account>
                // and its subaccounts together hold …" when inclusive.
                let account = failure["sentence"]
                    .as_str()
                    .and_then(|t| t.split_once(", "))
                    .and_then(|(_, t)| {
                        t.split_once(" and its subaccounts together hold ")
                            .or_else(|| t.split_once(" holds "))
                    })
                    .map_or("", |(a, _)| a);
                return WriteError::Assertion {
                    account: books.name(account),
                    asserted: asserted.to_string(),
                    calculated: calculated.to_string(),
                    transaction: s("transaction"),
                    posting: s("posting"),
                };
            }
            if rule == "amount_format" || rule.contains("amount") {
                return field("amount", message);
            }
            // A server older than the app ignores `new_fund`, so the new
            // fund reaches it undeclared and it refuses the fund by name.
            if let Draft::SetAside { from, to, .. } = draft {
                if let Ok((fund, true)) = books.set_aside_target(from, to) {
                    if message.starts_with(&format!("{fund} ")) {
                        return WriteError::Other {
                            message: i18nc!(
                                "%1 is the new reserve's name",
                                "kabacus-server didn't make the new reserve %1: it's older than this app. Update kabacus-server, or pick a reserve that exists.",
                                fund_name(to, &fund)
                            ),
                        };
                    }
                }
            }
            // "account X is not declared": point at the field holding it.
            // Only a word that reads as an account: other refusals say
            // "account" too ("a money account and its own funds").
            if let Some(account) = message
                .split_whitespace()
                .skip_while(|w| *w != "account")
                .nth(1)
                .filter(|a| a.contains(':') || draft_field_with(draft, a).is_some())
            {
                if books.get(account).is_none() {
                    let field_name = draft_field_with(draft, account).unwrap_or("to");
                    if let Err(e) = books.known(field_name, account) {
                        return e;
                    }
                }
            }
            WriteError::Other { message }
        }
        other => WriteError::Other {
            message: crate::model::ModelError::from(other).user_message(""),
        },
    }
}

/// Which of a draft's fields holds `account`.
fn draft_field_with(draft: &Draft, account: &str) -> Option<&'static str> {
    let fields: Vec<(&'static str, &String)> = match draft {
        Draft::Simple { from, to, .. } | Draft::Edit { from, to, .. } => {
            vec![("from", from), ("to", to)]
        }
        Draft::CardPurchase { card, category, .. } => vec![("card", card), ("category", category)],
        Draft::SetAside { from, to, .. } => vec![("from", from), ("to", to)],
        Draft::PayCard { card, .. } => vec![("card", card)],
        Draft::Split {
            paid_with,
            category,
            ..
        } => vec![("paid_with", paid_with), ("category", category)],
        _ => vec![],
    };
    fields
        .into_iter()
        .find(|(_, v)| v.as_str() == account)
        .map(|(k, _)| k)
}

/// On a conflict, say who changed the books (the newest commit).
async fn explain(client: &KabacusClient, e: WriteError) -> WriteError {
    match e {
        WriteError::Conflict { .. } => {
            let newest = client
                .history()
                .await
                .ok()
                .and_then(|h| h.into_iter().next());
            let (who, what) = match newest {
                Some(c) => (
                    match c.actor.as_deref() {
                        Some("mcp") => i18nc!("who made a change", "MCP"),
                        Some("app") => i18n!("This app, on another device"),
                        Some("external") => i18n!("An external edit"),
                        Some(other) => other.to_string(),
                        None => i18nc!("who made a change", "Someone"),
                    },
                    c.summary,
                ),
                None => (i18nc!("who made a change", "Someone"), String::new()),
            };
            WriteError::Conflict { who, what }
        }
        other => other,
    }
}

fn cash_change(books: &Books, result: &WriteResultView, coh_before: &Shown) -> Option<CashChange> {
    // Cash on Hand is the money accounts' own (exclusive) balances, so a
    // write moves it by exactly what it moves them by.
    let delta = result
        .balances
        .iter()
        .filter(|b| books.kind(&b.account) == "money")
        .fold(Money::zero(), |acc, b| {
            let d = b.after.money().unwrap_or_else(Money::zero)
                - b.before.money().unwrap_or_else(Money::zero);
            acc + d
        });
    let before = coh_before.money();
    Some(CashChange {
        before: coh_before.clone(),
        after: Shown::from_exact(&(before + delta.clone()).to_string(), None),
        delta: Shown::from_exact(&delta.to_string(), None),
    })
}

fn journal(result: &WriteResultView) -> Vec<DiffLine> {
    let mut lines = vec![];
    let changes: Vec<(Option<String>, Option<String>)> = if result.transactions.is_empty() {
        vec![(result.before.clone(), result.after.clone())]
    } else {
        result
            .transactions
            .iter()
            .map(|t| (t.before.clone(), t.after.clone()))
            .collect()
    };
    for (before, after) in changes {
        if before == after {
            continue;
        }
        for l in before.unwrap_or_default().lines() {
            lines.push(format!("-{l}"));
        }
        for l in after.unwrap_or_default().lines() {
            lines.push(format!("+{l}"));
        }
    }
    if lines.is_empty() && !result.preview.is_empty() {
        return diff_lines(&result.preview);
    }
    diff_lines(&lines.join("\n"))
}

/// Dry-run `draft` and describe what it would change. Nothing is
/// written.
pub async fn preview(client: &KabacusClient, draft: &Draft) -> Result<Preview, WriteError> {
    let books = Books::load(client).await?;
    validate(draft, &books)?;
    let head = head(client).await?;
    let coh = client.cash_on_hand(None, "summary").await?;
    let result = match send(client, &books, draft, &head, true).await {
        Ok(r) => r,
        Err(e) => return Err(explain(client, e).await),
    };
    let unchanged = result
        .warnings
        .iter()
        .any(|w| w["code"].as_str() == Some("unchanged"));
    if unchanged {
        return Ok(Preview {
            summary: i18n!("Nothing to save: the books already say this."),
            changes: vec![],
            cash_on_hand: None,
            journal: vec![],
            notes: vec![],
            warnings: vec![],
            unreserved: false,
            unchanged: true,
            head,
        });
    }
    let mut notes = vec![i18n!("The books still check out with this change")];
    if let Draft::CardPurchase { card, .. }
    | Draft::Split {
        paid_with: card, ..
    } = draft
    {
        if books.kind(card) == "revolving-debt" && !result.unreserved {
            notes.push(i18nc!(
                "%1 is a card",
                "%1 stays covered by its reserve",
                books.name(card)
            ));
        }
    }
    let mut warnings = Vec::new();
    if result.unreserved {
        warnings.push(i18n!(
            "A card is missing its reserve lines; it will be flagged"
        ));
    }
    for w in &result.warnings {
        if let Some(m) = w["message"].as_str() {
            warnings.push(m.to_string());
        }
    }
    Ok(Preview {
        summary: summary(draft, &books),
        changes: result
            .balances
            .iter()
            .map(|b| BalanceChange {
                name: match draft {
                    // A new fund isn't in the books yet: its name as typed.
                    Draft::SetAside { to, .. } if books.get(&b.account).is_none() => {
                        fund_name(to, &b.account)
                    }
                    _ => books.name(&b.account),
                },
                account: b.account.clone(),
                before: Shown::of(&b.before),
                after: Shown::of(&b.after),
            })
            .collect(),
        cash_on_hand: cash_change(&books, &result, &Shown::of(&coh.cash_on_hand)),
        journal: journal(&result),
        notes,
        warnings,
        unreserved: result.unreserved,
        unchanged: false,
        head,
    })
}

/// The preview's Save: the same call with `dry_run: false` against the preview's
/// `head`. A head that moved is a conflict, never a silent retry.
pub async fn commit(
    client: &KabacusClient,
    draft: &Draft,
    head: &str,
) -> Result<Saved, WriteError> {
    let books = Books::load(client).await?;
    validate(draft, &books)?;
    match send(client, &books, draft, head, false).await {
        Ok(r) => Ok(Saved {
            short: r.head.clone(),
            id: r.id.map(|i| i.to_string()).unwrap_or_default(),
            head: r.head,
        }),
        Err(e) => Err(explain(client, e).await),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_parse_from_the_forms_json() {
        let d: Draft = serde_json::from_str(
            r#"{"kind":"card_purchase","date":"2026-10-03","payee":"Corner Market","card":"liabilities:credit-card:visa","category":"expenses:groceries","source":"free","amount":"42.18"}"#,
        )
        .unwrap();
        assert!(matches!(d, Draft::CardPurchase { ref source, .. } if source == "free"));
    }
    #[test]
    fn amounts_use_cores_grammar_at_the_commodity_precision() {
        let problem = |text| amount_problem(text, 2, true);
        assert_eq!(problem("42.18"), "");
        assert_eq!(problem("1,234.50"), "");
        assert_eq!(problem("$9.99"), "");
        assert_eq!(problem("23,40"), "Use a decimal point: 23.40");
        assert_eq!(problem("12.345"), "Use at most 2 decimal places");
        assert_eq!(problem("99999999999999999999"), "");
        assert_eq!(problem("0"), "Must be more than zero");
        assert_eq!(problem("-5"), "Must be more than zero");
        assert!(problem("abc").starts_with("Not an amount"));
        // A statement balance may be zero or negative.
        assert_eq!(amount_problem("-5", 2, false), "");
    }

    /// The app previews and commits on the tokio runtime, off the GUI
    /// thread (bridge.rs), so typing never waits on a dry run.
    #[test]
    fn previews_and_commits_can_run_off_the_gui_thread() {
        fn spawnable<T: std::future::Future + Send + 'static>(_: T) {}
        let draft = set_aside("Gifts");
        let client = KabacusClient::connect_unix("/run/user/1000/kabacus.sock");
        let (c, d) = (client.clone(), draft.clone());
        spawnable(async move { preview(&c, &d).await });
        spawnable(async move { commit(&client, &draft, "4c67c22").await });
    }

    fn books() -> Books {
        let account = |account: &str, name: &str, kind: &str, home: &str| {
            serde_json::from_value::<AccountView>(serde_json::json!({
                "account": account, "display_name": name, "kind": kind,
                "home": home, "balances": [],
            }))
            .unwrap()
        };
        Books {
            accounts: vec![
                account("assets:bank:checking", "Checking", "money", ""),
                account(
                    "assets:bank:checking:gifts",
                    "Gifts",
                    "pre-allocation",
                    "assets:bank:checking",
                ),
                account(
                    "assets:bank:checking:brandon-birthday",
                    "Brandon Birthday",
                    "pre-allocation",
                    "assets:bank:checking",
                ),
            ],
            commodity: "USD".to_string(),
            places: 2,
        }
    }

    fn refused(message: &str) -> ClientError {
        ClientError::Invalid {
            rule: "allocate".to_string(),
            message: message.to_string(),
            failures: Vec::new(),
            details: serde_json::Value::Null,
        }
    }

    fn set_aside(to: &str) -> Draft {
        Draft::SetAside {
            date: "2026-10-15".into(),
            from: "assets:bank:checking".into(),
            to: to.into(),
            amount: "20.00".into(),
            description: String::new(),
        }
    }

    /// The word after "account" in a refusal is an account only when it
    /// reads like one: "a money account and its own funds" once came out
    /// as "There's no account called and", offering whichever fund had
    /// "and" in its name.
    #[test]
    fn a_refusal_is_not_read_as_naming_an_account_called_and() {
        let e = explain_sync(
            refused("assets:bank:checking:gifts is expense; allocate only moves money between a money account and its own funds and card reserves"),
            &set_aside("assets:bank:checking:gifts"),
            &books(),
        );
        match e {
            WriteError::Other { message } => {
                assert!(message.contains("allocate only moves"), "{message}")
            }
            other => panic!("{other:?}"),
        }
        // A real undeclared account still points at its field.
        let draft = Draft::Simple {
            date: "2026-10-15".into(),
            payee: "Corner Market".into(),
            memo: String::new(),
            from: "assets:bank:checking".into(),
            to: "expenses:grocerys".into(),
            amount: "12.40".into(),
        };
        match explain_sync(
            refused("account expenses:grocerys is not declared"),
            &draft,
            &books(),
        ) {
            WriteError::Field { field, message, .. } => {
                assert_eq!(field, "to");
                assert_eq!(message, "There's no account called expenses:grocerys");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A server older than the app ignores `new_fund` and refuses the new
    /// fund as one that isn't there: say so, rather than that the name is
    /// wrong.
    #[test]
    fn a_new_fund_refused_by_an_older_server_says_to_update_it() {
        let e = explain_sync(
            refused("assets:bank:checking:bathroom-renovation is pre-allocation; allocate only moves money between a money account and its own funds and card reserves"),
            &set_aside("Bathroom Renovation"),
            &books(),
        );
        match e {
            WriteError::Other { message } => {
                assert!(message.contains("Bathroom Renovation"), "{message}");
                assert!(message.contains("older than this app"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }
}
