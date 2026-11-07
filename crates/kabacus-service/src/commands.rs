// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared decoding of write arguments for single operations and batches.

use crate::error::Error as ApiError;
use crate::ops::WriteOp;
use crate::service::{self, invalid, AmountInput, DraftPostingDto};
use serde::Deserialize;
use serde_json::{json, Value};

/// A transaction id: its number, or the number as a string.
fn id_param<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Number(u64),
        Text(String),
    }
    match Id::deserialize(d)? {
        Id::Number(n) => Ok(n),
        Id::Text(t) => t
            .trim()
            .trim_start_matches('#')
            .parse()
            .map_err(|_| serde::de::Error::custom(format!("{t:?} isn't a transaction id"))),
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TagParam {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

pub fn to_tags(tags: Vec<TagParam>) -> Vec<(String, String)> {
    tags.into_iter().map(|t| (t.key, t.value)).collect()
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DraftPostingParam {
    pub account: String,
    /// A decimal string such as `"-12.34"` (or `{commodity, mantissa,
    /// places}`). Leave at most one posting's amount unset; the server
    /// fills it in as the negation of the others.
    #[serde(default)]
    pub amount: Option<AmountInput>,
    /// `"unmarked"` (default), `"pending"`, or `"cleared"`.
    #[serde(default)]
    pub status: Option<String>,
    /// The account's balance after this posting, written ` = <amount>`.
    /// When editing, omit it to keep the posting's current assertion as it
    /// is; pass it to change the asserted amount.
    #[serde(default)]
    pub balance_assertion: Option<AmountInput>,
    /// When editing, `true` removes this posting's balance assertion.
    #[serde(default)]
    pub clear_balance_assertion: Option<bool>,
}

pub fn to_postings(postings: Vec<DraftPostingParam>) -> Vec<DraftPostingDto> {
    postings
        .into_iter()
        .map(|p| DraftPostingDto {
            account: p.account,
            amount: p.amount,
            status: p.status.unwrap_or_default(),
            balance_assertion: p.balance_assertion,
            clear_balance_assertion: p.clear_balance_assertion.unwrap_or(false),
        })
        .collect()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NoParams {}

// --- read tool params ------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListAccountsParams {
    /// Optional fuzzy filter on the account path or friendly name, ranked
    /// best first.
    pub filter: Option<String>,
    /// Only accounts of this kind (`money`, `expense`, ...).
    pub kind: Option<String>,
    /// Only accounts at most this many segments deep.
    pub depth: Option<usize>,
    /// Default true; false leaves out accounts whose balances are zero.
    pub include_zero: Option<bool>,
    /// Only these fields of each account, e.g. `["account", "kind",
    /// "balance_inclusive"]`.
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListPayeesParams {
    /// Optional fuzzy filter on the payee.
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTransactionParams {
    /// The transaction's id, from search_transactions.
    #[serde(deserialize_with = "id_param")]
    pub id: u64,
    /// Also the writes that changed it (revision, date, actor, kind, user).
    pub include_history: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetRegisterParams {
    pub account: String,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Lines per page: 200 by default, at most 1000.
    pub limit: Option<usize>,
    /// `next_cursor` from the previous page.
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetBalanceParams {
    /// An account name or prefix, e.g. `"expenses:groceries"`.
    pub account_prefix: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CounterpartyStatementParams {
    pub name: String,
    pub from: String,
    pub to: String,
    /// Only uncleared lines ("open items").
    pub open_items: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExportStatementParams {
    pub name: String,
    pub from: String,
    pub to: String,
    /// Only uncleared lines ("open items").
    pub open_items: Option<bool>,
    /// `"csv"` or `"pdf"`.
    pub format: String,
    /// The PDF's letterhead: the sender's name.
    pub sender_name: Option<String>,
    /// The PDF's letterhead: address lines, separated by newlines.
    pub sender_address: Option<String>,
    /// The date printed on the PDF; defaults to today.
    pub statement_date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RevisionParams {
    /// A revision from `get_history` (its `hash`).
    pub rev: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TransactionsAtParams {
    /// A revision from `get_history` (its `hash`).
    pub rev: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CashOnHandParams {
    /// `YYYY-MM-DD`: as of the end of that day. Omit for today.
    pub as_of: Option<String>,
    /// `"summary"` (default): totals, funds, homes, warnings and each
    /// card's counts; `"full"` adds each card's first unreserved
    /// transactions.
    pub detail: Option<String>,
    /// With `"full"`: unreserved transactions per card (default 20).
    pub unreserved_limit: Option<usize>,
    /// A second `YYYY-MM-DD`: adds each component's change since then.
    pub compare_to: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CardReservesParams {
    /// One card, e.g. `"liabilities:credit-card:visa"`. Omit for every card.
    pub card: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSkillGuideParams {
    /// The skill, e.g. `"kabacus-bookkeeping"`. Omit for the only one.
    pub skill: Option<String>,
    /// A file exactly as SKILL.md links it, e.g.
    /// `"references/budget.md"`. Omit to read SKILL.md.
    pub file: Option<String>,
}

// --- write tool params ------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTransactionParams {
    pub date: String,
    pub description: String,
    pub postings: Vec<DraftPostingParam>,
    pub tags: Option<Vec<TagParam>>,
    /// The books' current head (their revision), from a prior read
    /// (`get_history`, `get_version`): required, so a write made on a
    /// stale view is refused as a conflict.
    pub head: String,
    /// Defaults to true: preview the write without committing.
    /// Call again with `dry_run: false` and the *same* `head` to commit.
    pub dry_run: Option<bool>,
    /// Also return a unified diff of the changed text.
    pub include_diff: Option<bool>,
}

impl AddTransactionParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        service::add_op(
            service::AddTransactionBody {
                date: self.date,
                description: self.description,
                postings: to_postings(self.postings),
                tags: to_tags(self.tags.unwrap_or_default()),
                dry_run: false,
                include_diff: false,
            },
            commodity,
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CorrectTransactionParams {
    /// The transaction's id.
    #[serde(deserialize_with = "id_param")]
    pub id: u64,
    pub description: String,
    pub postings: Vec<DraftPostingParam>,
    /// Accepted for older clients; a correction keeps its date.
    pub today: Option<String>,
    pub head: String,
    pub dry_run: Option<bool>,
    pub include_diff: Option<bool>,
}

impl CorrectTransactionParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        service::correction_op(
            &self.id.to_string(),
            service::CorrectionBody {
                void: false,
                description: Some(self.description),
                postings: Some(to_postings(self.postings)),
                today: self.today,
                ..Default::default()
            },
            commodity,
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VoidTransactionParams {
    #[serde(deserialize_with = "id_param")]
    pub id: u64,
    /// The date of the reversal; defaults to today.
    pub today: Option<String>,
    pub head: String,
    pub dry_run: Option<bool>,
    pub include_diff: Option<bool>,
}

impl VoidTransactionParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        service::correction_op(
            &self.id.to_string(),
            service::CorrectionBody {
                void: true,
                today: self.today,
                ..Default::default()
            },
            commodity,
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EditTransactionParams {
    /// The transaction's id, from `search_transactions`.
    #[serde(deserialize_with = "id_param")]
    pub id: u64,
    pub date: String,
    /// `Payee | memo`, or just the payee.
    pub description: String,
    /// The full new set of postings; at most one amount left unset.
    pub postings: Vec<DraftPostingParam>,
    /// The new tags. Omit to keep the transaction's tags as they are.
    pub tags: Option<Vec<TagParam>>,
    pub head: String,
    pub dry_run: Option<bool>,
    pub include_diff: Option<bool>,
}

impl EditTransactionParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        Ok(WriteOp::Edit(service::edit_request(
            &self.id.to_string(),
            Some(self.date),
            Some(self.description),
            Some(to_postings(self.postings)),
            self.tags.map(to_tags),
            commodity,
            "an edit",
        )?))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetClearedParams {
    #[serde(deserialize_with = "id_param")]
    pub id: u64,
    pub account: String,
    /// `"unmarked"`, `"pending"`, or `"cleared"`.
    pub status: String,
    pub head: String,
    pub dry_run: Option<bool>,
    pub include_diff: Option<bool>,
}

impl SetClearedParams {
    pub fn op(self) -> Result<WriteOp, ApiError> {
        service::status_op(
            &self.id.to_string(),
            &service::StatusBody {
                account: self.account,
                status: self.status,
                dry_run: false,
                include_diff: false,
            },
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AllocateParams {
    /// A money account (its free money) or a fund or card reserve in it,
    /// e.g. `"assets:bank:checking"`.
    pub from: String,
    /// A fund or card reserve, or the money account they sit in, e.g.
    /// `"assets:bank:checking:insurance"`. Must share `from`'s home.
    pub to: String,
    /// A decimal string, e.g. `"100.00"`.
    pub amount: AmountInput,
    pub date: String,
    /// `Payee | memo`; a description is made up if omitted.
    pub description: Option<String>,
    /// `to` is a new fund directly in the money account `from` (e.g.
    /// `"assets:bank:checking:vacation"`), declared by this write. Only a
    /// new fund there; any other account is declared with declare_account.
    pub new_fund: Option<bool>,
    pub head: String,
    pub dry_run: Option<bool>,
    pub include_diff: Option<bool>,
}

impl AllocateParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        service::allocate_op(
            service::AllocateBody {
                from: self.from,
                to: self.to,
                amount: self.amount,
                date: self.date,
                description: self.description,
                new_fund: self.new_fund.unwrap_or(false),
                dry_run: false,
                include_diff: false,
            },
            commodity,
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecordCardPurchaseParams {
    /// The card, e.g. `"liabilities:credit-card:rewards-visa"`.
    pub card: String,
    /// What was bought, e.g. `"expenses:household:groceries"`; for a
    /// reimbursable purchase, the receivable instead.
    pub expense_account: String,
    /// A decimal string, e.g. `"40.00"`. Negative records a refund back to
    /// the same source.
    pub amount: AmountInput,
    pub date: String,
    /// `Payee | memo`.
    pub description: String,
    /// `"free"` for a discretionary purchase from free money, or the fund
    /// paying for it, e.g. `"assets:bank:checking:insurance"`.
    pub source: String,
    pub tags: Option<Vec<TagParam>>,
    pub head: String,
    pub dry_run: Option<bool>,
    pub include_diff: Option<bool>,
}

impl RecordCardPurchaseParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        service::card_purchase_op(
            service::CardPurchaseBody {
                card: self.card,
                expense_account: self.expense_account,
                amount: self.amount,
                date: self.date,
                description: self.description,
                source: self.source,
                tags: to_tags(self.tags.unwrap_or_default()),
                dry_run: false,
                include_diff: false,
            },
            commodity,
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PayCardParams {
    /// The card, e.g. `"liabilities:credit-card:rewards-visa"`.
    pub card: String,
    /// A decimal string, e.g. `"425.00"`.
    pub amount: AmountInput,
    pub date: String,
    pub head: String,
    pub dry_run: Option<bool>,
    pub include_diff: Option<bool>,
}

impl PayCardParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        service::card_payment_op(
            service::CardPaymentBody {
                card: self.card,
                amount: self.amount,
                date: self.date,
                dry_run: false,
                include_diff: false,
            },
            commodity,
        )
    }
}

// --- rewrite tool params ------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RewriteTransactionParams {
    #[serde(deserialize_with = "id_param")]
    pub id: u64,
    pub date: String,
    pub description: String,
    pub postings: Vec<DraftPostingParam>,
    /// The complete new tags.
    pub tags: Option<Vec<TagParam>>,
    pub head: String,
    /// Defaults to true: validate and return the preview, change nothing.
    /// Call again with `dry_run: false` and the same `head` to commit.
    pub dry_run: Option<bool>,
}

impl RewriteTransactionParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        Ok(WriteOp::Rewrite(service::edit_request(
            &self.id.to_string(),
            Some(self.date),
            Some(self.description),
            Some(to_postings(self.postings)),
            Some(to_tags(self.tags.unwrap_or_default())),
            commodity,
            "a rewrite",
        )?))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteTransactionParams {
    #[serde(deserialize_with = "id_param")]
    pub id: u64,
    pub head: String,
    /// Defaults to true: validate and return the preview, change nothing.
    /// Call again with `dry_run: false` and the same `head` to commit.
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeclareAccountParams {
    /// The full account path, e.g. `"expenses:pets"`.
    pub account: String,
    /// Tags for the declaration, e.g. `{"key": "ynab-envelope", "value":
    /// "Pets"}` for a friendly name. An account's kind (fund, card
    /// reserve, ...) comes from its path, not a tag.
    pub tags: Option<Vec<TagParam>>,
    /// Override the default: real (false), except funds and card reserves (true).
    pub synthetic: Option<bool>,
    pub head: String,
    /// Defaults to true: validate and return the preview, change nothing.
    /// Call again with `dry_run: false` and the same `head` to commit.
    pub dry_run: Option<bool>,
}

impl DeclareAccountParams {
    pub fn op(self) -> Result<WriteOp, ApiError> {
        service::account_declaration_op(
            self.account,
            to_tags(self.tags.unwrap_or_default()),
            self.synthetic,
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetAccountTypeParams {
    pub account: String,
    /// true for a bookkeeping account; false for a real-world account.
    pub synthetic: bool,
    pub head: String,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenameAccountParams {
    /// The account to rename, with everything under it.
    pub from: String,
    /// Its new name; must not exist yet unless `merge`.
    pub to: String,
    /// Merge into an existing `to` of the same kind.
    pub merge: Option<bool>,
    /// Needed when the new name gives an account another kind.
    pub allow_kind_change: Option<bool>,
    pub head: String,
    /// Defaults to true: validate and return the preview, change nothing.
    /// Call again with `dry_run: false` and the same `head` to commit.
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RemoveAccountParams {
    /// A declared account nothing refers to.
    pub account: String,
    pub head: String,
    /// Defaults to true: validate and return the preview, change nothing.
    /// Call again with `dry_run: false` and the same `head` to commit.
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetBalanceAssertionParams {
    /// The transaction holding the assertion, as a failed assertion's
    /// error gives it (`details.transaction`).
    #[serde(deserialize_with = "id_param")]
    pub transaction: u64,
    /// The posting's account.
    pub account: String,
    /// The new asserted balance, a decimal string. Leave it out to remove
    /// the assertion.
    pub balance: Option<AmountInput>,
    pub head: String,
    /// Defaults to true: validate and return the preview, change nothing.
    /// Call again with `dry_run: false` and the same `head` to commit.
    pub dry_run: Option<bool>,
}

impl SetBalanceAssertionParams {
    pub fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
        service::balance_assertion_op(
            service::BalanceAssertionBody {
                transaction: self.transaction,
                account: self.account,
                balance: self.balance,
                dry_run: false,
            },
            commodity,
        )
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImportJournalParams {
    /// The main journal's name among `files`.
    pub main: String,
    /// Each journal's text, by its name (as `include` names it).
    pub files: std::collections::BTreeMap<String, String>,
    /// Defaults to true: read and check, import nothing.
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConvertMoneyAssertionsParams {
    pub head: String,
    /// Defaults to true: validate and return the preview, change nothing.
    /// Call again with `dry_run: false` and the same `head` to commit.
    pub dry_run: Option<bool>,
}

/// A batch operation's `{tool, args}` as a write operation, through
/// the same parameters and conversions as the tool itself. `args` have no
/// `head` or `dry_run`: the batch has them.
pub fn op_from_args(tool: &str, mut args: Value, commodity: &str) -> Result<WriteOp, ApiError> {
    if let Some(object) = args.as_object_mut() {
        object.insert("head".to_string(), json!(""));
        object.remove("dry_run");
        object.remove("include_diff");
    }
    fn parse<T: serde::de::DeserializeOwned>(tool: &str, args: Value) -> Result<T, ApiError> {
        serde_json::from_value(args)
            .map_err(|e| invalid("args", format!("{tool}'s arguments: {e}")))
    }
    match tool {
        "add_transaction" => parse::<AddTransactionParams>(tool, args)?.op(commodity),
        "edit_transaction" => parse::<EditTransactionParams>(tool, args)?.op(commodity),
        "correct_transaction" => parse::<CorrectTransactionParams>(tool, args)?.op(commodity),
        "void_transaction" => parse::<VoidTransactionParams>(tool, args)?.op(commodity),
        "set_cleared" => parse::<SetClearedParams>(tool, args)?.op(),
        "allocate" => parse::<AllocateParams>(tool, args)?.op(commodity),
        "record_card_purchase" => parse::<RecordCardPurchaseParams>(tool, args)?.op(commodity),
        "pay_card" => parse::<PayCardParams>(tool, args)?.op(commodity),
        "rewrite_transaction" => parse::<RewriteTransactionParams>(tool, args)?.op(commodity),
        "delete_transaction" => {
            let p = parse::<DeleteTransactionParams>(tool, args)?;
            Ok(WriteOp::Delete { id: p.id })
        }
        "declare_account" => {
            let p = parse::<DeclareAccountParams>(tool, args)?;
            p.op()
        }
        "set_account_type" => {
            let p = parse::<SetAccountTypeParams>(tool, args)?;
            Ok(WriteOp::SetAccountType {
                account: p.account,
                synthetic: p.synthetic,
            })
        }
        "rename_account" => {
            let p = parse::<RenameAccountParams>(tool, args)?;
            Ok(WriteOp::RenameAccount {
                from: p.from,
                to: p.to,
                merge: p.merge.unwrap_or(false),
                allow_kind_change: p.allow_kind_change.unwrap_or(false),
            })
        }
        "remove_account" => {
            let p = parse::<RemoveAccountParams>(tool, args)?;
            Ok(WriteOp::RemoveAccount { account: p.account })
        }
        "set_balance_assertion" => parse::<SetBalanceAssertionParams>(tool, args)?.op(commodity),
        "convert_money_assertions" => Ok(WriteOp::ConvertMoneyAssertions),
        other => Err(invalid("tool", format!("{other:?} isn't a write tool"))),
    }
}
