// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The MCP tools: one or more per API route ([`crate::api::ENDPOINTS`]),
//! each calling the same [`crate::service`] function as the route, plus
//! `get_balance` and `get_skill_guide`, which only MCP needs.

use super::skills;
use crate::api::ApiError;
use crate::auth::roles::Principal;
use crate::ops::{self, WriteOp};
use crate::search::SearchQuery;
use crate::service::{self, invalid, AmountInput, DraftPostingDto, WriteResultDto};
use crate::state::AppState;
use base64::Engine;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerConfig, SubscriptionFilter,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::RoleServer;
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

type Parts = axum::http::request::Parts;

/// The rewrite tools, listed only when the rewrite gates are open for the caller.
pub const REWRITE_TOOLS: &[&str] = &[
    "rewrite_transaction",
    "delete_transaction",
    "declare_account",
    "set_account_type",
    "rename_account",
    "remove_account",
    "set_balance_assertion",
    "convert_money_assertions",
    "import_journal",
];

/// Every error as `{code, message, path?, details?}` in the error's
/// data, with the message also as the error's own.
pub fn to_mcp_error(e: ApiError) -> McpError {
    let body = e.body();
    let data = serde_json::to_value(&body).ok();
    match &e {
        ApiError::Internal(_) => McpError::internal_error(body.message, data),
        ApiError::MissingRole(_) | ApiError::RewriteDisabled | ApiError::Auth(_) => {
            McpError::invalid_request(body.message, data)
        }
        _ => McpError::invalid_params(body.message, data),
    }
}

fn structured<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::structured(
        serde_json::to_value(value).map_err(|e| McpError::internal_error(e.to_string(), None))?,
    ))
}

/// A write's result as MCP shows it: the compact preview (`before`,
/// `after`, `balance_changes`) without the app's `preview`.
fn write_result(dto: WriteResultDto) -> Result<CallToolResult, McpError> {
    let mut value =
        serde_json::to_value(&dto).map_err(|e| McpError::internal_error(e.to_string(), None))?;
    if let Some(object) = value.as_object_mut() {
        object.remove("preview");
        if let Some(balances) = object.remove("balances") {
            object.insert("balance_changes".to_string(), balances);
        }
    }
    Ok(CallToolResult::structured(value))
}

