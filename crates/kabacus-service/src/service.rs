// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Book operations shared by local callers and server transports.

use crate::caller::{Caller, Permission as Role};
use crate::error::Error as ApiError;
use crate::ops;
use crate::state::Service;
use chrono::NaiveDate;
use kabacus_core::{
    format_money, AccountName, Amount, AssertionEdit, Chart, DraftPosting, Money, Status,
};

use serde::{Deserialize, Serialize};

// --- checks and parsing ---------------------------------------------------

pub fn require(principal: &dyn Caller, permission: Role) -> Result<(), ApiError> {
    principal.require(permission)
}

/// The first two rewrite gates. The third (the app's own setting) is
/// client-side and can't be checked here.
pub fn check_rewrite_gates(state: &Service, principal: &dyn Caller) -> Result<(), ApiError> {
    if !state.settings.allow_rewrite {
        return Err(ApiError::RewriteDisabled);
    }
    require(principal, Role::Rewrite)
}

pub fn invalid(rule: &str, message: String) -> ApiError {
    ApiError::Invalid {
        rule: rule.to_string(),
        message,
        failures: Vec::new(),
    }
}

pub fn parse_date(s: &str) -> Result<NaiveDate, ApiError> {
    kabacus_core::parse_date(s).map_err(|e| invalid("date", e.to_string()))
}

pub fn parse_optional_date(s: Option<&str>) -> Result<Option<NaiveDate>, ApiError> {
    s.map(parse_date).transpose()
}

pub fn parse_status(s: &str) -> Status {
    match s {
        "pending" => Status::Pending,
        "cleared" => Status::Cleared,
        _ => Status::Unmarked,
    }
}

pub fn status_str(s: Status) -> &'static str {
    match s {
        Status::Unmarked => "unmarked",
        Status::Pending => "pending",
        Status::Cleared => "cleared",
    }
}

/// The books' commodity, for Cash on Hand and counterparty statements.
pub async fn books_commodity(state: &Service) -> Result<String, ApiError> {
    let snapshot = state.books_snapshot().await.map_err(ApiError::Internal)?;
    Ok(crate::books::primary_commodity(
        &snapshot.book,
        &state.kind_rules_chart(),
    ))
}

/// The books' commodity and the decimal places it takes.
#[derive(Debug, Clone, Serialize)]
pub struct CommodityDto {
    pub commodities: Vec<CurrencyDto>,
    pub symbol: String,
    pub places: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrencyDto {
    pub symbol: String,
    pub places: u32,
}

pub async fn books_commodity_info(
    state: &Service,
    principal: &dyn Caller,
) -> Result<CommodityDto, ApiError> {
    require(principal, Role::Read)?;
    let symbol = books_commodity(state).await?;
    let books = state.books_snapshot().await.map_err(ApiError::Internal)?;
    let wc = crate::books::WorkingCopy::from_snapshot(books, state.kind_table());
    let places = wc.places(&symbol);
    let commodities = wc
        .book
        .commodities
        .keys()
        .map(|symbol| CurrencyDto {
            symbol: symbol.clone(),
            places: wc.places(symbol),
        })
        .collect();
    Ok(CommodityDto {
        symbol,
        places,
        commodities,
    })
}

// --- DTOs -------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AmountDto {
    pub commodity: String,
    pub exact: String,
    pub formatted: String,
}