/// The caller, as `/mcp`'s middleware authenticated it.
fn principal(parts: &Parts) -> Result<&Principal, McpError> {
    parts
        .extensions
        .get::<Principal>()
        .ok_or_else(|| McpError::internal_error("the request wasn't authenticated", None))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SharedExpenseParams {
    pub head: String,
    #[serde(flatten)]
    pub body: crate::split::SplitBody,
}

// --- shared params ---------------------------------------------------------

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

fn to_tags(tags: Vec<TagParam>) -> Vec<(String, String)> {
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

fn to_postings(postings: Vec<DraftPostingParam>) -> Vec<DraftPostingDto> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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
    fn op(self) -> Result<WriteOp, ApiError> {
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
    fn op(self, commodity: &str) -> Result<WriteOp, ApiError> {
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

#[derive(Clone)]
pub struct KabacusMcp {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

/// A stable hash of tools' names and schemas.
fn hash_tools(tools: &[rmcp::model::Tool]) -> String {
    let mut tools: Vec<Value> = tools
        .iter()
        .map(|t| serde_json::to_value(t).unwrap_or_default())
        .collect();
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let canonical = canonical_json(&Value::Array(tools));
    format!("{:016x}", crate::batch::fnv(canonical.as_bytes()))
}

/// JSON with object keys sorted, so the same tools always hash the same.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical_json(&map[k])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => other.to_string(),
    }
}

/// The hash of every tool the server has.
pub fn tool_list_hash() -> String {
    hash_tools(&KabacusMcp::tool_router().list_all())
}

impl KabacusMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        KabacusMcp {
            state,
            tool_router: Self::tool_router(),
        }
    }

    /// The tools `principal` may see: all but the rewrite tools unless the
    /// rewrite gates are open for them.
    pub fn visible_tools(&self, principal: &Principal) -> Vec<rmcp::model::Tool> {
        let rewrites_open = service::check_rewrite_gates(&self.state, principal).is_ok();
        self.tool_router
            .list_all()
            .into_iter()
            .filter(|t| rewrites_open || !REWRITE_TOOLS.contains(&t.name.as_ref()))
            .collect()
    }

    /// The hash of the tools `principal` sees.
    pub fn visible_hash(&self, principal: &Principal) -> String {
        hash_tools(&self.visible_tools(principal))
    }

    async fn commodity(&self) -> Result<String, McpError> {
        service::books_commodity(&self.state)
            .await
            .map_err(to_mcp_error)
    }

    async fn write(
        &self,
        parts: &Parts,
        op: Result<WriteOp, ApiError>,
        head: &str,
        dry_run: Option<bool>,
        include_diff: Option<bool>,
    ) -> Result<CallToolResult, McpError> {
        let principal = principal(parts)?;
        service::require(principal, crate::auth::roles::Role::Write).map_err(to_mcp_error)?;
        let op = op.map_err(to_mcp_error)?;
        let dto = service::run_write(
            &self.state,
            principal,
            op,
            head,
            dry_run.unwrap_or(true),
            include_diff.unwrap_or(false),
        )
        .await
        .map_err(to_mcp_error)?;
        write_result(dto)
    }

    async fn rewrite(
        &self,
        parts: &Parts,
        op: Result<WriteOp, ApiError>,
        head: &str,
        dry_run: Option<bool>,
    ) -> Result<CallToolResult, McpError> {
        let principal = principal(parts)?;
        service::check_rewrite_gates(&self.state, principal).map_err(to_mcp_error)?;
        let op = op.map_err(to_mcp_error)?;
        let dto = service::run_rewrite(&self.state, principal, op, head, dry_run.unwrap_or(true))
            .await
            .map_err(to_mcp_error)?;
        write_result(dto)
    }
}

#[tool_router]
impl KabacusMcp {
    // --- read tools ---------------------------------------------------------

    #[tool(
        description = "The server's version, the books' current head (for the next write), how it authenticates, and tool_list_hash: a stable hash of every tool's name and schema, which changes whenever the tools do (refresh your tool list if it differs from what you saw).",
        annotations(read_only_hint = true)
    )]
    async fn get_version(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(&service::version(&self.state).await)
    }

    #[tool(
        description = "The books' commodity and how many decimal places its amounts show. A write with an amount with more is refused.",
        annotations(read_only_hint = true)
    )]
    async fn get_commodity(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let commodity = service::books_commodity_info(&self.state, principal(&parts)?)
            .await
            .map_err(to_mcp_error)?;
        structured(&commodity)
    }

    #[tool(
        description = "Account tree with balances; optional fuzzy filter on path or friendly name, best match first, and filters by kind, depth and zero balance. Each account has its read-only kind (money, pre-allocation = fund, debt-reserve = card reserve, receivable, revolving-debt = card, ...), its home (the money account holding a fund or card reserve), synthetic (defaults false except funds and card reserves), physical_home (the real account holding its cash, or null for wholly synthetic accounts), a display_name, and balance_exclusive (its own postings; a money account's free money) and balance_inclusive (with sub-accounts; what the bank shows). fields picks which fields each account has.",
        annotations(read_only_hint = true)
    )]
    async fn list_accounts(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ListAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let q = service::AccountsQuery {
            filter: p.filter,
            kind: p.kind,
            depth: p.depth,
            include_zero: p.include_zero,
        };
        let accounts = service::accounts_filtered(&self.state, principal(&parts)?, &q)
            .await
            .map_err(to_mcp_error)?;
        let rows = crate::search::project(&accounts, p.fields.as_deref(), |_| Default::default());
        structured(&rows)
    }

    #[tool(
        description = "Known payees, most recently used first, each with a use count and its last transaction, to fill in a new transaction like the last one.",
        annotations(read_only_hint = true)
    )]
    async fn list_payees(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ListPayeesParams>,
    ) -> Result<CallToolResult, McpError> {
        let payees = service::payees(&self.state, principal(&parts)?, p.query.as_deref())
            .await
            .map_err(to_mcp_error)?;
        structured(&payees)
    }

    #[tool(
        description = "Search transactions. Filters combine with AND: plain-text query, date range, account (path or prefix; repeatable), kind, amount_min/amount_max (decimal strings, against the largest posting), payee and memo (substring or /regex/), tag ({key, value?}; repeatable), has_tag/lacks_tag, status (on any posting), cash_effect (zero or nonzero: postings to money accounts), unreserved_only (card transactions missing their reserve lines). Returns {transactions, next_cursor, total_count, truncated}: 200 rows by default (limit up to 1000), sort date_asc or date_desc, continue with cursor; fields picks columns (add \"cash_effect\" for each row's); count_only returns only {total_count}.",
        annotations(read_only_hint = true)
    )]
    async fn search_transactions(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(q): Parameters<SearchQuery>,
    ) -> Result<CallToolResult, McpError> {
        let result = crate::search::search(&self.state, principal(&parts)?, &q)
            .await
            .map_err(to_mcp_error)?;
        structured(&result)
    }

    #[tool(
        description = "One transaction, in the same shape as search_transactions, by id. include_history: true adds the writes that changed it (hash = revision, date, actor, kind, user).",
        annotations(read_only_hint = true)
    )]
    async fn get_transaction(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<GetTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let tx = crate::search::get_transaction(
            &self.state,
            principal(&parts)?,
            &p.id.to_string(),
            p.include_history.unwrap_or(false),
        )
        .await
        .map_err(to_mcp_error)?;
        structured(&tx)
    }

    #[tool(
        description = "One account's register: every posting with the other side of the transaction and a running balance, oldest first, paged: {lines, next_cursor, total_count, truncated}, 200 lines by default. Lines of card transactions missing their reserve line carry unreserved: true and missing_reserve.",
        annotations(read_only_hint = true)
    )]
    async fn get_register(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<GetRegisterParams>,
    ) -> Result<CallToolResult, McpError> {
        let page = crate::search::register_page(
            &self.state,
            principal(&parts)?,
            &p.account,
            p.from.as_deref(),
            p.to.as_deref(),
            p.cursor.as_deref(),
            p.limit,
        )
        .await
        .map_err(to_mcp_error)?;
        structured(&page)
    }

    #[tool(
        description = "Total balance for an account or account prefix over a period (e.g. monthly spending by category), summed per commodity over every transaction, as hledger sums it.",
        annotations(read_only_hint = true)
    )]
    async fn get_balance(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<GetBalanceParams>,
    ) -> Result<CallToolResult, McpError> {
        service::require(principal(&parts)?, crate::auth::roles::Role::Read)
            .map_err(to_mcp_error)?;
        let from = service::parse_optional_date(p.from.as_deref()).map_err(to_mcp_error)?;
        let to = service::parse_optional_date(p.to.as_deref()).map_err(to_mcp_error)?;
        let txs = ops::read_all_transactions(&self.state)
            .await
            .map_err(|e| to_mcp_error(e.into()))?;
        let mut totals: std::collections::BTreeMap<String, kabacus_core::Money> =
            std::collections::BTreeMap::new();
        for tx in txs
            .iter()
            .filter(|tx| from.is_none_or(|d| tx.date >= d) && to.is_none_or(|d| tx.date <= d))
        {
            for posting in &tx.postings {
                if posting.account.is_under(&p.account_prefix) {
                    let entry = totals
                        .entry(posting.amount.commodity.clone())
                        .or_insert_with(kabacus_core::Money::zero);
                    *entry = entry.clone() + posting.amount.money.clone();
                }
            }
        }
        let result: Vec<service::AmountDto> = totals
            .into_iter()
            .map(|(commodity, money)| (&kabacus_core::Amount::new(commodity, money)).into())
            .collect();
        structured(&result)
    }

    #[tool(
        description = "Every counterparty with their net position: \"Owes you $X\" or \"You owe $X\".",
        annotations(read_only_hint = true)
    )]
    async fn list_counterparties(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let list = service::counterparties(&self.state, principal(&parts)?)
            .await
            .map_err(to_mcp_error)?;
        structured(&list)
    }

    #[tool(
        description = "A netted statement for one counterparty over a date range: one line per transaction, opening/closing net balance.",
        annotations(read_only_hint = true)
    )]
    async fn counterparty_statement(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<CounterpartyStatementParams>,
    ) -> Result<CallToolResult, McpError> {
        let query = service::StatementQuery {
            from: p.from,
            to: p.to,
            open_items: p.open_items.unwrap_or(false),
            ..Default::default()
        };
        let stmt = service::statement(&self.state, principal(&parts)?, &p.name, &query)
            .await
            .map_err(to_mcp_error)?;
        structured(&service::statement_dto(stmt))
    }

    #[tool(
        description = "Export a counterparty statement as a CSV or PDF file, the same file the app exports. Returned as an embedded resource named statement-<name>-<end-date>.<ext>.",
        annotations(read_only_hint = true)
    )]
    async fn export_statement(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ExportStatementParams>,
    ) -> Result<CallToolResult, McpError> {
        let query = service::StatementQuery {
            from: p.from,
            to: p.to,
            open_items: p.open_items.unwrap_or(false),
            format: Some(p.format),
            sender_name: p.sender_name,
            sender_address: p.sender_address,
            statement_date: p.statement_date,
        };
        let stmt = service::statement(&self.state, principal(&parts)?, &p.name, &query)
            .await
            .map_err(to_mcp_error)?;
        let file = service::statement_file(&stmt, &query).map_err(to_mcp_error)?;
        let uri = format!("kabacus://statements/{}", file.filename);
        let mime = file.content_type.split(';').next().unwrap_or_default();
        let contents = if mime.starts_with("text/") {
            ResourceContents::text(String::from_utf8_lossy(&file.body), uri)
        } else {
            ResourceContents::blob(
                base64::engine::general_purpose::STANDARD.encode(&file.body),
                uri,
            )
        }
        .with_mime_type(mime);
        Ok(CallToolResult::success(vec![
            ContentBlock::text(format!("Exported {}", file.filename)),
            ContentBlock::resource(contents),
        ]))
    }

    #[tool(
        description = "Recent writes to the books, newest first, with date, actor (app/mcp/client), user and kind. Each one's hash is the revision it made; the first entry's is the current head.",
        annotations(read_only_hint = true)
    )]
    async fn get_history(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let history = service::history(&self.state, principal(&parts)?)
            .await
            .map_err(to_mcp_error)?;
        structured(&history)
    }

    #[tool(
        description = "What one write changed, as a diff of journal text before and after.",
        annotations(read_only_hint = true)
    )]
    async fn get_history_diff(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RevisionParams>,
    ) -> Result<CallToolResult, McpError> {
        let diff = service::history_diff(&self.state, principal(&parts)?, &p.rev)
            .await
            .map_err(to_mcp_error)?;
        structured(&json!({ "rev": p.rev, "diff": diff }))
    }

    #[tool(
        description = "The transactions as they were at a past revision, in the same shape as search_transactions' rows.",
        annotations(read_only_hint = true)
    )]
    async fn get_transactions_at(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<TransactionsAtParams>,
    ) -> Result<CallToolResult, McpError> {
        let txs = service::history_transactions(&self.state, principal(&parts)?, &p.rev)
            .await
            .map_err(to_mcp_error)?;
        structured(&txs)
    }

    #[tool(
        description = "Cash on Hand: how much can be spent right now. Money on Hand (money + funds + card reserves + receivables), Future Allocations (funds + card reserves), Receivables, and Cash on Hand = Money on Hand - Future Allocations - Receivables, the free money in the money accounts. An overdrawn fund counts as zero in Future Allocations; its overdraft comes off its home's free money. Also each account by kind, free money per home, every fund, every card with its reserve, difference (owed minus reserve), unreserved_count, and validation warnings (unreserved-unaccounted when a card's flagged transactions don't explain its difference). detail \"summary\" (default) has no transaction lists; \"full\" adds each card's first unreserved_limit (20) flagged transactions and a next_cursor to continue with search_transactions (unreserved_only, account = the card). as_of (YYYY-MM-DD) for another day, today by default, so a future-dated transaction counts only from its day; compare_to adds each component's change since a second day.",
        annotations(read_only_hint = true)
    )]
    async fn cash_on_hand(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<CashOnHandParams>,
    ) -> Result<CallToolResult, McpError> {
        let q = service::CashOnHandQuery {
            as_of: p.as_of,
            detail: p.detail,
            unreserved_limit: p.unreserved_limit,
            compare_to: p.compare_to,
        };
        let report = service::cash_on_hand(&self.state, principal(&parts)?, &q)
            .await
            .map_err(to_mcp_error)?;
        structured(&report)
    }

    #[tool(
        description = "The account kind rule table from the server's config, every account (parents included) with its kind and the rule it matched, the unclassified accounts, and validation warnings. Kinds come from account paths; they can't be set on an account.",
        annotations(read_only_hint = true)
    )]
    async fn account_kinds(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let kinds = service::account_kinds(&self.state, principal(&parts)?)
            .await
            .map_err(to_mcp_error)?;
        structured(&kinds)
    }

    #[tool(
        description = "Each card (or one) with its balance owed and its reserve's balance, which should be equal, and the reserve split by source: free (discretionary) and each fund (planned), from the counter-account of each posting to the reserve since it last reached zero, payments taken from the oldest first (FIFO). The response states the rule.",
        annotations(read_only_hint = true)
    )]
    async fn card_reserves(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<CardReservesParams>,
    ) -> Result<CallToolResult, McpError> {
        let reserves = service::card_reserves(&self.state, principal(&parts)?, p.card.as_deref())
            .await
            .map_err(to_mcp_error)?;
        structured(&reserves)
    }

    // --- reports ------------------------------------------------------------

    #[tool(
        description = "Income statement for from..to by interval (none, month, quarter, year): income (as positive amounts) and expense rows rolled up to depth (default 2), totals per period, and an equity_conversion section: the decrease in what is owed on term-liability accounts (loan principal paid), split into cash and non_cash; conversion_kinds adds e.g. non-liquid (money moved into investments). net = income - expenses - equity conversion; net_before_conversion = income - expenses. equity_conversion: false leaves the section out; exclude leaves kinds or account prefixes out. Amounts are decimal strings in the books' commodity.",
        annotations(read_only_hint = true)
    )]
    async fn income_statement(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::reports::IncomeStatementParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            &crate::reports::income_statement(&self.state, principal(&parts)?, &p)
                .await
                .map_err(to_mcp_error)?,
        )
    }

    #[tool(
        description = "Balance sheet as_of a day: assets and liabilities (as booked: negative while owed), grouped by kind (default) or account (to depth), per-kind subtotals, the liquid subtotal (money, funds, card reserves), and net worth (assets + liabilities).",
        annotations(read_only_hint = true)
    )]
    async fn balance_sheet(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::reports::BalanceSheetParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            &crate::reports::balance_sheet(&self.state, principal(&parts)?, &p)
                .await
                .map_err(to_mcp_error)?,
        )
    }

    #[tool(
        description = "The top expense categories (limit, default 20) over from..to, rolled up to depth (default 2), with each one's amount and share of all spending; compare_previous adds the change from the equal period just before.",
        annotations(read_only_hint = true)
    )]
    async fn category_breakdown(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::reports::CategoryBreakdownParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            &crate::reports::category_breakdown(&self.state, principal(&parts)?, &p)
                .await
                .map_err(to_mcp_error)?,
        )
    }

    #[tool(
        description = "Cash on Hand at the end of each period (interval month by default, quarter, year, or none for every day) from..to: cash on hand, money on hand, free money, funds, reserves and receivables. At most 366 points.",
        annotations(read_only_hint = true)
    )]
    async fn cash_on_hand_history(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::reports::CashHistoryParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            &crate::reports::cash_on_hand_history(&self.state, principal(&parts)?, &p)
                .await
                .map_err(to_mcp_error)?,
        )
    }

    #[tool(
        description = "Cash flow from..to: why Cash on Hand changed. opening_cash (end of the day before from), closing_cash, change, and lines grouped by kind (default; loan principal shows as equity_conversion) or account. Each line has cash (from transactions that move free money), non_cash (from those that don't, e.g. interest accruals and true-ups) and total. Card and card-reserve postings that cancel are left out. The cash column sums to change and the non_cash column to zero. exclude (kinds or prefixes) gives change_excluding without their cash. include_transactions lists each line's transactions (limit per line, next_cursor to page one line).",
        annotations(read_only_hint = true)
    )]
    async fn cash_flow(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::reports::CashFlowParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            &crate::reports::cash_flow(&self.state, principal(&parts)?, &p)
                .await
                .map_err(to_mcp_error)?,
        )
    }

    #[tool(
        description = "Receivables activity from..to, per counterparty (receivable account) and in total: opening, filed (debits), collected (credits), closing; filed_by_counter_account says what each filing was set against, netted_against_expense when it credited an expense. include_entries lists each entry with its counter postings.",
        annotations(read_only_hint = true)
    )]
    async fn receivables_activity(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::reports::ReceivablesParams>,
    ) -> Result<CallToolResult, McpError> {
        structured(
            &crate::reports::receivables_activity(&self.state, principal(&parts)?, &p)
                .await
                .map_err(to_mcp_error)?,
        )
    }

    #[tool(
        description = "Record a shared expense, netting their share against existing payables first. Card payments include reserve lines. Requires current head; dry_run previews without writing.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn record_shared_expense(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SharedExpenseParams>,
    ) -> Result<CallToolResult, McpError> {
        write_result(
            crate::split::record(&self.state, principal(&parts)?, p.body, &p.head)
                .await
                .map_err(to_mcp_error)?,
        )
    }

    // --- write tools --------------------------------------------------------

    #[tool(
        description = "Add a new transaction. Kabacus records transfers but cannot perform them. Accounts default to real physical accounts, except reserves are synthetic. Compare synthetic and physical_home with list_accounts; if physical homes differ and at least one is non-null, refuse to preview or record a transfer unless the user explicitly confirms it already happened in the real world. Never use this tool or batch to bypass allocate's physical-boundary rejection without that confirmation. Amounts are decimal strings (\"-12.34\"). Defaults to dry_run=true: the same validation as a commit (balance assertions included), returning a compact preview {id, before, after, balance_changes} and the head to commit with; call again with dry_run=false and that same head to commit. include_diff adds a unified diff of the changed text. Postings to funds and card reserves are ordinary postings. A card transaction without its matching reserve line is written but flagged unreserved: true, with missing_reserve.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn add_transaction(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<AddTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        let commodity = self.commodity().await?;
        self.write(&parts, p.op(&commodity), &head, dry_run, diff)
            .await
    }

    #[tool(
        description = "Allocate money: from a money account's free money to one of its funds or card reserves, back again (release), or between two funds or reserves of the same money account. With new_fund, sets money aside into a new fund directly in the money account `from`, declaring it in the same commit. Checked by account kind; refused if it takes more than the source holds on its date or any later one, naming the shortfall. Adds one ordinary allocation transaction. Check synthetic and physical_home: reserves in different physical accounts require a real transfer. Explain this and refuse to preview or record it unless the user explicitly confirms the real transfer already happened. Kabacus cannot perform bank transfers; never bypass this rejection with add_transaction or batch without that confirmation. amount is a decimal string. Defaults to dry_run=true.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn allocate(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<AllocateParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        let commodity = self.commodity().await?;
        self.write(&parts, p.op(&commodity), &head, dry_run, diff)
            .await
    }

    #[tool(
        description = "Record a purchase on a credit card as one transaction with its four lines: the expense, the charge to the card, and the amount moved from its source (\"free\" money or a fund) into the card's reserve. A negative amount (decimal string) is a refund back to the same source. Defaults to dry_run=true.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn record_card_purchase(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RecordCardPurchaseParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        let commodity = self.commodity().await?;
        self.write(&parts, p.op(&commodity), &head, dry_run, diff)
            .await
    }

    #[tool(
        description = "Pay a credit card from its reserve: the card goes up by the amount and its reserve down. Defaults to dry_run=true.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn pay_card(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PayCardParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        let commodity = self.commodity().await?;
        self.write(&parts, p.op(&commodity), &head, dry_run, diff)
            .await
    }

    #[tool(
        description = "Edit a transaction in place: replaces its date, description and postings, keeping its id and, unless given, its tags; postings keep their comments and assertions. Validated like any write and recorded as one revision, so the old version stays in history. For typos and wrong amounts, accounts, dates or payees, and for adding a card's reserve lines. Defaults to dry_run=true, returning {id, before, after, balance_changes}; include_diff adds a diff of the changed text. Flagged unreserved: true if the card's reserve line is still missing.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn edit_transaction(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<EditTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        let commodity = self.commodity().await?;
        self.write(&parts, p.op(&commodity), &head, dry_run, diff)
            .await
    }

    #[tool(
        description = "Correct a transaction: an edit in place of its description and postings, keeping its id and date; history keeps the old version. Defaults to dry_run=true.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn correct_transaction(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<CorrectTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        let commodity = self.commodity().await?;
        self.write(&parts, p.op(&commodity), &head, dry_run, diff)
            .await
    }

    #[tool(
        description = "Void a transaction: adds its reversal, dated today, whose comment says which transaction it reverses. Never removes the original. Defaults to dry_run=true.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn void_transaction(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<VoidTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        let commodity = self.commodity().await?;
        self.write(&parts, p.op(&commodity), &head, dry_run, diff)
            .await
    }

    #[tool(
        description = "Set one account's posting status on a transaction: unmarked, pending or cleared. Changes nothing else. Defaults to dry_run=true.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn set_cleared(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SetClearedParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run, diff) = (p.head.clone(), p.dry_run, p.include_diff);
        self.write(&parts, p.op(), &head, dry_run, diff).await
    }

    #[tool(
        description = "Reconcile an account (money, revolving-debt, non-liquid or term-liability) with a statement: balance (decimal string, as booked) as of the end of date; a money account is compared with its inclusive balance. Returns {ledger_balance, statement_balance, difference (ledger - statement), status: matched|mismatch}; on a mismatch, suggestions (sets of up to three uncleared transactions that sum to the difference) and later_transactions (dated after date, that would explain it if dated earlier). mode check (default) writes nothing; assert writes a zero-amount balance assertion tagged reconciled (=* on a money account), refused with reconcile_mismatch unless matched within tolerance; adjust first posts the difference to equity:adjustments (needs confirm_adjust: true). mark_cleared marks the account's postings up to date cleared when matched. Writing needs head; defaults to dry_run=true.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn reconcile(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::reports::ReconcileParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut result = crate::reports::reconcile(&self.state, principal(&parts)?, &p)
            .await
            .map_err(to_mcp_error)?;
        if let Some(written) = result.get_mut("written").and_then(Value::as_object_mut) {
            written.remove("preview");
            if let Some(b) = written.remove("balances") {
                written.insert("balance_changes".to_string(), b);
            }
        }
        structured(&result)
    }

    #[tool(
        description = "Run many write operations as one: ops (at most 100) are {tool, args, client_ref?}, args being the tool's own arguments without head and dry_run. Tools: add_transaction, edit_transaction, record_card_purchase, pay_card, allocate, set_cleared, void_transaction, correct_transaction, and (with ledger-rewrite) declare_account, set_account_type, rename_account, remove_account, set_balance_assertion, rewrite_transaction, delete_transaction, convert_money_assertions. Operations apply in order, each seeing the ones before it (declare a fund, then allocate into it); {\"$ref\": \"<client_ref>.id\"} in args is an earlier operation's id. head is checked once, the books are checked once in their final state, and commit \"single\" (default) makes one revision (\"per_op\": one per operation). mode \"atomic\" (default): any failure writes nothing and names its index and client_ref; \"continue_on_error\" (dry runs only) lists every failure. dry_run defaults to true. A batch with any rewrite tool needs the rewrite gates. batch_id is recorded with the write: committing it again returns the first result and writes nothing. Returns {head, dry_run, ops: [{index, client_ref, status, id, warnings}], balance_changes (touched accounts, at most 200), counts}; no diffs. Limits: 100 operations, 10 MB, 120 seconds (batch_too_large).",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn batch(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<crate::batch::BatchBody>,
    ) -> Result<CallToolResult, McpError> {
        let result = crate::batch::batch(&self.state, principal(&parts)?, p)
            .await
            .map_err(to_mcp_error)?;
        structured(&result)
    }

    // --- rewrite tools (gated; listed only when usable) ---------------------

    #[tool(
        description = "Rewrite a transaction outright, including its tags, without the checks an edit has. Needs your ledger-rewrite role, and is refused if the server was started with --no-rewrite. Defaults to dry_run=true, returning a diff of the changed text; call again with dry_run=false and the same head to commit. Prefer edit_transaction.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn rewrite_transaction(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RewriteTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run) = (p.head.clone(), p.dry_run);
        let commodity = self.commodity().await?;
        self.rewrite(&parts, p.op(&commodity), &head, dry_run).await
    }

    #[tool(
        description = "Delete a transaction from the books. Needs your ledger-rewrite role, and is refused if the server was started with --no-rewrite. Defaults to dry_run=true, returning a diff; call again with dry_run=false and the same head to commit. Prefer void_transaction.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn delete_transaction(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<DeleteTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.rewrite(&parts, Ok(WriteOp::Delete { id: p.id }), &p.head, p.dry_run)
            .await
    }

    #[tool(
        description = "Declare a new account, optionally with tags such as a friendly name and synthetic=true or false. Default is real (false), except funds and card reserves are synthetic (true). Its accounting kind comes from its path (see account_kinds). Use set_account_type to convert an existing account, only at the user's request. Needs your ledger-rewrite role, and is refused if the server was started with --no-rewrite. Defaults to dry_run=true, returning a diff; call again with dry_run=false and the same head to commit.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn declare_account(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<DeclareAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run) = (p.head.clone(), p.dry_run);
        self.rewrite(&parts, p.op(), &head, dry_run).await
    }

    #[tool(
        description = "Convert an account between real (synthetic=false) and synthetic (synthetic=true). Defaults are real, except funds and card reserves are synthetic. Preserves its accounting kind, postings, balances, other tags and history. Requires ledger-rewrite and server rewrites enabled. Defaults to dry_run=true. Only convert when the user asks to change the account's meaning; never convert to bypass a physical transfer confirmation. Read list_accounts' synthetic and physical_home before transfers.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn set_account_type(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SetAccountTypeParams>,
    ) -> Result<CallToolResult, McpError> {
        let op = WriteOp::SetAccountType {
            account: p.account,
            synthetic: p.synthetic,
        };
        self.rewrite(&parts, Ok(op), &p.head, p.dry_run).await
    }

    #[tool(
        description = "Rename an account, and every account under it, in its declaration and every posting, balance assertion and tag value. to must not exist; merge: true merges into an existing to of the same kind. A new path that changes an account's kind is refused unless allow_kind_change: true (the preview warns kind_change). Needs ledger-rewrite. The result gives the occurrences replaced, the transactions changed and three sample transactions (no diff). Defaults to dry_run=true; call again with dry_run=false and the same head to commit.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn rename_account(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RenameAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        let op = WriteOp::RenameAccount {
            from: p.from,
            to: p.to,
            merge: p.merge.unwrap_or(false),
            allow_kind_change: p.allow_kind_change.unwrap_or(false),
        };
        self.rewrite(&parts, Ok(op), &p.head, p.dry_run).await
    }

    #[tool(
        description = "Remove an account's declaration, for an account declared by mistake. Refused with account_in_use, and a count of each kind of reference (postings, assertions, sub_accounts), if anything still refers to it. Needs ledger-rewrite. Defaults to dry_run=true, returning a preview; call again with dry_run=false and the same head to commit.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn remove_account(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RemoveAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        let op = WriteOp::RemoveAccount { account: p.account };
        self.rewrite(&parts, Ok(op), &p.head, p.dry_run).await
    }

    #[tool(
        description = "Import hledger journals into these books, which must be empty: files maps each journal's name to its text and main names the one that includes the others. Each transaction gets the next id in read order. What Kabacus doesn't support (tags, P lines, automated and periodic transactions, comments outside transactions, payee and tag directives) is left out and listed in the report with file and line, as is a commodity whose places were raised or an assertion that doesn't hold at the end of its day; a date without a year, alias, apply account, decimal-mark, D, Y or an unreadable line stops the import, naming the file and line, and so does a book that fails the strict check. Needs your ledger-rewrite role. Defaults to dry_run=true, returning the counts and the report; call again with dry_run=false to import.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn import_journal(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ImportJournalParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = crate::import::import_journal(
            &self.state,
            principal(&parts)?,
            crate::import::JournalImportBody {
                main: p.main,
                files: p.files,
                dry_run: p.dry_run.unwrap_or(true),
            },
        )
        .await
        .map_err(to_mcp_error)?;
        structured(&result)
    }

    #[tool(
        description = "Remove or update one balance assertion, addressed by the transaction and account a failed assertion's error names (details.transaction). Use it when a write fails because an assertion is out of date: remove the assertion (no balance), retry the write, then set it to its new balance (a decimal string) if it should stay. A balance the books don't reach is refused. Needs your ledger-rewrite role, and is refused if the server was started with --no-rewrite. Defaults to dry_run=true, returning a diff; call again with dry_run=false and the same head to commit.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn set_balance_assertion(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SetBalanceAssertionParams>,
    ) -> Result<CallToolResult, McpError> {
        let (head, dry_run) = (p.head.clone(), p.dry_run);
        let commodity = self.commodity().await?;
        self.rewrite(&parts, p.op(&commodity), &head, dry_run).await
    }

    #[tool(
        description = "Make every exclusive balance assertion (= or ==) on a money account inclusive (=* or ==*), so funds and card reserves in sub-accounts never break bank reconciliation. Do this before adding reserve lines to past card transactions. Needs your ledger-rewrite role, and is refused if the server was started with --no-rewrite. Defaults to dry_run=true, returning a diff; call again with dry_run=false and the same head to commit.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn convert_money_assertions(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ConvertMoneyAssertionsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.rewrite(
            &parts,
            Ok(WriteOp::ConvertMoneyAssertions),
            &p.head,
            p.dry_run,
        )
        .await
    }

    // --- skill guides -------------------------------------------------------

    #[tool(
        description = "Get a bundled skill guide. CALL THIS FIRST, before changing the books, if your client can't read MCP resources (skill:// URIs); otherwise read those instead. With no arguments: SKILL.md of kabacus-bookkeeping, with the workflow, common mistakes and a table of which reference file to read for each task. Then pass file='<path>' exactly as SKILL.md links it, only for the task at hand.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_skill_guide(
        &self,
        Parameters(p): Parameters<GetSkillGuideParams>,
    ) -> Result<CallToolResult, McpError> {
        let names = || {
            skills::SKILLS
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let skill = match p.skill.as_deref() {
            None => &skills::SKILLS[0],
            Some(name) => skills::skill(name).ok_or_else(|| {
                McpError::invalid_params(
                    format!("no skill {name:?}; the skills are: {}", names()),
                    None,
                )
            })?,
        };
        let file = p.file.unwrap_or_else(|| "SKILL.md".to_string());
        let content = skill.file(&file).ok_or_else(|| {
            McpError::invalid_params(
                format!(
                    "{} has no file {file:?}; its files are: {}",
                    skill.name,
                    skill.paths().join(", ")
                ),
                None,
            )
        })?;
        structured(&json!({
            "skill": skill.name,
            "file": file,
            "uri": skill.uri(&file),
            "content": content,
            "files": skill.paths(),
        }))
    }
}