impl From<&Amount> for AmountDto {
    fn from(a: &Amount) -> Self {
        AmountDto {
            commodity: a.commodity.clone(),
            exact: a.money.to_string(),
            formatted: format_money(a),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PostingDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostDto>,
    pub account: String,
    pub amount: AmountDto,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostDto {
    pub kind: &'static str,
    pub amount: AmountDto,
}

impl From<&kabacus_core::Cost> for CostDto {
    fn from(cost: &kabacus_core::Cost) -> Self {
        Self {
            kind: match cost.kind {
                kabacus_core::CostKind::Unit => "unit",
                kabacus_core::CostKind::Total => "total",
            },
            amount: AmountDto::from(&cost.amount),
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CostInput {
    pub kind: CostKindInput,
    pub amount: AmountInput,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CostKindInput {
    Unit,
    Total,
}

/// What a card transaction didn't post to the card's reserve.
#[derive(Debug, Clone, Serialize)]
pub struct MissingReserveDto {
    pub card: String,
    pub reserve: String,
    /// What the reserve still needs from this transaction.
    pub amount: AmountDto,
}

fn missing_reserve_dtos(missing: &[kabacus_core::MissingReserve]) -> Vec<MissingReserveDto> {
    missing
        .iter()
        .map(|m| MissingReserveDto {
            card: m.card.to_string(),
            reserve: m.reserve.to_string(),
            amount: AmountDto::from(&m.amount),
        })
        .collect()
}

/// Which transactions are flagged unreserved, by id.
pub struct UnreservedIndex {
    flags: std::collections::HashMap<u64, Vec<kabacus_core::MissingReserve>>,
}

impl UnreservedIndex {
    pub fn new(chart: &Chart, all: &[kabacus_core::Transaction]) -> UnreservedIndex {
        UnreservedIndex {
            flags: kabacus_core::unreserved_flags(chart, all)
                .into_iter()
                .map(|(i, m)| (all[i].id, m))
                .collect(),
        }
    }

    pub fn get(&self, tx: &kabacus_core::Transaction) -> &[kabacus_core::MissingReserve] {
        self.flags
            .get(&tx.id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TransactionDto {
    /// The transaction's own number in the books.
    pub id: u64,
    pub date: String,
    pub payee: String,
    pub memo: String,
    pub status: String,
    /// The transaction's comment, tags included.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    pub postings: Vec<PostingDto>,
    pub tags: Vec<(String, String)>,
    /// It posts to a card without the matching reserve line.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unreserved: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_reserve: Vec<MissingReserveDto>,
}

pub fn to_dto(
    tx: &kabacus_core::Transaction,
    missing: &[kabacus_core::MissingReserve],
) -> TransactionDto {
    let missing_reserve = missing_reserve_dtos(missing);
    TransactionDto {
        unreserved: !missing_reserve.is_empty(),
        missing_reserve,
        id: tx.id,
        date: tx.date.format("%Y-%m-%d").to_string(),
        payee: tx.payee().to_string(),
        memo: tx.memo().to_string(),
        status: status_str(tx.status).to_string(),
        comment: tx.comment.clone(),
        postings: tx
            .postings
            .iter()
            .map(|p| PostingDto {
                cost: p.cost.as_ref().map(CostDto::from),
                account: p.account.as_str().to_string(),
                amount: AmountDto::from(&p.amount),
                status: status_str(p.status).to_string(),
            })
            .collect(),
        tags: tx.tags.clone(),
    }
}

/// One transaction's (or declaration's) change in a write's preview: its
/// text before and after.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

impl From<&crate::books::Change> for ChangeDto {
    fn from(c: &crate::books::Change) -> Self {
        ChangeDto {
            id: c.id,
            before: c.before.clone(),
            after: c.after.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteResultDto {
    /// The transaction written (none for an account operation).
    pub id: Option<u64>,
    pub head: String,
    pub dry_run: bool,
    /// The (first) transaction written: its text before and after.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Every transaction written, when there is more than one: at most
    /// [`CHANGES_SHOWN`] of them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transactions: Vec<ChangeDto>,
    /// How many changes the write makes, when there are more than shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed: Option<usize>,
    /// The app's preview: the transaction(s) written, or for an edit the
    /// diff of its lines.
    pub preview: String,
    /// Touched accounts' balances before and after.
    pub balances: Vec<BalanceChangeDto>,
    pub cash_on_hand: CashPreviewDto,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub balance_changes_truncated: bool,
    /// A unified diff of the changed text, only with `include_diff`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// The written transaction posts to a card without the
    /// matching reserve line. Flagged, never refused.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unreserved: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_reserve: Vec<MissingReserveDto>,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub details: serde_json::Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<crate::ops::OpWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CashPreviewDto {
    pub before: AmountDto,
    pub after: AmountDto,
}

impl From<&(Amount, Amount)> for CashPreviewDto {
    fn from(change: &(Amount, Amount)) -> Self {
        Self {
            before: AmountDto::from(&change.0),
            after: AmountDto::from(&change.1),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BalanceChangeDto {
    pub account: String,
    pub before: AmountDto,
    pub after: AmountDto,
}

pub fn balance_change_dtos(changes: &[kabacus_core::BalanceChange]) -> Vec<BalanceChangeDto> {
    changes
        .iter()
        .map(|c| BalanceChangeDto {
            account: c.account.clone(),
            before: AmountDto::from(&c.before),
            after: AmountDto::from(&c.after),
        })
        .collect()
}

/// How many changes a write's result shows: a rename can change thousands
/// of transactions.
pub const CHANGES_SHOWN: usize = 5;

/// The app's preview of a write: the transactions it adds, or a diff of
/// the text it changes.
fn app_preview(changes: &[crate::books::Change]) -> String {
    let mut preview: String = changes
        .iter()
        .take(CHANGES_SHOWN)
        .map(|c| {
            let name =
                c.id.map(|id| format!("transaction #{id}"))
                    .unwrap_or_else(|| "declarations".to_string());
            match (&c.before, &c.after) {
                (Some(before), Some(after)) => crate::diff::unified_diff(&name, before, after),
                (None, Some(after)) => after.clone(),
                (Some(before), None) => crate::diff::unified_diff(&name, before, ""),
                (None, None) => String::new(),
            }
        })
        .collect();
    if changes.len() > CHANGES_SHOWN {
        preview.push_str(&format!(
            "… and {} more changes\n",
            changes.len() - CHANGES_SHOWN
        ));
    }
    preview
}

pub fn write_result_dto(result: crate::ops::WriteResult, include_diff: bool) -> WriteResultDto {
    let missing_reserve: Vec<MissingReserveDto> = result
        .missing_reserve
        .iter()
        .map(|m| MissingReserveDto {
            card: m.card.to_string(),
            reserve: m.reserve.to_string(),
            amount: AmountDto::from(&m.amount),
        })
        .collect();
    let first = result.changes.first().map(ChangeDto::from);
    WriteResultDto {
        unreserved: !missing_reserve.is_empty(),
        missing_reserve,
        id: result.id,
        head: result.head,
        dry_run: result.dry_run,
        before: first.as_ref().and_then(|c| c.before.clone()),
        after: first.as_ref().and_then(|c| c.after.clone()),
        transactions: if result.changes.len() > 1 {
            result
                .changes
                .iter()
                .take(CHANGES_SHOWN)
                .map(ChangeDto::from)
                .collect()
        } else {
            Vec::new()
        },
        changed: (result.changes.len() > CHANGES_SHOWN).then_some(result.changes.len()),
        preview: app_preview(&result.changes),
        balances: balance_change_dtos(&result.balances),
        cash_on_hand: CashPreviewDto::from(&result.cash_on_hand),
        balance_changes_truncated: result.balances_truncated,
        diff: include_diff.then_some(result.diff),
        details: result.details,
        warnings: result.warnings,
    }
}

// --- request bodies ---------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct DraftPostingDto {
    #[serde(default)]
    pub cost: Option<CostInput>,
    #[serde(default)]
    pub clear_cost: bool,
    pub account: String,
    pub amount: Option<AmountInput>,
    #[serde(default)]
    pub status: String,
    /// Assert this balance after the posting (` = <amount>`). On an edit,
    /// leaving both this and `clear_balance_assertion` out keeps the
    /// posting's existing assertion.
    #[serde(default)]
    pub balance_assertion: Option<AmountInput>,
    #[serde(default)]
    pub clear_balance_assertion: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExactAmountDto {
    pub commodity: String,
    #[serde(with = "kabacus_core::money::wire_mantissa")]
    pub mantissa: kabacus_core::BigInt,
    pub places: u32,
}

/// An amount as a caller writes it: a decimal string such as
/// `"-12.34"` (in the books' commodity), or the exact
/// `{commodity, mantissa, places}` object. A JSON number is refused with
/// `amount_format`: money is never a float.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(transparent)]
pub struct AmountInput(pub serde_json::Value);

impl AmountInput {
    pub fn decimal(text: &str) -> AmountInput {
        AmountInput(serde_json::Value::String(text.to_string()))
    }

    /// The amount, in `commodity` for a decimal string. `path` names the
    /// field in an `amount_format` error.
    pub fn to_amount(&self, commodity: &str, path: &str) -> Result<Amount, ApiError> {
        let refuse = |message: String| ApiError::AmountFormat {
            path: path.to_string(),
            message,
        };
        match &self.0 {
            serde_json::Value::String(text) => text
                .parse()
                .map(|money| Amount::new(commodity, money))
                .map_err(|e: kabacus_core::AmountError| refuse(e.to_string())),
            serde_json::Value::Number(n) => Err(refuse(format!(
                "{n} is a JSON number; write amounts as decimal strings (\"{n}\") so they stay exact"
            ))),
            object @ serde_json::Value::Object(fields) => {
                // A JSON number beyond 64 bits has already been rounded by
                // the reader; only the string form is exact.
                if fields.get("mantissa").is_some_and(|m| m.is_f64()) {
                    return Err(refuse(
                        "write a mantissa beyond 64 bits as a string of digits".to_string(),
                    ));
                }
                let exact: ExactAmountDto = serde_json::from_value(object.clone()).map_err(|e| {
                    refuse(format!(
                        "an amount object needs commodity, mantissa and places: {e}"
                    ))
                })?;
                if exact.places > kabacus_core::money::MAX_PLACES {
                    return Err(refuse(format!(
                        "places can be at most {}",
                        kabacus_core::money::MAX_PLACES
                    )));
                }
                Ok(to_amount(exact))
            }
            other => Err(refuse(format!(
                "{other} isn't an amount; write a decimal string like \"-12.34\""
            ))),
        }
    }

    pub fn to_money(&self, commodity: &str, path: &str) -> Result<Money, ApiError> {
        Ok(self.to_amount(commodity, path)?.money)
    }
}

impl schemars::JsonSchema for AmountInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Amount".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "An exact amount: a decimal string such as \"-12.34\" in the books' commodity, or {commodity, mantissa, places}. Never a JSON number.",
            "anyOf": [
                {"type": "string", "pattern": "^-?\\d+(\\.\\d+)?$"},
                {
                    "type": "object",
                    "properties": {
                        "commodity": {"type": "string"},
                        "mantissa": {"description": "An integer; as a string of digits when beyond 64 bits.", "type": ["integer", "string"]},
                        "places": {"type": "integer", "minimum": 0}
                    },
                    "required": ["commodity", "mantissa", "places"]
                }
            ]
        })
    }
}

fn to_amount(a: ExactAmountDto) -> Amount {
    Amount::new(
        a.commodity,
        kabacus_core::Money::from_big(a.mantissa, a.places),
    )
}

pub fn to_draft_postings(
    postings: Vec<DraftPostingDto>,
    commodity: &str,
) -> Result<Vec<DraftPosting>, ApiError> {
    postings
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let amount = p
                .amount
                .as_ref()
                .map(|a| a.to_amount(commodity, &format!("postings[{i}].amount")))
                .transpose()?;
            let assertion = p
                .balance_assertion
                .as_ref()
                .map(|a| a.to_amount(commodity, &format!("postings[{i}].balance_assertion")))
                .transpose()?;
            let balance_assertion = match (assertion, p.clear_balance_assertion) {
                (None, false) => AssertionEdit::Keep,
                (Some(a), false) => AssertionEdit::Set(a),
                (None, true) => AssertionEdit::Clear,
                (Some(_), true) => {
                    return Err(invalid(
                        "postings",
                        format!(
                            "the posting on {} both sets and clears its balance assertion",
                            p.account
                        ),
                    ))
                }
            };
            let cost = match (p.cost, p.clear_cost) {
                (None, false) => kabacus_core::CostEdit::Keep,
                (None, true) => kabacus_core::CostEdit::Clear,
                (Some(cost), false) => kabacus_core::CostEdit::Set(kabacus_core::Cost {
                    kind: match cost.kind {
                        CostKindInput::Unit => kabacus_core::CostKind::Unit,
                        CostKindInput::Total => kabacus_core::CostKind::Total,
                    },
                    amount: cost
                        .amount
                        .to_amount(commodity, &format!("postings[{i}].cost.amount"))?,
                }),
                (Some(_), true) => {
                    return Err(invalid(
                        "cost",
                        "A posting cannot set and clear its cost".into(),
                    ))
                }
            };
            Ok(DraftPosting {
                cost,
                account: AccountName::new(p.account),
                amount,
                status: parse_status(&p.status),
                balance_assertion,
            })
        })
        .collect()
}

/// Run one write, mapping a preview to an error (only rewrites preview).
pub async fn run_write(
    state: &Service,
    principal: &dyn Caller,
    op: ops::WriteOp,
    if_match: &str,
    dry_run: bool,
    include_diff: bool,
) -> Result<WriteResultDto, ApiError> {
    let result = ops::write(state, principal, op, if_match, dry_run).await?;
    Ok(write_result_dto(result, include_diff))
}

/// Run one rewrite after its gates: a dry run, or committed. Account
/// renames and removals report per-file counts and samples in `details`
/// instead of a diff.
pub async fn run_rewrite(
    state: &Service,
    principal: &dyn Caller,
    op: ops::WriteOp,
    if_match: &str,
    dry_run: bool,
) -> Result<WriteResultDto, ApiError> {
    check_rewrite_gates(state, principal)?;
    let result = ops::write(state, principal, op, if_match, dry_run).await?;
    let counts_only =
        result.details.get("occurrences").is_some() || result.details.get("removed").is_some();
    Ok(write_result_dto(result, !counts_only))
}

// --- transactions -----------------------------------------------------------

/// `GET /transactions` without paging: every match, as the app reads them.
/// The same filters as `search_transactions`.
pub async fn list_transactions(
    state: &Service,
    principal: &dyn Caller,
    q: &crate::search::SearchQuery,
) -> Result<Vec<TransactionDto>, ApiError> {
    require(principal, Role::Read)?;
    Ok(crate::search::matching(state, q)
        .await?
        .into_iter()
        .map(|(dto, _)| dto)
        .collect())
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddTransactionBody {
    pub date: String,
    pub description: String,
    pub postings: Vec<DraftPostingDto>,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub include_diff: bool,
}

pub fn add_op(body: AddTransactionBody, commodity: &str) -> Result<ops::WriteOp, ApiError> {
    Ok(ops::WriteOp::Add(ops::AddRequest {
        date: parse_date(&body.date)?,
        description: body.description,
        postings: to_draft_postings(body.postings, commodity)?,
        tags: body.tags,
    }))
}

pub async fn add_transaction(
    state: &Service,
    principal: &dyn Caller,
    body: AddTransactionBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    require(principal, Role::Write)?;
    let commodity = books_commodity(state).await?;
    let (dry_run, include_diff) = (body.dry_run, body.include_diff);
    let op = add_op(body, &commodity)?;
    run_write(state, principal, op, if_match, dry_run, include_diff).await
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CorrectionBody {
    #[serde(default)]
    pub void: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub postings: Option<Vec<DraftPostingDto>>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub today: Option<String>,
    #[serde(default)]
    pub include_diff: bool,
}

pub fn correction_op(
    id: &str,
    body: CorrectionBody,
    commodity: &str,
) -> Result<ops::WriteOp, ApiError> {
    let today = match body.today {
        Some(s) => parse_date(&s)?,
        None => chrono::Utc::now().date_naive(),
    };
    let request = if body.void {
        ops::CorrectionRequest::Void
    } else {
        let description = body.description.ok_or_else(|| {
            invalid(
                "description",
                "description is required for a correct (non-void)".to_string(),
            )
        })?;
        let postings = body.postings.ok_or_else(|| {
            invalid(
                "postings",
                "postings are required for a correct (non-void)".to_string(),
            )
        })?;
        ops::CorrectionRequest::Correct {
            description,
            postings: to_draft_postings(postings, commodity)?,
        }
    };
    Ok(ops::WriteOp::Correct {
        id: ops::parse_id(id)?,
        request,
        today,
    })
}

/// Correct a transaction in place, or void it with a reversal.
pub async fn add_correction(
    state: &Service,
    principal: &dyn Caller,
    id: &str,
    body: CorrectionBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    require(principal, Role::Write)?;
    let commodity = books_commodity(state).await?;
    let (dry_run, include_diff) = (body.dry_run, body.include_diff);
    let op = correction_op(id, body, &commodity)?;
    run_write(state, principal, op, if_match, dry_run, include_diff).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusBody {
    pub account: String,
    pub status: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub include_diff: bool,
}

pub fn status_op(id: &str, body: &StatusBody) -> Result<ops::WriteOp, ApiError> {
    if !matches!(
        body.status.as_str(),
        "unmarked" | "pending" | "cleared" | ""
    ) {
        return Err(invalid(
            "status",
            format!(
                "{:?} isn't a status; use unmarked, pending or cleared",
                body.status
            ),
        ));
    }
    Ok(ops::WriteOp::SetStatus {
        id: ops::parse_id(id)?,
        account: AccountName::new(&body.account),
        status: parse_status(&body.status),
    })
}

/// One account's posting status, and nothing else.
pub async fn set_status(
    state: &Service,
    principal: &dyn Caller,
    id: &str,
    body: StatusBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    require(principal, Role::Write)?;
    let op = status_op(id, &body)?;
    run_write(
        state,
        principal,
        op,
        if_match,
        body.dry_run,
        body.include_diff,
    )
    .await
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RewriteBody {
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub postings: Option<Vec<DraftPostingDto>>,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    /// Validate and return the preview, commit nothing.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub include_diff: bool,
}

/// An edit (or rewrite) of transaction `id` from a body. `tags: None`
/// keeps the transaction's tags.
pub fn edit_request(
    id: &str,
    date: Option<String>,
    description: Option<String>,
    postings: Option<Vec<DraftPostingDto>>,
    tags: Option<Vec<(String, String)>>,
    commodity: &str,
    what: &str,
) -> Result<ops::EditRequest, ApiError> {
    let missing = |field: &str| invalid(field, format!("{field} is required for {what}"));
    Ok(ops::EditRequest {
        id: ops::parse_id(id)?,
        date: parse_date(&date.ok_or_else(|| missing("date"))?)?,
        description: description.ok_or_else(|| missing("description"))?,
        postings: to_draft_postings(postings.ok_or_else(|| missing("postings"))?, commodity)?,
        tags,
    })
}

/// The everyday in-place edit. Needs write access and is validated
/// and committed like any write, but has no rewrite gates:
/// it isn't a rewrite.
pub async fn edit_transaction(
    state: &Service,
    principal: &dyn Caller,
    id: &str,
    body: RewriteBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    require(principal, Role::Write)?;
    let commodity = books_commodity(state).await?;
    let request = edit_request(
        id,
        body.date,
        body.description,
        body.postings,
        Some(body.tags),
        &commodity,
        "an edit",
    )?;
    run_write(
        state,
        principal,
        ops::WriteOp::Edit(request),
        if_match,
        body.dry_run,
        body.include_diff,
    )
    .await
}

// --- history ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CommitDto {
    /// The revision this write made, as `head` gives it.
    pub hash: String,
    pub revision: i64,
    /// When, ISO 8601.
    pub date: String,
    pub summary: String,
    pub actor: Option<String>,
    pub kind: Option<String>,
    /// Who made it.
    pub user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// The latest writes, newest first.
pub async fn history(state: &Service, principal: &dyn Caller) -> Result<Vec<CommitDto>, ApiError> {
    use kabacus_store::BookStore;
    require(principal, Role::Read)?;
    let history = state
        .store
        .history(state.ledger, 50)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(history
        .into_iter()
        .map(|c| CommitDto {
            hash: c.revision.to_string(),
            revision: c.revision,
            date: c.at,
            summary: c.summary,
            actor: Some(c.actor),
            kind: Some(c.kind),
            user: c.user,
            agent: c
                .details
                .get("agent")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
        .collect())
}

/// What one write changed, as a diff of journal text.
pub async fn history_diff(
    state: &Service,
    principal: &dyn Caller,
    rev: &str,
) -> Result<String, ApiError> {
    use kabacus_store::BookStore;
    require(principal, Role::Read)?;
    let revision = crate::books::parse_head(rev)
        .ok_or_else(|| invalid("revision", format!("{rev:?} isn't a revision")))?;
    let record = state
        .store
        .change(state.ledger, revision)
        .await
        .map_err(|e| match e {
            kabacus_store::StoreError::NotFound(what) => ApiError::NotFound(what),
            other => ApiError::Internal(other.into()),
        })?;
    if revision < 2 {
        return Ok(String::new());
    }
    let before = state
        .books_at(Some(revision - 1))
        .await
        .map_err(ApiError::Internal)?;
    let after = state
        .books_at(Some(revision))
        .await
        .map_err(ApiError::Internal)?;
    Ok(crate::books::diff_changes(
        &before.book,
        &after.book,
        &kabacus_store::ChangeSet {
            changes: record.changes,
        },
    ))
}

/// The books as they were at `rev`, in the same shape as
/// [`list_transactions`].
pub async fn history_transactions(
    state: &Service,
    principal: &dyn Caller,
    rev: &str,
) -> Result<Vec<TransactionDto>, ApiError> {
    require(principal, Role::Read)?;
    let all = ops::transactions_at(state, rev).await?;
    let flags = UnreservedIndex::new(&state.kind_rules_chart(), &all);
    Ok(all.iter().map(|tx| to_dto(tx, flags.get(tx))).collect())
}

// --- funds, card reserves and Cash on Hand ------------------------------------

/// The books' accounts, classified by the kind rules in use, and
/// every transaction they were read from.
pub async fn books_chart(
    state: &Service,
) -> Result<(Chart, Vec<kabacus_core::Transaction>), ApiError> {
    let txs = ops::read_all_transactions(state).await?;
    let chart = books_chart_from(state, &txs).await?;
    Ok((chart, txs))
}

/// The chart of the books, given all their transactions.
pub async fn books_chart_from(
    state: &Service,
    txs: &[kabacus_core::Transaction],
) -> Result<Chart, ApiError> {
    let declared = state
        .declared_accounts()
        .await
        .map_err(ApiError::Internal)?;
    let friendly = state
        .friendly_account_names()
        .await
        .map_err(ApiError::Internal)?;
    let table = state.kind_table();
    let types = state.account_types().await.map_err(ApiError::Internal)?;
    Ok(Chart::from_books(
        table.rules,
        table.reserve_template,
        declared,
        txs,
        &friendly,
    )
    .with_account_types(&types))
}

/// A validation warning, never a refusal.
#[derive(Debug, Clone, Serialize)]
pub struct WarningDto {
    pub code: String,
    pub message: String,
    pub accounts: Vec<String>,
}

impl From<&kabacus_core::Warning> for WarningDto {
    fn from(w: &kabacus_core::Warning) -> Self {
        WarningDto {
            code: w.code.to_string(),
            message: w.message.clone(),
            accounts: w.accounts.clone(),
        }
    }
}

/// A warning while any money account still has an exclusive
/// (`=`) balance assertion, which funds and reserves would break.
async fn exclusive_assertion_warning(
    state: &Service,
    chart: &Chart,
) -> Result<Option<WarningDto>, ApiError> {
    let books = state.books_snapshot().await.map_err(ApiError::Internal)?;
    let mut places = Vec::new();
    let mut accounts = std::collections::BTreeSet::new();
    for tx in &books.transactions {
        for p in &tx.postings {
            let exclusive = p.balance_assertion.as_ref().is_some_and(|a| !a.inclusive);
            if exclusive && chart.kind_of(&p.account) == kabacus_core::AccountKind::Money {
                places.push(format!("#{}", tx.id));
                accounts.insert(p.account.as_str().to_string());
            }
        }
    }
    Ok((!places.is_empty()).then(|| WarningDto {
        code: "exclusive-assertion".to_string(),
        message: format!(
            "{} balance assertion(s) on money accounts are exclusive (=), so funds and card reserves would break them ({}); make them inclusive with convert_money_assertions before adding reserve lines",
            places.len(),
            places.join(", ")
        ),
        accounts: accounts.into_iter().collect(),
    }))
}

async fn all_warnings(
    state: &Service,
    chart: &Chart,
    warnings: &[kabacus_core::Warning],
) -> Result<Vec<WarningDto>, ApiError> {
    let mut out: Vec<WarningDto> = warnings.iter().map(WarningDto::from).collect();
    out.extend(exclusive_assertion_warning(state, chart).await?);
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct KindRuleDto {
    /// 1 for the first rule; the first match wins.
    pub number: usize,
    pub pattern: String,
    pub kind: kabacus_core::AccountKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassifiedAccountDto {
    pub account: String,
    pub display_name: String,
    pub kind: kabacus_core::AccountKind,
    pub synthetic: bool,
    pub physical_home: Option<String>,
    /// The rule it matched (its `number`), or none for `parent` and
    /// `unclassified`.
    pub rule: Option<usize>,
    pub pattern: Option<String>,
    /// For a fund or card reserve, the money account holding it.
    pub home: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountKindsDto {
    pub rules: Vec<KindRuleDto>,
    /// How each card's reserve is named; `{card}` is the card's last
    /// segment.
    pub card_reserve_account: String,
    pub accounts: Vec<ClassifiedAccountDto>,
    /// Accounts no rule matches (also among `accounts`).
    pub unclassified: Vec<String>,
    pub warnings: Vec<WarningDto>,
}

/// The rule table and every account (parents included) with the
/// rule it matched.
pub async fn account_kinds(
    state: &Service,
    principal: &dyn Caller,
) -> Result<AccountKindsDto, ApiError> {
    require(principal, Role::Read)?;
    let (chart, txs) = books_chart(state).await?;
    let commodity = books_commodity(state).await?;
    let warnings = kabacus_core::validation_warnings(&chart, &txs, &commodity);
    Ok(AccountKindsDto {
        rules: chart
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| KindRuleDto {
                number: i + 1,
                pattern: r.pattern.clone(),
                kind: r.kind,
            })
            .collect(),
        card_reserve_account: chart.reserve_template.clone(),
        accounts: chart
            .accounts
            .iter()
            .map(|c| ClassifiedAccountDto {
                account: c.account.to_string(),
                display_name: c.display_name.clone(),
                kind: c.kind,
                synthetic: c.synthetic,
                physical_home: chart
                    .physical_home(c.account.as_str())
                    .map(|a| a.to_string()),
                rule: c.rule.map(|i| i + 1),
                pattern: c.rule.map(|i| chart.rules[i].pattern.clone()),
                home: c.home.as_ref().map(|h| h.to_string()).unwrap_or_default(),
            })
            .collect(),
        unclassified: chart
            .accounts
            .iter()
            .filter(|c| c.kind == kabacus_core::AccountKind::Unclassified)
            .map(|c| c.account.to_string())
            .collect(),
        warnings: all_warnings(state, &chart, &warnings).await?,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountBalanceDto {
    pub account: String,
    pub display_name: String,
    pub balance: AmountDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct KindGroupDto {
    pub kind: kabacus_core::AccountKind,
    pub total: AmountDto,
    pub accounts: Vec<AccountBalanceDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomeDto {
    pub account: String,
    pub display_name: String,
    /// The exclusive balance: money in no fund or card reserve.
    pub free: AmountDto,
    pub funds: AmountDto,
    pub reserves: AmountDto,
    /// The inclusive balance, which matches the bank.
    pub inclusive: AmountDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct FundDto {
    pub account: String,
    pub display_name: String,
    pub home: String,
    pub balance: AmountDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnreservedTransactionDto {
    pub id: u64,
    pub date: String,
    pub description: String,
    /// What the card's reserve still needs from this transaction.
    pub missing_reserve: AmountDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardDto {
    pub card: String,
    pub display_name: String,
    /// As booked: negative while money is owed.
    pub balance: AmountDto,
    /// What is owed on the card.
    pub owed: AmountDto,
    pub reserve: String,
    pub reserve_exists: bool,
    pub reserve_balance: AmountDto,
    /// `owed - reserve_balance`: what the reserve still needs; zero when
    /// the reserve covers the card. Equals the flagged transactions'
    /// missing amounts.
    pub difference: AmountDto,
    /// Every flagged transaction.
    pub unreserved_count: usize,
    /// `detail: "full"` only: the first flagged transactions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreserved: Option<Vec<UnreservedTransactionDto>>,
    /// `detail: "full"` only: continue with `search_transactions`
    /// (`unreserved_only`, `account` = this card).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// A card; with `limit`, its first `limit` flagged transactions too.
fn card_dto(card: &kabacus_core::CardLine, limit: Option<usize>) -> CardDto {
    let unreserved: Vec<UnreservedTransactionDto> = card
        .unreserved
        .iter()
        .map(|u| UnreservedTransactionDto {
            id: u.id,
            date: u.date.format("%Y-%m-%d").to_string(),
            description: u.description.clone(),
            missing_reserve: AmountDto::from(&u.missing),
        })
        .collect();
    let (list, next_cursor) = match limit {
        None => (None, None),
        Some(limit) => {
            let more = unreserved.len() > limit;
            (
                Some(unreserved.into_iter().take(limit).collect()),
                more.then(|| crate::search::encode_cursor(limit)),
            )
        }
    };
    CardDto {
        card: card.card.to_string(),
        display_name: card.display_name.clone(),
        balance: AmountDto::from(&card.balance),
        owed: AmountDto::from(&card.owed),
        reserve: card.reserve.to_string(),
        reserve_exists: card.reserve_exists,
        reserve_balance: AmountDto::from(&card.reserve_balance),
        difference: AmountDto::from(&card.difference),
        unreserved_count: card.unreserved.len(),
        unreserved: list,
        next_cursor,
    }
}

/// The change in each Cash on Hand component since `compare_to`.
#[derive(Debug, Clone, Serialize)]
pub struct CashChangeDto {
    pub compare_to: String,
    pub money: AmountDto,
    pub funds: AmountDto,
    pub reserves: AmountDto,
    pub receivables: AmountDto,
    pub money_on_hand: AmountDto,
    pub cash_on_hand: AmountDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct CashOnHandDto {
    /// The day it is as of: the one asked for, or today.
    pub as_of: Option<String>,
    /// `summary` or `full`.
    pub detail: String,
    /// Money + funds + card reserves + receivables.
    pub money_on_hand: AmountDto,
    /// Funds + card reserves.
    pub future_allocations: AmountDto,
    pub receivables: AmountDto,
    /// Money on Hand - Future Allocations - Receivables: the free money.
    pub cash_on_hand: AmountDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<CashChangeDto>,
    pub by_kind: Vec<KindGroupDto>,
    /// Free money per home: each money account's exclusive balance.
    pub homes: Vec<HomeDto>,
    pub funds: Vec<FundDto>,
    pub cards: Vec<CardDto>,
    pub unreserved_count: usize,
    pub warnings: Vec<WarningDto>,
}

/// Cash on Hand's options.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CashOnHandQuery {
    /// `YYYY-MM-DD`; today if missing.
    #[serde(default)]
    pub as_of: Option<String>,
    /// `summary` (default) or `full`.
    #[serde(default)]
    pub detail: Option<String>,
    /// `full` only: flagged transactions listed per card (default 20).
    #[serde(default)]
    pub unreserved_limit: Option<usize>,
    /// A second day: adds the change in each component since then.
    #[serde(default)]
    pub compare_to: Option<String>,
}

/// Cash on Hand as of the end of `as_of` (every transaction
/// if `None`), counting every transaction.
pub async fn cash_on_hand(
    state: &Service,
    principal: &dyn Caller,
    q: &CashOnHandQuery,
) -> Result<CashOnHandDto, ApiError> {
    require(principal, Role::Read)?;
    // What can be spent right now: a future-dated transaction counts from
    // its day, not before.
    let as_of = parse_optional_date(q.as_of.as_deref())?
        .or_else(|| Some(chrono::Local::now().date_naive()));
    let compare_to = parse_optional_date(q.compare_to.as_deref())?;
    let full = match q.detail.as_deref() {
        None | Some("summary") => false,
        Some("full") => true,
        Some(other) => return Err(invalid("detail", format!("{other:?}: use summary or full"))),
    };
    let (chart, txs) = books_chart(state).await?;
    let commodity = books_commodity(state).await?;
    let report = kabacus_core::cash_on_hand(&chart, &txs, &commodity, as_of);
    let balance = |a: &kabacus_core::funds::AccountBalance| AccountBalanceDto {
        account: a.account.to_string(),
        display_name: a.display_name.clone(),
        balance: AmountDto::from(&a.balance),
    };
    let amount = |m: Money| AmountDto::from(&Amount::new(&commodity, m));
    let change = compare_to.map(|d| {
        let now = kabacus_core::reports::cash_totals(&chart, &txs, &commodity, as_of);
        let then = kabacus_core::reports::cash_totals(&chart, &txs, &commodity, Some(d));
        let c = now.change_since(&then);
        CashChangeDto {
            compare_to: d.format("%Y-%m-%d").to_string(),
            money: amount(c.money),
            funds: amount(c.funds),
            reserves: amount(c.reserves),
            receivables: amount(c.receivables),
            money_on_hand: amount(c.money_on_hand),
            cash_on_hand: amount(c.cash_on_hand),
        }
    });
    let limit = full.then(|| q.unreserved_limit.unwrap_or(20));
    let cards: Vec<CardDto> = report.cards.iter().map(|c| card_dto(c, limit)).collect();
    Ok(CashOnHandDto {
        as_of: report.as_of.map(|d| d.format("%Y-%m-%d").to_string()),
        detail: if full { "full" } else { "summary" }.to_string(),
        money_on_hand: AmountDto::from(&report.money_on_hand),
        future_allocations: AmountDto::from(&report.future_allocations),
        receivables: AmountDto::from(&report.receivables),
        cash_on_hand: AmountDto::from(&report.cash_on_hand),
        change,
        by_kind: report
            .by_kind
            .iter()
            .map(|g| KindGroupDto {
                kind: g.kind,
                total: AmountDto::from(&g.total),
                accounts: g.accounts.iter().map(balance).collect(),
            })
            .collect(),
        homes: report
            .homes
            .iter()
            .map(|h| HomeDto {
                account: h.account.to_string(),
                display_name: h.display_name.clone(),
                free: AmountDto::from(&h.free),
                funds: AmountDto::from(&h.funds),
                reserves: AmountDto::from(&h.reserves),
                inclusive: AmountDto::from(&h.inclusive),
            })
            .collect(),
        funds: report
            .funds
            .iter()
            .map(|f| FundDto {
                account: f.account.to_string(),
                display_name: f.display_name.clone(),
                home: f.home.to_string(),
                balance: AmountDto::from(&f.balance),
            })
            .collect(),
        unreserved_count: cards.iter().map(|c| c.unreserved_count).sum(),
        cards,
        warnings: all_warnings(state, &chart, &report.warnings).await?,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ReserveSourceDto {
    /// `free` (discretionary), `fund` (planned) or `unattributed`.
    pub source: String,
    /// The fund's account, for a fund.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fund: Option<String>,
    pub name: String,
    pub amount: AmountDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardReserveDto {
    #[serde(flatten)]
    pub card: CardDto,
    /// The card balance and reserve balance agree.
    pub matches: bool,
    /// When the reserve last reached zero; the split counts postings since.
    pub since: Option<String>,
    pub sources: Vec<ReserveSourceDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardReservesDto {
    /// How the split is worked out.
    pub rule: &'static str,
    pub cards: Vec<CardReserveDto>,
}

/// Each card (or only `card`) with its reserve split by source.
pub async fn card_reserves(
    state: &Service,
    principal: &dyn Caller,
    card: Option<&str>,
) -> Result<CardReservesDto, ApiError> {
    require(principal, Role::Read)?;
    let (chart, txs) = books_chart(state).await?;
    if let Some(card) = card {
        let kind = chart.kind_of(&AccountName::new(card));
        if kind != kabacus_core::AccountKind::RevolvingDebt {
            return Err(invalid(
                "card",
                format!("{card} is {kind}, not a card (revolving-debt)"),
            ));
        }
    }
    let commodity = books_commodity(state).await?;
    let cards = kabacus_core::card_reserves(&chart, &txs, &commodity, card);
    Ok(CardReservesDto {
        rule: kabacus_core::RESERVE_SPLIT_RULE,
        cards: cards
            .iter()
            .map(|c| CardReserveDto {
                card: card_dto(&c.card, None),
                matches: c.card.difference.money.is_zero(),
                since: c.since.map(|d| d.format("%Y-%m-%d").to_string()),
                sources: c
                    .sources
                    .iter()
                    .map(|(source, amount)| {
                        let (kind, fund, name) = match source {
                            kabacus_core::ReserveSource::Free => {
                                ("free", None, "Free money".to_string())
                            }
                            kabacus_core::ReserveSource::Fund(fund) => {
                                ("fund", Some(fund.to_string()), chart.display_name(fund))
                            }
                            kabacus_core::ReserveSource::Unattributed => {
                                ("unattributed", None, "Unattributed".to_string())
                            }
                        };
                        ReserveSourceDto {
                            source: kind.to_string(),
                            fund,
                            name,
                            amount: AmountDto::from(amount),
                        }
                    })
                    .collect(),
            })
            .collect(),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct AllocateBody {
    pub from: String,
    /// An account path, or a plain reserve name when `new_fund` is set.
    pub to: String,
    /// An exact decimal, e.g. `"400.00"`.
    pub amount: AmountInput,
    pub date: String,
    #[serde(default)]
    pub description: Option<String>,
    /// `to` is a new fund directly in the money account `from`: this
    /// write declares it.
    #[serde(default)]
    pub new_fund: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub include_diff: bool,
}

pub fn allocate_op(body: AllocateBody, commodity: &str) -> Result<ops::WriteOp, ApiError> {
    let to = if body.new_fund && !body.to.contains(':') {
        kabacus_core::new_fund_account(&body.from, &body.to)
            .map_err(|e| invalid("allocate", e.to_string()))?
    } else {
        body.to
    };
    Ok(ops::WriteOp::Allocate {
        amount: body.amount.to_money(commodity, "amount")?,
        date: parse_date(&body.date)?,
        from: body.from,
        to,
        description: body.description,
        new_fund: body.new_fund,
    })
}

/// Move money between a money account's free money and
/// its funds and card reserves, as one ordinary transaction.
pub async fn allocate(
    state: &Service,
    principal: &dyn Caller,
    body: AllocateBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    require(principal, Role::Write)?;
    let commodity = books_commodity(state).await?;
    let (dry_run, include_diff) = (body.dry_run, body.include_diff);
    let op = allocate_op(body, &commodity)?;
    run_write(state, principal, op, if_match, dry_run, include_diff).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct CardPurchaseBody {
    pub card: String,
    pub expense_account: String,
    /// An exact decimal; negative for a refund back to the same source.
    pub amount: AmountInput,
    pub date: String,
    pub description: String,
    /// `free`, or the fund paying for it.
    pub source: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub include_diff: bool,
}

pub fn card_purchase_op(body: CardPurchaseBody, commodity: &str) -> Result<ops::WriteOp, ApiError> {
    Ok(ops::WriteOp::CardPurchase {
        amount: body.amount.to_money(commodity, "amount")?,
        date: parse_date(&body.date)?,
        card: body.card,
        expense_account: body.expense_account,
        description: body.description,
        source: body.source,
        tags: body.tags,
    })
}

/// A purchase on a card with its reserve lines, as one
/// transaction.
pub async fn record_card_purchase(
    state: &Service,
    principal: &dyn Caller,
    body: CardPurchaseBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    require(principal, Role::Write)?;
    let commodity = books_commodity(state).await?;
    let (dry_run, include_diff) = (body.dry_run, body.include_diff);
    let op = card_purchase_op(body, &commodity)?;
    run_write(state, principal, op, if_match, dry_run, include_diff).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct CardPaymentBody {
    pub card: String,
    /// An exact decimal, e.g. `"425.00"`.
    pub amount: AmountInput,
    pub date: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub include_diff: bool,
}

pub fn card_payment_op(body: CardPaymentBody, commodity: &str) -> Result<ops::WriteOp, ApiError> {
    Ok(ops::WriteOp::PayCard {
        amount: body.amount.to_money(commodity, "amount")?,
        date: parse_date(&body.date)?,
        card: body.card,
    })
}

/// Pay a card from its reserve.
pub async fn pay_card(
    state: &Service,
    principal: &dyn Caller,
    body: CardPaymentBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    require(principal, Role::Write)?;
    let commodity = books_commodity(state).await?;
    let (dry_run, include_diff) = (body.dry_run, body.include_diff);
    let op = card_payment_op(body, &commodity)?;
    run_write(state, principal, op, if_match, dry_run, include_diff).await
}

// --- counterparties and statements ---------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CounterpartyDto {
    pub name: String,
    pub net: AmountDto,
    pub label: String,
}

/// Every counterparty's net position.
pub async fn counterparties(
    state: &Service,
    principal: &dyn Caller,
) -> Result<Vec<CounterpartyDto>, ApiError> {
    require(principal, Role::Read)?;
    let txs = ops::read_all_transactions(state).await?;
    let today = chrono::Utc::now().date_naive();
    let commodity = books_commodity(state).await?;
    let nets = kabacus_core::list_counterparties(&txs, &commodity, today);
    Ok(nets
        .into_iter()
        .map(|c| {
            let label = if c.net.money.is_negative() {
                format!(
                    "You owe {}",
                    format_money(&Amount::new(&c.net.commodity, -c.net.money.clone()))
                )
            } else {
                format!("Owes you {}", format_money(&c.net))
            };
            CounterpartyDto {
                name: c.name,
                net: AmountDto::from(&c.net),
                label,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StatementQuery {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub open_items: bool,
    /// `json` (default), `csv` or `pdf`.
    #[serde(default)]
    pub format: Option<String>,
    /// The PDF letterhead, which the app keeps in KConfig.
    #[serde(default)]
    pub sender_name: Option<String>,
    /// Address lines, separated by newlines.
    #[serde(default)]
    pub sender_address: Option<String>,
    /// Defaults to today.
    #[serde(default)]
    pub statement_date: Option<String>,
}

/// A netted statement for one counterparty.
pub async fn statement(
    state: &Service,
    principal: &dyn Caller,
    name: &str,
    q: &StatementQuery,
) -> Result<kabacus_core::Statement, ApiError> {
    require(principal, Role::Read)?;
    let from = parse_date(&q.from)?;
    let to = parse_date(&q.to)?;
    let txs = ops::read_all_transactions(state).await?;
    let commodity = books_commodity(state).await?;
    Ok(kabacus_core::build_statement(
        name,
        &txs,
        &commodity,
        from,
        to,
        q.open_items,
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct StatementLineDto {
    pub date: String,
    pub description: String,
    pub net_amount: AmountDto,
    pub running_balance: AmountDto,
    pub open: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatementDto {
    pub name: String,
    pub from: String,
    pub to: String,
    pub opening_balance: AmountDto,
    pub closing_balance: AmountDto,
    pub lines: Vec<StatementLineDto>,
}

pub fn statement_dto(stmt: kabacus_core::Statement) -> StatementDto {
    StatementDto {
        name: stmt.name,
        from: stmt.from.format("%Y-%m-%d").to_string(),
        to: stmt.to.format("%Y-%m-%d").to_string(),
        opening_balance: AmountDto::from(&stmt.opening_balance),
        closing_balance: AmountDto::from(&stmt.closing_balance),
        lines: stmt
            .lines
            .into_iter()
            .map(|l| StatementLineDto {
                date: l.date.format("%Y-%m-%d").to_string(),
                description: l.description,
                net_amount: AmountDto::from(&l.net_amount),
                running_balance: AmountDto::from(&l.running_balance),
                open: l.open,
            })
            .collect(),
    }
}

/// An exported statement, named `statement-<name>-<end-date>.<ext>`.
pub struct StatementFile {
    pub filename: String,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// Render `stmt` as `q.format` (`csv` or `pdf`).
pub fn statement_file(
    stmt: &kabacus_core::Statement,
    q: &StatementQuery,
) -> Result<StatementFile, ApiError> {
    let (extension, content_type, body) = match q.format.as_deref() {
        Some("csv") => (
            "csv",
            "text/csv; charset=utf-8",
            kabacus_core::statement_csv(stmt).into_bytes(),
        ),
        Some("pdf") => {
            let statement_date = match &q.statement_date {
                Some(s) => parse_date(s)?,
                None => chrono::Local::now().date_naive(),
            };
            let header = kabacus_core::StatementHeader {
                sender_name: q.sender_name.clone().unwrap_or_default(),
                sender_address: q
                    .sender_address
                    .as_deref()
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_string)
                    .collect(),
                statement_date,
            };
            (
                "pdf",
                "application/pdf",
                kabacus_core::statement_pdf(stmt, &header),
            )
        }
        other => {
            return Err(invalid(
                "format",
                format!("unknown statement format {other:?}; use json, csv or pdf"),
            ))
        }
    };
    Ok(StatementFile {
        filename: kabacus_core::statement_filename(&stmt.name, stmt.to, extension),
        content_type,
        body,
    })
}

// --- accounts and payees ------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AccountDto {
    pub account: String,
    /// From the declaration's `ynab-account` / `ynab-envelope` tag.
    pub friendly_name: Option<String>,
    /// The friendly name or readable leaf, with a kind qualifier
    /// where two accounts would otherwise share it.
    pub display_name: String,
    /// From the kind rules; read-only.
    pub kind: kabacus_core::AccountKind,
    pub synthetic: bool,
    pub physical_home: Option<String>,
    /// For a fund or card reserve, the money account holding it; empty
    /// otherwise.
    pub home: String,
    /// The account's own postings: the same as `balance_exclusive`.
    pub balances: Vec<AmountDto>,
    /// Without sub-accounts. For a money account, its free money.
    pub balance_exclusive: Vec<AmountDto>,
    /// With every sub-account. For a money account, what the bank shows.
    pub balance_inclusive: Vec<AmountDto>,
}

/// Every account with its balances. `filter` is the
/// account picker's query, matched fuzzily against paths and friendly names
/// and ranked best first. Without one, every account in path order.
pub async fn accounts(
    state: &Service,
    principal: &dyn Caller,
    filter: Option<&str>,
) -> Result<Vec<AccountDto>, ApiError> {
    require(principal, Role::Read)?;
    let declared = state
        .declared_accounts()
        .await
        .map_err(ApiError::Internal)?;
    let txs = ops::read_all_transactions(state).await?;

    let mut balances: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, kabacus_core::Money>,
    > = std::collections::BTreeMap::new();
    for account in &declared {
        balances.entry(account.as_str().to_string()).or_default();
    }
    for tx in &txs {
        for p in &tx.postings {
            let entry = balances.entry(p.account.as_str().to_string()).or_default();
            let commodity_total = entry
                .entry(p.amount.commodity.clone())
                .or_insert_with(kabacus_core::Money::zero);
            *commodity_total = commodity_total.clone() + p.amount.money.clone();
        }
    }

    let friendly = state
        .friendly_account_names()
        .await
        .map_err(ApiError::Internal)?;
    let table = state.kind_table();
    let types = state.account_types().await.map_err(ApiError::Internal)?;
    let chart = Chart::from_books(
        table.rules,
        table.reserve_template,
        declared,
        &txs,
        &friendly,
    )
    .with_account_types(&types);
    let amounts = |commodities: &std::collections::BTreeMap<String, Money>| -> Vec<AmountDto> {
        commodities
            .iter()
            .map(|(c, m)| AmountDto::from(&Amount::new(c, m.clone())))
            .collect()
    };
    let dtos: Vec<AccountDto> = balances
        .iter()
        .map(|(account, commodities)| {
            let name = AccountName::new(account);
            let mut inclusive: std::collections::BTreeMap<String, Money> =
                std::collections::BTreeMap::new();
            for (_, below) in balances
                .iter()
                .filter(|(a, _)| AccountName::new(a.as_str()).is_under(account))
            {
                for (c, m) in below {
                    let total = inclusive.entry(c.clone()).or_insert_with(Money::zero);
                    *total = total.clone() + m.clone();
                }
            }
            let classified = chart.get(account);
            AccountDto {
                friendly_name: friendly.get(account).cloned(),
                display_name: chart.display_name(&name),
                kind: chart.kind_of(&name),
                synthetic: classified.is_some_and(|c| c.synthetic),
                physical_home: chart.physical_home(account).map(|a| a.to_string()),
                home: classified
                    .and_then(|c| c.home.as_ref())
                    .map(|h| h.to_string())
                    .unwrap_or_default(),
                balances: amounts(commodities),
                balance_exclusive: amounts(commodities),
                balance_inclusive: amounts(&inclusive),
                account: account.clone(),
            }
        })
        .collect();

    Ok(match filter {
        None => dtos,
        Some(query) => {
            let entries: Vec<kabacus_core::PickerEntry> = dtos
                .iter()
                .map(|d| kabacus_core::PickerEntry {
                    name: &d.account,
                    friendly_name: d.friendly_name.as_deref(),
                })
                .collect();
            let order = kabacus_core::rank_accounts(query, &entries);
            let mut slots: Vec<Option<AccountDto>> = dtos.into_iter().map(Some).collect();
            order.into_iter().filter_map(|i| slots[i].take()).collect()
        }
    })
}

/// `list_accounts`' filters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountsQuery {
    #[serde(default)]
    pub filter: Option<String>,
    /// Only accounts of this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only accounts at most this many segments deep.
    #[serde(default)]
    pub depth: Option<usize>,
    /// `false` leaves out accounts whose balances are all zero.
    #[serde(default)]
    pub include_zero: Option<bool>,
}

/// [`accounts`], filtered.
pub async fn accounts_filtered(
    state: &Service,
    principal: &dyn Caller,
    q: &AccountsQuery,
) -> Result<Vec<AccountDto>, ApiError> {
    let kind: Option<kabacus_core::AccountKind> = q
        .kind
        .as_deref()
        .map(|k| {
            serde_json::from_value(serde_json::json!(k))
                .map_err(|_| invalid("kind", format!("{k:?} isn't an account kind")))
        })
        .transpose()?;
    let all = accounts(state, principal, q.filter.as_deref()).await?;
    let zero = |amounts: &[AmountDto]| {
        amounts
            .iter()
            .all(|a| a.exact.parse::<Money>().is_ok_and(|m| m.is_zero()))
    };
    Ok(all
        .into_iter()
        .filter(|a| kind.is_none_or(|k| a.kind == k))
        .filter(|a| crate::search::within_depth(&AccountName::new(&a.account), q.depth))
        .filter(|a| {
            q.include_zero.unwrap_or(true)
                || !(zero(&a.balance_exclusive) && zero(&a.balance_inclusive))
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct PayeeDto {
    pub payee: String,
    pub count: usize,
    /// What the editor pre-fills from when this payee is picked.
    pub last: TransactionDto,
}

/// Known payees, most recently used first, each with its last
/// transaction. `query` narrows them fuzzily.
pub async fn payees(
    state: &Service,
    principal: &dyn Caller,
    query: Option<&str>,
) -> Result<Vec<PayeeDto>, ApiError> {
    require(principal, Role::Read)?;
    let all = ops::read_all_transactions(state).await?;
    let flags = UnreservedIndex::new(&state.kind_rules_chart(), &all);
    let refs: Vec<&kabacus_core::Transaction> = all.iter().collect();
    let payees = kabacus_core::known_payees(&refs);
    let order: Vec<usize> = match query {
        None => (0..payees.len()).collect(),
        Some(query) => {
            let entries: Vec<kabacus_core::PickerEntry> = payees
                .iter()
                .map(|p| kabacus_core::PickerEntry {
                    name: p.payee,
                    friendly_name: None,
                })
                .collect();
            kabacus_core::rank_accounts(query, &entries)
        }
    };
    Ok(order
        .into_iter()
        .map(|i| PayeeDto {
            payee: payees[i].payee.to_string(),
            count: payees[i].count,
            last: to_dto(payees[i].last, flags.get(payees[i].last)),
        })
        .collect())
}

// --- register -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RegisterLineDto {
    pub id: u64,
    pub date: String,
    pub payee: String,
    pub memo: String,
    /// The other side of the transaction, or `"Split (n)"` for
    /// three or more postings.
    pub other_account: String,
    pub amount: AmountDto,
    pub balance: AmountDto,
    pub status: String,
    /// The transaction posts to a card without the matching
    /// reserve line.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unreserved: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_reserve: Vec<MissingReserveDto>,
}

/// One account's postings with a running balance.
pub async fn register(
    state: &Service,
    principal: &dyn Caller,
    account: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<RegisterLineDto>, ApiError> {
    require(principal, Role::Read)?;
    let from = parse_optional_date(from)?;
    let to = parse_optional_date(to)?;

    let account_name = AccountName::new(account);
    let all = ops::read_all_transactions(state).await?;
    let flags = UnreservedIndex::new(&state.kind_rules_chart(), &all);
    let mut touching: Vec<&kabacus_core::Transaction> = all
        .iter()
        .filter(|tx| tx.postings.iter().any(|p| p.account == account_name))
        .collect();
    touching.sort_by_key(|t| (t.date, t.id));

    let mut running: std::collections::HashMap<String, kabacus_core::Money> =
        std::collections::HashMap::new();
    let mut lines = Vec::new();
    for tx in touching {
        let this_posting = tx
            .postings
            .iter()
            .find(|p| p.account == account_name)
            .unwrap();
        let bal = running
            .entry(this_posting.amount.commodity.clone())
            .or_insert_with(kabacus_core::Money::zero);
        if from.is_some_and(|from| tx.date < from) {
            *bal = bal.clone() + this_posting.amount.money.clone();
            continue;
        }
        if to.is_some_and(|to| tx.date > to) {
            continue;
        }
        *bal = bal.clone() + this_posting.amount.money.clone();

        let others: Vec<&kabacus_core::Posting> = tx
            .postings
            .iter()
            .filter(|p| p.account != account_name)
            .collect();
        let other_account = match others.len() {
            0 => String::new(),
            1 => others[0].account.as_str().to_string(),
            n => format!("Split ({n})"),
        };

        let missing_reserve = missing_reserve_dtos(flags.get(tx));
        lines.push(RegisterLineDto {
            unreserved: !missing_reserve.is_empty(),
            missing_reserve,
            id: tx.id,
            date: tx.date.format("%Y-%m-%d").to_string(),
            payee: tx.payee().to_string(),
            memo: tx.memo().to_string(),
            other_account,
            amount: AmountDto::from(&this_posting.amount),
            balance: AmountDto::from(&Amount::new(
                this_posting.amount.commodity.clone(),
                bal.clone(),
            )),
            status: status_str(this_posting.status).to_string(),
        });
    }
    Ok(lines)
}

// --- rewrites: gated ------------------------------------------

/// Replace a transaction outright.
pub async fn rewrite_transaction(
    state: &Service,
    principal: &dyn Caller,
    id: &str,
    body: RewriteBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    check_rewrite_gates(state, principal)?;
    let commodity = books_commodity(state).await?;
    let request = edit_request(
        id,
        body.date,
        body.description,
        body.postings,
        Some(body.tags),
        &commodity,
        "a rewrite",
    )?;
    run_rewrite(
        state,
        principal,
        ops::WriteOp::Rewrite(request),
        if_match,
        body.dry_run,
    )
    .await
}

/// Remove a transaction from the books.
pub async fn delete_transaction(
    state: &Service,
    principal: &dyn Caller,
    id: &str,
    if_match: &str,
    dry_run: bool,
) -> Result<WriteResultDto, ApiError> {
    let op = ops::WriteOp::Delete {
        id: ops::parse_id(id)?,
    };
    run_rewrite(state, principal, op, if_match, dry_run).await
}

pub fn account_declaration_op(
    account: String,
    mut tags: Vec<kabacus_core::Tag>,
    synthetic: Option<bool>,
) -> Result<ops::WriteOp, ApiError> {
    let declaration = kabacus_core::AccountDeclaration {
        name: AccountName::new(&account),
        tags: tags.clone(),
    };
    let tagged = declaration
        .synthetic_override()
        .map_err(|e| invalid("synthetic", e))?;
    if let Some(synthetic) = synthetic {
        if tagged.is_some_and(|value| value != synthetic) {
            return Err(invalid(
                "synthetic",
                "synthetic conflicts with the declaration tag".to_string(),
            ));
        }
        if tagged.is_none() {
            tags.push(("synthetic".to_string(), synthetic.to_string()));
        }
    }
    Ok(ops::WriteOp::DeclareAccount { account, tags })
}

#[derive(Debug, Deserialize)]
pub struct DeclareAccountBody {
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub tags: Vec<kabacus_core::Tag>,
    #[serde(default)]
    pub synthetic: Option<bool>,
    #[serde(default)]
    pub dry_run: bool,
}

/// Declare a new account. A rewrite, so it's behind the rewrite gates.
pub async fn declare_account(
    state: &Service,
    principal: &dyn Caller,
    body: DeclareAccountBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    let op = account_declaration_op(body.account.unwrap_or_default(), body.tags, body.synthetic)?;
    run_rewrite(state, principal, op, if_match, body.dry_run).await
}

#[derive(Debug, Deserialize)]
pub struct SetAccountTypeBody {
    pub synthetic: bool,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn set_account_type(
    state: &Service,
    principal: &dyn Caller,
    account: String,
    body: SetAccountTypeBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    let op = ops::WriteOp::SetAccountType {
        account,
        synthetic: body.synthetic,
    };
    run_rewrite(state, principal, op, if_match, body.dry_run).await
}

/// What `PUT /assertions` takes: the assertion's place, as a failed
/// assertion's error names it (its transaction and account), and its new
/// balance (none removes it).
#[derive(Debug, Clone, Deserialize)]
pub struct BalanceAssertionBody {
    pub transaction: u64,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub balance: Option<AmountInput>,
    #[serde(default)]
    pub dry_run: bool,
}

pub fn balance_assertion_op(
    body: BalanceAssertionBody,
    commodity: &str,
) -> Result<ops::WriteOp, ApiError> {
    Ok(ops::WriteOp::SetBalanceAssertion {
        balance: body
            .balance
            .as_ref()
            .map(|b| b.to_amount(commodity, "balance"))
            .transpose()?,
        id: body.transaction,
        account: body.account,
    })
}

/// Remove or update one balance assertion, so a write it blocks
/// can be retried. Behind the rewrite gates.
pub async fn set_balance_assertion(
    state: &Service,
    principal: &dyn Caller,
    body: BalanceAssertionBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    check_rewrite_gates(state, principal)?;
    let commodity = books_commodity(state).await?;
    let dry_run = body.dry_run;
    let op = balance_assertion_op(body, &commodity)?;
    run_rewrite(state, principal, op, if_match, dry_run).await
}

/// Make every exclusive balance assertion on a money account
/// inclusive.
pub async fn convert_money_assertions(
    state: &Service,
    principal: &dyn Caller,
    if_match: &str,
    dry_run: bool,
) -> Result<WriteResultDto, ApiError> {
    run_rewrite(
        state,
        principal,
        ops::WriteOp::ConvertMoneyAssertions,
        if_match,
        dry_run,
    )
    .await
}

/// Rename an account everywhere.
#[derive(Debug, Clone, Deserialize)]
pub struct RenameAccountBody {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub merge: bool,
    #[serde(default)]
    pub allow_kind_change: bool,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn rename_account(
    state: &Service,
    principal: &dyn Caller,
    body: RenameAccountBody,
    if_match: &str,
) -> Result<WriteResultDto, ApiError> {
    let dry_run = body.dry_run;
    let op = ops::WriteOp::RenameAccount {
        from: body.from,
        to: body.to,
        merge: body.merge,
        allow_kind_change: body.allow_kind_change,
    };
    run_rewrite(state, principal, op, if_match, dry_run).await
}

/// Remove an account's declaration if nothing refers to it
///.
pub async fn remove_account(
    state: &Service,
    principal: &dyn Caller,
    account: &str,
    if_match: &str,
    dry_run: bool,
) -> Result<WriteResultDto, ApiError> {
    let op = ops::WriteOp::RemoveAccount {
        account: account.to_string(),
    };
    run_rewrite(state, principal, op, if_match, dry_run).await
}

#[cfg(test)]
mod amount_input_tests {
    use super::*;
    use serde_json::json;

    fn amount(value: serde_json::Value) -> Result<Amount, ApiError> {
        AmountInput(value).to_amount("USD", "amount")
    }

    #[test]
    fn an_exact_amount_of_any_size_round_trips_as_a_string_mantissa() {
        let digits = "7".repeat(60);
        let got =
            amount(json!({"commodity": "USD", "mantissa": format!("-{digits}"), "places": 255}))
                .unwrap();
        assert_eq!(got.money.places(), 255);
        assert_eq!(got.money.mantissa().to_string(), format!("-{digits}"));
        let dto = serde_json::to_value(AmountDto::from(&got)).unwrap();
        assert_eq!(dto["commodity"], "USD");
    }

    #[test]
    fn a_decimal_string_may_run_past_128_bits() {
        let text = format!("{}.{}", "9".repeat(50), "1".repeat(200));
        assert_eq!(amount(json!(text)).unwrap().money.to_string(), text);
    }

    #[test]
    fn more_than_255_places_is_refused() {
        let refused = amount(json!({"commodity": "USD", "mantissa": 1, "places": 256}));
        assert!(matches!(refused, Err(ApiError::AmountFormat { .. })));
        let text = format!("0.{}", "1".repeat(256));
        assert!(matches!(
            amount(json!(text)),
            Err(ApiError::AmountFormat { .. })
        ));
    }

    #[test]
    fn a_json_number_past_64_bits_is_refused_not_rounded() {
        let huge: serde_json::Value = serde_json::from_str(
            r#"{"commodity": "USD", "mantissa": 99999999999999999999, "places": 0}"#,
        )
        .unwrap();
        assert!(matches!(amount(huge), Err(ApiError::AmountFormat { .. })));
    }
}

#[cfg(test)]
mod allocation_name_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_reserves_accept_names_without_rewriting_explicit_paths() {
        for (typed, expected) in [
            (
                "  Emergency   Savings ",
                "assets:bank:checking:emergency-savings",
            ),
            (
                "assets:bank:checking:repairs",
                "assets:bank:checking:repairs",
            ),
        ] {
            let body = serde_json::from_value(json!({
                "from": "assets:bank:checking", "to": typed,
                "amount": "100.25", "date": "2026-10-25", "new_fund": true
            }))
            .unwrap();
            let ops::WriteOp::Allocate { to, .. } = allocate_op(body, "USD").unwrap() else {
                panic!("allocation expected")
            };
            assert_eq!(to, expected);
        }
    }
}

/// Sum an account or prefix over a period, separately for each commodity.
pub async fn get_balance(
    state: &Service,
    principal: &dyn Caller,
    account_prefix: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<AmountDto>, ApiError> {
    require(principal, Role::Read)?;
    let from = parse_optional_date(from)?;
    let to = parse_optional_date(to)?;
    let txs = ops::read_all_transactions(state).await?;
    let mut totals: std::collections::BTreeMap<String, kabacus_core::Money> =
        std::collections::BTreeMap::new();
    for tx in txs
        .iter()
        .filter(|tx| from.is_none_or(|d| tx.date >= d) && to.is_none_or(|d| tx.date <= d))
    {
        for posting in &tx.postings {
            if posting.account.is_under(account_prefix) {
                let entry = totals
                    .entry(posting.amount.commodity.clone())
                    .or_insert_with(kabacus_core::Money::zero);
                *entry = entry.clone() + posting.amount.money.clone();
            }
        }
    }
    let result: Vec<AmountDto> = totals
        .into_iter()
        .map(|(commodity, money)| (&kabacus_core::Amount::new(commodity, money)).into())
        .collect();
    Ok(result)
}