impl KabacusMcp {
    fn caller<'a>(&self, context: &'a RequestContext<RoleServer>) -> Option<&'a Principal> {
        context
            .extensions
            .get::<Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>())
    }

    /// Whether this caller last listed a different set of tools.
    fn stale_for(&self, principal: &Principal) -> bool {
        let seen = self.state.tool_lists.seen(principal);
        seen.is_some_and(|seen| seen != self.visible_hash(principal))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for KabacusMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .build(),
        )
        // The same version `get_version` reports.
        .with_server_info(Implementation::new(
            "kabacus-server",
            kabacus_ledger::VERSION,
        ))
        .with_instructions(skills::instructions())
    }

    /// Only the tools this caller can use: the rewrite tools are hidden
    /// unless the rewrite gates are open for them (hidden by default).
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let parts = context
            .extensions
            .get::<Parts>()
            .ok_or_else(|| McpError::internal_error("no HTTP request", None))?;
        let principal = principal(parts)?;
        let tools = self.visible_tools(principal);
        self.state
            .tool_lists
            .set_seen(principal, &hash_tools(&tools));
        Ok(ListToolsResult::with_all_items(tools))
    }

    /// A client still on a protocol without `subscriptions/listen`
    /// hears that its tool list is out of date with its next call.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let legacy = context
            .protocol_version()
            .is_none_or(|v| v < ProtocolVersion::V_2026_07_28);
        if legacy {
            if let Some(principal) = self.caller(&context) {
                if self.stale_for(principal) {
                    let _ = context.peer.notify_tool_list_changed().await;
                }
            }
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    /// `notifications/tools/list_changed` over `subscriptions/listen`.
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(match requested.tools_list_changed {
            Some(true) => SubscriptionFilter::builder().tools_list_changed().build(),
            _ => SubscriptionFilter::new(),
        })
    }

    /// Tell the client at once if its tool list is out of date (it
    /// reconnected after the server changed), then whenever the tools
    /// change, until it goes away.
    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        let principal = self.caller(context.request_context()).cloned();
        if context.accepted().tools_list_changed != Some(true) {
            context.cancelled().await;
            return Ok(());
        }
        if principal.as_ref().is_some_and(|p| self.stale_for(p)) {
            let _ = context.sink().notify_tool_list_changed().await;
        }
        let mut changes = self.state.tool_lists.subscribe();
        loop {
            tokio::select! {
                _ = context.cancelled() => return Ok(()),
                changed = changes.changed() => {
                    if changed.is_err() {
                        context.cancelled().await;
                        return Ok(());
                    }
                    let _ = context.sink().notify_tool_list_changed().await;
                }
            }
        }
    }

    /// Every bundled skill file, at `skill://<skill>/<path>`.
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let resources = skills::SKILLS
            .iter()
            .flat_map(|skill| {
                skill.files.iter().map(move |(path, body)| {
                    let mut resource =
                        Resource::new(skill.uri(path), format!("{}/{path}", skill.name));
                    resource.mime_type = Some("text/markdown".to_string());
                    resource.size = Some(body.len() as u64);
                    if *path == "SKILL.md" {
                        resource.description = Some(skill.description().to_string());
                    }
                    resource
                })
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let (_, _, body) = skills::resolve_uri(&request.uri).ok_or_else(|| {
            McpError::resource_not_found(format!("no resource {}", request.uri), None)
        })?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(body, request.uri).with_mime_type("text/markdown")
        ])
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash is the same for the same tools in any order, and changes
    /// when any tool's schema does.
    #[test]
    fn the_tool_list_hash_follows_the_schemas() {
        let tools = KabacusMcp::tool_router().list_all();
        let mut reversed = tools.clone();
        reversed.reverse();
        assert_eq!(hash_tools(&tools), hash_tools(&reversed));
        assert_eq!(hash_tools(&tools), tool_list_hash());

        let mut changed = tools.clone();
        let schema = Arc::make_mut(&mut changed[0].input_schema);
        schema.insert("x-new-field".to_string(), json!(true));
        assert_ne!(hash_tools(&tools), hash_tools(&changed));

        let fewer = &tools[1..];
        assert_ne!(hash_tools(&tools), hash_tools(fewer));
    }
}
